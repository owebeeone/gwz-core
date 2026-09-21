use std::path::Path;

use crate::artifact::{self};
use crate::git::{GitBackend, MergeAuthorityBackend};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{EventEmitter, EventSink, NullSink, OpenMergeCommand, OperationRequest};

use super::super::pull_head_barrier::validate_pull_barrier;
use super::super::pull_head_merge_preflight::{apply_root_merge_pull, plan_root_merge_pull};
use super::super::pull_head_plan::{
    PullHeadPlan, apply_pull_action, pull_aggregate_status, pull_response_aggregate,
    pull_result_response,
};
use super::super::*;
use super::member_preflight::pull_fetch_remote_name;
use super::member_validation::pull_head_preflight;
use super::root_lock::{pull_root_remote_name, pull_workspace_root};

pub fn handle_pull_head<B>(
    backend: &B,
    start: &Path,
    request: crate::PullHeadRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::PullHeadResponse>
where
    B: GitBackend + MergeAuthorityBackend + Sync,
{
    handle_pull_head_with_events(backend, start, request, operation_id, &NullSink)
}

pub fn handle_pull_head_with_events<B>(
    backend: &B,
    start: &Path,
    request: crate::PullHeadRequest,
    operation_id: impl Into<String>,
    events: &dyn EventSink,
) -> ModelResult<crate::PullHeadResponse>
where
    B: GitBackend + MergeAuthorityBackend + Sync,
{
    let start = invocation_start(start, &request.meta)?;
    let operation_id = operation_id.into();
    backend.validate_transport_scope(&request.meta, &operation_id)?;
    let scoped_backend = backend.with_transport(&start, request.meta.transport.as_ref())?;
    let backend = scoped_backend.as_ref().unwrap_or(backend);
    let services = crate::operation_context::OperationServices::for_merge(backend);
    handle_pull_head_with_events_in(&services, backend, &start, request, operation_id, events)
}

pub(crate) fn handle_pull_head_with_events_in<B>(
    services: &crate::operation_context::OperationServices,
    backend: &B,
    start: &Path,
    request: crate::PullHeadRequest,
    operation_id: impl Into<String>,
    events: &dyn EventSink,
) -> ModelResult<crate::PullHeadResponse>
where
    B: GitBackend + Sync,
{
    let context = OperationRequest::PullHead(request.clone()).context(operation_id.into())?;
    let error_context = context.clone();
    let result: ModelResult<crate::PullHeadResponse> = (|| {
        let dry_run = request.meta.dry_run.unwrap_or(false);
        let (_guard, root) = guarded_workspace_root_for_request_in(
            services,
            start,
            &request.meta,
            OpenMergeCommand::Pull,
            dry_run,
        )?;
        assert_conf_unmodified_for(
            backend,
            &root,
            OpenMergeCommand::Pull,
            reconcile_authority(_guard.as_ref(), dry_run),
        )?;
        let manifest_for_selection = artifact::read_manifest(&root)?;
        assert_workspace_id(&manifest_for_selection, request.meta.workspace.as_ref())?;
        let lock_for_selection = artifact::read_lock(&root)?;
        let selected_for_root = resolve_action_targets(
            &manifest_for_selection,
            request.meta.selection.as_ref(),
            crate::ActionKind::PullHead,
        )?;
        let pull_root_selected = selected_for_root
            .iter()
            .any(|target| matches!(target, SelectedTarget::Root));
        let mut identity_targets = Vec::new();
        for target in &selected_for_root {
            let (path, remote) = match target {
                SelectedTarget::Root => (
                    root.clone(),
                    pull_root_remote_name(backend, &root, request.meta.policy.as_ref())?,
                ),
                SelectedTarget::Member(member) => (
                    root.join(&member.path),
                    pull_fetch_remote_name(member, request.meta.policy.as_ref()),
                ),
            };
            if let Some(remote) = remote {
                identity_targets.push((path, remote));
            }
        }
        backend.validate_transport_remotes(
            &identity_targets
                .iter()
                .map(|(_, remote)| remote.clone())
                .collect::<Vec<_>>(),
        )?;
        for (path, remote) in identity_targets {
            if backend.is_repository(&path)? {
                backend.validate_remote_identity(&path, &remote, false)?;
            }
        }
        let sync = request
            .meta
            .policy
            .as_ref()
            .and_then(|policy| policy.sync)
            .unwrap_or(crate::SyncBehavior::FfOnly);
        let root_merge_plan = (pull_root_selected && sync == crate::SyncBehavior::Merge)
            .then(|| {
                plan_root_merge_pull(
                    backend,
                    &root,
                    request.meta.policy.as_ref(),
                    manifest_for_selection.clone(),
                    lock_for_selection.clone(),
                )
            })
            .transpose()?;
        let mut root_changed = if dry_run || !pull_root_selected || root_merge_plan.is_some() {
            false
        } else {
            pull_workspace_root(backend, &root, request.meta.policy.as_ref())?
        };
        let manifest = root_merge_plan
            .as_ref()
            .map(|plan| plan.manifest.clone())
            .map(Ok)
            .unwrap_or_else(|| artifact::read_manifest(&root))?;
        assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
        let mut lock = root_merge_plan
            .as_ref()
            .map(|plan| plan.lock.clone())
            .map(Ok)
            .unwrap_or_else(|| artifact::read_lock(&root))?;
        let selected_targets = resolve_action_targets(
            &manifest,
            request.meta.selection.as_ref(),
            crate::ActionKind::PullHead,
        )?;
        let mut selected = Vec::new();
        for target in selected_targets {
            if let SelectedTarget::Member(member) = target {
                if !lock.members.contains_key(&member.id) {
                    return Err(ModelError::new(
                        ErrorCode::LockNotFound,
                        format!("lock record missing for member '{}'", member.id),
                    ));
                }
                selected.push(member.id.clone());
            }
        }
        if dry_run {
            let plans = pull_head_preflight(
                backend,
                &root,
                &manifest,
                &lock,
                &selected,
                request.meta.policy.as_ref(),
                None,
            )?;
            return Ok(crate::PullHeadResponse {
                response: response_envelope(
                    context,
                    pull_aggregate_status(&plans),
                    plans.iter().map(PullHeadPlan::planned_response).collect(),
                ),
            });
        }

        let progress_interval = request
            .meta
            .policy
            .as_ref()
            .and_then(|policy| policy.progress_min_interval_ms)
            .unwrap_or(0);
        let emitter = EventEmitter::new(&context, events, progress_interval);
        emitter.operation_started();
        let plans = pull_head_preflight(
            backend,
            &root,
            &manifest,
            &lock,
            &selected,
            request.meta.policy.as_ref(),
            Some(&emitter),
        )?;
        validate_pull_barrier(backend, &root, root_merge_plan.as_ref(), &plans)?;
        if let Some(plan) = &root_merge_plan {
            root_changed = apply_root_merge_pull(backend, &root, plan)?;
        }
        let mut responses = Vec::with_capacity(plans.len());
        for plan in plans {
            let member_root = root.join(&plan.state.path);
            let conflicts = apply_pull_action(backend, &member_root, &plan)
                .map_err(|error| error.with_member(&plan.member_id, &plan.state.path))?;
            let member = manifest
                .members
                .iter()
                .find(|member| member.id == plan.member_id)
                .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))?;
            let state = if backend.is_repository(&member_root)? {
                let head = backend.head(&member_root)?;
                let status = backend.status(&member_root)?;
                resolved_member(member, &head, &status)
            } else {
                plan.state.clone()
            };
            lock.members.insert(plan.member_id.clone(), state.clone());
            responses.push(pull_result_response(
                member,
                &state,
                &plan.action,
                &conflicts,
            ));
        }
        // CAPABILITY-FREE EXCEPTION, §10 rows `:278`/`:279`: `gwz pull` is under the mutation guard, so all three lock writers and both boundary writers here stay raw permanently (2026-09-02, GwzM5-8R2E-CapabilityFreeAmendment.md §3).
        artifact::write_lock(&root, &lock)?;
        sync_workspace_boundary(backend, &root, &manifest, &lock)?;
        emitter.operation_finished();

        Ok(crate::PullHeadResponse {
            response: response_envelope(
                context,
                pull_response_aggregate(&responses, root_changed),
                responses,
            ),
        })
    })();
    result
        .map_err(|error| {
            super::super::publication::attach_transport_error(backend, error, &error_context)
        })
        .map(|mut response| {
            super::super::publication::attach_transport(backend, &mut response.response);
            response
        })
}

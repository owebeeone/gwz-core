use std::collections::BTreeMap;
use std::path::Path;

use crate::artifact::{
    self, CreatedByArtifact, LockArtifact, ManifestArtifact, ResolvedMemberArtifact,
};
use crate::git::{GitBackend, MergeAuthorityBackend};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{EventSink, OpenMergeCommand, OperationRequest};

use super::super::*;
use super::capture::observed_member_map;
use super::materialize::handle_materialize;

pub fn handle_snapshot<B>(
    backend: &B,
    start: &Path,
    request: crate::SnapshotRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::SnapshotResponse>
where
    B: GitBackend + MergeAuthorityBackend,
{
    let start = invocation_start(start, &request.meta)?;
    let context = OperationRequest::Snapshot(request.clone()).context(operation_id.into())?;
    let services = crate::operation_context::OperationServices::for_merge(backend);
    let access = acquire_workspace_mutation_guard_for_request_in(
        &services,
        &start,
        &request.meta,
        OpenMergeCommand::Snapshot,
        request.meta.dry_run.unwrap_or(false),
    )?;
    let root = access.root().to_path_buf();
    // F13: reject a duplicate snapshot id up front, the same guard `tag` already has —
    // never silently overwrite an existing snapshot.
    if artifact::snapshot_path(&root, &request.snapshot_id)?.exists() {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            format!("snapshot '{}' already exists", request.snapshot_id),
        ));
    }
    assert_conf_unmodified_for(backend, &root, OpenMergeCommand::Snapshot, access.writes())?;
    let manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let lock = artifact::read_lock(&root)?;
    let selected = resolve_locked_action_selection(
        &manifest,
        &lock,
        request.meta.selection.as_ref(),
        crate::ActionKind::Snapshot,
    )?;
    let members = snapshot_member_map(
        backend,
        &root,
        &manifest,
        &lock,
        &selected,
        request.source.as_ref(),
    )?;
    // Everything above is validation and planning; a dry run stops before the write.
    if request.meta.dry_run.unwrap_or(false) {
        return Ok(crate::SnapshotResponse {
            response: response_envelope(context, crate::AggregateStatus::Ok, Vec::new()),
        });
    }
    artifact::write_snapshot(
        &root,
        &artifact::SnapshotArtifact {
            schema: artifact::SNAPSHOT_SCHEMA.to_owned(),
            workspace_id: manifest.workspace.id.clone(),
            snapshot_id: request.snapshot_id,
            created_at: now_marker(),
            created_by: created_by(&context),
            selected_members: selected.clone(),
            members: members.clone(),
        },
    )?;

    Ok(crate::SnapshotResponse {
        response: response_envelope(
            context,
            crate::AggregateStatus::Ok,
            locked_member_responses(&manifest, &members),
        ),
    })
}

fn snapshot_member_map<B: GitBackend>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    lock: &LockArtifact,
    selected: &[String],
    source: Option<&crate::SnapshotSource>,
) -> ModelResult<BTreeMap<String, ResolvedMemberArtifact>> {
    match source.map(|source| source.kind) {
        None => observed_member_map(backend, root, manifest, lock, selected),
        Some(crate::SnapshotSourceKind::Current) => {
            let members = observed_member_map(backend, root, manifest, lock, selected)?;
            validate_current_snapshot_source(&members)?;
            Ok(members)
        }
        Some(crate::SnapshotSourceKind::Branch) => {
            let branch = source
                .and_then(|source| source.branch.as_ref())
                .ok_or_else(|| invalid("branch snapshot source requires a branch name"))?;
            named_branch_snapshot_members(backend, root, manifest, lock, selected, branch)
        }
    }
}

fn named_branch_snapshot_members<B: GitBackend>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    lock: &LockArtifact,
    selected: &[String],
    branch: &str,
) -> ModelResult<BTreeMap<String, ResolvedMemberArtifact>> {
    let mut members = BTreeMap::new();
    let ref_name = format!("refs/heads/{branch}");
    for member_id in selected {
        let member = manifest
            .members
            .iter()
            .find(|member| &member.id == member_id)
            .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))?;
        let lock_state = lock.members.get(member_id).ok_or_else(|| {
            ModelError::new(
                ErrorCode::LockNotFound,
                format!("lock record missing for member '{member_id}'"),
            )
        })?;
        let member_root = root.join(&member.path);
        if !member_root.exists() || !backend.is_repository(&member_root)? {
            return Err(ModelError::new(
                ErrorCode::MemberNotFound,
                format!("member '{member_id}' is not materialized"),
            ));
        }
        let commit = backend.read_ref(&member_root, &ref_name)?.ok_or_else(|| {
            ModelError::new(
                ErrorCode::GitCommandFailed,
                format!("branch '{branch}' not found for member '{member_id}'"),
            )
        })?;
        members.insert(
            member_id.clone(),
            ResolvedMemberArtifact {
                path: lock_state.path.clone(),
                source_id: Some(member.source_id.clone()),
                source_kind: lock_state.source_kind,
                commit: Some(commit),
                branch: Some(branch.to_owned()),
                detached: Some(false),
                upstream: None,
                dirty: Some(false),
                materialized: Some(true),
            },
        );
    }
    Ok(members)
}

fn validate_current_snapshot_source(
    members: &BTreeMap<String, ResolvedMemberArtifact>,
) -> ModelResult<()> {
    let mut branch: Option<&str> = None;
    for (member_id, state) in members {
        if state.detached == Some(true) {
            return Err(ModelError::new(
                ErrorCode::BranchDetachedHead,
                format!("member '{member_id}' is detached"),
            ));
        }
        let Some(current) = state.branch.as_deref() else {
            return Err(ModelError::new(
                ErrorCode::BranchUnbornHead,
                format!("member '{member_id}' has no attached branch"),
            ));
        };
        if state.commit.is_none() {
            return Err(ModelError::new(
                ErrorCode::BranchUnbornHead,
                format!("member '{member_id}' has an unborn HEAD"),
            ));
        }
        if let Some(first) = branch {
            if first != current {
                return Err(ModelError::new(
                    ErrorCode::BranchMixed,
                    "selected members are on different branches",
                ));
            }
        } else {
            branch = Some(current);
        }
    }
    Ok(())
}

pub(crate) fn locked_member_responses(
    manifest: &ManifestArtifact,
    members: &BTreeMap<String, ResolvedMemberArtifact>,
) -> Vec<crate::MemberResponse> {
    members
        .iter()
        .map(|(member_id, state)| {
            let manifest_member = manifest
                .members
                .iter()
                .find(|member| &member.id == member_id);
            crate::MemberResponse {
                member_id: member_id.clone(),
                member_path: state.path.clone(),
                source_kind: crate::SourceKind::Git,
                status: crate::MemberStatus::Ok,
                error: None,
                planned: None,
                state: manifest_member.map(|member| protocol_state(member, state)),
                git_status: None,
                target_kind: Some(crate::TargetKind::Member),
                lock_match: Some(crate::LockMatch::Unknown),
                lock_difference_reasons: Some(vec![
                    crate::LockDifferenceReason::UnavailableObservations,
                ]),
                url_resolution: None,
            }
        })
        .collect()
}

pub(crate) fn created_by(context: &crate::operation::OperationContext) -> CreatedByArtifact {
    CreatedByArtifact {
        actor_id: context
            .attribution
            .as_ref()
            .and_then(|attribution| attribution.actor.as_ref())
            .map(|actor| actor.actor_id.clone())
            .unwrap_or_else(|| "unknown".to_owned()),
    }
}

pub fn handle_pull_snapshot<B>(
    backend: &B,
    start: &Path,
    request: crate::PullSnapshotRequest,
    operation_id: impl Into<String>,
    events: &dyn EventSink,
) -> ModelResult<crate::PullSnapshotResponse>
where
    B: GitBackend + MergeAuthorityBackend + Sync,
{
    let start = invocation_start(start, &request.meta)?;
    let context = OperationRequest::PullSnapshot(request.clone()).context(operation_id.into())?;
    let scoped_backend = backend.with_transport(&start, request.meta.transport.as_ref())?;
    let backend = scoped_backend.as_ref().unwrap_or(backend);
    let error_context = context.clone();
    let result: ModelResult<crate::PullSnapshotResponse> = (|| {
        let materialize = crate::MaterializeRequest {
            meta: request.meta,
            target: crate::MaterializeTarget {
                kind: crate::MaterializeTargetKind::Snapshot,
                name: Some(request.snapshot_id),
                commit: None,
            },
        };
        let mut response = handle_materialize(
            backend,
            &start,
            materialize,
            context.operation_id.clone(),
            events,
        )?
        .response;
        response.meta = crate::ResponseMeta {
            transport: response.meta.transport,
            request_id: context.request_id,
            schema_version: context.schema_version,
            action: context.action.into(),
            aggregate_status: response.meta.aggregate_status,
            operation_id: Some(context.operation_id),
            message: response.meta.message,
            attribution: context.attribution.as_ref().map(Into::into),
        };
        Ok(crate::PullSnapshotResponse { response })
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

use std::path::{Path, PathBuf};

use crate::artifact::{self, LockArtifact, ManifestArtifact, ResolvedMemberArtifact};
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};

use super::super::*;
use super::apply::materialized_response;

pub(super) fn handle_materialize_branch<B>(
    backend: &B,
    root: PathBuf,
    manifest: ManifestArtifact,
    request: crate::MaterializeRequest,
    context: crate::operation::OperationContext,
) -> ModelResult<crate::MaterializeResponse>
where
    B: GitBackend,
{
    let branch = request
        .target
        .name
        .as_ref()
        .ok_or_else(|| invalid("branch target requires a name"))?;
    let lock = artifact::read_lock(&root)?;
    let selected = resolve_locked_action_selection(
        &manifest,
        &lock,
        request.meta.selection.as_ref(),
        crate::ActionKind::Materialize,
    )?;
    let plans = branch_switch_preflight(backend, &root, &manifest, &lock, &selected, branch)?;
    if request.meta.dry_run.unwrap_or(false) {
        return Ok(crate::MaterializeResponse {
            response: response_envelope(context, crate::AggregateStatus::Accepted, plans),
        });
    }
    let mut observed_states = Vec::with_capacity(selected.len());
    let mut responses = Vec::with_capacity(selected.len());
    for member_id in &selected {
        let member = manifest
            .members
            .iter()
            .find(|member| &member.id == member_id)
            .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))?;
        let member_root = root.join(&member.path);
        backend.switch_branch(&member_root, branch)?;
        let head = backend.head(&member_root)?;
        let status = backend.status(&member_root)?;
        let observed = resolved_member(member, &head, &status);
        responses.push(materialized_response(member, &observed, &observed, None));
        observed_states.push((member_id.clone(), observed));
    }

    let mut next = read_lock_or_empty(&root, &manifest.workspace.id)?;
    for (member_id, observed) in &observed_states {
        next.members.insert(member_id.clone(), observed.clone());
    }
    artifact::write_lock(&root, &next)?;
    sync_workspace_boundary(backend, &root, &manifest, &next)?;

    Ok(crate::MaterializeResponse {
        response: response_envelope(context, crate::AggregateStatus::Ok, responses),
    })
}

fn branch_switch_preflight<B: GitBackend>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    lock: &LockArtifact,
    selected: &[String],
    branch: &str,
) -> ModelResult<Vec<crate::MemberResponse>> {
    let mut plans = Vec::with_capacity(selected.len());
    let ref_name = format!("refs/heads/{branch}");
    for member_id in selected {
        let member = manifest
            .members
            .iter()
            .find(|member| &member.id == member_id)
            .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))?;
        let state = lock.members.get(member_id).ok_or_else(|| {
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
        let commit = backend
            .read_ref(&member_root, &ref_name)
            .map_err(|error| error.with_member(member_id, &state.path))?
            .ok_or_else(|| {
                ModelError::new(
                    ErrorCode::GitCommandFailed,
                    format!("branch '{branch}' not found"),
                )
                .with_member(member_id, &state.path)
            })?;
        let status = backend
            .status(&member_root)
            .map_err(|error| error.with_member(member_id, &state.path))?;
        preflight_branch_switch(
            backend,
            &member_root,
            member_id,
            &state.path,
            &commit,
            &status,
        )?;
        plans.push(crate::MemberResponse {
            member_id: member_id.clone(),
            member_path: state.path.clone(),
            source_kind: crate::SourceKind::Git,
            status: crate::MemberStatus::Planned,
            error: None,
            planned: Some(crate::PlannedChange {
                action: crate::PlannedAction::Checkout,
                from_ref: state.branch.clone(),
                to_ref: Some(branch.to_owned()),
                message: None,
            }),
            state: Some(protocol_state(
                member,
                &ResolvedMemberArtifact {
                    commit: Some(commit),
                    branch: Some(branch.to_owned()),
                    detached: Some(false),
                    dirty: Some(status.is_dirty),
                    materialized: Some(true),
                    ..state.clone()
                },
            )),
            git_status: None,
            target_kind: Some(crate::TargetKind::Member),
            lock_match: Some(crate::LockMatch::Differs),
            lock_difference_reasons: None,
            url_resolution: None,
        });
    }
    Ok(plans)
}

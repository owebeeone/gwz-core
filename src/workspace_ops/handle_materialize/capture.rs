use std::collections::BTreeMap;
use std::path::Path;

use crate::artifact::{self, LockArtifact, ManifestArtifact, ResolvedMemberArtifact};
use crate::git::{GitBackend, MergeAuthorityBackend};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{OpenMergeCommand, OperationRequest};

use super::super::*;
use super::snapshot::locked_member_responses;

/// Capture the live observed member state into the **lock** — no worktree mutation
/// (AD3 capture direction: "record where I am now"). Each materialized member's
/// observed head/status is written; unmaterialized members carry their lock state.
pub fn handle_capture<B>(
    backend: &B,
    start: &Path,
    request: crate::CaptureRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::CaptureResponse>
where
    B: GitBackend + MergeAuthorityBackend,
{
    let start = invocation_start(start, &request.meta)?;
    let context = OperationRequest::Capture(request.clone()).context(operation_id.into())?;
    let services = crate::operation_context::OperationServices::for_merge(backend);
    let access = acquire_workspace_mutation_guard_for_request_in(
        &services,
        &start,
        &request.meta,
        OpenMergeCommand::Capture,
        request.meta.dry_run.unwrap_or(false),
    )?;
    let root = access.root().to_path_buf();
    assert_conf_unmodified_for(backend, &root, OpenMergeCommand::Capture, access.writes())?;
    let manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let lock = artifact::read_lock(&root)?;
    let selected = resolve_locked_action_selection(
        &manifest,
        &lock,
        request.meta.selection.as_ref(),
        crate::ActionKind::Capture,
    )?;
    let members = observed_member_map(backend, &root, &manifest, &lock, &selected)?;
    // Everything above is observation; a dry run stops before the lock rewrite.
    if request.meta.dry_run.unwrap_or(false) {
        return Ok(crate::CaptureResponse {
            response: response_envelope(context, crate::AggregateStatus::Ok, Vec::new()),
        });
    }
    let mut next = read_lock_or_empty(&root, &manifest.workspace.id)?;
    for (member_id, state) in &members {
        next.members.insert(member_id.clone(), state.clone());
    }
    // CAPABILITY-FREE EXCEPTION, §10 rows `:278`/`:279`: `gwz materialize` is under the mutation guard, so all three writer pairs in this file stay raw permanently (2026-09-02, GwzM5-8R2E-CapabilityFreeAmendment.md §3).
    artifact::write_lock(&root, &next)?;
    sync_workspace_boundary(backend, &root, &manifest, &next)?;
    Ok(crate::CaptureResponse {
        response: response_envelope(
            context,
            crate::AggregateStatus::Ok,
            locked_member_responses(&manifest, &members),
        ),
    })
}

pub(crate) fn observed_member_map<B: GitBackend>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    lock: &LockArtifact,
    selected: &[String],
) -> ModelResult<BTreeMap<String, ResolvedMemberArtifact>> {
    // F3 + AD3: capture each member's LIVE observed state (head/status). A member that
    // isn't materialized can't be observed, so carry its existing lock state (AD3 b) —
    // the capture/snapshot stays complete and restorable rather than failing. Dirty
    // state is recorded honestly, never rejected (AD3 a).
    let mut members = BTreeMap::new();
    for member_id in selected {
        let member = manifest
            .members
            .iter()
            .find(|member| &member.id == member_id)
            .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))?;
        let member_root = root.join(&member.path);
        if member_root.exists() && backend.is_repository(&member_root)? {
            let head = backend.head(&member_root)?;
            let status = backend.status(&member_root)?;
            members.insert(member_id.clone(), resolved_member(member, &head, &status));
        } else if let Some(state) = lock.members.get(member_id) {
            members.insert(member_id.clone(), state.clone());
        } else {
            return Err(ModelError::new(
                ErrorCode::MemberNotFound,
                format!("member '{member_id}' is not materialized and has no lock state"),
            ));
        }
    }
    Ok(members)
}

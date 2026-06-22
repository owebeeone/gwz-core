use std::path::Path;

use crate::artifact;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::OperationRequest;

use super::*;

/// Fan out `git commit` across selected members and the root (root last) — the multi-repo
/// commit verb. Members with staged changes (or, with `all`, tracked modifications too)
/// are committed; a member with nothing to commit is skipped (never an empty commit). The
/// member commits update gwz's lock (new member HEADs); that lock update is then committed
/// into the root last, so the root records the post-commit composition. Members are hidden
/// via `.git/info/exclude`, not tracked. Nothing commit-able anywhere is a success no-op.
pub fn handle_commit<B>(
    backend: &B,
    start: &Path,
    request: crate::CommitRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::CommitResponse>
where
    B: GitBackend,
{
    let context = OperationRequest::Commit(request.clone()).context(operation_id.into())?;
    let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
    let manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let lock = artifact::read_lock(&root)?;
    let selected = resolve_locked_selection(&manifest, &lock, request.meta.selection.as_ref())?;
    let all = request.all.unwrap_or(false);

    // Validate (non-mutating): every selected member must be materialized before any commit.
    for member_id in &selected {
        let member = manifest
            .members
            .iter()
            .find(|member| &member.id == member_id)
            .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))?;
        if !backend.is_repository(&root.join(&member.path))? {
            return Err(ModelError::new(
                ErrorCode::MemberNotFound,
                format!("member '{member_id}' is not materialized; cannot commit"),
            ));
        }
    }

    // Commit members first; skip any with nothing to commit (never an empty commit).
    let mut committed_any = false;
    for member_id in &selected {
        let member = manifest
            .members
            .iter()
            .find(|member| &member.id == member_id)
            .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))?;
        let member_root = root.join(&member.path);
        let status = backend.status(&member_root)?;
        let has_changes = if all {
            status.staged > 0 || status.unstaged > 0
        } else {
            status.staged > 0
        };
        if has_changes {
            backend.commit(&member_root, &request.message, all)?;
            committed_any = true;
        }
    }

    // C5: nothing was commit-able anywhere — success, no mutation.
    if !committed_any {
        return Ok(crate::CommitResponse {
            response: response_envelope(context, crate::AggregateStatus::Ok, Vec::new()),
        });
    }

    // Observe → re-lock from the post-commit member HEADs (the capture machinery).
    let members = observed_member_map(backend, &root, &manifest, &lock, &selected)?;
    let mut next = read_lock_or_empty(&root, &manifest.workspace.id)?;
    for (member_id, state) in &members {
        next.members.insert(member_id.clone(), state.clone());
    }
    next.created_at = now_marker();
    artifact::write_lock(&root, &next)?;

    // Refresh the boundary excludes + stage gwz.conf, then commit the root LAST so the
    // lock update (the post-commit member HEADs) lands in one root commit.
    sync_workspace_boundary(backend, &root, &next)?;
    if backend.is_repository(&root)? && backend.status(&root)?.staged > 0 {
        backend.commit(&root, &request.message, false)?;
    }

    Ok(crate::CommitResponse {
        response: response_envelope(
            context,
            crate::AggregateStatus::Ok,
            locked_member_responses(&manifest, &members),
        ),
    })
}

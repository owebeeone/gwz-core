use std::path::Path;

use crate::artifact::{self, LockArtifact, ManifestArtifact};
use crate::git::{GitBackend, GitPullUpdate};
use crate::model::{ErrorCode, ModelError, ModelResult};

use super::super::*;

/// The ff-only / driver-selected repair of a diverged root whose remote changed only the
/// lock: merge, then record the members' live state in the lock (resolving a lock-only
/// conflict the same way). Untracked root files are tolerated, as for every root update.
pub(crate) fn apply_root_repair_merge<B>(
    backend: &B,
    root: &Path,
    branch: &str,
    local_commit: &str,
    remote_commit: &str,
    remote_ref: &str,
    fallback_lock: &LockArtifact,
) -> ModelResult<LockArtifact>
where
    B: GitBackend + GitPullUpdate,
{
    let prepared =
        backend.prepare_pull_update_checked(root, branch, local_commit, remote_commit)?;
    let result = backend.execute_prepared_pull_update(
        root,
        branch,
        local_commit,
        remote_commit,
        &format!("Merge {remote_ref} into {branch}"),
        &prepared,
    )?;
    if result.conflicts.is_empty() {
        return rewrite_root_lock_from_live_members(backend, root, fallback_lock);
    }
    resolve_repairable_root_conflicts(
        backend,
        root,
        branch,
        remote_ref,
        fallback_lock,
        &result.conflicts,
    )?;
    artifact::read_lock(root)
}

pub(crate) fn root_remote_changes_are_auto_repairable<B>(
    backend: &B,
    root: &Path,
    local_commit: &str,
    remote_commit: &str,
) -> ModelResult<bool>
where
    B: GitBackend,
{
    let Some(base) = backend.merge_base(root, local_commit, remote_commit)? else {
        return Ok(false);
    };
    let remote_paths = backend.changed_paths_between(root, &base, remote_commit)?;
    Ok(!remote_paths.is_empty() && remote_paths.iter().all(|path| path == artifact::LOCK_PATH))
}

pub(crate) fn pull_root_remote_name<B>(
    backend: &B,
    root: &Path,
    policy: Option<&crate::OperationPolicy>,
) -> ModelResult<Option<String>>
where
    B: GitBackend,
{
    if let Some(remote) = policy.and_then(|policy| policy.remote.clone()) {
        return Ok(Some(remote));
    }
    let remotes = backend.remotes(root)?;
    Ok(remotes
        .iter()
        .find(|remote| remote.name == "origin")
        .or_else(|| remotes.first())
        .map(|remote| remote.name.clone()))
}

fn resolve_repairable_root_conflicts<B>(
    backend: &B,
    root: &Path,
    branch: &str,
    remote_ref: &str,
    fallback_lock: &LockArtifact,
    conflicts: &[String],
) -> ModelResult<()>
where
    B: GitBackend,
{
    let supported = conflicts
        .iter()
        .all(|path| path == artifact::LOCK_PATH || path == ".gitignore");
    if !supported {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            format!(
                "workspace root merge left non-GWZ conflicted paths: {}",
                conflicts.join(", ")
            ),
        ));
    }

    if conflicts.iter().any(|path| path == artifact::LOCK_PATH) {
        rewrite_root_lock_from_live_members_allowing_other_conflicts(backend, root, fallback_lock)?;
    }
    let remaining = conflicts
        .iter()
        .filter(|path| path.as_str() != artifact::LOCK_PATH)
        .cloned()
        .collect::<Vec<_>>();
    if !remaining.is_empty() {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            format!(
                "workspace root merge left user-resolved conflicted paths: {}",
                remaining.join(", ")
            ),
        ));
    }

    backend.commit_merge_resolution(root, &format!("Merge {remote_ref} into {branch}"))?;
    Ok(())
}

/// The lock that records every active member's live state (head, status) as pull
/// records it (`pull_member_state`, against the `fallback` record), carrying the
/// `fallback` state of a member that is not materialized. Reads only; nothing is written.
pub(crate) fn live_lock<B>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    fallback: &LockArtifact,
) -> ModelResult<LockArtifact>
where
    B: GitBackend,
{
    let mut members = std::collections::BTreeMap::new();
    for member in manifest.members.iter().filter(|member| member.active) {
        let member_root = root.join(&member.path);
        let recorded = fallback.members.get(&member.id);
        if member_root.exists() && backend.is_repository(&member_root)? {
            let head = backend.head(&member_root)?;
            let status = backend.status(&member_root)?;
            members.insert(
                member.id.clone(),
                pull_member_state(member, &head, &status, recorded),
            );
        } else if let Some(state) = recorded {
            members.insert(member.id.clone(), state.clone());
        } else {
            return Err(ModelError::new(
                ErrorCode::MemberNotFound,
                format!(
                    "member '{}' is not materialized and has no lock state",
                    member.id
                ),
            ));
        }
    }
    Ok(LockArtifact {
        schema: artifact::LOCK_SCHEMA.to_owned(),
        workspace_id: manifest.workspace.id.clone(),
        manifest_schema: artifact::WORKSPACE_SCHEMA.to_owned(),
        members,
    })
}

/// A member's live lock state as pull records it: `resolved_member`, except that dirt
/// made only of untracked files keeps the `dirty` flag `recorded` already holds. Pull
/// integrates past such files, as git does, so they must not rewrite the lock either:
/// a rewrite would stage `gwz.conf` changes the next pull refuses on the root.
pub(crate) fn pull_member_state(
    member: &crate::artifact::ManifestMember,
    head: &crate::git::GitHeadState,
    status: &crate::git::GitStatus,
    recorded: Option<&crate::artifact::ResolvedMemberArtifact>,
) -> crate::artifact::ResolvedMemberArtifact {
    let mut state = resolved_member(member, head, status);
    if status.is_dirty
        && !status.has_tracked_changes()
        && let Some(dirty) = recorded.and_then(|recorded| recorded.dirty)
    {
        state.dirty = Some(dirty);
    }
    state
}

fn rewrite_root_lock_from_live_members_allowing_other_conflicts<B>(
    backend: &B,
    root: &Path,
    fallback_lock: &LockArtifact,
) -> ModelResult<LockArtifact>
where
    B: GitBackend,
{
    let manifest = artifact::read_manifest(root)?;
    let lock_fallback = artifact::read_lock(root).unwrap_or_else(|_| fallback_lock.clone());
    let lock = live_lock(backend, root, &manifest, &lock_fallback)?;
    artifact::write_lock(root, &lock)?;
    // `write_lock` re-records the conf-integrity marker, so the marker moves with the lock
    // here too. This conflicted-pull path stages explicitly instead of going through
    // `sync_workspace_boundary`, so it has to name the marker itself or leave it behind.
    backend.stage_paths_allowing_other_conflicts(
        root,
        &[artifact::LOCK_PATH, artifact::CONF_INTEGRITY_MARKER_PATH],
    )?;
    Ok(lock)
}

pub(crate) fn rewrite_root_lock_from_live_members<B>(
    backend: &B,
    root: &Path,
    fallback_lock: &LockArtifact,
) -> ModelResult<LockArtifact>
where
    B: GitBackend,
{
    let manifest = artifact::read_manifest(root)?;
    let lock_fallback = artifact::read_lock(root).unwrap_or_else(|_| fallback_lock.clone());
    let lock = live_lock(backend, root, &manifest, &lock_fallback)?;
    artifact::write_lock(root, &lock)?;
    sync_workspace_boundary(backend, root, &manifest, &lock)?;
    Ok(lock)
}

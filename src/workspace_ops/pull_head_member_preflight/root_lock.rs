use std::path::Path;

use crate::artifact::{self, LockArtifact};
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};

use super::super::*;
use super::member_validation::pull_dirty_guard;

pub(super) fn pull_workspace_root<B>(
    backend: &B,
    root: &Path,
    policy: Option<&crate::OperationPolicy>,
) -> ModelResult<bool>
where
    B: GitBackend,
{
    if !backend.is_repository(root)? {
        return Ok(false);
    }
    let Some(remote) = pull_root_remote_name(backend, root, policy)? else {
        return Ok(false);
    };
    let head = backend.head(root)?;
    if head.is_detached {
        return Err(ModelError::new(
            ErrorCode::BranchDetachedHead,
            "workspace root is detached; root pull requires an attached branch",
        ));
    }
    let branch = head.branch.clone().ok_or_else(|| {
        ModelError::new(
            ErrorCode::BranchUnbornHead,
            "workspace root has no current branch",
        )
    })?;
    let local_commit = head.commit.clone().ok_or_else(|| {
        ModelError::new(
            ErrorCode::BranchUnbornHead,
            "workspace root has an unborn HEAD",
        )
    })?;
    let sync = policy
        .and_then(|policy| policy.sync)
        .unwrap_or(crate::SyncBehavior::FfOnly);
    let status = backend.status(root)?;
    pull_dirty_guard(sync, &status, policy, "workspace root")?;

    let manifest_before = artifact::read_manifest(root)?;
    let fallback_lock = read_lock_or_empty(root, &manifest_before.workspace.id)?;

    backend.fetch(root, &remote)?;
    let remote_ref = format!("refs/remotes/{remote}/{branch}");
    let remote_commit = backend
        .read_ref(root, &remote_ref)?
        .ok_or_else(|| ModelError::new(ErrorCode::MissingRemote, "root remote branch not found"))?;
    if local_commit == remote_commit {
        return Ok(false);
    }
    let behind = backend.is_ancestor(root, &local_commit, &remote_commit)?;
    if !behind
        && !matches!(sync, crate::SyncBehavior::Reset)
        && backend.is_ancestor(root, &remote_commit, &local_commit)?
    {
        // Strictly ahead of the remote: integrating an ancestor is a no-op
        // under every non-destructive sync mode — up to date, not divergence.
        return Ok(false);
    }
    match sync {
        crate::SyncBehavior::FetchOnly => Ok(false),
        crate::SyncBehavior::FfOnly | crate::SyncBehavior::DriverSelected => {
            if !behind {
                if root_remote_changes_are_auto_repairable(
                    backend,
                    root,
                    &local_commit,
                    &remote_commit,
                )? {
                    let result = backend.merge_upstream_checked(
                        root,
                        &branch,
                        &local_commit,
                        &remote_commit,
                        &format!("Merge {remote_ref} into {branch}"),
                        None,
                    )?;
                    if result.conflicts.is_empty() {
                        rewrite_root_lock_from_live_members(backend, root, &fallback_lock)?;
                        return Ok(true);
                    }
                    resolve_repairable_root_conflicts(
                        backend,
                        root,
                        &branch,
                        &remote_ref,
                        &fallback_lock,
                        &result.conflicts,
                    )?;
                    return Ok(true);
                } else {
                    return Err(ModelError::new(
                        ErrorCode::DivergedMember,
                        "workspace root has diverged from remote; rerun with --sync merge, rebase, or reset",
                    ));
                }
            }
            backend.fast_forward(root, &branch, &remote_commit)?;
            rewrite_root_lock_from_live_members(backend, root, &fallback_lock)?;
            Ok(true)
        }
        crate::SyncBehavior::Merge => {
            if behind {
                backend.fast_forward(root, &branch, &remote_commit)?;
                rewrite_root_lock_from_live_members(backend, root, &fallback_lock)?;
                return Ok(true);
            }
            let result = backend.merge_upstream_checked(
                root,
                &branch,
                &local_commit,
                &remote_commit,
                &format!("Merge {remote_ref} into {branch}"),
                None,
            )?;
            if result.conflicts.is_empty() {
                rewrite_root_lock_from_live_members(backend, root, &fallback_lock)?;
                return Ok(true);
            }
            resolve_repairable_root_conflicts(
                backend,
                root,
                &branch,
                &remote_ref,
                &fallback_lock,
                &result.conflicts,
            )?;
            Ok(true)
        }
        crate::SyncBehavior::Rebase => {
            if behind {
                backend.fast_forward(root, &branch, &remote_commit)?;
            } else {
                let result = backend.rebase_onto(root, &branch, &remote_commit)?;
                if !result.conflicts.is_empty() {
                    return Err(ModelError::new(
                        ErrorCode::GitCommandFailed,
                        format!(
                            "workspace root rebase left conflicted paths: {}",
                            result.conflicts.join(", ")
                        ),
                    ));
                }
            }
            rewrite_root_lock_from_live_members(backend, root, &fallback_lock)?;
            Ok(true)
        }
        crate::SyncBehavior::Reset => {
            backend.reset_hard(root, &branch, &remote_commit)?;
            rewrite_root_lock_from_live_members(backend, root, &fallback_lock)?;
            Ok(true)
        }
    }
}

fn root_remote_changes_are_auto_repairable<B>(
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

pub(super) fn pull_root_remote_name<B>(
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

fn rewrite_root_lock_from_live_members_allowing_other_conflicts<B>(
    backend: &B,
    root: &Path,
    fallback_lock: &LockArtifact,
) -> ModelResult<LockArtifact>
where
    B: GitBackend,
{
    let manifest = artifact::read_manifest(root)?;
    let selected = manifest
        .members
        .iter()
        .filter(|member| member.active)
        .map(|member| member.id.clone())
        .collect::<Vec<_>>();
    let lock_fallback = artifact::read_lock(root).unwrap_or_else(|_| fallback_lock.clone());
    let members = observed_member_map(backend, root, &manifest, &lock_fallback, &selected)?;
    let lock = LockArtifact {
        schema: artifact::LOCK_SCHEMA.to_owned(),
        workspace_id: manifest.workspace.id.clone(),
        manifest_schema: artifact::WORKSPACE_SCHEMA.to_owned(),
        members,
    };
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

fn rewrite_root_lock_from_live_members<B>(
    backend: &B,
    root: &Path,
    fallback_lock: &LockArtifact,
) -> ModelResult<LockArtifact>
where
    B: GitBackend,
{
    let manifest = artifact::read_manifest(root)?;
    let selected = manifest
        .members
        .iter()
        .filter(|member| member.active)
        .map(|member| member.id.clone())
        .collect::<Vec<_>>();
    let lock_fallback = artifact::read_lock(root).unwrap_or_else(|_| fallback_lock.clone());
    let members = observed_member_map(backend, root, &manifest, &lock_fallback, &selected)?;
    let lock = LockArtifact {
        schema: artifact::LOCK_SCHEMA.to_owned(),
        workspace_id: manifest.workspace.id.clone(),
        manifest_schema: artifact::WORKSPACE_SCHEMA.to_owned(),
        members,
    };
    artifact::write_lock(root, &lock)?;
    sync_workspace_boundary(backend, root, &manifest, &lock)?;
    Ok(lock)
}

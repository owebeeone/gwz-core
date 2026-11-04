//! The root's part of `gwz pull`, in every sync mode: a plan made without mutating
//! anything but fetched objects and remote-tracking refs (B5), validated again at the
//! final barrier, and applied only after every selected member has passed its checks.
use std::path::Path;

use crate::artifact::{self, LockArtifact, ManifestArtifact};
use crate::git::{GitBackend, GitMergeSimulation, GitPreparedMerge, GitPullUpdate};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace::WORKSPACE_MANIFEST;

use super::pull_head_member_preflight::root_lock::{
    apply_root_repair_merge, live_lock, pull_root_remote_name, rewrite_root_lock_from_live_members,
    root_remote_changes_are_auto_repairable,
};
use super::pull_head_member_preflight::{pull_dirty_guard, refuse_update_collisions};
use super::read_lock_or_empty;

pub(crate) struct RootPullPlan {
    action: RootPullAction,
    /// The manifest and lock the root will have after the pull, before member updates.
    pub(crate) manifest: ManifestArtifact,
    pub(crate) lock: LockArtifact,
    /// The lock to fall back to when the post-update lock cannot be read.
    fallback_lock: LockArtifact,
}

struct RootTarget {
    branch: String,
    local_commit: String,
    remote_commit: String,
    remote_ref: String,
}

enum RootPullAction {
    Noop,
    /// A frozen prepared merge: `--sync merge` up to date, fast-forward or merge, and the
    /// fast-forward of every other integrating mode (`rewrite_lock` then records the
    /// members' live state in the lock, as the released root fast-forward did).
    Prepared {
        target: RootTarget,
        prepared: GitPreparedMerge,
        rewrite_lock: bool,
    },
    /// ff-only / driver-selected divergence where the remote changed only the lock.
    RepairMerge {
        target: RootTarget,
    },
    Rebase {
        target: RootTarget,
    },
    Reset {
        target: RootTarget,
    },
}

impl RootPullPlan {
    fn target(&self) -> Option<&RootTarget> {
        match &self.action {
            RootPullAction::Noop => None,
            RootPullAction::Prepared { target, .. }
            | RootPullAction::RepairMerge { target }
            | RootPullAction::Rebase { target }
            | RootPullAction::Reset { target } => Some(target),
        }
    }

    /// The commit, or for a true merge the merged tree, that applying the plan checks out
    /// over the root worktree (and so can meet untracked files there). The lock-only
    /// repair merge changes nothing but the tracked lock, and reset has its own rule.
    fn checkout_target(&self) -> Option<&str> {
        match &self.action {
            RootPullAction::Noop
            | RootPullAction::Reset { .. }
            | RootPullAction::RepairMerge { .. }
            | RootPullAction::Prepared {
                prepared: GitPreparedMerge::Unchanged,
                ..
            } => None,
            RootPullAction::Prepared {
                prepared: GitPreparedMerge::Commit(commit),
                ..
            } => Some(&commit.tree_oid),
            RootPullAction::Prepared { target, .. } | RootPullAction::Rebase { target } => {
                Some(&target.remote_commit)
            }
        }
    }

    /// For a rebase, the commit it replays onto and the local commit whose history it
    /// replays.
    fn replay(&self) -> Option<(&str, &str)> {
        match &self.action {
            RootPullAction::Rebase { target } => {
                Some((&target.remote_commit, &target.local_commit))
            }
            _ => None,
        }
    }
}

pub(crate) fn plan_root_pull<B: GitBackend + GitPullUpdate>(
    backend: &B,
    root: &Path,
    policy: Option<&crate::OperationPolicy>,
    manifest: ManifestArtifact,
    lock: LockArtifact,
) -> ModelResult<RootPullPlan> {
    plan_root_pull_inner(backend, root, policy, manifest, lock).map_err(root_context)
}

fn plan_root_pull_inner<B: GitBackend + GitPullUpdate>(
    backend: &B,
    root: &Path,
    policy: Option<&crate::OperationPolicy>,
    manifest: ManifestArtifact,
    lock: LockArtifact,
) -> ModelResult<RootPullPlan> {
    if !backend.is_repository(root)? {
        return Ok(noop(manifest, lock));
    }
    let Some(remote) = pull_root_remote_name(backend, root, policy)? else {
        return Ok(noop(manifest, lock));
    };
    let head = backend.head(root)?;
    if head.is_detached {
        return Err(root_error(
            ErrorCode::BranchDetachedHead,
            "workspace root is detached; root pull requires an attached branch",
        ));
    }
    let branch = head
        .branch
        .ok_or_else(|| root_error(ErrorCode::BranchUnbornHead, "workspace root has no branch"))?;
    let local_commit = head.commit.ok_or_else(|| {
        root_error(
            ErrorCode::BranchUnbornHead,
            "workspace root has unborn HEAD",
        )
    })?;
    let sync = policy
        .and_then(|policy| policy.sync)
        .unwrap_or(crate::SyncBehavior::FfOnly);
    pull_dirty_guard(sync, &backend.status(root)?, policy, "workspace root")?;
    let fallback_lock = read_lock_or_empty(root, &manifest.workspace.id)?;

    backend.fetch(root, &remote)?;
    let remote_ref = format!("refs/remotes/{remote}/{branch}");
    let remote_commit = backend
        .read_ref(root, &remote_ref)?
        .ok_or_else(|| root_error(ErrorCode::MissingRemote, "root remote branch not found"))?;
    let target = RootTarget {
        branch,
        local_commit,
        remote_commit,
        remote_ref,
    };
    let merge = sync == crate::SyncBehavior::Merge;
    let behind = target.local_commit != target.remote_commit
        && backend.is_ancestor(root, &target.local_commit, &target.remote_commit)?;
    // Equal, or strictly ahead of the remote: integrating an ancestor is a no-op under
    // every non-destructive sync mode -- up to date, not divergence.
    let up_to_date = target.local_commit == target.remote_commit
        || (!behind
            && sync != crate::SyncBehavior::Reset
            && backend.is_ancestor(root, &target.remote_commit, &target.local_commit)?);
    let plan = match sync {
        _ if up_to_date && merge => {
            let prepared = prepare(backend, root, &target, GitPreparedMerge::Unchanged)?;
            RootPullPlan {
                action: RootPullAction::Prepared {
                    target,
                    prepared,
                    rewrite_lock: false,
                },
                manifest,
                lock,
                fallback_lock,
            }
        }
        crate::SyncBehavior::FetchOnly => noop(manifest, lock),
        _ if up_to_date => noop(manifest, lock),
        crate::SyncBehavior::Reset => {
            let (manifest, fallback) =
                projected_at(backend, root, &target.remote_commit, fallback_lock)?;
            let lock = live_lock(backend, root, &manifest, &fallback)?;
            projected(RootPullAction::Reset { target }, manifest, lock, fallback)
        }
        _ if behind => {
            let prepared = prepare(backend, root, &target, GitPreparedMerge::FastForward)?;
            if merge {
                let projected_manifest =
                    read_manifest_at(backend, root, &target.remote_commit, WORKSPACE_MANIFEST)?;
                let projected_lock =
                    read_lock_at(backend, root, &target.remote_commit, artifact::LOCK_PATH)?;
                let action = RootPullAction::Prepared {
                    target,
                    prepared,
                    rewrite_lock: false,
                };
                projected(action, projected_manifest, projected_lock, fallback_lock)
            } else {
                let (manifest, fallback) =
                    projected_at(backend, root, &target.remote_commit, fallback_lock)?;
                let lock = live_lock(backend, root, &manifest, &fallback)?;
                let action = RootPullAction::Prepared {
                    target,
                    prepared,
                    rewrite_lock: true,
                };
                projected(action, manifest, lock, fallback)
            }
        }
        crate::SyncBehavior::FfOnly | crate::SyncBehavior::DriverSelected => {
            if !root_remote_changes_are_auto_repairable(
                backend,
                root,
                &target.local_commit,
                &target.remote_commit,
            )? {
                return Err(root_error(
                    ErrorCode::DivergedMember,
                    "workspace root has diverged from remote; rerun with --sync merge, rebase, or reset",
                ));
            }
            // The remote changed only the lock, which the repair rewrites from the
            // members' live state: the manifest stays the local one.
            let (_, fallback) = projected_at(backend, root, &target.remote_commit, fallback_lock)?;
            let lock = live_lock(backend, root, &manifest, &fallback)?;
            projected(
                RootPullAction::RepairMerge { target },
                manifest,
                lock,
                fallback,
            )
        }
        crate::SyncBehavior::Rebase => {
            let manifest = rebased_manifest(backend, root, &target, manifest)?;
            let (_, fallback) = projected_at(backend, root, &target.remote_commit, fallback_lock)?;
            let lock = live_lock(backend, root, &manifest, &fallback)?;
            projected(RootPullAction::Rebase { target }, manifest, lock, fallback)
        }
        crate::SyncBehavior::Merge => {
            plan_diverged_merge(backend, root, target, manifest, lock)?.with_fallback(fallback_lock)
        }
    };
    // B2: an entry in the way of the root update refuses here, before anything moves.
    refuse_update_collisions(backend, root, plan.checkout_target(), plan.replay())?;
    Ok(plan)
}

impl RootPullPlan {
    fn with_fallback(mut self, fallback_lock: LockArtifact) -> Self {
        self.fallback_lock = fallback_lock;
        self
    }
}

fn projected(
    action: RootPullAction,
    manifest: ManifestArtifact,
    lock: LockArtifact,
    fallback_lock: LockArtifact,
) -> RootPullPlan {
    RootPullPlan {
        action,
        manifest,
        lock,
        fallback_lock,
    }
}

fn prepare<B: GitBackend + GitPullUpdate>(
    backend: &B,
    root: &Path,
    target: &RootTarget,
    expected: GitPreparedMerge,
) -> ModelResult<GitPreparedMerge> {
    let prepared = backend.prepare_pull_update_checked(
        root,
        &target.branch,
        &target.local_commit,
        &target.remote_commit,
    )?;
    if prepared != expected {
        return Err(root_error(
            ErrorCode::MergeRecoveryRequired,
            "root pull result changed during pull preparation",
        ));
    }
    Ok(prepared)
}

/// The manifest at `commit` and the lock to fall back to after updating to it (the lock at
/// `commit`, else the current one), as the released root update re-read them.
fn projected_at<B: GitBackend>(
    backend: &B,
    root: &Path,
    commit: &str,
    fallback_lock: LockArtifact,
) -> ModelResult<(ManifestArtifact, LockArtifact)> {
    let manifest = read_manifest_at(backend, root, commit, WORKSPACE_MANIFEST)?;
    let lock = read_lock_at(backend, root, commit, artifact::LOCK_PATH).unwrap_or(fallback_lock);
    Ok((manifest, lock))
}

/// The manifest a diverged rebase leaves: the side that changed it since the merge base,
/// refused when both did (it cannot be projected without replaying the rebase).
fn rebased_manifest<B: GitBackend>(
    backend: &B,
    root: &Path,
    target: &RootTarget,
    local: ManifestArtifact,
) -> ModelResult<ManifestArtifact> {
    let base = backend
        .merge_base(root, &target.local_commit, &target.remote_commit)?
        .ok_or_else(|| {
            root_error(
                ErrorCode::GitCommandFailed,
                "workspace root and remote do not share a merge base",
            )
        })?;
    let changed = |commit: &str| -> ModelResult<bool> {
        Ok(backend
            .changed_paths_between(root, &base, commit)?
            .iter()
            .any(|path| path == WORKSPACE_MANIFEST))
    };
    match (
        changed(&target.local_commit)?,
        changed(&target.remote_commit)?,
    ) {
        (true, true) => Err(root_error(
            ErrorCode::MergeValidationFailed,
            "diverged root rebase changes the workspace manifest on both sides and cannot be projected safely",
        )),
        (false, true) => read_manifest_at(backend, root, &target.remote_commit, WORKSPACE_MANIFEST),
        _ => Ok(local),
    }
}

fn plan_diverged_merge<B: GitBackend + GitPullUpdate>(
    backend: &B,
    root: &Path,
    target: RootTarget,
    manifest: ManifestArtifact,
    lock: LockArtifact,
) -> ModelResult<RootPullPlan> {
    let base = backend
        .merge_base(root, &target.local_commit, &target.remote_commit)?
        .ok_or_else(|| {
            root_error(
                ErrorCode::GitCommandFailed,
                "workspace root and remote do not share a merge base",
            )
        })?;
    let remote_paths = backend.changed_paths_between(root, &base, &target.remote_commit)?;
    if remote_paths
        .iter()
        .any(|path| path == WORKSPACE_MANIFEST || path == artifact::LOCK_PATH)
    {
        return Err(root_error(
            ErrorCode::MergeValidationFailed,
            "diverged root pull changes workspace metadata and cannot be projected safely",
        ));
    }
    match backend.merge_simulate(root, &target.local_commit, &target.remote_commit)? {
        GitMergeSimulation::Clean => {
            let prepared = backend.prepare_pull_update_checked(
                root,
                &target.branch,
                &target.local_commit,
                &target.remote_commit,
            )?;
            if !matches!(prepared, GitPreparedMerge::Commit(_)) {
                return Err(root_error(
                    ErrorCode::MergeRecoveryRequired,
                    "root merge result changed during pull preparation",
                ));
            }
            let action = RootPullAction::Prepared {
                target,
                prepared,
                rewrite_lock: false,
            };
            Ok(projected(action, manifest, lock.clone(), lock))
        }
        GitMergeSimulation::Conflicts(paths) => Err(root_error(
            ErrorCode::MergeValidationFailed,
            &format!(
                "pull merge is predicted to conflict in: {}",
                paths.join(", ")
            ),
        )),
    }
}

pub(crate) fn validate_root_pull<B: GitBackend + GitPullUpdate>(
    backend: &B,
    root: &Path,
    plan: &RootPullPlan,
) -> ModelResult<()> {
    validate_root_pull_inner(backend, root, plan).map_err(root_context)
}

fn validate_root_pull_inner<B: GitBackend + GitPullUpdate>(
    backend: &B,
    root: &Path,
    plan: &RootPullPlan,
) -> ModelResult<()> {
    let Some(target) = plan.target() else {
        return Ok(());
    };
    if backend.read_ref(root, &target.remote_ref)?.as_deref() != Some(&target.remote_commit) {
        return Err(root_error(
            ErrorCode::MergeDrift,
            "workspace root remote-tracking ref changed after pull preparation",
        ));
    }
    // Drift first: the entries in the way are judged against the HEAD the plan saw.
    if let RootPullAction::Prepared { prepared, .. } = &plan.action {
        backend.validate_prepared_pull_update(
            root,
            &target.branch,
            &target.local_commit,
            &target.remote_commit,
            prepared,
        )?;
    } else {
        let head = backend.head(root)?;
        if head.is_detached
            || head.branch.as_deref() != Some(target.branch.as_str())
            || head.commit.as_deref() != Some(target.local_commit.as_str())
        {
            return Err(root_error(
                ErrorCode::MergeDrift,
                "workspace root branch changed after pull preparation",
            ));
        }
    }
    refuse_update_collisions(backend, root, plan.checkout_target(), plan.replay())
}

/// Apply a validated root plan. Returns whether the root moved and, for the modes that
/// record the members' live state after the update, the lock that update wrote.
pub(crate) fn apply_root_pull<B: GitBackend + GitPullUpdate>(
    backend: &B,
    root: &Path,
    plan: &RootPullPlan,
) -> ModelResult<(bool, Option<LockArtifact>)> {
    apply_root_pull_inner(backend, root, plan).map_err(root_context)
}

fn apply_root_pull_inner<B: GitBackend + GitPullUpdate>(
    backend: &B,
    root: &Path,
    plan: &RootPullPlan,
) -> ModelResult<(bool, Option<LockArtifact>)> {
    let Some(target) = plan.target() else {
        return Ok((false, None));
    };
    let message = format!("Merge {} into {}", target.remote_ref, target.branch);
    let rewrite = match &plan.action {
        RootPullAction::Noop => return Ok((false, None)),
        RootPullAction::Prepared {
            prepared,
            rewrite_lock,
            ..
        } => {
            let result = backend.execute_prepared_pull_update(
                root,
                &target.branch,
                &target.local_commit,
                &target.remote_commit,
                &message,
                prepared,
            )?;
            if !result.conflicts.is_empty() {
                return Err(root_error(
                    ErrorCode::MergeRecoveryRequired,
                    "workspace root update changed after pull preparation",
                ));
            }
            if *prepared == GitPreparedMerge::Unchanged {
                return Ok((false, None));
            }
            *rewrite_lock
        }
        RootPullAction::RepairMerge { .. } => {
            let lock = apply_root_repair_merge(
                backend,
                root,
                &target.branch,
                &target.local_commit,
                &target.remote_commit,
                &target.remote_ref,
                &plan.fallback_lock,
            )?;
            return Ok((true, Some(lock)));
        }
        RootPullAction::Rebase { .. } => {
            // A rebase has no branch lock to re-check under: do it just before.
            refuse_update_collisions(backend, root, plan.checkout_target(), plan.replay())?;
            let result = backend.rebase_onto(root, &target.branch, &target.remote_commit)?;
            if !result.conflicts.is_empty() {
                return Err(root_error(
                    ErrorCode::GitCommandFailed,
                    &format!(
                        "workspace root rebase left conflicted paths: {}",
                        result.conflicts.join(", ")
                    ),
                ));
            }
            true
        }
        RootPullAction::Reset { .. } => {
            backend.reset_hard(root, &target.branch, &target.remote_commit)?;
            true
        }
    };
    let lock = rewrite
        .then(|| rewrite_root_lock_from_live_members(backend, root, &plan.fallback_lock))
        .transpose()?;
    Ok((true, lock))
}

fn noop(manifest: ManifestArtifact, lock: LockArtifact) -> RootPullPlan {
    RootPullPlan {
        action: RootPullAction::Noop,
        manifest,
        fallback_lock: lock.clone(),
        lock,
    }
}

fn read_manifest_at<B: GitBackend>(
    backend: &B,
    root: &Path,
    commit: &str,
    path: &str,
) -> ModelResult<ManifestArtifact> {
    ManifestArtifact::from_yaml(&read_utf8_at(backend, root, commit, path)?)
}

fn read_lock_at<B: GitBackend>(
    backend: &B,
    root: &Path,
    commit: &str,
    path: &str,
) -> ModelResult<LockArtifact> {
    LockArtifact::from_yaml(&read_utf8_at(backend, root, commit, path)?)
}

fn read_utf8_at<B: GitBackend>(
    backend: &B,
    root: &Path,
    commit: &str,
    path: &str,
) -> ModelResult<String> {
    let bytes = backend
        .read_file_at_commit(root, commit, path)?
        .ok_or_else(|| root_error(ErrorCode::ManifestInvalid, &format!("missing '{path}'")))?;
    String::from_utf8(bytes).map_err(|_| {
        root_error(
            ErrorCode::ManifestInvalid,
            &format!("'{path}' is not valid UTF-8"),
        )
    })
}

fn root_error(code: ErrorCode, message: &str) -> ModelError {
    ModelError::new(code, message)
}

fn root_context(error: ModelError) -> ModelError {
    if error.member_id.is_some() {
        error
    } else {
        error.with_member("@root", ".")
    }
}

use std::path::Path;

use crate::artifact::{self, LockArtifact, ManifestArtifact};
use crate::git::{GitBackend, GitMergeSimulation, GitPreparedMerge};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace::WORKSPACE_MANIFEST;

use super::pull_head_member_preflight::pull_dirty_guard;

pub(crate) struct RootMergePullPlan {
    action: RootMergePullAction,
    pub(crate) manifest: ManifestArtifact,
    pub(crate) lock: LockArtifact,
}

enum RootMergePullAction {
    Noop,
    UpToDate {
        branch: String,
        local_commit: String,
        remote_commit: String,
        remote_ref: String,
        prepared: GitPreparedMerge,
    },
    FastForward {
        branch: String,
        local_commit: String,
        remote_commit: String,
        remote_ref: String,
        prepared: GitPreparedMerge,
    },
    Merge {
        branch: String,
        local_commit: String,
        remote_commit: String,
        remote_ref: String,
        prepared: GitPreparedMerge,
    },
}

pub(crate) fn plan_root_merge_pull<B: GitBackend>(
    backend: &B,
    root: &Path,
    policy: Option<&crate::OperationPolicy>,
    manifest: ManifestArtifact,
    lock: LockArtifact,
) -> ModelResult<RootMergePullPlan> {
    plan_root_merge_pull_inner(backend, root, policy, manifest, lock).map_err(root_context)
}

fn plan_root_merge_pull_inner<B: GitBackend>(
    backend: &B,
    root: &Path,
    policy: Option<&crate::OperationPolicy>,
    manifest: ManifestArtifact,
    lock: LockArtifact,
) -> ModelResult<RootMergePullPlan> {
    if !backend.is_repository(root)? {
        return Ok(noop(manifest, lock));
    }
    let Some(remote) = root_remote_name(backend, root, policy)? else {
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
    pull_dirty_guard(
        crate::SyncBehavior::Merge,
        &backend.status(root)?,
        policy,
        "workspace root",
    )?;

    backend.fetch(root, &remote)?;
    let remote_ref = format!("refs/remotes/{remote}/{branch}");
    let remote_commit = backend
        .read_ref(root, &remote_ref)?
        .ok_or_else(|| root_error(ErrorCode::MissingRemote, "root remote branch not found"))?;
    if local_commit == remote_commit {
        let prepared = backend.prepare_merge_upstream_checked(
            root,
            &branch,
            &local_commit,
            &remote_commit,
            None,
        )?;
        if prepared != GitPreparedMerge::Unchanged {
            return Err(root_error(
                ErrorCode::MergeRecoveryRequired,
                "root up-to-date result changed during pull preparation",
            ));
        }
        return Ok(RootMergePullPlan {
            action: RootMergePullAction::UpToDate {
                branch,
                local_commit,
                remote_commit,
                remote_ref,
                prepared,
            },
            manifest,
            lock,
        });
    }
    if backend.is_ancestor(root, &local_commit, &remote_commit)? {
        let prepared = backend.prepare_merge_upstream_checked(
            root,
            &branch,
            &local_commit,
            &remote_commit,
            None,
        )?;
        if prepared != GitPreparedMerge::FastForward {
            return Err(root_error(
                ErrorCode::MergeRecoveryRequired,
                "root fast-forward result changed during pull preparation",
            ));
        }
        let projected_manifest =
            read_manifest_at(backend, root, &remote_commit, WORKSPACE_MANIFEST)?;
        let projected_lock = read_lock_at(backend, root, &remote_commit, artifact::LOCK_PATH)?;
        return Ok(RootMergePullPlan {
            action: RootMergePullAction::FastForward {
                branch,
                local_commit,
                remote_commit,
                remote_ref,
                prepared,
            },
            manifest: projected_manifest,
            lock: projected_lock,
        });
    }

    let base = backend
        .merge_base(root, &local_commit, &remote_commit)?
        .ok_or_else(|| {
            root_error(
                ErrorCode::GitCommandFailed,
                "workspace root and remote do not share a merge base",
            )
        })?;
    let remote_paths = backend.changed_paths_between(root, &base, &remote_commit)?;
    if remote_paths
        .iter()
        .any(|path| path == WORKSPACE_MANIFEST || path == artifact::LOCK_PATH)
    {
        return Err(root_error(
            ErrorCode::MergeValidationFailed,
            "diverged root pull changes workspace metadata and cannot be projected safely",
        ));
    }
    match backend.merge_simulate(root, &local_commit, &remote_commit)? {
        GitMergeSimulation::Clean => {
            let prepared = backend.prepare_merge_upstream_checked(
                root,
                &branch,
                &local_commit,
                &remote_commit,
                None,
            )?;
            if !matches!(prepared, GitPreparedMerge::Commit(_)) {
                return Err(root_error(
                    ErrorCode::MergeRecoveryRequired,
                    "root merge result changed during pull preparation",
                ));
            }
            Ok(RootMergePullPlan {
                action: RootMergePullAction::Merge {
                    branch,
                    local_commit,
                    remote_commit,
                    remote_ref,
                    prepared,
                },
                manifest,
                lock,
            })
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

pub(crate) fn validate_root_merge_pull<B: GitBackend>(
    backend: &B,
    root: &Path,
    plan: &RootMergePullPlan,
) -> ModelResult<()> {
    validate_root_merge_pull_inner(backend, root, plan).map_err(root_context)
}

fn validate_root_merge_pull_inner<B: GitBackend>(
    backend: &B,
    root: &Path,
    plan: &RootMergePullPlan,
) -> ModelResult<()> {
    let (branch, local_commit, remote_commit, remote_ref, prepared) = match &plan.action {
        RootMergePullAction::Noop => return Ok(()),
        RootMergePullAction::UpToDate {
            branch,
            local_commit,
            remote_commit,
            remote_ref,
            prepared,
        }
        | RootMergePullAction::FastForward {
            branch,
            local_commit,
            remote_commit,
            remote_ref,
            prepared,
        }
        | RootMergePullAction::Merge {
            branch,
            local_commit,
            remote_commit,
            remote_ref,
            prepared,
        } => (branch, local_commit, remote_commit, remote_ref, prepared),
    };
    if backend.read_ref(root, remote_ref)?.as_deref() != Some(remote_commit) {
        return Err(root_error(
            ErrorCode::MergeDrift,
            "workspace root remote-tracking ref changed after pull preparation",
        ));
    }
    backend.validate_prepared_merge_upstream_state(
        root,
        branch,
        local_commit,
        remote_commit,
        prepared,
    )
}

pub(crate) fn apply_root_merge_pull<B: GitBackend>(
    backend: &B,
    root: &Path,
    plan: &RootMergePullPlan,
) -> ModelResult<bool> {
    apply_root_merge_pull_inner(backend, root, plan).map_err(root_context)
}

fn apply_root_merge_pull_inner<B: GitBackend>(
    backend: &B,
    root: &Path,
    plan: &RootMergePullPlan,
) -> ModelResult<bool> {
    match &plan.action {
        RootMergePullAction::Noop => Ok(false),
        RootMergePullAction::UpToDate {
            branch,
            local_commit,
            remote_commit,
            remote_ref,
            prepared,
        } => {
            let result = backend.execute_prepared_merge_upstream_checked(
                root,
                branch,
                local_commit,
                remote_commit,
                &format!("Merge {remote_ref} into {branch}"),
                prepared,
            )?;
            if !result.conflicts.is_empty() {
                return Err(root_error(
                    ErrorCode::MergeRecoveryRequired,
                    "workspace root up-to-date result changed after pull preparation",
                ));
            }
            Ok(false)
        }
        RootMergePullAction::FastForward {
            branch,
            local_commit,
            remote_commit,
            remote_ref,
            prepared,
        } => {
            let result = backend.execute_prepared_merge_upstream_checked(
                root,
                branch,
                local_commit,
                remote_commit,
                &format!("Merge {remote_ref} into {branch}"),
                prepared,
            )?;
            if !result.conflicts.is_empty() {
                return Err(root_error(
                    ErrorCode::MergeRecoveryRequired,
                    "workspace root fast-forward changed after pull preparation",
                ));
            }
            Ok(true)
        }
        RootMergePullAction::Merge {
            branch,
            local_commit,
            remote_commit,
            remote_ref,
            prepared,
        } => {
            let result = backend.execute_prepared_merge_upstream_checked(
                root,
                branch,
                local_commit,
                remote_commit,
                &format!("Merge {remote_ref} into {branch}"),
                prepared,
            )?;
            if !result.conflicts.is_empty() {
                return Err(root_error(
                    ErrorCode::MergeRecoveryRequired,
                    "workspace root merge changed after conflict prediction",
                ));
            }
            Ok(true)
        }
    }
}

fn noop(manifest: ManifestArtifact, lock: LockArtifact) -> RootMergePullPlan {
    RootMergePullPlan {
        action: RootMergePullAction::Noop,
        manifest,
        lock,
    }
}

fn root_remote_name<B: GitBackend>(
    backend: &B,
    root: &Path,
    policy: Option<&crate::OperationPolicy>,
) -> ModelResult<Option<String>> {
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

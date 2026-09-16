use std::path::Path;

use crate::artifact::{LockArtifact, ManifestArtifact, ManifestMember};
use crate::git::{GitBackend, git_host};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::EventEmitter;

use super::super::pull_head_plan::{PullHeadAction, PullHeadPlan, PullHeadSource};
use super::member_validation::{pull_branch, pull_dirty_guard};

pub(crate) fn pull_head_member_preflight<B>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    lock: &LockArtifact,
    member_id: String,
    policy: Option<&crate::OperationPolicy>,
    emitter: Option<&EventEmitter<'_>>,
) -> ModelResult<PullHeadPlan>
where
    B: GitBackend,
{
    let member = manifest
        .members
        .iter()
        .find(|member| member.id == member_id)
        .ok_or_else(|| ModelError::new(ErrorCode::MemberNotFound, "member not found"))?;
    let state = lock.members.get(&member_id).cloned().ok_or_else(|| {
        ModelError::new(
            ErrorCode::LockNotFound,
            format!("lock record missing for member '{member_id}'"),
        )
    })?;
    let branch = pull_branch(member, &state);
    if let Some(emitter) = emitter {
        emitter.member_started(&member.id, &state.path);
    }
    if member
        .desired
        .as_ref()
        .and_then(|desired| desired.local_only)
        == Some(true)
    {
        if let Some(emitter) = emitter {
            emitter.member_finished(&member.id, &state.path);
        }
        return Ok(PullHeadPlan {
            member_id,
            branch,
            state,
            action: PullHeadAction::Noop,
        });
    }

    let member_root = root.join(&state.path);
    if !backend.is_repository(&member_root)? {
        return Err(ModelError::new(
            ErrorCode::MemberNotFound,
            format!("member '{member_id}' is not materialized"),
        ));
    }
    let sync = policy
        .and_then(|policy| policy.sync)
        .unwrap_or(crate::SyncBehavior::FfOnly);
    let status = backend.status(&member_root)?;
    pull_dirty_guard(sync, &status, policy, &member_id)?;
    let Some(remote) = pull_fetch_remote_name(member, policy) else {
        if let Some(emitter) = emitter {
            emitter.member_finished(&member.id, &state.path);
        }
        return Ok(PullHeadPlan {
            member_id,
            branch,
            state,
            action: PullHeadAction::SkipNoFetchRemote,
        });
    };
    backend.fetch(&member_root, &remote)?;
    let remote_ref = format!("refs/remotes/{remote}/{branch}");
    let remote_commit = backend
        .read_ref(&member_root, &remote_ref)?
        .ok_or_else(|| ModelError::new(ErrorCode::MissingRemote, "remote branch not found"))?;
    let head = backend.head(&member_root)?;
    let Some(local_commit) = head.commit.clone() else {
        return Err(ModelError::new(
            ErrorCode::DivergedMember,
            "cannot fast-forward unborn member",
        ));
    };
    let source = PullHeadSource {
        remote_ref,
        expected_local: local_commit.clone(),
        source_commit: remote_commit.clone(),
    };
    let action = if local_commit == remote_commit {
        let prepared = match sync {
            crate::SyncBehavior::FetchOnly | crate::SyncBehavior::Reset => None,
            crate::SyncBehavior::FfOnly
            | crate::SyncBehavior::DriverSelected
            | crate::SyncBehavior::Merge
            | crate::SyncBehavior::Rebase => Some(prepare_pull_unchanged(
                backend,
                &member_root,
                &branch,
                &source,
                &member.id,
                &state.path,
            )?),
        };
        PullHeadAction::UpToDate { source, prepared }
    } else {
        // Strictly behind ⇒ a fast-forward is always available, and the integration
        // modes take it too (git merge/rebase fast-forward by default). Diverged ⇒
        // the chosen sync mode decides; fetch-only never integrates either way.
        let behind = backend.is_ancestor(&member_root, &local_commit, &remote_commit)?;
        let ahead = !behind
            && !matches!(
                sync,
                crate::SyncBehavior::FetchOnly | crate::SyncBehavior::Reset
            )
            && backend.is_ancestor(&member_root, &remote_commit, &local_commit)?;
        if ahead {
            // Strictly ahead of the remote: integrating an ancestor is a
            // no-op under every non-destructive sync mode — up to date, not
            // divergence.
            PullHeadAction::UpToDate {
                source: source.clone(),
                prepared: Some(prepare_pull_unchanged(
                    backend,
                    &member_root,
                    &branch,
                    &source,
                    &member.id,
                    &state.path,
                )?),
            }
        } else {
            match sync {
                crate::SyncBehavior::FetchOnly => PullHeadAction::FetchOnly,
                crate::SyncBehavior::FfOnly | crate::SyncBehavior::DriverSelected => {
                    if behind {
                        PullHeadAction::FastForward {
                            prepared: prepare_pull_fast_forward(
                                backend,
                                &member_root,
                                &branch,
                                &source,
                                &member.id,
                                &state.path,
                            )?,
                            source,
                        }
                    } else {
                        return Err(ModelError::new(
                            ErrorCode::DivergedMember,
                            format!("member '{member_id}' has diverged from remote"),
                        ));
                    }
                }
                crate::SyncBehavior::Merge => {
                    if behind {
                        PullHeadAction::FastForward {
                            prepared: prepare_pull_fast_forward(
                                backend,
                                &member_root,
                                &branch,
                                &source,
                                &member.id,
                                &state.path,
                            )?,
                            source,
                        }
                    } else {
                        match backend
                            .merge_simulate(&member_root, &local_commit, &remote_commit)
                            .map_err(|error| error.with_member(&member.id, &state.path))?
                        {
                            crate::git::GitMergeSimulation::Clean => {
                                let prepared = backend
                                    .prepare_merge_upstream_checked(
                                        &member_root,
                                        &branch,
                                        &source.expected_local,
                                        &source.source_commit,
                                        None,
                                    )
                                    .map_err(|error| error.with_member(&member.id, &state.path))?;
                                if !matches!(prepared, crate::git::GitPreparedMerge::Commit(_)) {
                                    return Err(ModelError::new(
                                        ErrorCode::MergeRecoveryRequired,
                                        "pull merge result changed during preparation",
                                    )
                                    .with_member(&member.id, &state.path));
                                }
                                PullHeadAction::Merge { source, prepared }
                            }
                            crate::git::GitMergeSimulation::Conflicts(conflicts)
                                if policy.and_then(|policy| policy.partial)
                                    == Some(crate::PartialBehavior::Partial) =>
                            {
                                PullHeadAction::PredictedConflict { conflicts }
                            }
                            crate::git::GitMergeSimulation::Conflicts(conflicts) => {
                                return Err(ModelError::new(
                                    ErrorCode::MergeValidationFailed,
                                    format!(
                                        "pull merge is predicted to conflict in: {}",
                                        conflicts.join(", ")
                                    ),
                                )
                                .with_member(&member.id, &state.path));
                            }
                        }
                    }
                }
                crate::SyncBehavior::Rebase => {
                    if behind {
                        PullHeadAction::FastForward {
                            prepared: prepare_pull_fast_forward(
                                backend,
                                &member_root,
                                &branch,
                                &source,
                                &member.id,
                                &state.path,
                            )?,
                            source,
                        }
                    } else {
                        PullHeadAction::Rebase { source }
                    }
                }
                crate::SyncBehavior::Reset => PullHeadAction::Reset { source },
            }
        }
    };
    if let Some(emitter) = emitter {
        emitter.member_finished(&member.id, &state.path);
    }
    Ok(PullHeadPlan {
        member_id,
        branch,
        state,
        action,
    })
}

fn prepare_pull_unchanged<B: GitBackend>(
    backend: &B,
    member_root: &Path,
    branch: &str,
    source: &PullHeadSource,
    member_id: &str,
    member_path: &str,
) -> ModelResult<crate::git::GitPreparedMerge> {
    let prepared = backend
        .prepare_merge_upstream_checked(
            member_root,
            branch,
            &source.expected_local,
            &source.source_commit,
            None,
        )
        .map_err(|error| error.with_member(member_id, member_path))?;
    if prepared != crate::git::GitPreparedMerge::Unchanged {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            "pull up-to-date result changed during preparation",
        )
        .with_member(member_id, member_path));
    }
    Ok(prepared)
}

fn prepare_pull_fast_forward<B: GitBackend>(
    backend: &B,
    member_root: &Path,
    branch: &str,
    source: &PullHeadSource,
    member_id: &str,
    member_path: &str,
) -> ModelResult<crate::git::GitPreparedMerge> {
    let prepared = backend
        .prepare_merge_upstream_checked(
            member_root,
            branch,
            &source.expected_local,
            &source.source_commit,
            None,
        )
        .map_err(|error| error.with_member(member_id, member_path))?;
    if prepared != crate::git::GitPreparedMerge::FastForward {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            "pull fast-forward result changed during preparation",
        )
        .with_member(member_id, member_path));
    }
    Ok(prepared)
}

pub(crate) fn pull_fetch_remote_name(
    member: &ManifestMember,
    policy: Option<&crate::OperationPolicy>,
) -> Option<String> {
    policy
        .and_then(|policy| policy.remote.as_ref())
        .cloned()
        .or_else(|| {
            member
                .remotes
                .iter()
                .find(|remote| remote.fetch)
                .map(|remote| remote.name.clone())
        })
}

pub(crate) fn pull_remote_host(
    member: &ManifestMember,
    policy: Option<&crate::OperationPolicy>,
) -> Option<String> {
    let remote = pull_fetch_remote_name(member, policy)?;
    member
        .remotes
        .iter()
        .find(|candidate| candidate.name == remote)
        .and_then(|candidate| git_host(&candidate.url))
}

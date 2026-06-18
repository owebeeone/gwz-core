use std::path::Path;

use crate::artifact::{
    self, LockArtifact,
    ManifestArtifact, ManifestMember, ResolvedMemberArtifact,
};
use crate::git::{GitBackend, git_host};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{
    EventEmitter, EventSink, NullSink, OperationRequest, par_map_per_host, resolve_jobs,
    resolve_per_host,
};


use super::*;

pub fn handle_pull_head<B>(
    backend: &B,
    start: &Path,
    request: crate::PullHeadRequest,
    operation_id: impl Into<String>,
) -> ModelResult<crate::PullHeadResponse>
where
    B: GitBackend + Sync,
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
    B: GitBackend + Sync,
{
    let context = OperationRequest::PullHead(request.clone()).context(operation_id.into())?;
    let root = resolve_workspace_root(start, request.meta.workspace.as_ref())?;
    let manifest = artifact::read_manifest(&root)?;
    assert_workspace_id(&manifest, request.meta.workspace.as_ref())?;
    let mut lock = artifact::read_lock(&root)?;
    let selected = resolve_locked_selection(&manifest, &lock, request.meta.selection.as_ref())?;
    if request.meta.dry_run.unwrap_or(false) {
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
    let mut responses = Vec::with_capacity(plans.len());
    for plan in plans {
        let member_root = root.join(&plan.state.path);
        let conflicts = apply_pull_action(backend, &member_root, &plan)?;
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
        responses.push(pull_result_response(member, &state, &plan.action, &conflicts));
    }
    lock.created_at = now_marker();
    artifact::write_lock(&root, &lock)?;
    emitter.operation_finished();

    Ok(crate::PullHeadResponse {
        response: response_envelope(context, pull_response_aggregate(&responses), responses),
    })
}

pub(crate) const NO_FETCH_REMOTE_PULL_MESSAGE: &str = "no fetch remote configured; skipping pull";
pub(crate) const FETCH_ONLY_PULL_MESSAGE: &str = "fetched; not integrated (fetch-only)";

pub(crate) enum PullHeadAction {
    Noop,
    SkipNoFetchRemote,
    /// Fetched but deliberately not integrated (`--sync fetch-only`).
    FetchOnly,
    FastForward { remote_ref: String },
    Merge { remote_ref: String },
    Rebase { remote_ref: String },
    Reset { remote_ref: String },
}

impl PullHeadAction {
    pub(crate) fn is_noop(&self) -> bool {
        matches!(self, Self::Noop | Self::SkipNoFetchRemote)
    }

    pub(crate) fn planned_message(&self) -> Option<String> {
        match self {
            Self::SkipNoFetchRemote => Some(NO_FETCH_REMOTE_PULL_MESSAGE.to_owned()),
            Self::FetchOnly => Some(FETCH_ONLY_PULL_MESSAGE.to_owned()),
            Self::Noop
            | Self::FastForward { .. }
            | Self::Merge { .. }
            | Self::Rebase { .. }
            | Self::Reset { .. } => None,
        }
    }
}

pub(crate) struct PullHeadPlan {
    pub(crate) member_id: String,
    pub(crate) branch: String,
    pub(crate) state: ResolvedMemberArtifact,
    pub(crate) action: PullHeadAction,
}

impl PullHeadPlan {
    pub(crate) fn planned_response(&self) -> crate::MemberResponse {
        crate::MemberResponse {
            member_id: self.member_id.clone(),
            member_path: self.state.path.clone(),
            source_kind: crate::SourceKind::Git,
            status: match self.action {
                PullHeadAction::Noop | PullHeadAction::SkipNoFetchRemote => {
                    crate::MemberStatus::Noop
                }
                PullHeadAction::FetchOnly
                | PullHeadAction::FastForward { .. }
                | PullHeadAction::Merge { .. }
                | PullHeadAction::Rebase { .. }
                | PullHeadAction::Reset { .. } => crate::MemberStatus::Planned,
            },
            error: None,
            planned: Some(crate::PlannedChange {
                action: match self.action {
                    PullHeadAction::Noop | PullHeadAction::SkipNoFetchRemote => {
                        crate::PlannedAction::Noop
                    }
                    PullHeadAction::FetchOnly => crate::PlannedAction::Fetch,
                    PullHeadAction::FastForward { .. } => crate::PlannedAction::FastForward,
                    PullHeadAction::Merge { .. } => crate::PlannedAction::Merge,
                    PullHeadAction::Rebase { .. } => crate::PlannedAction::Rebase,
                    PullHeadAction::Reset { .. } => crate::PlannedAction::Reset,
                },
                from_ref: self.state.commit.clone(),
                to_ref: None,
                message: self.action.planned_message(),
            }),
            state: None,
            git_status: None,
            lock_match: None,
        }
    }
}

pub(crate) fn pull_head_preflight<B>(
    backend: &B,
    root: &Path,
    manifest: &ManifestArtifact,
    lock: &LockArtifact,
    selected: &[String],
    policy: Option<&crate::OperationPolicy>,
    emitter: Option<&EventEmitter<'_>>,
) -> ModelResult<Vec<PullHeadPlan>>
where
    B: GitBackend + Sync,
{
    par_map_per_host(
        selected.to_vec(),
        resolve_jobs(policy.and_then(|policy| policy.concurrency)),
        resolve_per_host(policy.and_then(|policy| policy.max_connections_per_host)),
        |member_id| {
            manifest
                .members
                .iter()
                .find(|member| member.id == *member_id)
                .and_then(|member| pull_remote_host(member, policy))
        },
        |member_id| {
            pull_head_member_preflight(backend, root, manifest, lock, member_id, policy, emitter)
        },
    )
    .into_iter()
    .collect()
}

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
    let branch = state
        .branch
        .clone()
        .or_else(|| {
            member
                .desired
                .as_ref()
                .and_then(|desired| desired.branch.clone())
        })
        .unwrap_or_else(|| "main".to_owned());
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
    if status.is_dirty {
        // fetch-only never touches the worktree, so a dirty member is fine. reset is
        // destructive and gated on the destructive policy. Everything that integrates
        // (ff/merge/rebase) refuses, matching porcelain git's "local changes" guard.
        match sync {
            crate::SyncBehavior::FetchOnly => {}
            crate::SyncBehavior::Reset
                if policy.and_then(|policy| policy.destructive)
                    == Some(crate::DestructiveBehavior::Allow) => {}
            crate::SyncBehavior::Reset => {
                return Err(ModelError::new(
                    ErrorCode::DirtyMember,
                    format!(
                        "member '{member_id}' has uncommitted changes; reset would discard them"
                    ),
                ));
            }
            _ => {
                return Err(ModelError::new(
                    ErrorCode::DirtyMember,
                    format!("member '{member_id}' has uncommitted changes"),
                ));
            }
        }
    }
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
    let action = if local_commit == remote_commit {
        PullHeadAction::Noop
    } else {
        // Strictly behind ⇒ a fast-forward is always available, and the integration
        // modes take it too (git merge/rebase fast-forward by default). Diverged ⇒
        // the chosen sync mode decides; fetch-only never integrates either way.
        let behind = backend.is_ancestor(&member_root, &local_commit, &remote_commit)?;
        match sync {
            crate::SyncBehavior::FetchOnly => PullHeadAction::FetchOnly,
            crate::SyncBehavior::FfOnly | crate::SyncBehavior::DriverSelected => {
                if behind {
                    PullHeadAction::FastForward { remote_ref }
                } else {
                    return Err(ModelError::new(
                        ErrorCode::DivergedMember,
                        format!("member '{member_id}' has diverged from remote"),
                    ));
                }
            }
            crate::SyncBehavior::Merge => {
                if behind {
                    PullHeadAction::FastForward { remote_ref }
                } else {
                    PullHeadAction::Merge { remote_ref }
                }
            }
            crate::SyncBehavior::Rebase => {
                if behind {
                    PullHeadAction::FastForward { remote_ref }
                } else {
                    PullHeadAction::Rebase { remote_ref }
                }
            }
            crate::SyncBehavior::Reset => PullHeadAction::Reset { remote_ref },
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

/// Execute a planned pull action against the materialized member, returning any
/// conflicted paths (empty when clean or non-integrating). Merge/rebase conflicts are
/// RETURNED, not errored — the worktree is left `--continue`-able for the developer.
pub(crate) fn apply_pull_action<B>(
    backend: &B,
    member_root: &Path,
    plan: &PullHeadPlan,
) -> ModelResult<Vec<String>>
where
    B: GitBackend,
{
    match &plan.action {
        PullHeadAction::Noop | PullHeadAction::SkipNoFetchRemote | PullHeadAction::FetchOnly => {
            Ok(Vec::new())
        }
        PullHeadAction::FastForward { remote_ref } => {
            backend.fast_forward(member_root, &plan.branch, remote_ref)?;
            Ok(Vec::new())
        }
        PullHeadAction::Merge { remote_ref } => Ok(backend
            .merge_upstream(member_root, &plan.branch, remote_ref)?
            .conflicts),
        PullHeadAction::Rebase { remote_ref } => Ok(backend
            .rebase_onto(member_root, &plan.branch, remote_ref)?
            .conflicts),
        PullHeadAction::Reset { remote_ref } => {
            backend.reset_hard(member_root, &plan.branch, remote_ref)?;
            Ok(Vec::new())
        }
    }
}

pub(crate) fn pull_result_response(
    member: &ManifestMember,
    state: &ResolvedMemberArtifact,
    action: &PullHeadAction,
    conflicts: &[String],
) -> crate::MemberResponse {
    let status = if conflicts.is_empty() {
        match action {
            PullHeadAction::Noop | PullHeadAction::SkipNoFetchRemote => crate::MemberStatus::Noop,
            PullHeadAction::FetchOnly
            | PullHeadAction::FastForward { .. }
            | PullHeadAction::Merge { .. }
            | PullHeadAction::Rebase { .. }
            | PullHeadAction::Reset { .. } => crate::MemberStatus::Ok,
        }
    } else {
        crate::MemberStatus::Conflicted
    };
    let message = if conflicts.is_empty() {
        action.planned_message()
    } else {
        Some(format!(
            "integration left {} conflicted path(s); resolve and continue: {}",
            conflicts.len(),
            conflicts.join(", ")
        ))
    };
    crate::MemberResponse {
        member_id: member.id.clone(),
        member_path: state.path.clone(),
        source_kind: crate::SourceKind::Git,
        status,
        error: None,
        planned: message.map(|message| crate::PlannedChange {
            action: crate::PlannedAction::Noop,
            from_ref: state.commit.clone(),
            to_ref: None,
            message: Some(message),
        }),
        state: Some(protocol_state(member, state)),
        git_status: None,
        // A conflicted worktree no longer matches the lock; don't claim it does.
        lock_match: conflicts
            .is_empty()
            .then_some(crate::LockMatch::Matches),
    }
}

pub(crate) fn pull_aggregate_status(plans: &[PullHeadPlan]) -> crate::AggregateStatus {
    if plans.iter().all(|plan| plan.action.is_noop()) {
        crate::AggregateStatus::Noop
    } else {
        crate::AggregateStatus::Accepted
    }
}

pub(crate) fn pull_response_aggregate(responses: &[crate::MemberResponse]) -> crate::AggregateStatus {
    if responses
        .iter()
        .any(|response| response.status == crate::MemberStatus::Conflicted)
    {
        crate::AggregateStatus::Conflicted
    } else if responses
        .iter()
        .all(|response| response.status == crate::MemberStatus::Noop)
    {
        crate::AggregateStatus::Noop
    } else {
        crate::AggregateStatus::Ok
    }
}


use std::path::Path;

use crate::artifact::{ManifestMember, ResolvedMemberArtifact};
use crate::git::{GitBackend, GitPreparedMerge};
use crate::model::{ErrorCode, ModelError, ModelResult};

use super::protocol_state;

pub(crate) const NO_FETCH_REMOTE_PULL_MESSAGE: &str = "no fetch remote configured; skipping pull";
pub(crate) const FETCH_ONLY_PULL_MESSAGE: &str = "fetched; not integrated (fetch-only)";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PullHeadSource {
    pub(crate) remote_ref: String,
    pub(crate) expected_local: String,
    pub(crate) source_commit: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PullHeadAction {
    Noop,
    UpToDate {
        source: PullHeadSource,
        prepared: Option<GitPreparedMerge>,
    },
    SkipNoFetchRemote,
    /// Fetched but deliberately not integrated (`--sync fetch-only`).
    FetchOnly,
    FastForward {
        source: PullHeadSource,
        prepared: GitPreparedMerge,
    },
    Merge {
        source: PullHeadSource,
        prepared: GitPreparedMerge,
    },
    PredictedConflict {
        conflicts: Vec<String>,
    },
    Rebase {
        source: PullHeadSource,
    },
    Reset {
        source: PullHeadSource,
    },
}

impl PullHeadAction {
    pub(crate) fn is_noop(&self) -> bool {
        matches!(
            self,
            Self::Noop | Self::UpToDate { .. } | Self::SkipNoFetchRemote
        )
    }

    pub(crate) fn planned_message(&self) -> Option<String> {
        match self {
            Self::SkipNoFetchRemote => Some(NO_FETCH_REMOTE_PULL_MESSAGE.to_owned()),
            Self::FetchOnly => Some(FETCH_ONLY_PULL_MESSAGE.to_owned()),
            Self::PredictedConflict { conflicts } => Some(format!(
                "skipped predicted merge conflict in: {}",
                conflicts.join(", ")
            )),
            Self::Noop
            | Self::UpToDate { .. }
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
                PullHeadAction::Noop
                | PullHeadAction::UpToDate { .. }
                | PullHeadAction::SkipNoFetchRemote => crate::MemberStatus::Noop,
                PullHeadAction::FetchOnly
                | PullHeadAction::FastForward { .. }
                | PullHeadAction::Merge { .. }
                | PullHeadAction::Rebase { .. }
                | PullHeadAction::Reset { .. } => crate::MemberStatus::Planned,
                PullHeadAction::PredictedConflict { .. } => crate::MemberStatus::Skipped,
            },
            error: None,
            planned: Some(crate::PlannedChange {
                action: match self.action {
                    PullHeadAction::Noop
                    | PullHeadAction::UpToDate { .. }
                    | PullHeadAction::SkipNoFetchRemote => crate::PlannedAction::Noop,
                    PullHeadAction::FetchOnly => crate::PlannedAction::Fetch,
                    PullHeadAction::FastForward { .. } => crate::PlannedAction::FastForward,
                    PullHeadAction::Merge { .. } => crate::PlannedAction::Merge,
                    PullHeadAction::Rebase { .. } => crate::PlannedAction::Rebase,
                    PullHeadAction::Reset { .. } => crate::PlannedAction::Reset,
                    PullHeadAction::PredictedConflict { .. } => crate::PlannedAction::Noop,
                },
                from_ref: self.state.commit.clone(),
                to_ref: None,
                message: self.action.planned_message(),
            }),
            state: None,
            git_status: None,
            target_kind: Some(crate::TargetKind::Member),
            lock_match: None,
        }
    }
}

fn execute_prepared_pull_merge<B>(
    backend: &B,
    member_root: &Path,
    branch: &str,
    source: &PullHeadSource,
    prepared: &GitPreparedMerge,
) -> ModelResult<Vec<String>>
where
    B: GitBackend,
{
    let result = backend.execute_prepared_merge_upstream_checked(
        member_root,
        branch,
        &source.expected_local,
        &source.source_commit,
        &format!("Merge {} into {branch}", source.remote_ref),
        prepared,
    )?;
    if result.conflicts.is_empty() {
        Ok(Vec::new())
    } else {
        Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            "pull merge changed after its clean preflight prediction",
        ))
    }
}

/// Execute a planned pull action against the materialized member, returning any
/// conflicted paths (empty when clean or non-integrating). Merge/rebase conflicts are
/// returned rather than errored so the worktree remains continue-able.
pub(crate) fn apply_pull_action<B>(
    backend: &B,
    member_root: &Path,
    plan: &PullHeadPlan,
) -> ModelResult<Vec<String>>
where
    B: GitBackend,
{
    match &plan.action {
        PullHeadAction::Noop
        | PullHeadAction::SkipNoFetchRemote
        | PullHeadAction::FetchOnly
        | PullHeadAction::PredictedConflict { .. } => Ok(Vec::new()),
        PullHeadAction::UpToDate {
            source,
            prepared: Some(prepared),
        } => execute_prepared_pull_merge(backend, member_root, &plan.branch, source, prepared),
        PullHeadAction::UpToDate { prepared: None, .. } => Ok(Vec::new()),
        PullHeadAction::FastForward { source, prepared }
        | PullHeadAction::Merge { source, prepared } => {
            execute_prepared_pull_merge(backend, member_root, &plan.branch, source, prepared)
        }
        PullHeadAction::Rebase { source } => Ok(backend
            .rebase_onto(member_root, &plan.branch, &source.source_commit)?
            .conflicts),
        PullHeadAction::Reset { source } => {
            backend.reset_hard(member_root, &plan.branch, &source.source_commit)?;
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
            PullHeadAction::Noop
            | PullHeadAction::UpToDate { .. }
            | PullHeadAction::SkipNoFetchRemote => crate::MemberStatus::Noop,
            PullHeadAction::PredictedConflict { .. } => crate::MemberStatus::Skipped,
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
        target_kind: Some(crate::TargetKind::Member),
        lock_match: conflicts.is_empty().then_some(crate::LockMatch::Matches),
    }
}

pub(crate) fn pull_aggregate_status(plans: &[PullHeadPlan]) -> crate::AggregateStatus {
    if plans
        .iter()
        .any(|plan| matches!(plan.action, PullHeadAction::PredictedConflict { .. }))
    {
        crate::AggregateStatus::Partial
    } else if plans.iter().all(|plan| plan.action.is_noop()) {
        crate::AggregateStatus::Noop
    } else {
        crate::AggregateStatus::Accepted
    }
}

pub(crate) fn pull_response_aggregate(
    responses: &[crate::MemberResponse],
    root_changed: bool,
) -> crate::AggregateStatus {
    if responses
        .iter()
        .any(|response| response.status == crate::MemberStatus::Conflicted)
    {
        crate::AggregateStatus::Conflicted
    } else if responses
        .iter()
        .any(|response| response.status == crate::MemberStatus::Skipped)
    {
        crate::AggregateStatus::Partial
    } else if responses
        .iter()
        .all(|response| response.status == crate::MemberStatus::Noop)
    {
        if root_changed {
            crate::AggregateStatus::Ok
        } else {
            crate::AggregateStatus::Noop
        }
    } else {
        crate::AggregateStatus::Ok
    }
}

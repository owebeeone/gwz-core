use super::super::{
    MergeParticipantObservation, MergeParticipantRecord, OperationState, ParticipantDriftKind,
    ParticipantState, RollbackEligibility,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) enum RollbackClass {
    RequiresExactBeforeNoGit,
    NoOwnedMutation,
    NativeConflict,
    IntegratedResult,
    Complete,
}

pub(in crate::workspace_ops::merge) fn rollback_class(state: ParticipantState) -> RollbackClass {
    match state {
        ParticipantState::Planned | ParticipantState::Failed => {
            RollbackClass::RequiresExactBeforeNoGit
        }
        ParticipantState::UpToDate | ParticipantState::Unattempted => {
            RollbackClass::NoOwnedMutation
        }
        ParticipantState::Conflicted => RollbackClass::NativeConflict,
        ParticipantState::FastForwarded
        | ParticipantState::Merged
        | ParticipantState::Continued => RollbackClass::IntegratedResult,
        ParticipantState::Aborted | ParticipantState::RolledBack => RollbackClass::Complete,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) enum AbortPreflightDecision {
    Reject,
    Proceed,
    AlreadyApplied,
}

pub(in crate::workspace_ops::merge) fn abort_preflight_decision(
    operation: OperationState,
    participant: &MergeParticipantRecord,
    observation: &MergeParticipantObservation,
) -> AbortPreflightDecision {
    if !observation.abort_eligibility.eligible {
        return AbortPreflightDecision::Reject;
    }

    match participant.state {
        ParticipantState::Aborted | ParticipantState::RolledBack => {
            AbortPreflightDecision::AlreadyApplied
        }
        ParticipantState::Conflicted
            if observation.live_commit.as_deref() == Some(&participant.before_commit)
                && !observation.drift.is_empty()
                && observation
                    .drift
                    .iter()
                    .all(|drift| drift.kind == ParticipantDriftKind::MergeStateMissing) =>
        {
            AbortPreflightDecision::AlreadyApplied
        }
        ParticipantState::FastForwarded
        | ParticipantState::Merged
        | ParticipantState::Continued
            if operation == OperationState::RollingBack
                && observation.live_commit.as_deref() == Some(&participant.before_commit)
                && !observation.drift.is_empty()
                && observation.drift.iter().all(|drift| {
                    matches!(
                        drift.kind,
                        ParticipantDriftKind::TargetRefChanged | ParticipantDriftKind::HeadRewound
                    )
                }) =>
        {
            AbortPreflightDecision::AlreadyApplied
        }
        ParticipantState::Planned
        | ParticipantState::UpToDate
        | ParticipantState::FastForwarded
        | ParticipantState::Merged
        | ParticipantState::Conflicted
        | ParticipantState::Failed
        | ParticipantState::Unattempted
        | ParticipantState::Continued => AbortPreflightDecision::Proceed,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) enum RollbackGitAction {
    None,
    AbortConflict,
    ResetIntegrated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) struct ParticipantRollbackDecision {
    pub(in crate::workspace_ops::merge) git_action: RollbackGitAction,
    pub(in crate::workspace_ops::merge) terminal_state: Option<ParticipantState>,
}

pub(in crate::workspace_ops::merge) fn participant_rollback_decision(
    state: ParticipantState,
    already_applied: bool,
) -> ParticipantRollbackDecision {
    match state {
        ParticipantState::Planned
        | ParticipantState::UpToDate
        | ParticipantState::Failed
        | ParticipantState::Unattempted => ParticipantRollbackDecision {
            git_action: RollbackGitAction::None,
            terminal_state: Some(ParticipantState::Aborted),
        },
        ParticipantState::Conflicted => ParticipantRollbackDecision {
            git_action: if already_applied {
                RollbackGitAction::None
            } else {
                RollbackGitAction::AbortConflict
            },
            terminal_state: Some(ParticipantState::Aborted),
        },
        ParticipantState::FastForwarded
        | ParticipantState::Merged
        | ParticipantState::Continued => ParticipantRollbackDecision {
            git_action: if already_applied {
                RollbackGitAction::None
            } else {
                RollbackGitAction::ResetIntegrated
            },
            terminal_state: Some(ParticipantState::RolledBack),
        },
        ParticipantState::Aborted | ParticipantState::RolledBack => ParticipantRollbackDecision {
            git_action: RollbackGitAction::None,
            terminal_state: None,
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) struct RollbackEligibilityFacts {
    pub(in crate::workspace_ops::merge) native_merge_matches: bool,
    pub(in crate::workspace_ops::merge) exact_before_clean: bool,
    pub(in crate::workspace_ops::merge) target_ref_matches_before: bool,
    pub(in crate::workspace_ops::merge) repository_is_clean: bool,
    pub(in crate::workspace_ops::merge) worktree_is_clean: bool,
}

pub(in crate::workspace_ops::merge) fn rollback_eligibility(
    state: ParticipantState,
    facts: RollbackEligibilityFacts,
    drift: Vec<ParticipantDriftKind>,
) -> RollbackEligibility {
    let ordinary_clean = drift.is_empty() && facts.repository_is_clean && facts.worktree_is_clean;
    let eligible = match rollback_class(state) {
        RollbackClass::RequiresExactBeforeNoGit => facts.exact_before_clean,
        RollbackClass::NoOwnedMutation => true,
        RollbackClass::NativeConflict => {
            facts.exact_before_clean || (facts.native_merge_matches && drift.is_empty())
        }
        RollbackClass::IntegratedResult => facts.exact_before_clean || ordinary_clean,
        RollbackClass::Complete => facts.target_ref_matches_before || ordinary_clean,
    };
    RollbackEligibility {
        eligible,
        blockers: if eligible { Vec::new() } else { drift },
    }
}

pub(in crate::workspace_ops::merge) fn missing_repository_abort_eligibility(
    state: ParticipantState,
    has_pending: bool,
) -> RollbackEligibility {
    let eligible = !has_pending && rollback_class(state) == RollbackClass::NoOwnedMutation;
    RollbackEligibility {
        eligible,
        blockers: if eligible {
            Vec::new()
        } else {
            vec![ParticipantDriftKind::RepositoryMissing]
        },
    }
}

pub(in crate::workspace_ops::merge) fn block_abort(
    eligibility: &mut RollbackEligibility,
    blocker: ParticipantDriftKind,
) {
    eligibility.eligible = false;
    if !eligibility.blockers.contains(&blocker) {
        eligibility.blockers.push(blocker);
    }
}

#[cfg(test)]
#[path = "rollback_tests/mod.rs"]
mod tests;

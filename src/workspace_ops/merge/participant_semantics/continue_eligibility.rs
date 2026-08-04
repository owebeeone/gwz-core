use super::super::{OperationState, ParticipantDriftKind, ParticipantState, RetryEligibility};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) enum ContinueDisposition {
    ResolveConflict,
    RetryIntegration,
    Settled,
    RejectedTerminal,
}

pub(in crate::workspace_ops::merge) fn continue_disposition(
    state: ParticipantState,
) -> ContinueDisposition {
    match state {
        ParticipantState::Planned => ContinueDisposition::RetryIntegration,
        ParticipantState::UpToDate => ContinueDisposition::Settled,
        ParticipantState::FastForwarded => ContinueDisposition::Settled,
        ParticipantState::Merged => ContinueDisposition::Settled,
        ParticipantState::Conflicted => ContinueDisposition::ResolveConflict,
        ParticipantState::Failed => ContinueDisposition::RetryIntegration,
        ParticipantState::Unattempted => ContinueDisposition::RetryIntegration,
        ParticipantState::Continued => ContinueDisposition::Settled,
        ParticipantState::Aborted => ContinueDisposition::RejectedTerminal,
        ParticipantState::RolledBack => ContinueDisposition::RejectedTerminal,
    }
}

pub(in crate::workspace_ops::merge) fn post_start_state(
    states: impl IntoIterator<Item = ParticipantState>,
) -> OperationState {
    let mut conflicted = false;
    for state in states {
        match state {
            ParticipantState::Failed => return OperationState::Halted,
            ParticipantState::Conflicted => conflicted = true,
            ParticipantState::Planned
            | ParticipantState::UpToDate
            | ParticipantState::FastForwarded
            | ParticipantState::Merged
            | ParticipantState::Unattempted
            | ParticipantState::Continued
            | ParticipantState::Aborted
            | ParticipantState::RolledBack => {}
        }
    }
    if conflicted {
        OperationState::AwaitingResolution
    } else {
        OperationState::Finalizing
    }
}

pub(in crate::workspace_ops::merge) fn post_continue_state(
    states: impl IntoIterator<Item = ParticipantState>,
) -> OperationState {
    let mut unresolved = false;
    for state in states {
        match state {
            ParticipantState::Failed => return OperationState::Halted,
            ParticipantState::Planned
            | ParticipantState::Conflicted
            | ParticipantState::Unattempted => unresolved = true,
            ParticipantState::UpToDate
            | ParticipantState::FastForwarded
            | ParticipantState::Merged
            | ParticipantState::Continued
            | ParticipantState::Aborted
            | ParticipantState::RolledBack => {}
        }
    }
    if unresolved {
        OperationState::AwaitingResolution
    } else {
        OperationState::Finalizing
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge) struct ContinueEligibilityFacts {
    pub(in crate::workspace_ops::merge) conflicted: bool,
    pub(in crate::workspace_ops::merge) native_merge_matches: bool,
    pub(in crate::workspace_ops::merge) conflict_has_unresolved_or_unstaged: bool,
    pub(in crate::workspace_ops::merge) repository_is_clean: bool,
    pub(in crate::workspace_ops::merge) worktree_is_clean: bool,
}

pub(in crate::workspace_ops::merge) fn continue_eligibility(
    facts: ContinueEligibilityFacts,
    drift_blockers: Vec<ParticipantDriftKind>,
) -> RetryEligibility {
    let eligible = if facts.conflicted {
        drift_blockers.is_empty()
            && facts.native_merge_matches
            && !facts.conflict_has_unresolved_or_unstaged
    } else {
        drift_blockers.is_empty() && facts.repository_is_clean && facts.worktree_is_clean
    };
    let mut eligibility = RetryEligibility {
        eligible,
        blockers: drift_blockers,
    };
    if facts.conflicted && facts.conflict_has_unresolved_or_unstaged {
        block_continue(&mut eligibility, ParticipantDriftKind::IndexModified);
    }
    eligibility
}

pub(in crate::workspace_ops::merge) fn missing_repository_continue_eligibility() -> RetryEligibility
{
    RetryEligibility {
        eligible: false,
        blockers: vec![ParticipantDriftKind::RepositoryMissing],
    }
}

pub(in crate::workspace_ops::merge) fn block_continue(
    eligibility: &mut RetryEligibility,
    blocker: ParticipantDriftKind,
) {
    eligibility.eligible = false;
    if !eligibility.blockers.contains(&blocker) {
        eligibility.blockers.push(blocker);
    }
}

#[cfg(test)]
#[path = "continue_eligibility_tests.rs"]
mod tests;

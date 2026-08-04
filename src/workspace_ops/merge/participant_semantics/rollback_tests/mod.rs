use super::*;
use crate::workspace_ops::merge::{
    MergeParticipantRecord, MergeTargetKind, ParticipantDrift, RetryEligibility,
};
use std::collections::BTreeMap;

const STATES: [ParticipantState; 10] = [
    ParticipantState::Planned,
    ParticipantState::UpToDate,
    ParticipantState::FastForwarded,
    ParticipantState::Merged,
    ParticipantState::Conflicted,
    ParticipantState::Failed,
    ParticipantState::Unattempted,
    ParticipantState::Continued,
    ParticipantState::Aborted,
    ParticipantState::RolledBack,
];

const OPERATIONS: [OperationState; 9] = [
    OperationState::Executing,
    OperationState::AwaitingResolution,
    OperationState::Halted,
    OperationState::Finalizing,
    OperationState::Preserving,
    OperationState::RollingBack,
    OperationState::Completed,
    OperationState::Aborted,
    OperationState::RecoveryRequired,
];

fn participant(state: ParticipantState) -> MergeParticipantRecord {
    MergeParticipantRecord {
        path: "app".to_owned(),
        target_kind: MergeTargetKind::Member,
        target_branch: "main".to_owned(),
        before_commit: "before".to_owned(),
        source_commit: "source".to_owned(),
        commit_message: "merge".to_owned(),
        state,
        resulting_commit: matches!(
            state,
            ParticipantState::UpToDate
                | ParticipantState::FastForwarded
                | ParticipantState::Merged
                | ParticipantState::Continued
        )
        .then(|| "result".to_owned()),
        expected_merge_head: None,
        conflict_paths: Vec::new(),
        conflict_snapshot: Vec::new(),
        error: None,
        pending_action: None,
        preservation: Vec::new(),
        drift: Vec::new(),
        extensions: BTreeMap::new(),
    }
}

fn drift(kind: ParticipantDriftKind) -> ParticipantDrift {
    ParticipantDrift {
        kind,
        message: format!("{kind:?}"),
        expected_branch: None,
        live_branch: None,
        expected_head: None,
        live_head: None,
        expected_merge_head: None,
        live_merge_head: None,
    }
}

fn observation(
    eligible: bool,
    live_commit: Option<&str>,
    kinds: &[ParticipantDriftKind],
) -> MergeParticipantObservation {
    MergeParticipantObservation {
        live_commit: live_commit.map(str::to_owned),
        conflict_paths: Vec::new(),
        drift: kinds.iter().copied().map(drift).collect(),
        continue_eligibility: RetryEligibility::default(),
        abort_eligibility: RollbackEligibility {
            eligible,
            blockers: Vec::new(),
        },
        pending_action: None,
    }
}

mod decision;
mod eligibility;

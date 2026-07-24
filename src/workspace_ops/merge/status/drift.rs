use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::model::{ErrorCode, ModelError, ModelResult};

use super::super::{
    MergeParticipantObservation, MergeParticipantRecord, OperationDrift, OperationDriftKind,
    ParticipantDrift, ParticipantDriftKind, ParticipantState, RetryEligibility,
    RollbackEligibility,
};
use super::*;

pub(super) fn expected_head(participant: &MergeParticipantRecord) -> ModelResult<&str> {
    match participant.state {
        ParticipantState::UpToDate
        | ParticipantState::FastForwarded
        | ParticipantState::Merged
        | ParticipantState::Continued => participant.resulting_commit.as_deref().ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!(
                    "merge participant '{}' has no resulting commit",
                    participant.path
                ),
            )
        }),
        _ => Ok(&participant.before_commit),
    }
}

pub(super) fn missing_observation(
    target_id: &str,
    participant: &MergeParticipantRecord,
) -> MergeParticipantObservation {
    let repository_missing = ParticipantDrift {
        kind: ParticipantDriftKind::RepositoryMissing,
        message: format!(
            "participant '{target_id}' at '{}' is missing; restore it at the recorded path before recovery",
            participant.path
        ),
        expected_branch: Some(participant.target_branch.clone()),
        live_branch: None,
        expected_head: Some(participant.before_commit.clone()),
        live_head: None,
        expected_merge_head: participant.expected_merge_head.clone(),
        live_merge_head: None,
    };
    let mut drift = vec![repository_missing];
    if participant.pending_action.is_some() {
        drift.push(ParticipantDrift {
            kind: ParticipantDriftKind::PendingActionAmbiguous,
            message: format!(
                "participant '{target_id}' at '{}': pending action cannot be reconciled because the repository is missing",
                participant.path
            ),
            expected_branch: Some(participant.target_branch.clone()),
            live_branch: None,
            expected_head: Some(participant.before_commit.clone()),
            live_head: None,
            expected_merge_head: participant.expected_merge_head.clone(),
            live_merge_head: None,
        });
    }
    MergeParticipantObservation {
        live_commit: None,
        conflict_paths: Vec::new(),
        drift,
        continue_eligibility: RetryEligibility {
            eligible: false,
            blockers: vec![ParticipantDriftKind::RepositoryMissing],
        },
        abort_eligibility: RollbackEligibility {
            eligible: participant.pending_action.is_none()
                && does_not_require_rollback(participant.state),
            blockers: (participant.pending_action.is_some()
                || !does_not_require_rollback(participant.state))
            .then_some(ParticipantDriftKind::RepositoryMissing)
            .into_iter()
            .collect(),
        },
        pending_action: participant.pending_action.as_ref().map(|pending| {
            super::super::PendingActionObservation {
                kind: pending.kind,
                state: super::super::PendingActionObservationState::Ambiguous,
                message: Some("recorded participant repository is missing".to_owned()),
            }
        }),
    }
}

pub(super) fn does_not_require_rollback(state: ParticipantState) -> bool {
    matches!(
        state,
        ParticipantState::UpToDate | ParticipantState::Unattempted
    )
}

pub(super) fn participant_drift(
    kind: ParticipantDriftKind,
    target_id: &str,
    participant: &MergeParticipantRecord,
    live: &ParticipantLiveState,
    guidance: &str,
) -> ParticipantDrift {
    ParticipantDrift {
        kind,
        message: format!(
            "participant '{target_id}' at '{}': {guidance}",
            participant.path
        ),
        expected_branch: Some(participant.target_branch.clone()),
        live_branch: live.branch.clone(),
        expected_head: expected_head(participant).ok().map(str::to_owned),
        live_head: live.head.clone(),
        expected_merge_head: participant.expected_merge_head.clone(),
        live_merge_head: live
            .merge_state
            .as_ref()
            .map(|state| state.merge_head.clone()),
    }
}

pub(super) fn push_once(values: &mut Vec<ParticipantDriftKind>, value: ParticipantDriftKind) {
    if !values.contains(&value) {
        values.push(value);
    }
}

pub(super) fn compare_digest(
    root: &Path,
    relative: &str,
    expected: &str,
    kind: OperationDriftKind,
    drift: &mut Vec<OperationDrift>,
) {
    let actual = fs::read(root.join(relative))
        .ok()
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)));
    if actual.as_deref() != Some(expected) && !drift.iter().any(|item| item.kind == kind) {
        drift.push(OperationDrift {
            kind,
            message: format!(
                "workspace artifact '{relative}' changed from the recorded merge baseline"
            ),
        });
    }
}

pub(super) fn push_operation_drift(
    drift: &mut Vec<OperationDrift>,
    kind: OperationDriftKind,
    message: &str,
) {
    if !drift.iter().any(|item| item.kind == kind) {
        drift.push(OperationDrift {
            kind,
            message: message.to_owned(),
        });
    }
}

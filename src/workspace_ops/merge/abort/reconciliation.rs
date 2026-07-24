use super::super::{
    MergeOperationRecord, MergeParticipantObservation, MergeParticipantRecord, ParticipantState,
    PendingActionObservationState, PendingMergeActionKind, status::PendingActionReconciliation,
};
use crate::model::{ErrorCode, ModelError, ModelResult};
use std::collections::BTreeMap;

pub(super) fn pending_reconciliation(
    target_id: &str,
    participant: &MergeParticipantRecord,
    observation: &MergeParticipantObservation,
) -> ModelResult<PendingActionReconciliation> {
    let pending = observation.pending_action.as_ref().ok_or_else(|| {
        ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            "status snapshot omitted a durable pending action",
        )
        .with_member(target_id, &participant.path)
    })?;
    if participant
        .pending_action
        .as_ref()
        .is_none_or(|durable| durable.kind != pending.kind)
    {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            "status snapshot pending-action kind does not match the durable record",
        )
        .with_member(target_id, &participant.path));
    }
    match pending.state {
        PendingActionObservationState::NotStarted => {
            if pending.kind == PendingMergeActionKind::ResolveConflict
                && !observation.abort_eligibility.eligible
            {
                let message = observation.drift.first().map_or_else(
                    || {
                        format!(
                            "participant '{target_id}' at '{}': the staged resolution cannot be discarded by ordinary abort",
                            participant.path
                        )
                    },
                    |drift| drift.message.clone(),
                );
                return Err(ModelError::new(ErrorCode::MergeDrift, message)
                    .with_member(target_id, &participant.path));
            }
            Ok(PendingActionReconciliation::NotStarted)
        }
        PendingActionObservationState::ExpectedConflict => {
            Ok(PendingActionReconciliation::ExpectedConflict {
                conflict_paths: observation.conflict_paths.clone(),
            })
        }
        PendingActionObservationState::CompletedExactly => {
            let resulting_commit = observation.live_commit.clone().ok_or_else(|| {
                ModelError::new(
                    ErrorCode::MergeRecoveryRequired,
                    "completed pending action has no exact live commit",
                )
                .with_member(target_id, &participant.path)
            })?;
            Ok(PendingActionReconciliation::Completed { resulting_commit })
        }
        PendingActionObservationState::Ambiguous => {
            let reason = pending
                .message
                .as_deref()
                .unwrap_or("pending action is not at an exact recovery point");
            let message = observation.drift.first().map_or_else(
                || {
                    format!(
                        "participant '{target_id}' at '{}': {reason}",
                        participant.path
                    )
                },
                |drift| drift.message.clone(),
            );
            Err(ModelError::new(ErrorCode::MergeDrift, message)
                .with_member(target_id, &participant.path))
        }
    }
}

pub(super) fn apply_pending_reconciliations(
    record: &mut MergeOperationRecord,
    reconciliations: &BTreeMap<String, PendingActionReconciliation>,
) -> ModelResult<bool> {
    for (target_id, reconciliation) in reconciliations {
        let participant = record.participants.get_mut(target_id).ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!("merge record is missing participant '{target_id}'"),
            )
        })?;
        let pending = participant.pending_action.clone().ok_or_else(|| {
            ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                format!("merge participant '{target_id}' lost its pending action"),
            )
        })?;
        match reconciliation {
            PendingActionReconciliation::NotStarted => {}
            PendingActionReconciliation::ExpectedConflict { conflict_paths } => {
                participant.state = participant.state.transition(ParticipantState::Conflicted)?;
                participant.expected_merge_head = Some(pending.source_commit);
                participant.conflict_paths.clone_from(conflict_paths);
                participant.resulting_commit = None;
                participant.error = None;
            }
            PendingActionReconciliation::Completed { resulting_commit } => {
                let next = match pending.kind {
                    PendingMergeActionKind::VerifyUpToDate => ParticipantState::UpToDate,
                    PendingMergeActionKind::FastForward => ParticipantState::FastForwarded,
                    PendingMergeActionKind::TrueMerge => ParticipantState::Merged,
                    PendingMergeActionKind::ResolveConflict => ParticipantState::Continued,
                };
                participant.state = participant.state.transition(next)?;
                participant.resulting_commit = Some(resulting_commit.clone());
                participant.expected_merge_head = None;
                participant.conflict_paths.clear();
                participant.error = None;
            }
            PendingActionReconciliation::Ambiguous { .. } => {
                return Err(ModelError::new(
                    ErrorCode::InternalError,
                    "ambiguous pending action escaped abort preflight",
                ));
            }
        }
        participant.pending_action = None;
    }
    Ok(!reconciliations.is_empty())
}

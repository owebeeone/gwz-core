use crate::model::{ErrorCode, ModelError, ModelResult};

use super::super::super::{OperationState, ParticipantState};
use super::super::{MergeOperationRecordV1, RecoveryOriginStateV1};

pub(crate) fn validate_v1_lifecycle(record: &MergeOperationRecordV1) -> ModelResult<()> {
    for participant in record.participants.values() {
        validate_participant_shape(record, participant)?;
    }
    let effective = effective_state(record)?;
    let states = record
        .participants
        .values()
        .map(|participant| participant.state)
        .collect::<Vec<_>>();
    let forward_pending = record
        .participants
        .values()
        .any(|participant| participant.pending_action.is_some());

    let participant_states_are_legal = match effective {
        OperationState::Executing | OperationState::Preserving => {
            states.iter().copied().all(is_pre_rollback)
        }
        OperationState::AwaitingResolution => {
            states.contains(&ParticipantState::Conflicted)
                && record
                    .participants
                    .values()
                    .all(|participant| participant.error.is_none())
                && states
                    .iter()
                    .copied()
                    .all(|state| is_successful(state) || state == ParticipantState::Conflicted)
        }
        OperationState::Halted => {
            record.participants.values().any(|participant| {
                participant.state == ParticipantState::Failed
                    || participant.state == ParticipantState::Conflicted
                        && participant.error.is_some()
            }) && states.iter().copied().all(|state| {
                is_successful(state)
                    || matches!(
                        state,
                        ParticipantState::Conflicted
                            | ParticipantState::Failed
                            | ParticipantState::Unattempted
                    )
            })
        }
        OperationState::Finalizing | OperationState::Completed => {
            states.iter().copied().all(is_successful)
        }
        OperationState::RollingBack => true,
        OperationState::Aborted => states.iter().copied().all(is_rollback_terminal),
        OperationState::RecoveryRequired => unreachable!("recovery state was resolved above"),
    };
    if !participant_states_are_legal {
        return Err(lifecycle_error(record));
    }

    let forward_pending_is_legal = !forward_pending
        || matches!(
            effective,
            OperationState::Executing | OperationState::Halted | OperationState::Preserving
        );
    if !forward_pending_is_legal {
        return Err(lifecycle_error(record));
    }

    let reverse_pending_is_legal = match effective {
        OperationState::RollingBack => record.pending_preservation.is_none(),
        OperationState::Preserving => record.pending_rollback.is_none(),
        _ => record.pending_rollback.is_none() && record.pending_preservation.is_none(),
    };
    if !reverse_pending_is_legal {
        return Err(lifecycle_error(record));
    }

    Ok(())
}

fn validate_participant_shape(
    record: &MergeOperationRecordV1,
    participant: &super::super::super::MergeParticipantRecord,
) -> ModelResult<()> {
    let result = participant.resulting_commit.as_deref();
    let result_is_legal = match participant.state {
        ParticipantState::Planned
        | ParticipantState::Conflicted
        | ParticipantState::Failed
        | ParticipantState::Unattempted => result.is_none(),
        ParticipantState::UpToDate => result == Some(participant.before_commit.as_str()),
        ParticipantState::FastForwarded
        | ParticipantState::Merged
        | ParticipantState::Continued
        | ParticipantState::RolledBack => {
            result.is_some_and(|commit| commit != participant.before_commit)
        }
        ParticipantState::Aborted => {
            result.is_none() || result == Some(participant.before_commit.as_str())
        }
    };
    let expected_merge_head_is_exact =
        participant.expected_merge_head.as_deref() == Some(participant.source_commit.as_str());
    let conflict_fields_are_legal = match participant.state {
        ParticipantState::Conflicted => expected_merge_head_is_exact,
        ParticipantState::Aborted => {
            expected_merge_head_is_exact
                || participant.expected_merge_head.is_none()
                    && participant.conflict_paths.is_empty()
                    && participant.conflict_snapshot.is_empty()
        }
        _ => {
            participant.expected_merge_head.is_none()
                && participant.conflict_paths.is_empty()
                && participant.conflict_snapshot.is_empty()
        }
    };
    let error_is_legal = match participant.state {
        ParticipantState::Failed => participant.error.is_some(),
        ParticipantState::Conflicted | ParticipantState::Aborted => true,
        _ => participant.error.is_none(),
    };
    if result_is_legal && conflict_fields_are_legal && error_is_legal {
        Ok(())
    } else {
        Err(lifecycle_error(record))
    }
}

fn effective_state(record: &MergeOperationRecordV1) -> ModelResult<OperationState> {
    if record.state != OperationState::RecoveryRequired {
        return Ok(record.state);
    }
    let Some(context) = record.recovery_context.as_ref() else {
        return Err(lifecycle_error(record));
    };
    Ok(match context.origin_state {
        RecoveryOriginStateV1::Executing => OperationState::Executing,
        RecoveryOriginStateV1::AwaitingResolution => OperationState::AwaitingResolution,
        RecoveryOriginStateV1::Halted => OperationState::Halted,
        RecoveryOriginStateV1::Finalizing => OperationState::Finalizing,
        RecoveryOriginStateV1::Preserving => OperationState::Preserving,
        RecoveryOriginStateV1::RollingBack => OperationState::RollingBack,
    })
}

fn is_pre_rollback(state: ParticipantState) -> bool {
    !is_rollback_terminal(state)
}

fn is_successful(state: ParticipantState) -> bool {
    matches!(
        state,
        ParticipantState::UpToDate
            | ParticipantState::FastForwarded
            | ParticipantState::Merged
            | ParticipantState::Continued
    )
}

fn is_rollback_terminal(state: ParticipantState) -> bool {
    matches!(
        state,
        ParticipantState::Aborted | ParticipantState::RolledBack
    )
}

fn lifecycle_error(record: &MergeOperationRecordV1) -> ModelError {
    let (code, reason) = match record.state {
        OperationState::Completed => (
            ErrorCode::TerminalEvidenceMismatch,
            "completed record retains an unfinished participant or action",
        ),
        OperationState::Aborted => (
            ErrorCode::TerminalRollbackMismatch,
            "aborted record retains an unfinished participant or action",
        ),
        OperationState::RecoveryRequired => (
            ErrorCode::RecoveryEvidenceMismatch,
            "recovery origin does not match participant or action state",
        ),
        _ => (
            ErrorCode::MergeRecordUnreadable,
            "operation state does not match participant or action state",
        ),
    };
    ModelError::new(
        code,
        format!("merge record '{}' is invalid: {reason}", record.merge_id),
    )
}

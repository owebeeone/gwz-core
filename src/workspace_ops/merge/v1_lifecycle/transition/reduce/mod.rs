mod journal;
mod participant;
mod preservation;
mod publication;

use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::model::v1::{
    MergeOperationRecordV1, RecoveryContextV1, RecoveryOriginStateV1, ValidatedV1Record,
    validate_v1_record,
};
use crate::workspace_ops::merge::{OperationState, ParticipantState};

use super::super::authority::{BoundAuthority, ParticipantDriftIdentity, RollbackEntryOrigin};
use super::super::checked::StoredV1Record;
use super::effect::{EffectKind, TransitionEffect};
use super::{
    AcceptanceTransition, DriftTransition, OperationTransition, RecoveryTransition, V1Transition,
};

pub(super) fn apply(
    current: &StoredV1Record,
    transition: V1Transition,
    kind: EffectKind,
) -> ModelResult<(ValidatedV1Record, TransitionEffect)> {
    let mut next = current.record().clone();
    let effect = match transition {
        V1Transition::Operation(value) => operation(current, &mut next, *value, kind)?,
        V1Transition::Participant(value) => participant::apply(current, &mut next, *value, kind)?,
        V1Transition::Acceptance(value) => acceptance(current, &mut next, *value, kind)?,
        V1Transition::Publication(value) => publication::apply(current, &mut next, *value, kind)?,
        V1Transition::Recovery(value) => recovery(current, &mut next, *value, kind)?,
        V1Transition::Preservation(value) => preservation::apply(current, &mut next, *value, kind)?,
        V1Transition::Rollback(value) => journal::rollback(current, &mut next, *value, kind)?,
        V1Transition::Drift(value) => drift(current, &mut next, *value, kind)?,
    };
    next.writer_version = crate::VERSION.into();
    Ok((validate_v1_record(next)?, effect))
}

fn operation(
    current: &StoredV1Record,
    next: &mut MergeOperationRecordV1,
    transition: OperationTransition,
    kind: EffectKind,
) -> ModelResult<TransitionEffect> {
    use OperationState as S;
    let record = current.record();
    match transition {
        OperationTransition::BeginExecution => {
            require(matches!(record.state, S::AwaitingResolution | S::Halted))?;
            require(record.pending_rollback.is_none() && record.pending_preservation.is_none())?;
            next.state = S::Executing;
        }
        OperationTransition::AwaitResolution => {
            require(record.state == S::Executing && no_journals(record))?;
            require(
                record
                    .participants
                    .values()
                    .any(|row| row.state == ParticipantState::Conflicted),
            )?;
            require(record.participants.values().all(|row| {
                row.pending_action.is_none()
                    && row.error.is_none()
                    && matches!(
                        row.state,
                        ParticipantState::UpToDate
                            | ParticipantState::FastForwarded
                            | ParticipantState::Merged
                            | ParticipantState::Continued
                            | ParticipantState::Conflicted
                    )
            }))?;
            next.state = S::AwaitingResolution;
        }
        OperationTransition::Halt => {
            require(record.state == S::Executing && has_halt_cause(record))?;
            next.state = S::Halted;
        }
        OperationTransition::EnterFinalizing(proof) => {
            require(record.state == S::Executing && all_success(record) && no_actions(record))?;
            bound(
                &proof,
                current,
                "@operation",
                "enter_finalizing",
                "executing",
            )?;
            next.state = S::Finalizing;
        }
        OperationTransition::BeginPreservation(entry) => {
            require(matches!(
                record.state,
                S::Executing | S::AwaitingResolution | S::Halted | S::Finalizing
            ))?;
            require(no_actions(record))?;
            bound(
                &*entry,
                current,
                "@operation",
                "begin_preservation",
                "preflight",
            )?;
            require(entry.anticipated_model_matches(record))?;
            next.state = S::Preserving;
        }
        OperationTransition::BeginRollback(entry) => {
            require(matches!(
                record.state,
                S::Executing | S::AwaitingResolution | S::Halted | S::Finalizing | S::Preserving
            ))?;
            require(
                record
                    .participants
                    .values()
                    .all(|row| row.pending_action.is_none()),
            )?;
            require(record.pending_rollback.is_none() && record.pending_preservation.is_none())?;
            bound(
                &*entry,
                current,
                "@operation",
                "begin_rollback",
                "preflight",
            )?;
            require(entry.anticipated_model_matches(record))?;
            require(
                (record.state == S::Preserving)
                    == (entry.origin() == RollbackEntryOrigin::FromPreserving),
            )?;
            next.state = S::RollingBack;
        }
        OperationTransition::CompleteOperation(proof) => {
            require(
                record.state == S::Finalizing
                    && record.accepted_workspace.is_some()
                    && no_actions(record),
            )?;
            bound(
                &proof,
                current,
                "@operation",
                "publication_complete",
                "verified",
            )?;
            next.state = S::Completed;
        }
        OperationTransition::AbortOperation(proof) => {
            require(record.state == S::RollingBack && no_actions(record))?;
            bound(
                &proof,
                current,
                "@operation",
                "rollback_exhausted",
                "cursor_verified",
            )?;
            next.state = S::Aborted;
        }
    }
    Ok(TransitionEffect::operation(kind))
}

fn acceptance(
    current: &StoredV1Record,
    next: &mut MergeOperationRecordV1,
    transition: AcceptanceTransition,
    kind: EffectKind,
) -> ModelResult<TransitionEffect> {
    let AcceptanceTransition::Freeze(accepted) = transition;
    require(current.record().state == OperationState::Finalizing && all_success(current.record()))?;
    require(
        current.record().accepted_workspace.is_none()
            && current.record().publication.is_none()
            && no_actions(current.record()),
    )?;
    bound(
        &*accepted,
        current,
        "@operation",
        "freeze_acceptance",
        "prepared",
    )?;
    next.accepted_workspace = Some(accepted.value().clone());
    Ok(TransitionEffect::operation(kind))
}

fn recovery(
    current: &StoredV1Record,
    next: &mut MergeOperationRecordV1,
    transition: RecoveryTransition,
    kind: EffectKind,
) -> ModelResult<TransitionEffect> {
    match transition {
        RecoveryTransition::Enter(ambiguity) => {
            let origin = recovery_origin(current.record().state).ok_or_else(rejected)?;
            require(*ambiguity.value() == origin)?;
            bound(
                &ambiguity,
                current,
                "@operation",
                "enter_recovery",
                "ambiguous",
            )?;
            require(match current.record().state {
                OperationState::Preserving => current.record().pending_preservation.is_some(),
                OperationState::RollingBack => current.record().pending_rollback.is_some(),
                _ => true,
            })?;
            next.state = OperationState::RecoveryRequired;
            next.recovery_context = Some(RecoveryContextV1 {
                origin_state: origin,
            });
        }
        RecoveryTransition::Resume(proof) => {
            require(current.record().state == OperationState::RecoveryRequired)?;
            let context = current
                .record()
                .recovery_context
                .as_ref()
                .ok_or_else(rejected)?;
            require(*proof.value() == context.origin_state)?;
            bound(&proof, current, "@operation", "resume_recovery", "verified")?;
            next.state = operation_state(context.origin_state);
            next.recovery_context = None;
        }
    }
    Ok(TransitionEffect::operation(kind))
}

fn drift(
    current: &StoredV1Record,
    next: &mut MergeOperationRecordV1,
    transition: DriftTransition,
    kind: EffectKind,
) -> ModelResult<TransitionEffect> {
    match transition {
        DriftTransition::RecordParticipant(fact) => {
            let payload = fact.value();
            bound(
                &*fact,
                current,
                &payload.member_id,
                "record_drift",
                "observed",
            )?;
            require(payload.identity.matches(&payload.drift))?;
            let rows = &mut next
                .participants
                .get_mut(&payload.member_id)
                .ok_or_else(rejected)?
                .drift;
            require(record_participant_drift(
                rows,
                &payload.identity,
                payload.drift.clone(),
            ))?;
            Ok(TransitionEffect::participant_drift(
                kind,
                &payload.member_id,
                payload.identity.clone(),
            ))
        }
        DriftTransition::ClearParticipant(proof) => {
            let payload = proof.value();
            bound(
                &*proof,
                current,
                &payload.member_id,
                "clear_drift",
                "verified",
            )?;
            require(payload.identity.matches(&payload.drift))?;
            let rows = &mut next
                .participants
                .get_mut(&payload.member_id)
                .ok_or_else(rejected)?
                .drift;
            require(remove_participant_drift(rows, &payload.identity))?;
            Ok(TransitionEffect::participant_drift(
                kind,
                &payload.member_id,
                payload.identity.clone(),
            ))
        }
        DriftTransition::RecordOperation(fact) => {
            bound(&fact, current, "@operation", "record_drift", "observed")?;
            let value = fact.value().clone();
            replace_or_push(
                &mut next.operation_drift,
                value.kind,
                value.clone(),
                |row| row.kind,
            );
            Ok(TransitionEffect::operation_drift(kind, value.kind))
        }
        DriftTransition::ClearOperation(proof) => {
            bound(&proof, current, "@operation", "clear_drift", "verified")?;
            let value = proof.value();
            require(remove_exact(&mut next.operation_drift, value))?;
            Ok(TransitionEffect::operation_drift(kind, value.kind))
        }
    }
}

fn all_success(record: &MergeOperationRecordV1) -> bool {
    !record.selected_targets.is_empty()
        && record.selected_targets.iter().all(|id| {
            record.participants.get(id).is_some_and(|row| {
                matches!(
                    row.state,
                    ParticipantState::UpToDate
                        | ParticipantState::FastForwarded
                        | ParticipantState::Merged
                        | ParticipantState::Continued
                )
            })
        })
}

fn has_halt_cause(record: &MergeOperationRecordV1) -> bool {
    record.participants.values().any(|row| {
        row.state == ParticipantState::Failed
            || row.state == ParticipantState::Conflicted && row.error.is_some()
    })
}

fn no_forward(record: &MergeOperationRecordV1) -> bool {
    record
        .participants
        .values()
        .all(|row| row.pending_action.is_none())
}

fn no_journals(record: &MergeOperationRecordV1) -> bool {
    record.pending_rollback.is_none() && record.pending_preservation.is_none()
}

fn no_actions(record: &MergeOperationRecordV1) -> bool {
    no_forward(record) && no_journals(record)
}

fn participant_row<'a>(
    record: &'a MergeOperationRecordV1,
    id: &str,
) -> ModelResult<&'a crate::workspace_ops::merge::MergeParticipantRecord> {
    record.participants.get(id).ok_or_else(rejected)
}

fn recovery_origin(state: OperationState) -> Option<RecoveryOriginStateV1> {
    Some(match state {
        OperationState::Executing => RecoveryOriginStateV1::Executing,
        OperationState::AwaitingResolution => RecoveryOriginStateV1::AwaitingResolution,
        OperationState::Halted => RecoveryOriginStateV1::Halted,
        OperationState::Finalizing => RecoveryOriginStateV1::Finalizing,
        OperationState::Preserving => RecoveryOriginStateV1::Preserving,
        OperationState::RollingBack => RecoveryOriginStateV1::RollingBack,
        OperationState::Completed | OperationState::Aborted | OperationState::RecoveryRequired => {
            return None;
        }
    })
}

fn operation_state(origin: RecoveryOriginStateV1) -> OperationState {
    match origin {
        RecoveryOriginStateV1::Executing => OperationState::Executing,
        RecoveryOriginStateV1::AwaitingResolution => OperationState::AwaitingResolution,
        RecoveryOriginStateV1::Halted => OperationState::Halted,
        RecoveryOriginStateV1::Finalizing => OperationState::Finalizing,
        RecoveryOriginStateV1::Preserving => OperationState::Preserving,
        RecoveryOriginStateV1::RollingBack => OperationState::RollingBack,
    }
}

fn replace_or_push<T, K: Eq + Copy>(rows: &mut Vec<T>, key: K, value: T, get: impl Fn(&T) -> K) {
    if let Some(row) = rows.iter_mut().find(|row| get(row) == key) {
        *row = value
    } else {
        rows.push(value)
    }
}

fn record_participant_drift(
    rows: &mut Vec<crate::workspace_ops::merge::ParticipantDrift>,
    identity: &ParticipantDriftIdentity,
    value: crate::workspace_ops::merge::ParticipantDrift,
) -> bool {
    let matching = rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| identity.matches(row).then_some(index))
        .collect::<Vec<_>>();
    if let Some(index) = matching.get(identity.occurrence) {
        rows[*index] = value;
        true
    } else if identity.occurrence == matching.len() {
        rows.push(value);
        true
    } else {
        false
    }
}

fn remove_participant_drift(
    rows: &mut Vec<crate::workspace_ops::merge::ParticipantDrift>,
    identity: &ParticipantDriftIdentity,
) -> bool {
    let Some(index) = rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| identity.matches(row).then_some(index))
        .nth(identity.occurrence)
    else {
        return false;
    };
    rows.remove(index);
    true
}

fn remove_exact<T: PartialEq>(rows: &mut Vec<T>, expected: &T) -> bool {
    let Some(index) = rows.iter().position(|row| row == expected) else {
        return false;
    };
    rows.remove(index);
    true
}

fn bound(
    value: &impl BoundAuthority,
    current: &StoredV1Record,
    owner: &str,
    action: &str,
    phase: &str,
) -> ModelResult<()> {
    require(value.matches(current, owner, action, phase))
}

fn require(condition: bool) -> ModelResult<()> {
    condition.then_some(()).ok_or_else(rejected)
}

fn rejected() -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecoveryRequired,
        "v1 transition predecessor or authority mismatch",
    )
}

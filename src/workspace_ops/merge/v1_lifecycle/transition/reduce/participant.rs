use crate::model::ModelResult;
use crate::workspace_ops::merge::model::v1::MergeOperationRecordV1;
use crate::workspace_ops::merge::{OperationState, ParticipantState};

use super::super::super::authority::{
    ParticipantFailurePayload, RollbackEntryOrigin, VerifiedParticipantNotStarted,
    VerifiedParticipantOutcome,
};
use super::super::super::checked::StoredV1Record;
use super::super::ParticipantTransition;
use super::super::effect::{EffectKind, TransitionEffect};
use super::{bound, has_halt_cause, no_forward, participant_row, rejected, require};

pub(super) fn apply(
    current: &StoredV1Record,
    next: &mut MergeOperationRecordV1,
    transition: ParticipantTransition,
    kind: EffectKind,
) -> ModelResult<TransitionEffect> {
    match transition {
        ParticipantTransition::Prepare(intent) => {
            require(
                current.record().state == OperationState::Executing && no_forward(current.record()),
            )?;
            let payload = intent.value();
            bound(
                &*intent,
                current,
                &payload.member_id,
                "prepare_participant",
                "prepared",
            )?;
            let old = participant_row(current.record(), &payload.member_id)?;
            require(old.pending_action.is_none() && payload.row.pending_action.is_some())?;
            require(matches!(
                old.state,
                ParticipantState::Planned
                    | ParticipantState::Failed
                    | ParticipantState::Unattempted
                    | ParticipantState::Conflicted
            ))?;
            next.participants
                .insert(payload.member_id.clone(), payload.row.clone());
            Ok(TransitionEffect::participant(kind, &payload.member_id))
        }
        ParticipantTransition::RecordOutcome(proof) => {
            record_outcome(current, next, &proof, false)?;
            if current.record().state == OperationState::Halted {
                require(has_halt_cause(next))?;
            }
            Ok(TransitionEffect::participant(
                kind,
                &proof.value().member_id,
            ))
        }
        ParticipantTransition::RecordHaltedOutcomeAndResumeExecution(proof) => {
            require(current.record().state == OperationState::Halted)?;
            record_outcome(current, next, &proof, true)?;
            next.state = OperationState::Executing;
            require(!has_halt_cause(next))?;
            Ok(TransitionEffect::participant(
                kind,
                &proof.value().member_id,
            ))
        }
        ParticipantTransition::RecordHaltedOutcomeAndBeginRollback(proof, entry) => {
            require(current.record().state == OperationState::Halted)?;
            record_outcome(current, next, &proof, true)?;
            require(entry.origin() == RollbackEntryOrigin::Direct)?;
            require(entry.anticipated_model_matches(next))?;
            bound(
                &*entry,
                current,
                "@operation",
                "begin_rollback",
                "preflight",
            )?;
            next.state = OperationState::RollingBack;
            Ok(TransitionEffect::participant(
                kind,
                &proof.value().member_id,
            ))
        }
        ParticipantTransition::RecordHaltedOutcomeAndBeginPreservation(proof, entry) => {
            require(current.record().state == OperationState::Halted)?;
            record_outcome(current, next, &proof, true)?;
            require(entry.anticipated_model_matches(next))?;
            bound(
                &*entry,
                current,
                "@operation",
                "begin_preservation",
                "preflight",
            )?;
            next.state = OperationState::Preserving;
            Ok(TransitionEffect::participant(
                kind,
                &proof.value().member_id,
            ))
        }
        ParticipantTransition::AbandonNotStartedAndBeginRollback(proof, entry) => {
            abandon(current, next, &proof)?;
            require(entry.origin() == RollbackEntryOrigin::Direct)?;
            require(entry.anticipated_model_matches(next))?;
            bound(
                &*entry,
                current,
                "@operation",
                "begin_rollback",
                "preflight",
            )?;
            next.state = OperationState::RollingBack;
            Ok(TransitionEffect::participant(kind, proof.value()))
        }
        ParticipantTransition::AbandonNotStartedAndBeginPreservation(proof, entry) => {
            abandon(current, next, &proof)?;
            require(entry.anticipated_model_matches(next))?;
            bound(
                &*entry,
                current,
                "@operation",
                "begin_preservation",
                "preflight",
            )?;
            next.state = OperationState::Preserving;
            Ok(TransitionEffect::participant(kind, proof.value()))
        }
        ParticipantTransition::RecordPreparationFailureAndHalt(batch) => {
            failure_batch(current, next, batch.value(), false, false)?;
            bound(
                &*batch,
                current,
                &batch.value().member_id,
                "preparation_failure",
                "verified",
            )?;
            Ok(failure_effect(kind, batch.value()))
        }
        ParticipantTransition::RecordOwnedRetryFailureAndHalt(batch) => {
            failure_batch(current, next, batch.value(), true, false)?;
            bound(
                &*batch,
                current,
                &batch.value().member_id,
                "owned_retry_failure",
                "verified",
            )?;
            Ok(failure_effect(kind, batch.value()))
        }
        ParticipantTransition::RecordOwnedResolutionFailureAndHalt(batch) => {
            failure_batch(current, next, batch.value(), true, true)?;
            bound(
                &*batch,
                current,
                &batch.value().member_id,
                "owned_resolution_failure",
                "verified",
            )?;
            Ok(failure_effect(kind, batch.value()))
        }
        ParticipantTransition::RecordNoMutationAbort(proof) => {
            let member_id = proof.value();
            bound(
                &*proof,
                current,
                member_id,
                "record_no_mutation_abort",
                "cursor_verified",
            )?;
            require(current.record().state == OperationState::RollingBack)?;
            let row = next.participants.get_mut(member_id).ok_or_else(rejected)?;
            require(matches!(
                row.state,
                ParticipantState::Planned
                    | ParticipantState::UpToDate
                    | ParticipantState::Failed
                    | ParticipantState::Unattempted
            ))?;
            row.state = ParticipantState::Aborted;
            Ok(TransitionEffect::participant(kind, member_id))
        }
    }
}

fn record_outcome(
    current: &StoredV1Record,
    next: &mut MergeOperationRecordV1,
    proof: &VerifiedParticipantOutcome,
    halted: bool,
) -> ModelResult<()> {
    let payload = proof.value();
    require(matches!(
        current.record().state,
        OperationState::Executing | OperationState::Halted
    ))?;
    require(!halted || current.record().state == OperationState::Halted)?;
    bound(
        proof,
        current,
        &payload.member_id,
        "participant_outcome",
        "completed",
    )?;
    require(
        participant_row(current.record(), &payload.member_id)?
            .pending_action
            .is_some(),
    )?;
    require(payload.row.pending_action.is_none() && successful_or_conflicted(payload.row.state))?;
    next.participants
        .insert(payload.member_id.clone(), payload.row.clone());
    Ok(())
}

fn abandon(
    current: &StoredV1Record,
    next: &mut MergeOperationRecordV1,
    proof: &VerifiedParticipantNotStarted,
) -> ModelResult<()> {
    let member_id = proof.value();
    require(matches!(
        current.record().state,
        OperationState::Executing | OperationState::Halted
    ))?;
    bound(
        proof,
        current,
        member_id,
        "participant_action",
        "not_started",
    )?;
    let row = next.participants.get_mut(member_id).ok_or_else(rejected)?;
    require(row.pending_action.is_some())?;
    row.pending_action = None;
    Ok(())
}

fn failure_batch(
    current: &StoredV1Record,
    next: &mut MergeOperationRecordV1,
    payload: &ParticipantFailurePayload,
    pending_required: bool,
    resolution: bool,
) -> ModelResult<()> {
    require(current.record().state == OperationState::Executing)?;
    let old = participant_row(current.record(), &payload.member_id)?;
    require(old.pending_action.is_some() == pending_required && payload.row.error.is_some())?;
    require(if resolution {
        old.state == ParticipantState::Conflicted
            && payload.row.state == ParticipantState::Conflicted
            && payload.row.pending_action == old.pending_action
    } else {
        payload.row.state == ParticipantState::Failed
            && payload.row.pending_action == old.pending_action
            && (pending_required || payload.row.pending_action.is_none())
    })?;
    require(exact_later_planned(
        current.record(),
        &payload.member_id,
        &payload.later_unattempted,
    ))?;
    next.participants
        .insert(payload.member_id.clone(), payload.row.clone());
    for member_id in &payload.later_unattempted {
        next.participants
            .get_mut(member_id)
            .ok_or_else(rejected)?
            .state = ParticipantState::Unattempted;
    }
    next.state = OperationState::Halted;
    Ok(())
}

fn exact_later_planned(record: &MergeOperationRecordV1, primary: &str, actual: &[String]) -> bool {
    let Some(position) = record
        .selected_targets
        .iter()
        .position(|member| member == primary)
    else {
        return false;
    };
    let expected = record.selected_targets[position + 1..]
        .iter()
        .filter(|member| {
            record
                .participants
                .get(*member)
                .is_some_and(|row| row.state == ParticipantState::Planned)
        })
        .cloned()
        .collect::<Vec<_>>();
    actual == expected
}

fn failure_effect(kind: EffectKind, payload: &ParticipantFailurePayload) -> TransitionEffect {
    TransitionEffect::failure(kind, &payload.member_id, payload.later_unattempted.clone())
}

fn successful_or_conflicted(state: ParticipantState) -> bool {
    matches!(
        state,
        ParticipantState::UpToDate
            | ParticipantState::FastForwarded
            | ParticipantState::Merged
            | ParticipantState::Conflicted
            | ParticipantState::Continued
    )
}

use std::collections::BTreeSet;

use super::effect::{EffectKind, EffectSubject, TransitionEffect, bad_subject};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::model::v1::{
    MergeOperationRecordV1, PendingPreservationActionV1, PreservationOwnerV1,
    PreservationRefResetPhaseV1, PreservationStashPhaseV1,
};

mod diff;

use diff::known_diff;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ParticipantField {
    PendingAction,
    Outcome,
    Error,
    Preservation,
    Drift,
}
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum PublicationField {
    Decision,
    Candidate,
    Evidence,
    Step,
    Preservation,
    EvidenceRollback,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum KnownField {
    WriterVersion,
    OperationState,
    RecoveryContext,
    Acceptance,
    PendingRollback,
    PendingPreservation,
    PreservationPublicationHandoff,
    Participant {
        member_id: String,
        field: ParticipantField,
    },
    Publication(PublicationField),
    OperationDrift,
}

pub(super) fn verify(
    effect: &TransitionEffect,
    old: &MergeOperationRecordV1,
    new: &MergeOperationRecordV1,
) -> ModelResult<()> {
    let mut actual = known_diff(old, new)?;
    mark(
        &mut actual,
        old.preservation_publication_handoff != new.preservation_publication_handoff,
        KnownField::PreservationPublicationHandoff,
    );
    actual.remove(&KnownField::WriterVersion);
    let expected = expected_diff(effect.kind, &effect.subject, old, new)?;
    if actual == expected {
        Ok(())
    } else {
        Err(rejected(format!(
            "transition effect mismatch: actual={actual:?}, expected={expected:?}"
        )))
    }
}

fn expected_diff(
    kind: EffectKind,
    subject: &EffectSubject,
    old: &MergeOperationRecordV1,
    new: &MergeOperationRecordV1,
) -> ModelResult<BTreeSet<KnownField>> {
    use EffectKind as K;
    use KnownField as F;
    use ParticipantField as P;
    use PublicationField as U;
    match (kind, subject) {
        (
            K::BeginExecution
            | K::AwaitResolution
            | K::Halt
            | K::EnterFinalizing
            | K::BeginRollback
            | K::CompleteOperation
            | K::AbortOperation,
            EffectSubject::None,
        ) => Ok(fields([F::OperationState])),
        (K::BeginPreservation, EffectSubject::None) => Ok(fields([
            F::OperationState,
            F::PreservationPublicationHandoff,
        ])),
        (K::PrepareParticipant, EffectSubject::Participant(member_id)) => {
            Ok(fields([participant(member_id, P::PendingAction)]))
        }
        (K::RecordParticipantOutcome, EffectSubject::Participant(member_id)) => {
            participant_result_with_error_retirement(old, new, member_id)
        }
        (
            K::RecordHaltedOutcomeAndResumeExecution
            | K::RecordHaltedOutcomeAndBeginRollback
            | K::RecordHaltedOutcomeAndBeginPreservation,
            EffectSubject::Participant(member_id),
        ) => {
            let mut fields = participant_result_with_error_retirement(old, new, member_id)?;
            if kind == K::RecordHaltedOutcomeAndBeginPreservation {
                fields.insert(F::PreservationPublicationHandoff);
            }
            Ok(fields)
        }
        (
            K::AbandonNotStartedAndBeginRollback | K::AbandonNotStartedAndBeginPreservation,
            EffectSubject::Participant(member_id),
        ) => {
            let mut fields = fields([participant(member_id, P::PendingAction), F::OperationState]);
            if kind == K::AbandonNotStartedAndBeginPreservation {
                fields.insert(F::PreservationPublicationHandoff);
            }
            Ok(fields)
        }
        (
            K::RecordPreparationFailureAndHalt | K::RecordOwnedRetryFailureAndHalt,
            EffectSubject::Failure {
                primary_member_id,
                later_unattempted,
            },
        ) => failure_fields(old, new, primary_member_id, later_unattempted, true),
        (
            K::RecordOwnedResolutionFailureAndHalt,
            EffectSubject::Failure {
                primary_member_id,
                later_unattempted,
            },
        ) => failure_fields(old, new, primary_member_id, later_unattempted, false),
        (K::RecordNoMutationAbort, EffectSubject::Participant(member_id)) => {
            Ok(fields([participant(member_id, P::Outcome)]))
        }
        (K::FreezeAcceptance, EffectSubject::None) => Ok(fields([F::Acceptance])),
        (K::ClassifyPublicationRequired | K::ClassifyNoPublication, EffectSubject::None) => {
            Ok(fields([F::Publication(U::Decision)]))
        }
        (
            K::BeginMigratedValidation
            | K::ClassifyMigratedPublicationRequired
            | K::ClassifyMigratedNoPublication
            | K::BeginEvidence
            | K::BeginCandidatePublication
            | K::RecordCandidatePublished
            | K::RecordPublicationVerified,
            EffectSubject::None,
        ) => Ok(fields([F::Publication(U::Step)])),
        (K::RecordCandidate, EffectSubject::None) => Ok(fields([F::Publication(U::Candidate)])),
        (K::RecordEvidence, EffectSubject::None) => Ok(fields([F::Publication(U::Evidence)])),
        (K::EnterRecovery | K::ResumeRecovery, EffectSubject::None) => {
            Ok(fields([F::OperationState, F::RecoveryContext]))
        }
        (
            K::BeginBackupRef
            | K::FinishBackupRef
            | K::BeginStash
            | K::AdvanceStash
            | K::FinishStash
            | K::BeginResetAttachedRef
            | K::AdvanceResetAttachedRef
            | K::FinishResetAttachedRef,
            EffectSubject::Preservation(owner),
        ) => preservation_diff(kind, owner, old, new),
        (K::BeginParticipantRollback, EffectSubject::Participant(_))
        | (
            K::BeginEvidenceRollback
            | K::AdvanceEvidenceRollback
            | K::BeginSelectedRootRollback
            | K::AdvanceSelectedRootRollback
            | K::FinishSelectedRootRollback,
            EffectSubject::None,
        ) => Ok(fields([F::PendingRollback])),
        (K::FinishParticipantRollback, EffectSubject::Participant(member_id)) => {
            rollback_participant_result(old, new, member_id)
        }
        (K::FinishEvidenceRollback, EffectSubject::None) => Ok(fields([
            F::PendingRollback,
            F::Publication(U::EvidenceRollback),
        ])),
        (
            K::RecordParticipantDrift | K::ClearParticipantDrift,
            EffectSubject::ParticipantDrift { member_id, .. },
        ) => Ok(fields([participant(member_id, P::Drift)])),
        (K::RecordOperationDrift | K::ClearOperationDrift, EffectSubject::OperationDrift(_)) => {
            Ok(fields([F::OperationDrift]))
        }
        _ => Err(bad_subject()),
    }
}

fn preservation_diff(
    kind: EffectKind,
    owner: &PreservationOwnerV1,
    old: &MergeOperationRecordV1,
    new: &MergeOperationRecordV1,
) -> ModelResult<BTreeSet<KnownField>> {
    use EffectKind as K;
    use KnownField as F;
    let mut expected = fields([F::PendingPreservation]);
    let before = old.pending_preservation.as_ref();
    let after = new.pending_preservation.as_ref();
    let valid_shape = match kind {
        K::BeginBackupRef => {
            before.is_none()
                && matches!(after, Some(PendingPreservationActionV1::BackupRef { owner: actual, .. }) if actual == owner)
        }
        K::FinishBackupRef => {
            expected.insert(preservation_owner(owner));
            matches!(before, Some(PendingPreservationActionV1::BackupRef { owner: actual, .. }) if actual == owner)
                && after.is_none()
        }
        K::BeginStash => {
            before.is_none()
                && matches!(after, Some(PendingPreservationActionV1::Stash { owner: actual, .. }) if actual == owner)
        }
        K::AdvanceStash => {
            if matches!(
                before,
                Some(PendingPreservationActionV1::Stash {
                    phase: PreservationStashPhaseV1::CreateStash,
                    ..
                })
            ) {
                expected.insert(preservation_owner(owner));
            }
            matches!((before, after),
                (Some(PendingPreservationActionV1::Stash { owner: prior, .. }), Some(PendingPreservationActionV1::Stash { owner: next, .. }))
                if prior == owner && next == owner)
        }
        K::FinishStash => {
            matches!(before,
            Some(PendingPreservationActionV1::Stash { owner: actual, phase: PreservationStashPhaseV1::Complete, .. }) if actual == owner)
                && after.is_none()
        }
        K::BeginResetAttachedRef => {
            before.is_none()
                && matches!(after, Some(PendingPreservationActionV1::ResetAttachedRef { owner: actual, .. }) if actual == owner)
        }
        K::AdvanceResetAttachedRef => matches!((before, after),
            (Some(PendingPreservationActionV1::ResetAttachedRef { owner: prior, .. }), Some(PendingPreservationActionV1::ResetAttachedRef { owner: next, .. }))
            if prior == owner && next == owner),
        K::FinishResetAttachedRef => {
            matches!(before,
            Some(PendingPreservationActionV1::ResetAttachedRef { owner: actual, phase: PreservationRefResetPhaseV1::Complete, .. }) if actual == owner)
                && after.is_none()
        }
        _ => return Err(bad_subject()),
    };
    if valid_shape {
        Ok(expected)
    } else {
        Err(rejected(
            "preservation effect does not match its typed owner or phase",
        ))
    }
}

fn participant_result_with_error_retirement(
    old: &MergeOperationRecordV1,
    new: &MergeOperationRecordV1,
    member_id: &str,
) -> ModelResult<BTreeSet<KnownField>> {
    let mut result = participant_result(member_id, old.state != new.state);
    let before = old.participants.get(member_id).ok_or_else(bad_subject)?;
    let after = new.participants.get(member_id).ok_or_else(bad_subject)?;
    if before.error != after.error {
        if before.error.is_some() && after.error.is_none() {
            result.insert(participant(member_id, ParticipantField::Error));
        } else {
            return Err(rejected(
                "participant outcome may only clear the participant's existing error",
            ));
        }
    }
    Ok(result)
}

fn rollback_participant_result(
    old: &MergeOperationRecordV1,
    new: &MergeOperationRecordV1,
    member_id: &str,
) -> ModelResult<BTreeSet<KnownField>> {
    use KnownField as F;
    use ParticipantField as P;
    let before = old.participants.get(member_id).ok_or_else(bad_subject)?;
    let after = new.participants.get(member_id).ok_or_else(bad_subject)?;
    let mut result = fields([F::PendingRollback, participant(member_id, P::Outcome)]);
    if before.error != after.error {
        if before.error.is_some() && after.error.is_none() {
            result.insert(participant(member_id, P::Error));
        } else {
            return Err(rejected(
                "participant rollback may only clear the participant's existing error",
            ));
        }
    }
    Ok(result)
}

fn failure_fields(
    old: &MergeOperationRecordV1,
    new: &MergeOperationRecordV1,
    primary_member_id: &str,
    later_unattempted: &[String],
    allow_outcome: bool,
) -> ModelResult<BTreeSet<KnownField>> {
    use KnownField as F;
    use ParticipantField as P;
    let before = old
        .participants
        .get(primary_member_id)
        .ok_or_else(bad_subject)?;
    let after = new
        .participants
        .get(primary_member_id)
        .ok_or_else(bad_subject)?;
    let mut result = fields([F::OperationState]);
    if before.error != after.error {
        result.insert(participant(primary_member_id, P::Error));
    }
    if before.state != after.state
        || before.resulting_commit != after.resulting_commit
        || before.expected_merge_head != after.expected_merge_head
        || before.conflict_paths != after.conflict_paths
        || before.conflict_snapshot != after.conflict_snapshot
    {
        if !allow_outcome {
            return Err(rejected(
                "owned resolution failure may not change participant outcome fields",
            ));
        }
        result.insert(participant(primary_member_id, P::Outcome));
    }
    result.extend(
        later_unattempted
            .iter()
            .map(|member| participant(member, P::Outcome)),
    );
    Ok(result)
}

fn participant_result(member: &str, state: bool) -> BTreeSet<KnownField> {
    use KnownField as F;
    use ParticipantField as P;
    let mut result = fields([
        participant(member, P::PendingAction),
        participant(member, P::Outcome),
    ]);
    if state {
        result.insert(F::OperationState);
    }
    result
}

fn preservation_owner(owner: &PreservationOwnerV1) -> KnownField {
    match owner {
        PreservationOwnerV1::Participant { member_id } => {
            participant(member_id, ParticipantField::Preservation)
        }
        PreservationOwnerV1::PublicationRoot => {
            KnownField::Publication(PublicationField::Preservation)
        }
    }
}

fn participant(member_id: &str, field: ParticipantField) -> KnownField {
    KnownField::Participant {
        member_id: member_id.into(),
        field,
    }
}

fn fields<const N: usize>(items: [KnownField; N]) -> BTreeSet<KnownField> {
    items.into_iter().collect()
}

fn mark(changed: &mut BTreeSet<KnownField>, condition: bool, field: KnownField) {
    if condition {
        changed.insert(field);
    }
}

fn rejected(detail: impl Into<String>) -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecoveryRequired,
        format!("v1 transition effect rejected: {}", detail.into()),
    )
}

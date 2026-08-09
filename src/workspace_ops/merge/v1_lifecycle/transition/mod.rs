mod effect;
mod footprint;
mod reduce;

pub(super) use effect::{EFFECT_VARIANT_COUNT, EffectKind, RetiredContainer, TransitionEffect};

#[cfg(test)]
pub(super) const TRANSITION_VARIANT_COUNT: usize = 53;

use super::super::model::v1::{MergeOperationRecordV1, ValidatedV1Record};
use super::authority::*;
use super::checked::{RecordDigest, StoredV1Record, V1MutationLease};
use crate::model::{ErrorCode, ModelError, ModelResult};

pub(super) struct PreparedV1Rewrite {
    base_digest: RecordDigest,
    next: ValidatedV1Record,
    effect: TransitionEffect,
}

impl PreparedV1Rewrite {
    pub(super) fn base_digest(&self) -> RecordDigest {
        self.base_digest
    }

    pub(super) fn next(&self) -> &MergeOperationRecordV1 {
        self.next.record()
    }

    pub(super) fn effect(&self) -> &TransitionEffect {
        &self.effect
    }
}

pub(super) fn prepare(
    lease: &V1MutationLease,
    current: &StoredV1Record,
    transition: V1Transition,
) -> ModelResult<PreparedV1Rewrite> {
    if !lease.covers(current.location()) {
        return Err(transition_error(
            "mutation lease does not cover the checked record",
        ));
    }
    let base_digest = current.source_digest();
    let effect_kind = transition.effect_kind();
    let (next, effect) = reduce::apply(current, transition, effect_kind)?;
    effect.verify_known_diff(current.record(), next.record())?;
    #[cfg(test)]
    super::tests::predecessor_matrix::record_effect(
        effect_kind,
        current.record(),
        next.record(),
        &effect,
    );
    Ok(PreparedV1Rewrite {
        base_digest,
        next,
        effect,
    })
}

#[cfg(test)]
pub(super) fn prepared_for_store_matrix(
    current: &StoredV1Record,
    next: MergeOperationRecordV1,
    effect: TransitionEffect,
) -> ModelResult<PreparedV1Rewrite> {
    effect.verify_known_diff(current.record(), &next)?;
    Ok(PreparedV1Rewrite {
        base_digest: current.source_digest(),
        next: crate::workspace_ops::merge::model::v1::validate_v1_record(next)?,
        effect,
    })
}

fn transition_error(detail: impl Into<String>) -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecoveryRequired,
        format!("v1 transition rejected: {}", detail.into()),
    )
}

pub(super) enum V1Transition {
    Operation(Box<OperationTransition>),
    Participant(Box<ParticipantTransition>),
    Acceptance(Box<AcceptanceTransition>),
    Publication(Box<PublicationTransition>),
    Recovery(Box<RecoveryTransition>),
    Preservation(Box<PreservationTransition>),
    Rollback(Box<RollbackTransition>),
    Drift(Box<DriftTransition>),
}

impl V1Transition {
    pub(super) fn effect_kind(&self) -> EffectKind {
        match self {
            Self::Operation(value) => value.effect_kind(),
            Self::Participant(value) => value.effect_kind(),
            Self::Acceptance(value) => value.effect_kind(),
            Self::Publication(value) => value.effect_kind(),
            Self::Recovery(value) => value.effect_kind(),
            Self::Preservation(value) => value.effect_kind(),
            Self::Rollback(value) => value.effect_kind(),
            Self::Drift(value) => value.effect_kind(),
        }
    }
}

pub(super) enum OperationTransition {
    BeginExecution,
    AwaitResolution,
    Halt,
    EnterFinalizing(VerifiedParticipants),
    BeginPreservation(Box<PreparedPreservationEntry>),
    BeginRollback(Box<PreparedRollbackEntry>),
    CompleteOperation(VerifiedPublicationCompletion),
    AbortOperation(VerifiedRollbackExhausted),
}

impl OperationTransition {
    fn effect_kind(&self) -> EffectKind {
        match self {
            Self::BeginExecution => EffectKind::BeginExecution,
            Self::AwaitResolution => EffectKind::AwaitResolution,
            Self::Halt => EffectKind::Halt,
            Self::EnterFinalizing(_) => EffectKind::EnterFinalizing,
            Self::BeginPreservation(_) => EffectKind::BeginPreservation,
            Self::BeginRollback(_) => EffectKind::BeginRollback,
            Self::CompleteOperation(_) => EffectKind::CompleteOperation,
            Self::AbortOperation(_) => EffectKind::AbortOperation,
        }
    }
}

pub(super) enum ParticipantTransition {
    Prepare(Box<PreparedParticipantAction>),
    RecordOutcome(Box<VerifiedParticipantOutcome>),
    RecordHaltedOutcomeAndResumeExecution(Box<VerifiedParticipantOutcome>),
    RecordHaltedOutcomeAndBeginRollback(
        Box<VerifiedParticipantOutcome>,
        Box<PreparedRollbackEntry>,
    ),
    RecordHaltedOutcomeAndBeginPreservation(
        Box<VerifiedParticipantOutcome>,
        Box<PreparedPreservationEntry>,
    ),
    AbandonNotStartedAndBeginRollback(
        Box<VerifiedParticipantNotStarted>,
        Box<PreparedRollbackEntry>,
    ),
    AbandonNotStartedAndBeginPreservation(
        Box<VerifiedParticipantNotStarted>,
        Box<PreparedPreservationEntry>,
    ),
    RecordPreparationFailureAndHalt(Box<PreparedFailureHaltBatch>),
    RecordOwnedRetryFailureAndHalt(Box<BoundOwnedRetryFailureHaltBatch>),
    RecordOwnedResolutionFailureAndHalt(Box<BoundOwnedResolutionFailureHaltBatch>),
    RecordNoMutationAbort(Box<VerifiedNoMutationAbort>),
}

impl ParticipantTransition {
    fn effect_kind(&self) -> EffectKind {
        match self {
            Self::Prepare(_) => EffectKind::PrepareParticipant,
            Self::RecordOutcome(_) => EffectKind::RecordParticipantOutcome,
            Self::RecordHaltedOutcomeAndResumeExecution(_) => {
                EffectKind::RecordHaltedOutcomeAndResumeExecution
            }
            Self::RecordHaltedOutcomeAndBeginRollback(_, _) => {
                EffectKind::RecordHaltedOutcomeAndBeginRollback
            }
            Self::RecordHaltedOutcomeAndBeginPreservation(_, _) => {
                EffectKind::RecordHaltedOutcomeAndBeginPreservation
            }
            Self::AbandonNotStartedAndBeginRollback(_, _) => {
                EffectKind::AbandonNotStartedAndBeginRollback
            }
            Self::AbandonNotStartedAndBeginPreservation(_, _) => {
                EffectKind::AbandonNotStartedAndBeginPreservation
            }
            Self::RecordPreparationFailureAndHalt(_) => EffectKind::RecordPreparationFailureAndHalt,
            Self::RecordOwnedRetryFailureAndHalt(_) => EffectKind::RecordOwnedRetryFailureAndHalt,
            Self::RecordOwnedResolutionFailureAndHalt(_) => {
                EffectKind::RecordOwnedResolutionFailureAndHalt
            }
            Self::RecordNoMutationAbort(_) => EffectKind::RecordNoMutationAbort,
        }
    }
}

pub(super) enum AcceptanceTransition {
    Freeze(Box<PreparedAcceptedWorkspace>),
}

impl AcceptanceTransition {
    fn effect_kind(&self) -> EffectKind {
        match self {
            Self::Freeze(_) => EffectKind::FreezeAcceptance,
        }
    }
}

pub(super) enum PublicationTransition {
    ClassifyRequired(BoundPublicationDecision),
    ClassifyNone(BoundPublicationDecision),
    BeginMigratedValidation,
    ClassifyMigratedRequired(VerifiedResults),
    ClassifyMigratedNone(VerifiedResults),
    RecordCandidate(Box<PreparedCandidate>),
    BeginEvidence(PreparedEvidenceIntent),
    RecordEvidence(Box<VerifiedEvidenceResult>),
    BeginCandidatePublication(PreparedPublicationIntent),
    RecordCandidatePublished(VerifiedCandidatePublicationCompletion),
    RecordPublicationVerified(VerifiedPublicationCompletion),
}

impl PublicationTransition {
    fn effect_kind(&self) -> EffectKind {
        match self {
            Self::ClassifyRequired(_) => EffectKind::ClassifyPublicationRequired,
            Self::ClassifyNone(_) => EffectKind::ClassifyNoPublication,
            Self::BeginMigratedValidation => EffectKind::BeginMigratedValidation,
            Self::ClassifyMigratedRequired(_) => EffectKind::ClassifyMigratedPublicationRequired,
            Self::ClassifyMigratedNone(_) => EffectKind::ClassifyMigratedNoPublication,
            Self::RecordCandidate(_) => EffectKind::RecordCandidate,
            Self::BeginEvidence(_) => EffectKind::BeginEvidence,
            Self::RecordEvidence(_) => EffectKind::RecordEvidence,
            Self::BeginCandidatePublication(_) => EffectKind::BeginCandidatePublication,
            Self::RecordCandidatePublished(_) => EffectKind::RecordCandidatePublished,
            Self::RecordPublicationVerified(_) => EffectKind::RecordPublicationVerified,
        }
    }
}

pub(super) enum RecoveryTransition {
    Enter(BoundAmbiguityEvidence),
    Resume(VerifiedRecoveryOrigin),
}

impl RecoveryTransition {
    fn effect_kind(&self) -> EffectKind {
        match self {
            Self::Enter(_) => EffectKind::EnterRecovery,
            Self::Resume(_) => EffectKind::ResumeRecovery,
        }
    }
}

pub(super) enum PreservationTransition {
    BeginBackupRef(Box<PreparedBackupRefIntent>),
    FinishBackupRef(Box<VerifiedBackupRef>),
    BeginStash(Box<PreparedStashIntent>),
    AdvanceStash(Box<VerifiedStashPhase>),
    FinishStash(Box<VerifiedStashCompletion>),
    BeginResetAttachedRef(Box<PreparedRefResetIntent>),
    AdvanceResetAttachedRef(Box<VerifiedRefResetPhase>),
    FinishResetAttachedRef(Box<VerifiedRefResetCompletion>),
}

impl PreservationTransition {
    fn effect_kind(&self) -> EffectKind {
        match self {
            Self::BeginBackupRef(_) => EffectKind::BeginBackupRef,
            Self::FinishBackupRef(_) => EffectKind::FinishBackupRef,
            Self::BeginStash(_) => EffectKind::BeginStash,
            Self::AdvanceStash(_) => EffectKind::AdvanceStash,
            Self::FinishStash(_) => EffectKind::FinishStash,
            Self::BeginResetAttachedRef(_) => EffectKind::BeginResetAttachedRef,
            Self::AdvanceResetAttachedRef(_) => EffectKind::AdvanceResetAttachedRef,
            Self::FinishResetAttachedRef(_) => EffectKind::FinishResetAttachedRef,
        }
    }
}

pub(super) enum RollbackTransition {
    BeginParticipant(Box<PreparedParticipantRollback>),
    FinishParticipant(Box<VerifiedParticipantRollback>),
    BeginEvidence(Box<PreparedEvidenceRollback>),
    AdvanceEvidence(Box<VerifiedEvidenceRollbackStep>),
    FinishEvidence(VerifiedEvidenceRollbackCompletion),
    BeginSelectedRoot(Box<PreparedRootMetadataRollback>),
    AdvanceSelectedRoot(Box<VerifiedRootMetadataRollbackStep>),
    FinishSelectedRoot(VerifiedRootMetadataRollbackCompletion),
}

impl RollbackTransition {
    fn effect_kind(&self) -> EffectKind {
        match self {
            Self::BeginParticipant(_) => EffectKind::BeginParticipantRollback,
            Self::FinishParticipant(_) => EffectKind::FinishParticipantRollback,
            Self::BeginEvidence(_) => EffectKind::BeginEvidenceRollback,
            Self::AdvanceEvidence(_) => EffectKind::AdvanceEvidenceRollback,
            Self::FinishEvidence(_) => EffectKind::FinishEvidenceRollback,
            Self::BeginSelectedRoot(_) => EffectKind::BeginSelectedRootRollback,
            Self::AdvanceSelectedRoot(_) => EffectKind::AdvanceSelectedRootRollback,
            Self::FinishSelectedRoot(_) => EffectKind::FinishSelectedRootRollback,
        }
    }
}

pub(super) enum DriftTransition {
    RecordParticipant(Box<BoundParticipantDrift>),
    ClearParticipant(Box<VerifiedParticipantDriftClear>),
    RecordOperation(BoundOperationDrift),
    ClearOperation(VerifiedOperationDriftClear),
}

impl DriftTransition {
    fn effect_kind(&self) -> EffectKind {
        match self {
            Self::RecordParticipant(_) => EffectKind::RecordParticipantDrift,
            Self::ClearParticipant(_) => EffectKind::ClearParticipantDrift,
            Self::RecordOperation(_) => EffectKind::RecordOperationDrift,
            Self::ClearOperation(_) => EffectKind::ClearOperationDrift,
        }
    }
}

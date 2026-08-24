use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::OperationDriftKind;
#[cfg(test)]
use crate::workspace_ops::merge::ParticipantDriftKind;
use crate::workspace_ops::merge::model::v1::{MergeOperationRecordV1, PreservationOwnerV1};

use super::super::authority::ParticipantDriftIdentity;
use super::footprint;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge::v1_lifecycle) enum RetiredContainer {
    RecoveryContext,
    ParticipantPendingAction(String),
    ParticipantConflictEvidence(String),
    ParticipantError(String),
    PendingRollback,
    PendingPreservation,
    ParticipantDrift {
        member_id: String,
        identity: ParticipantDriftIdentity,
    },
    OperationDrift(OperationDriftKind),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge::v1_lifecycle) enum EffectKind {
    BeginExecution,
    AwaitResolution,
    Halt,
    EnterFinalizing,
    BeginPreservation,
    BeginRollback,
    CompleteOperation,
    AbortOperation,
    PrepareParticipant,
    RecordParticipantOutcome,
    RecordHaltedOutcomeAndResumeExecution,
    RecordHaltedOutcomeAndBeginRollback,
    RecordHaltedOutcomeAndBeginPreservation,
    AbandonNotStartedAndBeginRollback,
    AbandonNotStartedAndBeginPreservation,
    RecordPreparationFailureAndHalt,
    RecordOwnedRetryFailureAndHalt,
    RecordOwnedResolutionFailureAndHalt,
    RecordNoMutationAbort,
    FreezeAcceptance,
    ClassifyPublicationRequired,
    ClassifyNoPublication,
    BeginMigratedValidation,
    ClassifyMigratedPublicationRequired,
    ClassifyMigratedNoPublication,
    RecordCandidate,
    BeginEvidence,
    RecordEvidence,
    BeginCandidatePublication,
    RecordCandidatePublished,
    RecordPublicationVerified,
    EnterRecovery,
    ResumeRecovery,
    BeginBackupRef,
    FinishBackupRef,
    BeginStash,
    AdvanceStash,
    FinishStash,
    BeginResetAttachedRef,
    AdvanceResetAttachedRef,
    FinishResetAttachedRef,
    RecordArtifactNoop,
    RecordResetNoop,
    BeginParticipantRollback,
    FinishParticipantRollback,
    BeginEvidenceRollback,
    AdvanceEvidenceRollback,
    FinishEvidenceRollback,
    BeginSelectedRootRollback,
    AdvanceSelectedRootRollback,
    FinishSelectedRootRollback,
    RecordParticipantDrift,
    ClearParticipantDrift,
    RecordOperationDrift,
    ClearOperationDrift,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum EffectSubject {
    None,
    Participant(String),
    Failure {
        primary_member_id: String,
        later_unattempted: Vec<String>,
    },
    Preservation(PreservationOwnerV1),
    ParticipantDrift {
        member_id: String,
        identity: ParticipantDriftIdentity,
    },
    OperationDrift(OperationDriftKind),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::workspace_ops::merge::v1_lifecycle) struct TransitionEffect {
    pub(super) kind: EffectKind,
    pub(super) subject: EffectSubject,
}

impl TransitionEffect {
    pub(super) fn operation(kind: EffectKind) -> Self {
        Self {
            kind,
            subject: EffectSubject::None,
        }
    }

    pub(super) fn participant(kind: EffectKind, member_id: impl Into<String>) -> Self {
        Self {
            kind,
            subject: EffectSubject::Participant(member_id.into()),
        }
    }

    pub(super) fn failure(
        kind: EffectKind,
        primary_member_id: impl Into<String>,
        later_unattempted: Vec<String>,
    ) -> Self {
        Self {
            kind,
            subject: EffectSubject::Failure {
                primary_member_id: primary_member_id.into(),
                later_unattempted,
            },
        }
    }

    pub(super) fn preservation(kind: EffectKind, owner: PreservationOwnerV1) -> Self {
        Self {
            kind,
            subject: EffectSubject::Preservation(owner),
        }
    }

    pub(super) fn participant_drift(
        kind: EffectKind,
        member_id: impl Into<String>,
        identity: ParticipantDriftIdentity,
    ) -> Self {
        Self {
            kind,
            subject: EffectSubject::ParticipantDrift {
                member_id: member_id.into(),
                identity,
            },
        }
    }

    pub(super) fn operation_drift(kind: EffectKind, drift_kind: OperationDriftKind) -> Self {
        Self {
            kind,
            subject: EffectSubject::OperationDrift(drift_kind),
        }
    }

    pub(in crate::workspace_ops::merge::v1_lifecycle) fn verify_known_diff(
        &self,
        old: &MergeOperationRecordV1,
        new: &MergeOperationRecordV1,
    ) -> ModelResult<()> {
        if new.writer_version != crate::VERSION {
            return Err(effect_error(
                "transition did not install the current writer version",
            ));
        }
        footprint::verify(self, old, new)
    }

    pub(in crate::workspace_ops::merge::v1_lifecycle) fn retired(
        &self,
    ) -> ModelResult<Vec<RetiredContainer>> {
        use EffectKind as K;
        Ok(match (&self.kind, &self.subject) {
            (
                K::RecordParticipantOutcome
                | K::RecordHaltedOutcomeAndResumeExecution
                | K::RecordHaltedOutcomeAndBeginRollback
                | K::RecordHaltedOutcomeAndBeginPreservation,
                EffectSubject::Participant(member_id),
            ) => vec![
                RetiredContainer::ParticipantPendingAction(member_id.clone()),
                RetiredContainer::ParticipantConflictEvidence(member_id.clone()),
                RetiredContainer::ParticipantError(member_id.clone()),
            ],
            (
                K::AbandonNotStartedAndBeginRollback | K::AbandonNotStartedAndBeginPreservation,
                EffectSubject::Participant(member_id),
            ) => vec![RetiredContainer::ParticipantPendingAction(
                member_id.clone(),
            )],
            (
                K::RecordPreparationFailureAndHalt
                | K::RecordOwnedRetryFailureAndHalt
                | K::RecordOwnedResolutionFailureAndHalt,
                EffectSubject::Failure {
                    primary_member_id, ..
                },
            ) => vec![RetiredContainer::ParticipantError(
                primary_member_id.clone(),
            )],
            (K::ResumeRecovery, EffectSubject::None) => {
                vec![RetiredContainer::RecoveryContext]
            }
            (
                K::FinishBackupRef | K::FinishStash | K::FinishResetAttachedRef,
                EffectSubject::Preservation(_),
            ) => vec![RetiredContainer::PendingPreservation],
            (K::FinishParticipantRollback, EffectSubject::Participant(member_id)) => vec![
                RetiredContainer::PendingRollback,
                RetiredContainer::ParticipantConflictEvidence(member_id.clone()),
                RetiredContainer::ParticipantError(member_id.clone()),
            ],
            (K::FinishEvidenceRollback | K::FinishSelectedRootRollback, EffectSubject::None) => {
                vec![RetiredContainer::PendingRollback]
            }
            (
                K::ClearParticipantDrift,
                EffectSubject::ParticipantDrift {
                    member_id,
                    identity,
                },
            ) => {
                vec![RetiredContainer::ParticipantDrift {
                    member_id: member_id.clone(),
                    identity: identity.clone(),
                }]
            }
            (K::ClearOperationDrift, EffectSubject::OperationDrift(kind)) => {
                vec![RetiredContainer::OperationDrift(*kind)]
            }
            (K::ResumeRecovery, _)
            | (K::FinishBackupRef | K::FinishStash | K::FinishResetAttachedRef, _)
            | (
                K::FinishParticipantRollback
                | K::FinishEvidenceRollback
                | K::FinishSelectedRootRollback,
                _,
            )
            | (K::ClearParticipantDrift | K::ClearOperationDrift, _)
            | (
                K::RecordParticipantOutcome
                | K::RecordHaltedOutcomeAndResumeExecution
                | K::RecordHaltedOutcomeAndBeginRollback
                | K::RecordHaltedOutcomeAndBeginPreservation
                | K::AbandonNotStartedAndBeginRollback
                | K::AbandonNotStartedAndBeginPreservation,
                _,
            ) => return Err(bad_subject()),
            (
                K::RecordPreparationFailureAndHalt
                | K::RecordOwnedRetryFailureAndHalt
                | K::RecordOwnedResolutionFailureAndHalt,
                _,
            ) => return Err(bad_subject()),
            _ => Vec::new(),
        })
    }

    pub(in crate::workspace_ops::merge::v1_lifecycle) fn allows_derived_acceptance_unknowns(
        &self,
    ) -> ModelResult<bool> {
        match (self.kind, &self.subject) {
            (EffectKind::FreezeAcceptance, EffectSubject::None) => Ok(true),
            (EffectKind::FreezeAcceptance, _) => Err(bad_subject()),
            _ => Ok(false),
        }
    }

    #[cfg(test)]
    pub(in crate::workspace_ops::merge::v1_lifecycle) fn operation_for_test(
        kind: EffectKind,
    ) -> Self {
        Self::operation(kind)
    }

    #[cfg(test)]
    pub(in crate::workspace_ops::merge::v1_lifecycle) fn participant_for_test(
        kind: EffectKind,
        member_id: &str,
    ) -> Self {
        Self::participant(kind, member_id)
    }

    #[cfg(test)]
    pub(in crate::workspace_ops::merge::v1_lifecycle) fn participant_drift_for_test(
        kind: EffectKind,
        member_id: &str,
        drift_kind: ParticipantDriftKind,
    ) -> Self {
        let drift = crate::workspace_ops::merge::ParticipantDrift {
            kind: drift_kind,
            message: String::new(),
            expected_branch: None,
            live_branch: None,
            expected_head: None,
            live_head: None,
            expected_merge_head: None,
            live_merge_head: None,
        };
        Self::participant_drift(kind, member_id, ParticipantDriftIdentity::new(&drift, 0))
    }

    #[cfg(test)]
    pub(in crate::workspace_ops::merge::v1_lifecycle) fn failure_for_test(
        kind: EffectKind,
        primary_member_id: &str,
        later_unattempted: &[&str],
    ) -> Self {
        Self::failure(
            kind,
            primary_member_id,
            later_unattempted
                .iter()
                .map(|value| (*value).into())
                .collect(),
        )
    }

    #[cfg(test)]
    pub(in crate::workspace_ops::merge::v1_lifecycle) fn preservation_for_test(
        kind: EffectKind,
        owner: PreservationOwnerV1,
    ) -> Self {
        Self::preservation(kind, owner)
    }
}

pub(super) fn bad_subject() -> ModelError {
    effect_error("transition effect has the wrong typed subject")
}

fn effect_error(detail: &str) -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecoveryRequired,
        format!("v1 transition effect rejected: {detail}"),
    )
}

#[cfg(test)]
pub(in crate::workspace_ops::merge::v1_lifecycle) const EFFECT_VARIANT_COUNT: usize = 53;

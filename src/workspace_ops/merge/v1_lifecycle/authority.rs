use serde::Serialize;

mod binding;
mod dispatcher;
mod drift;
mod resolver;

pub(super) use dispatcher::*;
pub(super) use resolver::*;

use binding::{AuthorityIssuer, BoundValue, payload_hash};
pub(super) use drift::{ParticipantDriftIdentity, ParticipantDriftPayload};

use super::super::model::v1::{
    AcceptedWorkspaceV1, MergeOperationRecordV1, PendingPreservationActionV1,
    PendingRollbackActionV1, PreservationOwnerV1, PreservationRefResetPhaseV1,
    PreservationStashPhaseV1, RecoveryOriginStateV1, RollbackCursor,
};
use super::super::{
    MergeParticipantRecord, OperationDrift, PreservationEvidence, PublicationCandidate,
    PublicationCandidateHash,
};
use super::checked::StoredV1Record;
use crate::model::{ErrorCode, ModelError, ModelResult};

pub(super) trait BoundAuthority {
    fn matches(&self, current: &StoredV1Record, owner: &str, action: &str, phase: &str) -> bool;
}

macro_rules! token {
    ($name:ident, $value:ty) => {
        #[derive(Debug)]
        pub(super) struct $name(BoundValue<$value>);

        impl $name {
            fn issue(
                issuer: &AuthorityIssuer<'_>,
                owner: &str,
                action: &str,
                phase: &str,
                value: $value,
            ) -> ModelResult<Self> {
                Ok(Self(issuer.bind(owner, action, phase, value)?))
            }

            #[cfg(test)]
            #[allow(
                dead_code,
                reason = "observer-owned tokens do not all need forged fixtures"
            )]
            pub(super) fn for_test(
                current: &StoredV1Record,
                owner: &str,
                action: &str,
                phase: &str,
                value: $value,
            ) -> ModelResult<Self> {
                Self::issue(
                    &AuthorityIssuer::for_observer(current),
                    owner,
                    action,
                    phase,
                    value,
                )
            }

            pub(super) fn value(&self) -> &$value {
                &self.0.value
            }
        }

        impl BoundAuthority for $name {
            fn matches(
                &self,
                current: &StoredV1Record,
                owner: &str,
                action: &str,
                phase: &str,
            ) -> bool {
                self.0.matches(current, owner, action, phase)
            }
        }
    };
}

token!(VerifiedParticipants, ());
token!(VerifiedPublicationHandoff, ());
token!(VerifiedPublicationCompletion, ());
#[derive(Debug, Serialize)]
pub(super) struct RollbackExhaustedPayload {
    selected_root_manifest_sha256: Option<String>,
    selected_root_lock_sha256: Option<String>,
}

#[cfg(test)]
impl RollbackExhaustedPayload {
    pub(super) fn empty_for_test() -> Self {
        Self {
            selected_root_manifest_sha256: None,
            selected_root_lock_sha256: None,
        }
    }
}

token!(VerifiedRollbackExhausted, RollbackExhaustedPayload);
token!(VerifiedPreservationExhausted, ());

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(super) enum RollbackEntryOrigin {
    Direct,
    FromPreserving,
}

#[derive(Debug, Serialize)]
struct EntryPayload {
    origin: RollbackEntryOrigin,
    anticipated_model_sha256: [u8; 32],
}

#[derive(Debug)]
pub(super) struct PreparedPreservationEntry {
    bound: BoundValue<EntryPayload>,
    handoff: VerifiedPublicationHandoff,
}

#[derive(Debug)]
pub(super) struct PreparedRollbackEntry {
    bound: BoundValue<EntryPayload>,
    handoff: VerifiedPublicationHandoff,
    preservation_exhausted: Option<VerifiedPreservationExhausted>,
}

macro_rules! entry_authority {
    ($name:ident) => {
        impl $name {
            pub(super) fn anticipated_model_matches(&self, model: &MergeOperationRecordV1) -> bool {
                payload_hash(model)
                    .is_ok_and(|hash| self.bound.value.anticipated_model_sha256 == hash)
            }
        }

        impl BoundAuthority for $name {
            fn matches(
                &self,
                current: &StoredV1Record,
                owner: &str,
                action: &str,
                phase: &str,
            ) -> bool {
                self.bound.matches(current, owner, action, phase)
                    && self
                        .handoff
                        .matches(current, "@publication", "handoff", "verified")
            }
        }
    };
}

entry_authority!(PreparedPreservationEntry);

impl PreparedRollbackEntry {
    pub(super) fn anticipated_model_matches(&self, model: &MergeOperationRecordV1) -> bool {
        payload_hash(model).is_ok_and(|hash| self.bound.value.anticipated_model_sha256 == hash)
    }
}

impl BoundAuthority for PreparedRollbackEntry {
    fn matches(&self, current: &StoredV1Record, owner: &str, action: &str, phase: &str) -> bool {
        let handoff = self
            .handoff
            .matches(current, "@publication", "handoff", "verified");
        let exhaustion = self.preservation_exhausted.as_ref().is_some_and(|proof| {
            proof.matches(current, "@operation", "preservation_exhausted", "verified")
        });
        self.bound.matches(current, owner, action, phase)
            && handoff
            && match self.bound.value.origin {
                RollbackEntryOrigin::Direct => self.preservation_exhausted.is_none(),
                RollbackEntryOrigin::FromPreserving => exhaustion,
            }
    }
}

impl PreparedPreservationEntry {
    #[cfg(test)]
    pub(super) fn for_test(
        current: &StoredV1Record,
        anticipated: &MergeOperationRecordV1,
        handoff: VerifiedPublicationHandoff,
    ) -> ModelResult<Self> {
        Ok(Self {
            bound: BoundValue::new(
                current,
                "@operation",
                "begin_preservation",
                "preflight",
                EntryPayload {
                    origin: RollbackEntryOrigin::Direct,
                    anticipated_model_sha256: payload_hash(anticipated)?,
                },
            )?,
            handoff,
        })
    }
}

impl PreparedRollbackEntry {
    #[cfg(test)]
    pub(super) fn direct_for_test(
        current: &StoredV1Record,
        anticipated: &MergeOperationRecordV1,
        handoff: VerifiedPublicationHandoff,
    ) -> ModelResult<Self> {
        Self::for_test(current, anticipated, handoff, None)
    }

    #[cfg(test)]
    pub(super) fn from_preserving_for_test(
        current: &StoredV1Record,
        anticipated: &MergeOperationRecordV1,
        handoff: VerifiedPublicationHandoff,
        exhausted: VerifiedPreservationExhausted,
    ) -> ModelResult<Self> {
        Self::for_test(current, anticipated, handoff, Some(exhausted))
    }

    #[cfg(test)]
    fn for_test(
        current: &StoredV1Record,
        anticipated: &MergeOperationRecordV1,
        handoff: VerifiedPublicationHandoff,
        preservation_exhausted: Option<VerifiedPreservationExhausted>,
    ) -> ModelResult<Self> {
        let origin = if preservation_exhausted.is_some() {
            RollbackEntryOrigin::FromPreserving
        } else {
            RollbackEntryOrigin::Direct
        };
        Ok(Self {
            bound: BoundValue::new(
                current,
                "@operation",
                "begin_rollback",
                "preflight",
                EntryPayload {
                    origin,
                    anticipated_model_sha256: payload_hash(anticipated)?,
                },
            )?,
            handoff,
            preservation_exhausted,
        })
    }

    pub(super) fn origin(&self) -> RollbackEntryOrigin {
        self.bound.value.origin
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct ParticipantActionPayload {
    pub(super) member_id: String,
    pub(super) row: MergeParticipantRecord,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct ParticipantFailurePayload {
    pub(super) member_id: String,
    pub(super) row: MergeParticipantRecord,
    pub(super) later_unattempted: Vec<String>,
}

token!(PreparedParticipantAction, ParticipantActionPayload);
token!(VerifiedParticipantOutcome, ParticipantActionPayload);
token!(VerifiedParticipantNotStarted, String);
token!(PreparedFailureHaltBatch, ParticipantFailurePayload);
token!(BoundOwnedRetryFailureHaltBatch, ParticipantFailurePayload);
token!(
    BoundOwnedResolutionFailureHaltBatch,
    ParticipantFailurePayload
);
token!(VerifiedNoMutationAbort, String);
token!(PreparedAcceptedWorkspace, AcceptedWorkspaceV1);
token!(BoundPublicationDecision, bool);
token!(VerifiedResults, bool);

#[cfg(test)]
impl BoundPublicationDecision {
    pub(super) fn corrupt_payload_for_test(&mut self, value: bool) {
        self.0.value = value;
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct CandidatePayload {
    pub(super) candidate: PublicationCandidate,
    pub(super) marker_path: String,
    pub(super) lock_sha256: String,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct EvidencePayload {
    pub(super) composition_commit: String,
    pub(super) composition_tree: String,
    pub(super) root_merge_commit: Option<String>,
    pub(super) candidate_hashes: Vec<PublicationCandidateHash>,
}

token!(PreparedCandidate, CandidatePayload);
token!(PreparedEvidenceIntent, ());
token!(VerifiedEvidenceResult, EvidencePayload);
token!(PreparedPublicationIntent, ());
token!(VerifiedCandidatePublicationCompletion, ());
token!(VerifiedPublicationAction, PublicationPhysicalAction);
token!(BoundAmbiguityEvidence, RecoveryOriginStateV1);
token!(VerifiedRecoveryOrigin, RecoveryOriginStateV1);

#[derive(Clone, Debug, Serialize)]
pub(super) struct PreservationPayload {
    pub(super) owner: PreservationOwnerV1,
    pub(super) observed_position: PreservationCursorPosition,
    pub(super) pending: Option<PendingPreservationActionV1>,
    pub(super) evidence: Option<PreservationEvidence>,
    pub(super) publication_prefix: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct PreservationCursorPrefix {
    pub(super) owner: PreservationOwnerV1,
    pub(super) position: PreservationCursorPosition,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(super) enum PreservationCursorPosition {
    BackupRef,
    Stash(PreservationStashPhaseV1),
    ResetAttachedRef(PreservationRefResetPhaseV1),
}

token!(VerifiedPreservationCursorPrefix, PreservationCursorPrefix);

macro_rules! preservation_token {
    ($name:ident) => {
        #[derive(Debug)]
        pub(super) struct $name {
            bound: BoundValue<PreservationPayload>,
            prefix: VerifiedPreservationCursorPrefix,
        }

        impl $name {
            #[cfg(test)]
            pub(super) fn for_test(
                current: &StoredV1Record,
                owner: &str,
                action: &str,
                phase: &str,
                value: PreservationPayload,
                prefix: VerifiedPreservationCursorPrefix,
            ) -> ModelResult<Self> {
                Ok(Self {
                    bound: BoundValue::new(current, owner, action, phase, value)?,
                    prefix,
                })
            }

            pub(super) fn value(&self) -> &PreservationPayload {
                &self.bound.value
            }
        }

        impl BoundAuthority for $name {
            fn matches(
                &self,
                current: &StoredV1Record,
                owner: &str,
                action: &str,
                phase: &str,
            ) -> bool {
                self.bound.matches(current, owner, action, phase)
                    && self
                        .prefix
                        .matches(current, owner, "preservation_cursor", "prefix_verified")
                    && self.prefix.value().owner == self.bound.value.owner
                    && self.prefix.value().position == self.bound.value.observed_position
            }
        }
    };
}

preservation_token!(PreparedBackupRefIntent);
preservation_token!(VerifiedBackupRef);
preservation_token!(PreparedStashIntent);
preservation_token!(VerifiedStashPhase);
preservation_token!(VerifiedStashCompletion);
preservation_token!(PreparedRefResetIntent);
preservation_token!(VerifiedRefResetPhase);
preservation_token!(VerifiedRefResetCompletion);

token!(PreparedParticipantRollback, PendingRollbackActionV1);
token!(VerifiedParticipantRollback, ParticipantActionPayload);
token!(PreparedEvidenceRollback, PendingRollbackActionV1);
token!(VerifiedEvidenceRollbackStep, PendingRollbackActionV1);
token!(VerifiedEvidenceRollbackCompletion, ());
token!(PreparedRootMetadataRollback, PendingRollbackActionV1);
token!(VerifiedRootMetadataRollbackStep, PendingRollbackActionV1);
token!(VerifiedRootMetadataRollbackCompletion, ());

token!(BoundParticipantDrift, ParticipantDriftPayload);
token!(VerifiedParticipantDriftClear, ParticipantDriftPayload);
token!(BoundOperationDrift, OperationDrift);
token!(VerifiedOperationDriftClear, OperationDrift);

// Production token issuance lives below the authority owner. Callers can ask
// these observers to classify an exact checked record, but cannot construct an
// issuer, binding, cursor result, or caller-selected proof payload.
mod observe;

pub(super) use observe::{
    no_mutation_abort, observe_finalization, observe_forward, rollback_exhausted,
    verify_finalization_action, verify_participant_action,
};

fn authority_error(detail: impl Into<String>) -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecoveryRequired,
        format!("v1 transition authority rejected: {}", detail.into()),
    )
}

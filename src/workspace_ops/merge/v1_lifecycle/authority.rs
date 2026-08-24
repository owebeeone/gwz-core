use serde::Serialize;

mod binding;
mod dispatcher;
mod drift;
mod resolver;

pub(super) use dispatcher::*;
pub(super) use resolver::*;

use binding::{AuthorityIssuer, BoundValue};
pub(super) use drift::{ParticipantDriftIdentity, ParticipantDriftPayload};

use super::super::model::v1::{
    AcceptedWorkspaceV1, EvidenceRollbackStepV1, MergeOperationRecordV1,
    PendingPreservationActionV1, PendingRollbackActionV1, PreservationOwnerV1,
    PreservationRefResetPhaseV1, PreservationStashPhaseV1, RecoveryOriginStateV1, RollbackCursor,
    RootMetadataRollbackStepV1,
};
use super::super::{
    MergeParticipantRecord, OperationDrift, PreservationEvidence, PublicationCandidate,
    PublicationCandidateHash,
};
use super::checked::StoredV1Record;
use super::transition::ReverseEntryKind;
use crate::model::{ErrorCode, ModelError, ModelResult};

pub(super) trait BoundAuthority {
    fn matches(&self, current: &StoredV1Record, owner: &str, action: &str, phase: &str) -> bool;
}

macro_rules! token {
    ($name:ident, $value:ty) => {
        #[derive(Debug)]
        pub(super) struct $name(BoundValue<$value>);

        impl $name {
            #[allow(dead_code, reason = "A1 activation: reached only by this tree's own suites; the compile gate's blanket `dead_code` allowance expired with the activation, so the residue is named item by item.")]
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

            #[allow(dead_code, reason = "A1 activation: reached only by this tree's own suites; the compile gate's blanket `dead_code` allowance expired with the activation, so the residue is named item by item.")]
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
enum RollbackAggregatePosition {
    ReverseEntry,
    EvidencePending(EvidenceRollbackStepV1),
    BetweenParticipants(String),
    ParticipantPending {
        member_id: String,
        kind: super::super::model::v1::ParticipantRollbackKindV1,
    },
    NoMutationParticipant(String),
    SelectedRootMetadataPending(RootMetadataRollbackStepV1),
    Exhaustion,
    RecoveryPending(PendingRollbackActionV1),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct RollbackAggregatePayload {
    position: RollbackAggregatePosition,
    completed_participants: Vec<String>,
    publication_evidence_complete: bool,
    selected_root_projection: Option<RootMetadataRollbackStepV1>,
    projection_sha256: [u8; 32],
}

#[derive(Debug)]
struct VerifiedRollbackPrefix(BoundValue<RollbackAggregatePayload>);

impl VerifiedRollbackPrefix {
    fn issue(issuer: &AuthorityIssuer<'_>, value: RollbackAggregatePayload) -> ModelResult<Self> {
        Ok(Self(issuer.bind(
            "@operation",
            "rollback_prefix",
            "aggregate_verified",
            value,
        )?))
    }

    fn matches(&self, current: &StoredV1Record) -> bool {
        self.0.matches(
            current,
            "@operation",
            "rollback_prefix",
            "aggregate_verified",
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct ReverseEntryAuthorityPayload {
    pub(super) request: V1LifecycleRequest,
    pub(super) kind: ReverseEntryKind,
    pub(super) anticipated_model_sha256: [u8; 32],
    pub(super) publication: PublicationHandoffFact,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(super) enum PublicationHandoffPrefix {
    Baseline,
    Marker,
    Lock,
    Boundary,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(super) enum PublicationHandoffIndex {
    Pre,
    Staged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(super) enum PublicationHandoffFact {
    NoCandidate,
    EvidencePending,
    Candidate {
        prefix: PublicationHandoffPrefix,
        index: PublicationHandoffIndex,
    },
}

token!(VerifiedPublicationHandoff, ReverseEntryAuthorityPayload);
token!(
    VerifiedPreservationEntryPreflight,
    ReverseEntryAuthorityPayload
);

#[derive(Debug)]
pub(super) struct VerifiedRollbackEntryPreflight {
    bound: BoundValue<ReverseEntryAuthorityPayload>,
    prefix: VerifiedRollbackPrefix,
}

impl VerifiedRollbackEntryPreflight {
    fn issue(
        issuer: &AuthorityIssuer<'_>,
        owner: &str,
        action: &str,
        phase: &str,
        value: ReverseEntryAuthorityPayload,
        prefix: VerifiedRollbackPrefix,
    ) -> ModelResult<Self> {
        Ok(Self {
            bound: issuer.bind(owner, action, phase, value)?,
            prefix,
        })
    }

    pub(super) fn value(&self) -> &ReverseEntryAuthorityPayload {
        &self.bound.value
    }
}

impl BoundAuthority for VerifiedRollbackEntryPreflight {
    fn matches(&self, current: &StoredV1Record, owner: &str, action: &str, phase: &str) -> bool {
        self.bound.matches(current, owner, action, phase) && self.prefix.matches(current)
    }
}

#[cfg(test)]
impl VerifiedPublicationHandoff {
    pub(super) fn for_entry_test(
        current: &StoredV1Record,
        kind: ReverseEntryKind,
        anticipated: &MergeOperationRecordV1,
    ) -> ModelResult<Self> {
        let request = match kind {
            ReverseEntryKind::Preservation => V1LifecycleRequest::Preserve,
            ReverseEntryKind::DirectRollback | ReverseEntryKind::ExhaustedRollback => {
                V1LifecycleRequest::Abort
            }
        };
        Self::for_entry_request_test(
            current,
            request,
            kind,
            anticipated,
            PublicationHandoffFact::NoCandidate,
        )
    }

    pub(super) fn for_entry_request_test(
        current: &StoredV1Record,
        request: V1LifecycleRequest,
        kind: ReverseEntryKind,
        anticipated: &MergeOperationRecordV1,
        publication: PublicationHandoffFact,
    ) -> ModelResult<Self> {
        Self::issue(
            &AuthorityIssuer::for_observer(current),
            "@publication",
            "handoff",
            "verified",
            ReverseEntryAuthorityPayload {
                request,
                kind,
                anticipated_model_sha256: payload_hash(anticipated)?,
                publication,
            },
        )
    }
}

#[cfg(test)]
macro_rules! reverse_entry_preflight_fixture {
    ($name:ident, $action:literal) => {
        impl $name {
            pub(super) fn for_entry_test(
                current: &StoredV1Record,
                handoff: &VerifiedPublicationHandoff,
            ) -> ModelResult<Self> {
                Self::issue(
                    &AuthorityIssuer::for_observer(current),
                    "@operation",
                    $action,
                    "verified",
                    handoff.value().clone(),
                )
            }
        }
    };
}

#[cfg(test)]
reverse_entry_preflight_fixture!(
    VerifiedPreservationEntryPreflight,
    "preservation_entry_preflight"
);

#[cfg(test)]
impl VerifiedRollbackEntryPreflight {
    pub(super) fn for_entry_test(
        current: &StoredV1Record,
        handoff: &VerifiedPublicationHandoff,
    ) -> ModelResult<Self> {
        let projection = RollbackAggregatePayload {
            position: RollbackAggregatePosition::ReverseEntry,
            completed_participants: Vec::new(),
            publication_evidence_complete: false,
            selected_root_projection: None,
            projection_sha256: [0; 32],
        };
        let prefix =
            VerifiedRollbackPrefix::issue(&AuthorityIssuer::for_observer(current), projection)?;
        Self::issue(
            &AuthorityIssuer::for_observer(current),
            "@operation",
            "rollback_entry_preflight",
            "verified",
            handoff.value().clone(),
            prefix,
        )
    }
}

pub(super) fn payload_hash<T: Serialize>(value: &T) -> ModelResult<[u8; 32]> {
    binding::payload_hash(value)
}

pub(super) struct ReverseEntryInspectionPermit {
    bound: BoundValue<()>,
}

impl ReverseEntryInspectionPermit {
    fn issue(issuer: &AuthorityIssuer<'_>) -> ModelResult<Self> {
        Ok(Self {
            bound: issuer.bind("@operation", "inspect_reverse_entry", "authorized", ())?,
        })
    }

    pub(super) fn matches(&self, current: &StoredV1Record) -> bool {
        self.bound
            .matches(current, "@operation", "inspect_reverse_entry", "authorized")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(super) enum RollbackEntryOrigin {
    Direct,
    FromPreserving,
}

#[derive(Debug, Serialize)]
struct EntryPayload {
    origin: RollbackEntryOrigin,
    authority: ReverseEntryAuthorityPayload,
}

#[derive(Debug)]
pub(super) struct PreparedPreservationEntry {
    bound: BoundValue<EntryPayload>,
    handoff: VerifiedPublicationHandoff,
    preflight: VerifiedPreservationEntryPreflight,
}

#[derive(Debug)]
pub(super) struct PreparedRollbackEntry {
    bound: BoundValue<EntryPayload>,
    handoff: VerifiedPublicationHandoff,
    preflight: VerifiedRollbackEntryPreflight,
    preservation_exhausted: Option<VerifiedPreservationExhausted>,
}

macro_rules! entry_authority {
    ($name:ident) => {
        impl $name {
            pub(super) fn anticipated_model_matches(&self, model: &MergeOperationRecordV1) -> bool {
                payload_hash(model)
                    .is_ok_and(|hash| self.bound.value.authority.anticipated_model_sha256 == hash)
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
                    && self.preflight.matches(
                        current,
                        "@operation",
                        "preservation_entry_preflight",
                        "verified",
                    )
                    && &self.bound.value.authority == self.handoff.value()
                    && self.handoff.value() == self.preflight.value()
            }
        }
    };
}

entry_authority!(PreparedPreservationEntry);

impl PreparedRollbackEntry {
    pub(super) fn anticipated_model_matches(&self, model: &MergeOperationRecordV1) -> bool {
        payload_hash(model)
            .is_ok_and(|hash| self.bound.value.authority.anticipated_model_sha256 == hash)
    }
}

impl BoundAuthority for PreparedRollbackEntry {
    fn matches(&self, current: &StoredV1Record, owner: &str, action: &str, phase: &str) -> bool {
        let handoff = self
            .handoff
            .matches(current, "@publication", "handoff", "verified");
        let preflight = self.preflight.matches(
            current,
            "@operation",
            "rollback_entry_preflight",
            "verified",
        );
        let exhaustion = self.preservation_exhausted.as_ref().is_some_and(|proof| {
            proof.matches(current, "@operation", "preservation_exhausted", "verified")
        });
        self.bound.matches(current, owner, action, phase)
            && handoff
            && preflight
            && &self.bound.value.authority == self.handoff.value()
            && self.handoff.value() == self.preflight.value()
            && match self.bound.value.origin {
                RollbackEntryOrigin::Direct => self.preservation_exhausted.is_none(),
                RollbackEntryOrigin::FromPreserving => exhaustion,
            }
    }
}

impl PreparedPreservationEntry {
    pub(super) fn publication_handoff(&self) -> PublicationHandoffFact {
        self.bound.value.authority.publication
    }

    #[cfg(test)]
    pub(super) fn for_test(
        current: &StoredV1Record,
        anticipated: &MergeOperationRecordV1,
        handoff: VerifiedPublicationHandoff,
    ) -> ModelResult<Self> {
        let authority = ReverseEntryAuthorityPayload {
            request: V1LifecycleRequest::Preserve,
            kind: ReverseEntryKind::Preservation,
            anticipated_model_sha256: payload_hash(anticipated)?,
            publication: handoff.value().publication,
        };
        if handoff.value() != &authority {
            return Err(authority_error(
                "preservation handoff does not match the anticipated model",
            ));
        }
        let preflight = VerifiedPreservationEntryPreflight::issue(
            &AuthorityIssuer::for_observer(current),
            "@operation",
            "preservation_entry_preflight",
            "verified",
            authority.clone(),
        )?;
        Ok(Self {
            bound: BoundValue::new(
                current,
                "@operation",
                "begin_preservation",
                "preflight",
                EntryPayload {
                    origin: RollbackEntryOrigin::Direct,
                    authority,
                },
            )?,
            handoff,
            preflight,
        })
    }
}

impl PreparedRollbackEntry {
    pub(super) fn publication_handoff(&self) -> PublicationHandoffFact {
        self.bound.value.authority.publication
    }

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
        let authority = ReverseEntryAuthorityPayload {
            request: V1LifecycleRequest::Abort,
            kind: if origin == RollbackEntryOrigin::FromPreserving {
                ReverseEntryKind::ExhaustedRollback
            } else {
                ReverseEntryKind::DirectRollback
            },
            anticipated_model_sha256: payload_hash(anticipated)?,
            publication: handoff.value().publication,
        };
        if handoff.value() != &authority {
            return Err(authority_error(
                "rollback handoff does not match the anticipated model",
            ));
        }
        let preflight = VerifiedRollbackEntryPreflight::for_entry_test(current, &handoff)?;
        Ok(Self {
            bound: BoundValue::new(
                current,
                "@operation",
                "begin_rollback",
                "preflight",
                EntryPayload { origin, authority },
            )?,
            handoff,
            preflight,
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
// `GwzM5-8DurableCursorAmendment.md` §3.1: the two durable cursor marker
// writes. These are evidence-only record rewrites inside `Preserving` — no
// physical mutation occurs, so no pending action is journaled for them and the
// durable write IS the step.
preservation_token!(PreparedArtifactNoop);
preservation_token!(PreparedResetNoop);

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
    RecordEvidenceOr, SealedReverseEntryVisitor, no_mutation_abort, observe_archive,
    observe_finalization, observe_forward, observe_preservation,
    observe_reverse_publication_handoff, observe_rollback, prepare_direct_rollback_entry,
    prepare_exhausted_rollback_entry, prepare_preservation_entry,
    preservation_execution_prefix_is_exact, preservation_reset_step, preservation_stash_guard,
    preservation_stash_step, require_rollback_aggregate, verify_finalization_action,
    verify_participant_action,
};
#[cfg(test)]
pub(super) use observe::{
    preservation_durability_fact, preserving_verify_recovery_origin,
    rolling_back_verify_recovery_origin,
};

#[cfg(test)]
pub(super) use observe::rollback_exhausted_for_test;

fn authority_error(detail: impl Into<String>) -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecoveryRequired,
        format!("v1 transition authority rejected: {}", detail.into()),
    )
}

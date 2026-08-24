use serde::Deserialize;

use super::super::ParticipantState;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, serde::Serialize)]
pub(crate) struct RecoveryContextV1 {
    pub(crate) origin_state: RecoveryOriginStateV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecoveryOriginStateV1 {
    Executing,
    AwaitingResolution,
    Halted,
    Finalizing,
    Preserving,
    RollingBack,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum PendingRollbackActionV1 {
    Participant {
        member_id: String,
        action: ParticipantRollbackKindV1,
        terminal_state: ParticipantState,
    },
    PublicationEvidence {
        next_step: EvidenceRollbackStepV1,
    },
    SelectedRootMetadata {
        next_step: RootMetadataRollbackStepV1,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ParticipantRollbackKindV1 {
    AbortConflict,
    ResetIntegrated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EvidenceRollbackStepV1 {
    EvidenceCommit,
    Boundary,
    Lock,
    Marker,
    Index,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RootMetadataRollbackStepV1 {
    Manifest,
    Lock,
    Complete,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum PendingPreservationActionV1 {
    BackupRef {
        owner: PreservationOwnerV1,
        name: String,
        target_commit: String,
    },
    Stash {
        owner: PreservationOwnerV1,
        phase: PreservationStashPhaseV1,
        stash_id: Option<String>,
        stash_object_id: Option<GitObjectIdV1>,
        message: String,
        head_commit: String,
        preimage_sha256: String,
        root_publication_handoff: Option<PreservationPublicationCandidateV1>,
    },
    ResetAttachedRef {
        owner: PreservationOwnerV1,
        branch: String,
        expected_commit: String,
        restore_commit: String,
        phase: PreservationRefResetPhaseV1,
        root_publication_handoff: Option<PreservationPublicationCandidateV1>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum PreservationOwnerV1 {
    Participant { member_id: String },
    PublicationRoot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PreservationStashPhaseV1 {
    NormalizeParent,
    NormalizeMarker,
    NormalizeLock,
    NormalizeIndex,
    CreateStash,
    RestoreIndex,
    RestoreLock,
    RestoreParent,
    RestoreMarker,
    WriteBundle,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PreservationRefResetPhaseV1 {
    PrepareParent,
    PrepareMarker,
    PrepareLock,
    PrepareIndex,
    ResetRef,
    RestoreIndex,
    RestoreLock,
    RestoreParent,
    RestoreMarker,
    Complete,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, serde::Serialize)]
pub(crate) struct GitObjectIdV1 {
    pub(crate) algorithm: GitObjectAlgorithmV1,
    pub(crate) digest_hex: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GitObjectAlgorithmV1 {
    Sha1,
    Sha256,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PublicationPrefixV1 {
    Baseline,
    Marker,
    Lock,
    Boundary,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PublicationIndexFormV1 {
    Pre,
    Staged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, serde::Serialize)]
pub(crate) struct PreservationPublicationCandidateV1 {
    pub(crate) prefix: PublicationPrefixV1,
    pub(crate) index: PublicationIndexFormV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum PreservationPublicationHandoffV1 {
    NoCandidate,
    EvidencePending,
    Candidate {
        prefix: PublicationPrefixV1,
        index: PublicationIndexFormV1,
    },
}

impl PreservationPublicationHandoffV1 {
    pub(crate) fn candidate(self) -> Option<PreservationPublicationCandidateV1> {
        match self {
            Self::Candidate { prefix, index } => {
                Some(PreservationPublicationCandidateV1 { prefix, index })
            }
            Self::NoCandidate | Self::EvidencePending => None,
        }
    }
}

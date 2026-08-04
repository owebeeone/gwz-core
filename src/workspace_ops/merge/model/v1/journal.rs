use serde::Deserialize;

use super::super::ParticipantState;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(crate) struct RecoveryContextV1 {
    pub(crate) origin_state: RecoveryOriginStateV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecoveryOriginStateV1 {
    Executing,
    AwaitingResolution,
    Halted,
    Finalizing,
    Preserving,
    RollingBack,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(rename_all = "snake_case")]
pub(crate) enum ParticipantRollbackKindV1 {
    AbortConflict,
    ResetIntegrated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(rename_all = "snake_case")]
pub(crate) enum EvidenceRollbackStepV1 {
    EvidenceCommit,
    Boundary,
    Lock,
    Marker,
    Index,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(rename_all = "snake_case")]
pub(crate) enum RootMetadataRollbackStepV1 {
    Manifest,
    Lock,
    Complete,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
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
        root_publication_prefix: Option<PublicationPrefixV1>,
    },
    ResetAttachedRef {
        owner: PreservationOwnerV1,
        branch: String,
        expected_commit: String,
        restore_commit: String,
        phase: PreservationRefResetPhaseV1,
        root_publication_prefix: Option<PublicationPrefixV1>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum PreservationOwnerV1 {
    Participant { member_id: String },
    PublicationRoot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(rename_all = "snake_case")]
pub(crate) enum PreservationStashPhaseV1 {
    NormalizeRoot,
    CreateStash,
    RestoreRoot,
    WriteBundle,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(rename_all = "snake_case")]
pub(crate) enum PreservationRefResetPhaseV1 {
    ResetRef,
    RestoreRoot,
    Complete,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(crate) struct GitObjectIdV1 {
    pub(crate) algorithm: GitObjectAlgorithmV1,
    pub(crate) digest_hex: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(rename_all = "snake_case")]
pub(crate) enum GitObjectAlgorithmV1 {
    Sha1,
    Sha256,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
#[serde(rename_all = "snake_case")]
pub(crate) enum PublicationPrefixV1 {
    Baseline,
    Marker,
    Lock,
    Boundary,
}

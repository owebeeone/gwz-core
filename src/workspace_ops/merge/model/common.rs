//! The merge record's shared vocabulary.
//!
//! **M5d (`GwzM5-8M5d-Charter.md` §1).** This file was `model/v0.rs`, which
//! read as "the v0 record" and was not: `MergeOperationRecordV1`
//! (`model/v1/record.rs`) embeds `MergeBaseline`, `MergeParticipantRecord`,
//! `PreservationEvidence`, `PublicationProgress` and `OperationDrift` from
//! here field for field, and the v1 authority imports them by these names.
//! The one thing here that really was the v0 record — the
//! `MergeOperationRecordV0` serde struct — moved to
//! `record_wire/archive/v0_record.rs`, where the archive decoder that is now
//! its only reader owns it (charter §5). Everything else stayed.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use super::{
    MergeExecutionMode, MergeTargetKind, OperationState, ParticipantState, PublicationStep,
};
use crate::model::ErrorCode;

pub(crate) const MERGE_RECORD_SCHEMA: &str = "gwz.merge-operation/v0";
pub(crate) const MERGE_RECORD_SCHEMA_VERSION: u32 = 0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct MergeBaseline {
    pub lock_sha256: String,
    pub manifest_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock_yaml: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_yaml: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock_commit_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_commit_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_head: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_branch: Option<String>,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct MergeParticipantRecord {
    pub path: String,
    pub target_kind: MergeTargetKind,
    pub target_branch: String,
    pub before_commit: String,
    pub source_commit: String,
    pub commit_message: String,
    pub state: ParticipantState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resulting_commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_merge_head: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflict_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflict_snapshot: Vec<ConflictFileEvidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<MergeRecordError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_action: Option<PendingMergeAction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preservation: Vec<PreservationEvidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drift: Vec<ParticipantDrift>,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ConflictFileEvidence {
    pub path: String,
    pub sha256: String,
}

/// Durable intent written before an individual participant Git action.
///
/// Presence means the action may not have started, may have completed without
/// its outcome row, or may have stopped in an ambiguous intermediate state.
/// Recovery must reconcile the live repository against these exact inputs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct PendingMergeAction {
    pub kind: PendingMergeActionKind,
    pub target_branch: String,
    pub before_commit: String,
    pub source_commit: String,
    pub commit_message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_result: Option<PendingMergeExpectedResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_spec: Option<PendingCommitSpec>,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PendingMergeExpectedResult {
    Unchanged,
    FastForward,
    ExpectedConflict,
    Commit,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct PendingCommitSpec {
    pub tree_oid: String,
    pub author: PendingGitSignature,
    pub committer: PendingGitSignature,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct PendingGitSignature {
    pub name: String,
    pub email: String,
    pub time_seconds: i64,
    pub timezone_offset_minutes: i32,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PendingMergeActionKind {
    VerifyUpToDate,
    FastForward,
    TrueMerge,
    ResolveConflict,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct MergeRecordError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct PreservationEvidence {
    pub backup_ref: Option<String>,
    pub backup_commit: Option<String>,
    pub stash_id: Option<String>,
    pub stash_object_id: Option<String>,
    /// Per-owner no-op skip marker. Absent-by-default on the wire; no v0
    /// writer ever emits it. See `GwzM5-8DurableCursorAmendment.md` §2.1/§2.2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noop_commit: Option<String>,
    /// Reset completion marker, spelled as the owner anchor commit id rather
    /// than a boolean so a decoder can reject a fabricated value without any
    /// live observation. See `GwzM5-8DurableCursorAmendment.md` §2.1/§2.2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_commit: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct PublicationProgress {
    pub step: PublicationStep,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_lock_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_marker_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_merge_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composition_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composition_tree: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_hashes: Vec<PublicationCandidateHash>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<PublicationCandidate>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub evidence_rolled_back: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub root_preservation: Vec<PreservationEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preservation_prefix: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct PublicationCandidate {
    pub marker_id: String,
    pub root_branch: String,
    pub actor_id: String,
    pub baseline_lock_yaml: String,
    pub lock_yaml: String,
    pub marker_yaml: String,
    pub baseline_boundary_text: String,
    pub boundary_text: String,
    pub baseline_boundary_sha256: String,
    pub marker_sha256: String,
    pub boundary_sha256: String,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct PublicationCandidateHash {
    pub path: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ParticipantDrift {
    pub kind: ParticipantDriftKind,
    pub message: String,
    pub expected_branch: Option<String>,
    pub live_branch: Option<String>,
    pub expected_head: Option<String>,
    pub live_head: Option<String>,
    pub expected_merge_head: Option<String>,
    pub live_merge_head: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ParticipantDriftKind {
    BranchChanged,
    HeadAdvanced,
    HeadRewound,
    HeadDiverged,
    ObjectMissing,
    TargetRefChanged,
    WorktreeModified,
    IndexModified,
    MergeStateMissing,
    MergeHeadChanged,
    NewIntegrationState,
    ForeignIntegrationState,
    PendingActionAmbiguous,
    RepositoryMissing,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct OperationDrift {
    pub kind: OperationDriftKind,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OperationDriftKind {
    BaselineLockChanged,
    BaselineManifestChanged,
    RootCandidateMetadataInvalid,
    RootCandidateStateChanged,
    RecordUnreadable,
}

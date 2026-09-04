//! The archived v0 record body.
//!
//! **M5d (`GwzM5-8M5d-Charter.md` §2, §5).** 0.14 has no v0 merge lifecycle
//! and never decodes an OPEN v0 body — an open v0 envelope is the §2 refusal,
//! classified from its header alone. This struct exists for one reason: a
//! `done/` record written before 0.14 must still project under `--status <id>`
//! and must still be seen by GC, "enough that old history is not a silent
//! hole". It is therefore the archive decoder's own body type, reachable only
//! from `merge::record_wire` and the GC/retention readers it feeds, and it is
//! never constructed by this binary — nothing here writes v0.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use super::super::super::model::{
    MergeBaseline, MergeExecutionMode, MergeParticipantRecord, OperationDrift, OperationState,
    PublicationProgress,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(in crate::workspace_ops::merge) struct MergeOperationRecordV0 {
    pub schema: String,
    pub record_schema_version: u32,
    pub writer_version: String,
    pub workspace_id: String,
    pub merge_id: String,
    pub operation_id: String,
    pub state: OperationState,
    pub source_ref: String,
    #[serde(default, skip_serializing_if = "mode_is_normal")]
    pub mode: MergeExecutionMode,
    pub created_at: String,
    pub baseline: MergeBaseline,
    pub selected_targets: Vec<String>,
    pub participants: BTreeMap<String, MergeParticipantRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publication: Option<PublicationProgress>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operation_drift: Vec<OperationDrift>,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}

/// The v0 envelope omits `mode` when it is `Normal`, exactly as the writer
/// that produced these bytes did. `MergeExecutionMode::is_normal` is private
/// to the model, and widening it for a decoder that never writes would be the
/// wrong direction, so the predicate is stated here.
fn mode_is_normal(mode: &MergeExecutionMode) -> bool {
    matches!(mode, MergeExecutionMode::Normal)
}

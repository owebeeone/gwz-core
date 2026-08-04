use std::collections::BTreeMap;

use serde::Deserialize;
use serde_yaml::Value;

use super::super::{
    MergeBaseline, MergeExecutionMode, MergeParticipantRecord, OperationDrift, OperationState,
    PublicationProgress,
};
use super::{
    AcceptedWorkspaceV1, PendingPreservationActionV1, PendingRollbackActionV1, RecoveryContextV1,
};

pub(crate) const MERGE_RECORD_SCHEMA_V1: &str = "gwz.merge-operation/v1";
pub(crate) const MERGE_RECORD_SCHEMA_VERSION_V1: u32 = 1;

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[cfg_attr(test, derive(serde::Serialize))]
pub(crate) struct MergeOperationRecordV1 {
    pub(crate) schema: String,
    pub(crate) record_schema_version: u32,
    pub(crate) writer_version: String,
    pub(crate) workspace_id: String,
    pub(crate) merge_id: String,
    pub(crate) operation_id: String,
    pub(crate) state: OperationState,
    pub(crate) source_ref: String,
    #[serde(default, skip_serializing_if = "MergeExecutionMode::is_normal")]
    pub(crate) mode: MergeExecutionMode,
    pub(crate) created_at: String,
    pub(crate) baseline: MergeBaseline,
    pub(crate) selected_targets: Vec<String>,
    pub(crate) participants: BTreeMap<String, MergeParticipantRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) publication: Option<PublicationProgress>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) operation_drift: Vec<OperationDrift>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) accepted_workspace: Option<AcceptedWorkspaceV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) recovery_context: Option<RecoveryContextV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) pending_rollback: Option<PendingRollbackActionV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) pending_preservation: Option<PendingPreservationActionV1>,
    #[serde(default, flatten)]
    pub(crate) extensions: BTreeMap<String, Value>,
}

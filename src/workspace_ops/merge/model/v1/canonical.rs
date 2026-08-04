use std::collections::BTreeMap;

use super::super::{
    MergeBaseline, MergeExecutionMode, MergeParticipantRecord, OperationDrift, OperationState,
    PublicationProgress,
};
use super::validate::ValidatedV1Record;
use super::{
    AcceptedWorkspaceV1, PendingPreservationActionV1, PendingRollbackActionV1, RecoveryContextV1,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RecordVersion {
    V0,
    V1,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CanonicalMergeRecord {
    source_version: RecordVersion,
    common: CanonicalMergeCommon,
    installed: CanonicalInstalledState,
}

#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)]
pub(crate) struct CanonicalMergeCommon {
    writer_version: String,
    workspace_id: String,
    merge_id: String,
    operation_id: String,
    state: OperationState,
    source_ref: String,
    mode: MergeExecutionMode,
    created_at: String,
    baseline: MergeBaseline,
    selected_targets: Vec<String>,
    participants: BTreeMap<String, MergeParticipantRecord>,
    publication: Option<PublicationProgress>,
    operation_drift: Vec<OperationDrift>,
}

#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)]
enum CanonicalInstalledState {
    V0,
    V1(Box<CanonicalV1State>),
}

#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)]
pub(crate) struct CanonicalV1State {
    accepted_workspace: Option<AcceptedWorkspaceV1>,
    recovery_context: Option<RecoveryContextV1>,
    pending_rollback: Option<PendingRollbackActionV1>,
    pending_preservation: Option<PendingPreservationActionV1>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CanonicalInstalledKind {
    V0,
    V1,
}

impl CanonicalMergeRecord {
    pub(crate) fn source_version(&self) -> RecordVersion {
        self.source_version
    }

    pub(crate) fn common(&self) -> &CanonicalMergeCommon {
        &self.common
    }

    pub(crate) fn installed_kind(&self) -> CanonicalInstalledKind {
        match self.installed {
            CanonicalInstalledState::V0 => CanonicalInstalledKind::V0,
            CanonicalInstalledState::V1(_) => CanonicalInstalledKind::V1,
        }
    }

    pub(crate) fn v1_state(&self) -> Option<&CanonicalV1State> {
        match &self.installed {
            CanonicalInstalledState::V0 => None,
            CanonicalInstalledState::V1(state) => Some(state),
        }
    }
}

impl CanonicalMergeCommon {
    pub(crate) fn merge_id(&self) -> &str {
        &self.merge_id
    }

    pub(crate) fn participants(&self) -> &BTreeMap<String, MergeParticipantRecord> {
        &self.participants
    }
}

impl CanonicalV1State {
    pub(crate) fn is_empty(&self) -> bool {
        self.accepted_workspace.is_none()
            && self.recovery_context.is_none()
            && self.pending_rollback.is_none()
            && self.pending_preservation.is_none()
    }
}

impl From<ValidatedV1Record> for CanonicalMergeRecord {
    fn from(validated: ValidatedV1Record) -> Self {
        let record = validated.into_record();
        Self {
            source_version: RecordVersion::V1,
            common: CanonicalMergeCommon {
                writer_version: record.writer_version,
                workspace_id: record.workspace_id,
                merge_id: record.merge_id,
                operation_id: record.operation_id,
                state: record.state,
                source_ref: record.source_ref,
                mode: record.mode,
                created_at: record.created_at,
                baseline: record.baseline,
                selected_targets: record.selected_targets,
                participants: record.participants,
                publication: record.publication,
                operation_drift: record.operation_drift,
            },
            installed: CanonicalInstalledState::V1(Box::new(CanonicalV1State {
                accepted_workspace: record.accepted_workspace,
                recovery_context: record.recovery_context,
                pending_rollback: record.pending_rollback,
                pending_preservation: record.pending_preservation,
            })),
        }
    }
}

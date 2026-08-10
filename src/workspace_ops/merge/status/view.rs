use std::collections::BTreeMap;

use super::super::{
    MergeBaseline, MergeOperationRecord, MergeParticipantRecord, MergeTargetKind, OperationDrift,
    OperationState, PublicationProgress,
};
use crate::model::{ErrorCode, ModelError, ModelResult};

/// Immutable, non-serializable common record view used by live status facts.
///
/// The view cannot be converted back into a durable record and deliberately
/// excludes extensions and v1-only journals.
#[derive(Clone, Copy)]
pub(in crate::workspace_ops::merge) struct MergeStatusRecordView<'a> {
    #[allow(
        dead_code,
        reason = "workspace identity is part of the frozen complete view"
    )]
    workspace_id: &'a str,
    merge_id: &'a str,
    operation_id: &'a str,
    state: OperationState,
    source_ref: &'a str,
    baseline: &'a MergeBaseline,
    selected_targets: &'a [String],
    participants: &'a BTreeMap<String, MergeParticipantRecord>,
    publication: Option<&'a PublicationProgress>,
    operation_drift: &'a [OperationDrift],
}

impl<'a> MergeStatusRecordView<'a> {
    pub(in crate::workspace_ops::merge) fn from_v0(record: &'a MergeOperationRecord) -> Self {
        Self {
            workspace_id: &record.workspace_id,
            merge_id: &record.merge_id,
            operation_id: &record.operation_id,
            state: record.state,
            source_ref: &record.source_ref,
            baseline: &record.baseline,
            selected_targets: &record.selected_targets,
            participants: &record.participants,
            publication: record.publication.as_ref(),
            operation_drift: &record.operation_drift,
        }
    }

    #[cfg(test)]
    pub(in crate::workspace_ops::merge) fn from_v1(
        record: &'a super::super::model::v1::MergeOperationRecordV1,
    ) -> Self {
        Self {
            workspace_id: &record.workspace_id,
            merge_id: &record.merge_id,
            operation_id: &record.operation_id,
            state: record.state,
            source_ref: &record.source_ref,
            baseline: &record.baseline,
            selected_targets: &record.selected_targets,
            participants: &record.participants,
            publication: record.publication.as_ref(),
            operation_drift: &record.operation_drift,
        }
    }

    #[allow(
        dead_code,
        reason = "workspace identity is part of the frozen complete view"
    )]
    pub(in crate::workspace_ops::merge) fn workspace_id(self) -> &'a str {
        self.workspace_id
    }

    pub(in crate::workspace_ops::merge) fn merge_id(self) -> &'a str {
        self.merge_id
    }

    pub(in crate::workspace_ops::merge) fn operation_id(self) -> &'a str {
        self.operation_id
    }

    pub(in crate::workspace_ops::merge) fn state(self) -> OperationState {
        self.state
    }

    pub(in crate::workspace_ops::merge) fn source_ref(self) -> &'a str {
        self.source_ref
    }

    pub(in crate::workspace_ops::merge) fn baseline(self) -> &'a MergeBaseline {
        self.baseline
    }

    pub(in crate::workspace_ops::merge) fn selected_targets(self) -> &'a [String] {
        self.selected_targets
    }

    pub(in crate::workspace_ops::merge) fn participants(
        self,
    ) -> &'a BTreeMap<String, MergeParticipantRecord> {
        self.participants
    }

    pub(in crate::workspace_ops::merge) fn publication(self) -> Option<&'a PublicationProgress> {
        self.publication
    }

    pub(in crate::workspace_ops::merge) fn operation_drift(self) -> &'a [OperationDrift] {
        self.operation_drift
    }

    pub(in crate::workspace_ops::merge) fn selected_root_participant(
        self,
    ) -> ModelResult<Option<&'a MergeParticipantRecord>> {
        let participant = self.participants.get("@root");
        let selected = self.selected_targets.iter().any(|target| target == "@root");
        match (selected, participant) {
            (false, None) => Ok(None),
            (true, Some(participant))
                if participant.target_kind == MergeTargetKind::Root
                    && participant.path == "."
                    && super::super::participant_semantics::result::is_successful_result(
                        participant.state,
                    ) =>
            {
                Ok(Some(participant))
            }
            _ => Err(ModelError::new(
                ErrorCode::MergeRecordUnreadable,
                "selected root participant identity or successful state is inconsistent",
            )),
        }
    }
}

use std::collections::BTreeMap;

use super::super::{
    MergeBaseline, MergeParticipantRecord, MergeTargetKind, OperationDrift, OperationState,
    PublicationProgress,
};
use crate::model::{ErrorCode, ModelError, ModelResult};

/// Immutable, non-serializable common record view used by live status facts,
/// by the open-merge gates and by `add`'s conflict routing.
///
/// The view cannot be converted back into a durable record and deliberately
/// excludes extensions and v1-only journals. It is the one place a
/// version-agnostic reader names the half the v0 and v1 records held
/// IDENTICALLY. M5d deleted `from_v0` with the v0 engine and nothing else:
/// the view is still the shape every non-merge consumer of merge state reads,
/// it just has one constructor now.
#[derive(Clone, Copy)]
pub(in crate::workspace_ops) struct MergeStatusRecordView<'a> {
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
    /// The same view over an ARCHIVED record's common projection (I2 §7).
    ///
    /// **M5d.** `from_v0` left with the v0 engine; this is not it. An archived
    /// `done/` record is decoded over both envelopes into the half they hold
    /// identically (`record_wire::decode_archived_common`), and suites that
    /// assert on a finished merge's durable body read it here rather than
    /// through a store that no longer exists.
    #[cfg(test)]
    #[allow(
        private_interfaces,
        reason = "M5d lint sweep: `MergeOperationRecordV0` is merge-private while this `cfg(test)` reader is `pub(in crate::workspace_ops)`; narrowing the reader would move a crate-visible signature, so the mismatch is held instead."
    )]
    pub(in crate::workspace_ops) fn from_archived(
        record: &'a super::super::record_wire::MergeOperationRecordV0,
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

    pub(in crate::workspace_ops) fn from_v1(
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

    pub(in crate::workspace_ops) fn workspace_id(self) -> &'a str {
        self.workspace_id
    }

    pub(in crate::workspace_ops) fn merge_id(self) -> &'a str {
        self.merge_id
    }

    pub(in crate::workspace_ops) fn operation_id(self) -> &'a str {
        self.operation_id
    }

    pub(in crate::workspace_ops) fn state(self) -> OperationState {
        self.state
    }

    pub(in crate::workspace_ops) fn source_ref(self) -> &'a str {
        self.source_ref
    }

    pub(in crate::workspace_ops) fn baseline(self) -> &'a MergeBaseline {
        self.baseline
    }

    pub(in crate::workspace_ops) fn selected_targets(self) -> &'a [String] {
        self.selected_targets
    }

    pub(in crate::workspace_ops) fn participants(
        self,
    ) -> &'a BTreeMap<String, MergeParticipantRecord> {
        self.participants
    }

    pub(in crate::workspace_ops) fn publication(self) -> Option<&'a PublicationProgress> {
        self.publication
    }

    pub(in crate::workspace_ops) fn operation_drift(self) -> &'a [OperationDrift] {
        self.operation_drift
    }

    pub(in crate::workspace_ops) fn selected_root_participant(
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

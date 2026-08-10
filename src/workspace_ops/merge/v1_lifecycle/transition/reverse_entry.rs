use serde::Serialize;

use super::super::authority::{
    ReverseEntryInspectionPermit, SealedReverseEntryVisitor, V1LifecycleRequest,
    VerifiedParticipantNotStarted, VerifiedParticipantOutcome, payload_hash,
};
use super::super::checked::{RecordDigest, StoredV1Record};
use super::reduce::participant::{abandon, record_outcome};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::OperationState;
use crate::workspace_ops::merge::model::v1::MergeOperationRecordV1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(in crate::workspace_ops::merge::v1_lifecycle) enum ReverseEntryKind {
    Preservation,
    DirectRollback,
    ExhaustedRollback,
}

pub(in crate::workspace_ops::merge::v1_lifecycle) enum ReverseEntryPredecessor<'a> {
    ActionFree,
    ParticipantOutcome(&'a VerifiedParticipantOutcome),
    ParticipantNotStarted(&'a VerifiedParticipantNotStarted),
}

pub(in crate::workspace_ops::merge::v1_lifecycle) struct PreparedReverseEntryView {
    source_digest: RecordDigest,
    workspace_id: String,
    merge_id: String,
    operation_id: String,
    request: V1LifecycleRequest,
    kind: ReverseEntryKind,
    anticipated_model_sha256: [u8; 32],
    anticipated: MergeOperationRecordV1,
}

impl PreparedReverseEntryView {
    pub(in crate::workspace_ops::merge::v1_lifecycle) fn kind(&self) -> ReverseEntryKind {
        self.kind
    }

    pub(in crate::workspace_ops::merge::v1_lifecycle) fn request(&self) -> V1LifecycleRequest {
        self.request
    }

    pub(in crate::workspace_ops::merge::v1_lifecycle) fn anticipated_model_sha256(
        &self,
    ) -> [u8; 32] {
        self.anticipated_model_sha256
    }

    fn matches(&self, current: &StoredV1Record) -> bool {
        let record = current.record();
        self.source_digest == current.source_digest()
            && self.workspace_id == record.workspace_id
            && self.merge_id == record.merge_id
            && self.operation_id == record.operation_id
            && payload_hash(&self.anticipated)
                .is_ok_and(|digest| digest == self.anticipated_model_sha256)
    }
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn preview_reverse_entry(
    current: &StoredV1Record,
    request: V1LifecycleRequest,
    predecessor: ReverseEntryPredecessor<'_>,
) -> ModelResult<PreparedReverseEntryView> {
    let kind = reverse_entry_kind(current.record().state, request)?;
    let mut anticipated = current.record().clone();
    match predecessor {
        ReverseEntryPredecessor::ActionFree => require_action_free(&anticipated)?,
        ReverseEntryPredecessor::ParticipantOutcome(proof) => {
            if current.record().state != OperationState::Halted
                || kind == ReverseEntryKind::ExhaustedRollback
            {
                return Err(entry_error(
                    "participant outcome is not a legal reverse-entry predecessor",
                ));
            }
            record_outcome(current, &mut anticipated, proof, true)?;
        }
        ReverseEntryPredecessor::ParticipantNotStarted(proof) => {
            if kind == ReverseEntryKind::ExhaustedRollback {
                return Err(entry_error(
                    "participant abandonment is not legal after preservation",
                ));
            }
            abandon(current, &mut anticipated, proof)?;
        }
    }
    let record = current.record();
    let anticipated_model_sha256 = payload_hash(&anticipated)?;
    Ok(PreparedReverseEntryView {
        source_digest: current.source_digest(),
        workspace_id: record.workspace_id.clone(),
        merge_id: record.merge_id.clone(),
        operation_id: record.operation_id.clone(),
        request,
        kind,
        anticipated_model_sha256,
        anticipated,
    })
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn visit_reverse_entry<
    V: SealedReverseEntryVisitor,
>(
    permit: ReverseEntryInspectionPermit,
    current: &StoredV1Record,
    preview: &PreparedReverseEntryView,
    visitor: &mut V,
) -> ModelResult<V::SealedAuthority> {
    if !permit.matches(current) || !preview.matches(current) {
        return Err(entry_error("reverse-entry inspection authority is stale"));
    }
    visitor.inspect(
        current,
        &preview.anticipated,
        preview.request,
        preview.kind,
        preview.anticipated_model_sha256,
    )
}

pub(in crate::workspace_ops::merge::v1_lifecycle) fn reverse_entry_kind(
    state: OperationState,
    request: V1LifecycleRequest,
) -> ModelResult<ReverseEntryKind> {
    if state == OperationState::Preserving {
        if request == V1LifecycleRequest::Status {
            return Err(entry_error("status cannot prepare a reverse entry"));
        }
        return Ok(ReverseEntryKind::ExhaustedRollback);
    }
    if !matches!(
        state,
        OperationState::Executing
            | OperationState::AwaitingResolution
            | OperationState::Halted
            | OperationState::Finalizing
    ) {
        return Err(entry_error(
            "operation state does not authorize direct reverse entry",
        ));
    }
    match request {
        V1LifecycleRequest::Preserve => Ok(ReverseEntryKind::Preservation),
        V1LifecycleRequest::Abort => Ok(ReverseEntryKind::DirectRollback),
        _ => Err(entry_error(
            "request does not authorize preservation or rollback entry",
        )),
    }
}

fn require_action_free(record: &MergeOperationRecordV1) -> ModelResult<()> {
    if record
        .participants
        .values()
        .any(|row| row.pending_action.is_some())
        || record.pending_preservation.is_some()
        || record.pending_rollback.is_some()
    {
        return Err(entry_error(
            "reverse-entry preview requires an action-free predecessor",
        ));
    }
    Ok(())
}

fn entry_error(detail: impl Into<String>) -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecoveryRequired,
        format!("v1 reverse-entry preview rejected: {}", detail.into()),
    )
}

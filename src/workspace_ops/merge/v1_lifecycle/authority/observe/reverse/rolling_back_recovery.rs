use super::super::super::*;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::workspace_ops::merge::model::v1::{PendingRollbackActionV1, RecoveryOriginStateV1};

pub(in crate::workspace_ops::merge::v1_lifecycle) fn verify_recovery_origin<B: GitBackend>(
    backend: &B,
    current: &StoredV1Record,
) -> ModelResult<VerifiedRecoveryOrigin> {
    let context =
        current.record().recovery_context.as_ref().ok_or_else(|| {
            recovery_error("rolling-back recovery has no retained origin context")
        })?;
    if context.origin_state != RecoveryOriginStateV1::RollingBack {
        return Err(recovery_error(
            "rollback verifier received a different recovery origin",
        ));
    }
    let aggregate_position = super::rollback_prefix::recovery_position(current)?;
    if matches!(
        super::rollback_prefix::classify_rollback_aggregate(backend, current, aggregate_position,)?,
        super::rollback_prefix::RollbackAggregateClassification::Mismatch
    ) {
        return Err(ModelError::new(
            ErrorCode::RecoveryEvidenceMismatch,
            "live rollback aggregate prefix has drifted",
        ));
    }
    let action =
        current.record().pending_rollback.as_ref().ok_or_else(|| {
            recovery_error("rolling-back recovery has no retained rollback journal")
        })?;
    let exact = match action {
        PendingRollbackActionV1::Participant {
            member_id, action, ..
        } => {
            let row = current
                .record()
                .participants
                .get(member_id)
                .ok_or_else(|| recovery_error("rolling-back recovery participant is missing"))?;
            crate::workspace_ops::merge::abort::observe_v1_participant_rollback(
                backend,
                current.location().root(),
                current.record(),
                member_id,
                row,
                *action,
            )? != crate::workspace_ops::merge::abort::V1ParticipantRollbackObservation::Ambiguous
        }
        PendingRollbackActionV1::PublicationEvidence { next_step } => {
            crate::workspace_ops::merge::abort::observe_v1_evidence_rollback(
                backend,
                current.location().root(),
                current.record(),
                *next_step,
            )? != crate::workspace_ops::merge::abort::V1EvidenceRollbackObservation::Ambiguous
        }
        PendingRollbackActionV1::SelectedRootMetadata { next_step } => {
            crate::workspace_ops::merge::root::observe_v1_root_metadata_rollback(
                backend,
                current.location().root(),
                current.record(),
                *next_step,
            )? != crate::workspace_ops::merge::root::V1RootRollbackObservation::Ambiguous
        }
    };
    if !exact {
        return Err(ModelError::new(
            ErrorCode::RecoveryEvidenceMismatch,
            "live rollback state is neither the exact before nor after state",
        ));
    }
    VerifiedRecoveryOrigin::issue(
        &AuthorityIssuer::for_observer(current),
        "@operation",
        "resume_recovery",
        "verified",
        RecoveryOriginStateV1::RollingBack,
    )
}

fn recovery_error(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::RecoveryEvidenceMismatch, detail.into())
}

mod cursor;
mod entry;
mod phase;

use super::super::super::*;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::OperationContext;
use crate::workspace_ops::merge::model::v1::{
    PendingPreservationActionV1, PreservationOwnerV1, PreservationPublicationHandoffV1,
};
use crate::workspace_ops::merge::preserve::V1PreservationOwnerPlan;

pub(in crate::workspace_ops::merge::v1_lifecycle) use cursor::{
    execution_prefix_is_exact, pending_recovery_is_exact,
};
pub(in crate::workspace_ops::merge::v1_lifecycle) use phase::{
    durability_fact, reset_step, stash_guard, stash_step,
};

pub(in crate::workspace_ops::merge::v1_lifecycle::authority) fn observe<B: GitBackend>(
    backend: &B,
    context: &OperationContext,
    current: &StoredV1Record,
    request: &BoundObservationRequest,
) -> ModelResult<BoundExactObservation> {
    let fact = match request.kind() {
        ObservationKind::ParticipantAction { member_id }
            if request.lifecycle() == V1LifecycleRequest::Preserve =>
        {
            entry::observe_preserve_participant(backend, context, current, request, member_id)?
        }
        ObservationKind::PreservationEntry
            if request.lifecycle() == V1LifecycleRequest::Preserve =>
        {
            entry::observe_entry(backend, context, current)?
        }
        ObservationKind::PreservationCursor => cursor::observe_cursor(backend, context, current)?,
        ObservationKind::Recovery => {
            ExactObservationFact::Completed(CompletedObservation::Recovery(
                super::preserving_recovery::verify_recovery_origin(backend, current)?,
            ))
        }
        _ => {
            return Err(preservation_error(
                "preservation lane received another observation",
            ));
        }
    };
    BoundExactObservation::issue(current, request, fact)
}

fn action_owner(action: &PendingPreservationActionV1) -> &PreservationOwnerV1 {
    match action {
        PendingPreservationActionV1::BackupRef { owner, .. }
        | PendingPreservationActionV1::Stash { owner, .. }
        | PendingPreservationActionV1::ResetAttachedRef { owner, .. } => owner,
    }
}

fn action_position(action: &PendingPreservationActionV1) -> PreservationCursorPosition {
    match action {
        PendingPreservationActionV1::BackupRef { .. } => PreservationCursorPosition::BackupRef,
        PendingPreservationActionV1::Stash { phase, .. } => {
            PreservationCursorPosition::Stash(*phase)
        }
        PendingPreservationActionV1::ResetAttachedRef { phase, .. } => {
            PreservationCursorPosition::ResetAttachedRef(*phase)
        }
    }
}

fn owner_binding(owner: &PreservationOwnerV1) -> &str {
    match owner {
        PreservationOwnerV1::Participant { member_id } => member_id,
        PreservationOwnerV1::PublicationRoot => "@publication-root",
    }
}

fn completed(value: CompletedObservation) -> ExactObservationFact {
    ExactObservationFact::Completed(value)
}

fn model_handoff(value: PublicationHandoffFact) -> PreservationPublicationHandoffV1 {
    use PublicationHandoffFact as H;
    match value {
        H::NoCandidate => PreservationPublicationHandoffV1::NoCandidate,
        H::EvidencePending => PreservationPublicationHandoffV1::EvidencePending,
        H::Candidate { prefix, index } => PreservationPublicationHandoffV1::Candidate {
            prefix: match prefix {
                PublicationHandoffPrefix::Baseline => {
                    crate::workspace_ops::merge::model::v1::PublicationPrefixV1::Baseline
                }
                PublicationHandoffPrefix::Marker => {
                    crate::workspace_ops::merge::model::v1::PublicationPrefixV1::Marker
                }
                PublicationHandoffPrefix::Lock => {
                    crate::workspace_ops::merge::model::v1::PublicationPrefixV1::Lock
                }
                PublicationHandoffPrefix::Boundary => {
                    crate::workspace_ops::merge::model::v1::PublicationPrefixV1::Boundary
                }
            },
            index: match index {
                PublicationHandoffIndex::Pre => {
                    crate::workspace_ops::merge::model::v1::PublicationIndexFormV1::Pre
                }
                PublicationHandoffIndex::Staged => {
                    crate::workspace_ops::merge::model::v1::PublicationIndexFormV1::Staged
                }
            },
        },
    }
}

fn plan_for_action<'a>(
    plans: &'a [V1PreservationOwnerPlan],
    action: &PendingPreservationActionV1,
) -> ModelResult<&'a V1PreservationOwnerPlan> {
    let owner = action_owner(action);
    plans
        .iter()
        .find(|plan| &plan.owner == owner)
        .ok_or_else(|| {
            preservation_error("pending preservation owner is outside the frozen owner order")
        })
}

fn owner_error(plan: &V1PreservationOwnerPlan, detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::PreservationEvidenceMismatch, detail.into())
        .with_member(&plan.target_id, &plan.relative_path)
}

fn preservation_error(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::PreservationEvidenceMismatch, detail.into())
}

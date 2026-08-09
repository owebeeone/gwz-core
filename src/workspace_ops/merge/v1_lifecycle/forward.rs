use super::authority::{
    BoundExactObservation, BoundObservationRequest, ExecutionDiagnostic, ObservationKind,
    PhysicalActionKind, observe_finalization, observe_forward,
};
use super::checked::{StoredV1Record, V1MutationLease};
use super::finalization::FinalizationRuntime;
use super::service::{ExactObserver, PhysicalExecutor};
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::OperationContext;

mod execute;

pub(super) struct ForwardRuntime<'a, B> {
    backend: &'a B,
    context: &'a OperationContext,
}

impl<'a, B> ForwardRuntime<'a, B> {
    pub(super) fn new(backend: &'a B, context: &'a OperationContext) -> Self {
        Self { backend, context }
    }
}

impl<B: GitBackend> ExactObserver for ForwardRuntime<'_, B> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        match request.kind() {
            ObservationKind::ParticipantPreparation { .. }
            | ObservationKind::ParticipantAction { .. }
            | ObservationKind::Recovery => {
                observe_forward(self.backend, self.context, current, request)
            }
            ObservationKind::ParticipantsComplete
            | ObservationKind::Acceptance
            | ObservationKind::Publication => {
                observe_finalization(self.backend, self.context, current, request)
            }
            _ => Err(ModelError::new(
                ErrorCode::MergePhaseUnsupported,
                "v1 forward runtime received an unsupported reverse-lifecycle observation",
            )),
        }
    }
}

impl<B: GitBackend> PhysicalExecutor for ForwardRuntime<'_, B> {
    fn execute(
        &mut self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        if !lease.covers(current.location()) {
            return failure(ModelError::new(
                ErrorCode::MergeRecoveryRequired,
                "v1 forward action is outside the retained mutation lease",
            ));
        }
        match action {
            PhysicalActionKind::Participant { member_id, action } => {
                execute::participant(self.backend, current, member_id, action)
                    .map_or_else(failure, |_| ExecutionDiagnostic::Success)
            }
            PhysicalActionKind::Publication(_) => {
                FinalizationRuntime::new(self.backend, self.context).execute(lease, current, action)
            }
            PhysicalActionKind::Preservation(_)
            | PhysicalActionKind::Rollback(_)
            | PhysicalActionKind::Archive => failure(ModelError::new(
                ErrorCode::MergePhaseUnsupported,
                "v1 forward runtime received a reverse or archive physical action",
            )),
        }
    }
}

fn failure(error: ModelError) -> ExecutionDiagnostic {
    ExecutionDiagnostic::Failed {
        code: error.code,
        message: error.message,
        detail: None,
    }
}

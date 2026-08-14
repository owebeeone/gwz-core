use super::authority::{
    BoundExactObservation, BoundObservationRequest, ExecutionDiagnostic, PhysicalActionKind,
    PublicationPhysicalAction, observe_finalization, verify_finalization_action,
};
use super::checked::{StoredV1Record, V1MutationLease};
use super::service::{ExactObserver, PhysicalExecutor};
use crate::git::MergeAuthorityBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::OperationContext;

mod execute;

pub(super) struct FinalizationRuntime<'a, B> {
    backend: &'a B,
    context: &'a OperationContext,
}

impl<'a, B> FinalizationRuntime<'a, B> {
    pub(super) fn new(backend: &'a B, context: &'a OperationContext) -> Self {
        Self { backend, context }
    }
}

impl<B: MergeAuthorityBackend> ExactObserver for FinalizationRuntime<'_, B> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        observe_finalization(self.backend, self.context, current, request)
    }
}

impl<B: MergeAuthorityBackend> PhysicalExecutor for FinalizationRuntime<'_, B> {
    fn execute(
        &mut self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        let PhysicalActionKind::Publication(action) = action else {
            return failure(ModelError::new(
                ErrorCode::MergePhaseUnsupported,
                "v1 finalization runtime received a non-publication physical action",
            ));
        };
        if !lease.covers(current.location()) {
            return failure(ModelError::new(
                ErrorCode::MergeRecoveryRequired,
                "v1 finalization action is outside the retained mutation lease",
            ));
        }
        execute::publication(self.backend, current, *action)
            .map_or_else(failure, |_| ExecutionDiagnostic::Success)
    }
}

fn failure(error: ModelError) -> ExecutionDiagnostic {
    ExecutionDiagnostic::Failed {
        code: error.code,
        message: error.message,
        detail: None,
    }
}

#[cfg(test)]
#[path = "tests/finalization.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/finalization_inputs.rs"]
mod input_tests;

#[cfg(test)]
#[path = "tests/finalization_root.rs"]
mod root_tests;

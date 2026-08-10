use super::archive;
use super::authority::{
    BoundExactObservation, BoundObservationRequest, ExecutionDiagnostic, ObservationKind,
    PhysicalActionKind, V1LifecycleRequest, observe_forward,
};
use super::checked::{StoredV1Record, V1MutationLease};
use super::service::{ExactObserver, PhysicalExecutor};
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::OperationContext;
use crate::workspace_ops::merge::model::v1::RecoveryOriginStateV1;

mod execute;
mod preservation;
mod rollback;

pub(in crate::workspace_ops::merge::v1_lifecycle) use execute::preservation::durability_diagnostic as preservation_durability_diagnostic;

pub(super) struct ReverseRuntime<'a, B> {
    backend: &'a B,
    context: &'a OperationContext,
}

impl<'a, B> ReverseRuntime<'a, B> {
    pub(super) fn new(backend: &'a B, context: &'a OperationContext) -> Self {
        Self { backend, context }
    }
}

impl<B: GitBackend> ExactObserver for ReverseRuntime<'_, B> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        let origin = current
            .record()
            .recovery_context
            .as_ref()
            .map(|context| context.origin_state);
        match observation_route(request.lifecycle(), request.kind(), origin)? {
            ObservationRoute::Preservation => {
                preservation::observe(self.backend, self.context, current, request)
            }
            ObservationRoute::Rollback => {
                rollback::observe(self.backend, self.context, current, request)
            }
            ObservationRoute::Forward => {
                observe_forward(self.backend, self.context, current, request)
            }
            ObservationRoute::Archive => {
                archive::observe_open(self.backend, self.context, current, request)
            }
        }
    }
}

impl<B: GitBackend> PhysicalExecutor for ReverseRuntime<'_, B> {
    fn execute(
        &mut self,
        lease: &V1MutationLease,
        current: &StoredV1Record,
        action: &PhysicalActionKind,
    ) -> ExecutionDiagnostic {
        if !lease.covers(current.location()) {
            return failure(route_error(
                "reverse action is outside the retained mutation lease",
            ));
        }
        match action {
            PhysicalActionKind::Preservation(_) => {
                execute::preservation::execute(self.backend, lease, current, action)
            }
            PhysicalActionKind::Rollback(_) => {
                execute::rollback::execute(self.backend, lease, current, action)
            }
            PhysicalActionKind::Participant { .. }
            | PhysicalActionKind::Publication(_)
            | PhysicalActionKind::Archive => failure(route_error(
                "reverse runtime received a forward or archive physical action",
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ObservationRoute {
    Preservation,
    Rollback,
    Forward,
    Archive,
}

pub(super) fn observation_route(
    request: V1LifecycleRequest,
    kind: &ObservationKind,
    recovery_origin: Option<RecoveryOriginStateV1>,
) -> ModelResult<ObservationRoute> {
    match kind {
        ObservationKind::ParticipantAction { .. } if request == V1LifecycleRequest::Preserve => {
            Ok(ObservationRoute::Preservation)
        }
        ObservationKind::ParticipantAction { .. } if request == V1LifecycleRequest::Abort => {
            Ok(ObservationRoute::Rollback)
        }
        ObservationKind::PreservationEntry if request == V1LifecycleRequest::Preserve => {
            Ok(ObservationRoute::Preservation)
        }
        ObservationKind::PreservationCursor if request != V1LifecycleRequest::Status => {
            Ok(ObservationRoute::Preservation)
        }
        ObservationKind::RollbackEntry if request == V1LifecycleRequest::Abort => {
            Ok(ObservationRoute::Rollback)
        }
        ObservationKind::RollbackCursor if request != V1LifecycleRequest::Status => {
            Ok(ObservationRoute::Rollback)
        }
        ObservationKind::Recovery => match recovery_origin {
            Some(RecoveryOriginStateV1::Preserving) => Ok(ObservationRoute::Preservation),
            Some(RecoveryOriginStateV1::RollingBack) => Ok(ObservationRoute::Rollback),
            Some(_) => Ok(ObservationRoute::Forward),
            None => Err(route_error("recovery observation has no recorded origin")),
        },
        ObservationKind::Archive if request == V1LifecycleRequest::Archive => {
            Ok(ObservationRoute::Archive)
        }
        _ => Err(route_error(
            "reverse runtime received an unsupported forward-lifecycle observation",
        )),
    }
}

pub(super) fn route_error(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergePhaseUnsupported, detail.into())
}

pub(super) fn failure(error: ModelError) -> ExecutionDiagnostic {
    ExecutionDiagnostic::Failed {
        code: error.code,
        message: error.message,
        detail: None,
    }
}

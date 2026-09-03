use super::authority::{
    BoundExactObservation, BoundObservationRequest, ExecutionDiagnostic, ObservationKind,
    PhysicalActionKind, V1LifecycleRequest, observe_finalization, observe_forward,
    preflight_continue_siblings,
};
use std::collections::BTreeSet;

use super::checked::{StoredV1Record, V1MutationLease};
use super::finalization::FinalizationRuntime;
use super::service::{ExactObserver, PhysicalExecutor};
use crate::git::MergeAuthorityBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::OperationContext;

mod execute;

pub(super) struct ForwardRuntime<'a, B> {
    backend: &'a B,
    context: &'a OperationContext,
    /// v0 parity: `continue_op::execution::preflight` ran ONCE per
    /// `handle_continue` call. One `ForwardRuntime` is built per invocation
    /// (`start.rs:84` for a start, `start.rs:209` for a continue), so this
    /// latch is exactly "once per continue" — see
    /// `preflight_continue_siblings`.
    continue_preflight_done: bool,
    /// The participants whose physical action THIS invocation performed.
    /// `observe_participant_action` needs it to know whether the live conflict
    /// markers it is about to observe are the original ones (this process just
    /// produced them) or bytes that may have been edited while the pending
    /// action sat durable across processes — see that function's v0-parity
    /// note. Same per-invocation lifetime as the latch above.
    executed_participants: BTreeSet<String>,
}

impl<'a, B> ForwardRuntime<'a, B> {
    pub(super) fn new(backend: &'a B, context: &'a OperationContext) -> Self {
        Self {
            backend,
            context,
            continue_preflight_done: false,
            executed_participants: BTreeSet::new(),
        }
    }
}

impl<B: MergeAuthorityBackend> ExactObserver for ForwardRuntime<'_, B> {
    fn observe(
        &mut self,
        current: &StoredV1Record,
        request: &BoundObservationRequest,
    ) -> ModelResult<BoundExactObservation> {
        match request.kind() {
            ObservationKind::ParticipantPreparation { .. }
            | ObservationKind::ParticipantAction { .. }
            | ObservationKind::Recovery => {
                // The whole-set continue gate, at v0's exact point: the first
                // PREPARATION of a continue. `next_action` dispatches every
                // durable `ParticipantAction` reconciliation before any
                // preparation, so this lands after reconciliation and before
                // the first mutation, and refuses the continue as a unit.
                if let ObservationKind::ParticipantPreparation { member_id } = request.kind()
                    && !self.continue_preflight_done
                    && request.lifecycle() == V1LifecycleRequest::Continue
                {
                    preflight_continue_siblings(self.backend, current, member_id)?;
                    self.continue_preflight_done = true;
                }
                let executed_here = match request.kind() {
                    ObservationKind::ParticipantAction { member_id } => {
                        self.executed_participants.contains(member_id)
                    }
                    _ => false,
                };
                observe_forward(
                    self.backend,
                    self.context,
                    current,
                    request,
                    executed_here,
                )
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

impl<B: MergeAuthorityBackend> PhysicalExecutor for ForwardRuntime<'_, B> {
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
                // Recorded BEFORE the attempt: once this process has touched
                // this repository's merge, the live markers are its own work,
                // whether the attempt reported success or failure.
                self.executed_participants.insert(member_id.clone());
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

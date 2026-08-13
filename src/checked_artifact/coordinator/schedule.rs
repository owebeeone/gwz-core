//! Coordinator-owned mapping from a validated request to an R1 reservation.

use crate::checked_artifact::bootstrap::ManagedParentPlanV1;
use crate::checked_artifact::capability::CheckedFsError;
use crate::checked_artifact::protocol::{
    ActionCapacityReservationV1, ActionScheduleV1, CleanupAliasSetV1,
    MAX_BARRIER_INVOCATIONS_PER_ACTION,
};

use super::{CheckedActionOperationV1, CheckedActionRequestV1, CheckedLeafFactV1};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum CoordinatorScheduleDecisionV1 {
    ProofOnly,
    Reserve(Box<ActionCapacityReservationV1>),
}

pub(in crate::checked_artifact) fn derive_new_reservation(
    request: &CheckedActionRequestV1,
    managed_plan: Option<&ManagedParentPlanV1>,
) -> Result<CoordinatorScheduleDecisionV1, CheckedFsError> {
    if managed_plan.is_some_and(|plan| {
        plan.action_digest() != request.action_digest()
            || plan.request_owner_binding() != request.owner_binding()
    }) {
        return Err(schedule_error(
            "managed-parent plan belongs to another checked action",
        ));
    }
    let cleanup_mask = match request.operation() {
        CheckedActionOperationV1::Observe => return Ok(CoordinatorScheduleDecisionV1::ProofOnly),
        CheckedActionOperationV1::Replace if request.expected() == request.goal() => {
            return Ok(CoordinatorScheduleDecisionV1::ProofOnly);
        }
        CheckedActionOperationV1::ParentOnly => 0,
        CheckedActionOperationV1::Replace => match request.expected() {
            CheckedLeafFactV1::Missing => 0b110,
            CheckedLeafFactV1::Exact { .. } => 0b111,
        },
        CheckedActionOperationV1::Remove => 0b101,
    };
    if request.operation() == CheckedActionOperationV1::ParentOnly
        && managed_plan.is_some_and(ManagedParentPlanV1::is_proof_only)
    {
        return Ok(CoordinatorScheduleDecisionV1::ProofOnly);
    }
    if request.operation() == CheckedActionOperationV1::ParentOnly && managed_plan.is_none() {
        return Err(schedule_error("parent-only action has no immutable plan"));
    }
    let cleanup = CleanupAliasSetV1::from_mask(cleanup_mask)
        .ok_or_else(|| schedule_error("coordinator derived an invalid cleanup set"))?;
    let schedule = match managed_plan {
        Some(plan) => ActionScheduleV1::try_from_managed_plan(
            MAX_BARRIER_INVOCATIONS_PER_ACTION,
            plan.schedule_inputs(),
            cleanup,
        ),
        None => ActionScheduleV1::try_new(MAX_BARRIER_INVOCATIONS_PER_ACTION, Vec::new(), cleanup),
    }
    .map_err(|_| schedule_error("coordinator request cannot fit the v1 schedule"))?;
    Ok(CoordinatorScheduleDecisionV1::Reserve(Box::new(
        ActionCapacityReservationV1::new(
            request.action_digest(),
            request.owner_binding(),
            schedule,
        ),
    )))
}

fn schedule_error(detail: &'static str) -> CheckedFsError {
    CheckedFsError::ambiguous("checked action schedule", detail)
}

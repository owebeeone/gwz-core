use super::super::*;
use crate::workspace_ops::merge::OperationState;
use crate::workspace_ops::merge::model::v1::PendingRollbackActionV1;

pub(in crate::workspace_ops::merge::v1_lifecycle::reverse) fn execute<B: MergeAuthorityBackend>(
    backend: &B,
    lease: &V1MutationLease,
    current: &StoredV1Record,
    action: &PhysicalActionKind,
) -> ExecutionDiagnostic {
    match execute_checked(backend, lease, current, action) {
        Ok(()) => ExecutionDiagnostic::Success,
        Err(error) => failure(error),
    }
}

fn execute_checked<B: MergeAuthorityBackend>(
    backend: &B,
    lease: &V1MutationLease,
    current: &StoredV1Record,
    action: &PhysicalActionKind,
) -> ModelResult<()> {
    if !lease.covers(current.location()) || current.record().state != OperationState::RollingBack {
        return Err(route_error(
            "rollback execution is outside its checked lease or durable state",
        ));
    }
    let PhysicalActionKind::Rollback(action) = action else {
        return Err(route_error(
            "rollback executor received another action kind",
        ));
    };
    if current.record().pending_rollback.as_ref() != Some(action) {
        return Err(route_error(
            "rollback executor action does not match the persisted journal",
        ));
    }
    crate::workspace_ops::merge::v1_lifecycle::authority::require_rollback_aggregate(
        backend, current,
    )?;
    match action {
        PendingRollbackActionV1::Participant {
            member_id, action, ..
        } => {
            let row = current
                .record()
                .participants
                .get(member_id)
                .ok_or_else(|| {
                    route_error("rollback participant is missing from the checked record")
                })?;
            crate::workspace_ops::merge::abort::execute_v1_participant_rollback(
                backend,
                current.location().root(),
                current.record(),
                member_id,
                row,
                *action,
            )
        }
        PendingRollbackActionV1::PublicationEvidence { next_step } => {
            crate::workspace_ops::merge::abort::execute_v1_evidence_rollback(
                backend,
                current.location().root(),
                current.record(),
                *next_step,
            )
        }
        PendingRollbackActionV1::SelectedRootMetadata { next_step } => {
            crate::workspace_ops::merge::root::execute_v1_root_metadata_rollback(
                backend,
                current.location().root(),
                current.record(),
                *next_step,
            )
        }
    }
}

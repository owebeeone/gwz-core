use super::super::authority::{
    BoundObservationRequest, ExecutionDiagnostic, ObservationKind, PhysicalActionKind,
    PublicationPhysicalAction, V1LifecycleRequest,
};
use super::super::checked::{StoredV1Record, V1MutationLease};
use super::super::reverse::{ObservationRoute, ReverseRuntime, observation_route};
use super::super::service::{ExactObserver, PhysicalExecutor};
use super::fixtures::{backup_action, preserving_record, up_to_date_action};
use crate::git::Git2Backend;
use crate::model::ErrorCode;
use crate::operation::{ActionKind, OperationContext};
use crate::workspace_ops::merge::ParticipantState;
use crate::workspace_ops::merge::model::v1::{
    ParticipantRollbackKindV1, PendingRollbackActionV1, RecoveryOriginStateV1, test_record,
};
use crate::workspace_ops::tests::TempDir;

#[test]
fn observation_route_matrix_is_closed_without_replaceable_lane_traits() {
    let participant = ObservationKind::ParticipantAction {
        member_id: "mem_a".into(),
    };
    for (request, kind, origin, expected) in [
        (
            V1LifecycleRequest::Preserve,
            participant.clone(),
            None,
            ObservationRoute::Preservation,
        ),
        (
            V1LifecycleRequest::Abort,
            participant.clone(),
            None,
            ObservationRoute::Rollback,
        ),
        (
            V1LifecycleRequest::Preserve,
            ObservationKind::PreservationEntry,
            None,
            ObservationRoute::Preservation,
        ),
        (
            V1LifecycleRequest::Continue,
            ObservationKind::PreservationCursor,
            None,
            ObservationRoute::Preservation,
        ),
        (
            V1LifecycleRequest::Abort,
            ObservationKind::RollbackEntry,
            None,
            ObservationRoute::Rollback,
        ),
        (
            V1LifecycleRequest::Continue,
            ObservationKind::RollbackCursor,
            None,
            ObservationRoute::Rollback,
        ),
        (
            V1LifecycleRequest::Continue,
            ObservationKind::Recovery,
            Some(RecoveryOriginStateV1::Preserving),
            ObservationRoute::Preservation,
        ),
        (
            V1LifecycleRequest::Continue,
            ObservationKind::Recovery,
            Some(RecoveryOriginStateV1::RollingBack),
            ObservationRoute::Rollback,
        ),
        (
            V1LifecycleRequest::Continue,
            ObservationKind::Recovery,
            Some(RecoveryOriginStateV1::Executing),
            ObservationRoute::Forward,
        ),
        (
            V1LifecycleRequest::Continue,
            ObservationKind::Recovery,
            Some(RecoveryOriginStateV1::AwaitingResolution),
            ObservationRoute::Forward,
        ),
        (
            V1LifecycleRequest::Continue,
            ObservationKind::Recovery,
            Some(RecoveryOriginStateV1::Halted),
            ObservationRoute::Forward,
        ),
        (
            V1LifecycleRequest::Continue,
            ObservationKind::Recovery,
            Some(RecoveryOriginStateV1::Finalizing),
            ObservationRoute::Forward,
        ),
        (
            V1LifecycleRequest::Archive,
            ObservationKind::Archive,
            None,
            ObservationRoute::Archive,
        ),
    ] {
        assert_eq!(observation_route(request, &kind, origin).unwrap(), expected);
    }

    for (request, kind, origin) in [
        (V1LifecycleRequest::Continue, participant, None),
        (
            V1LifecycleRequest::Abort,
            ObservationKind::Publication,
            None,
        ),
        (
            V1LifecycleRequest::Abort,
            ObservationKind::PreservationEntry,
            None,
        ),
        (
            V1LifecycleRequest::Preserve,
            ObservationKind::RollbackEntry,
            None,
        ),
        (
            V1LifecycleRequest::Status,
            ObservationKind::PreservationCursor,
            None,
        ),
        (
            V1LifecycleRequest::Status,
            ObservationKind::RollbackCursor,
            None,
        ),
        (V1LifecycleRequest::Continue, ObservationKind::Archive, None),
        (
            V1LifecycleRequest::Continue,
            ObservationKind::Recovery,
            None,
        ),
    ] {
        assert_eq!(
            observation_route(request, &kind, origin).unwrap_err().code,
            ErrorCode::MergePhaseUnsupported
        );
    }
}

#[test]
fn runtime_delegates_to_the_authority_owned_reverse_and_archive_observers() {
    let root = TempDir::new("merge-v1-reverse-runtime-delegates");
    let backend = Git2Backend::new();
    let operation_context = context();
    let current = StoredV1Record::for_test(&root.path, test_record()).unwrap();
    let mut runtime = ReverseRuntime::new(&backend, &operation_context);

    let preservation = BoundObservationRequest::for_test(
        &current,
        V1LifecycleRequest::Preserve,
        ObservationKind::PreservationEntry,
    )
    .unwrap();
    let error = match runtime.observe(&current, &preservation) {
        Ok(_) => panic!("preservation unexpectedly produced an observation"),
        Err(error) => error,
    };
    assert_eq!(error.code, ErrorCode::GitCommandFailed);
    assert_ne!(error.message, "preservation observation is not implemented");

    let rollback = BoundObservationRequest::for_test(
        &current,
        V1LifecycleRequest::Abort,
        ObservationKind::RollbackEntry,
    )
    .unwrap();
    let error = match runtime.observe(&current, &rollback) {
        Ok(_) => panic!("rollback unexpectedly produced an observation"),
        Err(error) => error,
    };
    assert_ne!(error.message, "rollback observation is not implemented");

    let archive = BoundObservationRequest::for_test(
        &current,
        V1LifecycleRequest::Archive,
        ObservationKind::Archive,
    )
    .unwrap();
    let error = match runtime.observe(&current, &archive) {
        Ok(_) => panic!("archive unexpectedly produced an observation"),
        Err(error) => error,
    };
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert!(
        error
            .message
            .contains("exact bound terminal archive request")
    );
}

#[test]
fn physical_router_has_only_two_closed_reverse_delegates() {
    let root = TempDir::new("merge-v1-reverse-physical-router");
    let other = TempDir::new("merge-v1-reverse-physical-router-other");
    let backend = Git2Backend::new();
    let operation_context = context();
    let current = StoredV1Record::for_test(&root.path, preserving_record()).unwrap();
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let wrong_lease = V1MutationLease::acquire_for_test(&other.path).unwrap();
    let mut runtime = ReverseRuntime::new(&backend, &operation_context);

    assert_failure(
        runtime.execute(
            &lease,
            &current,
            &PhysicalActionKind::Preservation(backup_action()),
        ),
        "preservation executor action does not match the persisted journal",
    );
    assert_failure(
        runtime.execute(
            &lease,
            &current,
            &PhysicalActionKind::Rollback(rollback_action()),
        ),
        "rollback execution is outside its checked lease or durable state",
    );
    for action in [
        PhysicalActionKind::Participant {
            member_id: "mem_a".into(),
            action: Box::new(up_to_date_action()),
        },
        PhysicalActionKind::Publication(PublicationPhysicalAction::WriteMarker),
        PhysicalActionKind::Archive,
    ] {
        assert_failure(
            runtime.execute(&lease, &current, &action),
            "reverse runtime received a forward or archive physical action",
        );
    }
    assert_failure(
        runtime.execute(
            &wrong_lease,
            &current,
            &PhysicalActionKind::Rollback(rollback_action()),
        ),
        "reverse action is outside the retained mutation lease",
    );
}

fn assert_failure(diagnostic: ExecutionDiagnostic, expected: &str) {
    assert_eq!(
        diagnostic,
        ExecutionDiagnostic::Failed {
            code: ErrorCode::MergePhaseUnsupported,
            message: expected.into(),
            detail: None,
        }
    );
}

fn rollback_action() -> PendingRollbackActionV1 {
    PendingRollbackActionV1::Participant {
        member_id: "mem_a".into(),
        action: ParticipantRollbackKindV1::ResetIntegrated,
        terminal_state: ParticipantState::RolledBack,
    }
}

fn context() -> OperationContext {
    OperationContext {
        operation_id: "op_1".into(),
        request_id: "req_1".into(),
        schema_version: "gwz.protocol/v0".into(),
        action: ActionKind::Merge,
        dry_run: false,
        attribution: None,
    }
}

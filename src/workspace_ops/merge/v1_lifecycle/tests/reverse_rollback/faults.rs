use super::*;
use crate::workspace_ops::merge::model::v1::ParticipantRollbackKindV1;
use crate::workspace_ops::merge::model::v1::PendingRollbackActionV1;
use crate::workspace_ops::merge::v1_lifecycle::authority::{
    BoundObservationRequest, ObservationKind, PhysicalActionKind, ResolvedV1Action,
    V1LifecycleRequest, observe_rollback, resolve_observation,
};
use crate::workspace_ops::merge::v1_lifecycle::checked::StoredV1Record;
use crate::workspace_ops::merge::v1_rollback::{
    V1ParticipantRollbackObservation, execute_v1_participant_rollback,
    observe_v1_participant_rollback,
};

#[test]
fn participant_executor_is_checked_and_restart_observes_completion() {
    let fixture = integrated_fixture("v1-rollback-participant-execute");
    let row = &fixture.model.participants["mem_a"];
    execute_v1_participant_rollback(
        &fixture.backend,
        &fixture.root.path,
        &fixture.model,
        "mem_a",
        row,
        ParticipantRollbackKindV1::ResetIntegrated,
    )
    .unwrap();
    assert_eq!(
        observe_v1_participant_rollback(
            &fixture.backend,
            &fixture.root.path,
            &fixture.model,
            "mem_a",
            row,
            ParticipantRollbackKindV1::ResetIntegrated,
        )
        .unwrap(),
        V1ParticipantRollbackObservation::After
    );
    assert!(
        execute_v1_participant_rollback(
            &fixture.backend,
            &fixture.root.path,
            &fixture.model,
            "mem_a",
            row,
            ParticipantRollbackKindV1::ResetIntegrated,
        )
        .is_err()
    );
}

#[test]
fn authority_wrapper_binds_before_and_after_to_the_exact_rollback_action() {
    let mut fixture = integrated_fixture("v1-rollback-authority-wrapper");
    let pending = PendingRollbackActionV1::Participant {
        member_id: "mem_a".into(),
        action: ParticipantRollbackKindV1::ResetIntegrated,
        terminal_state: ParticipantState::RolledBack,
    };
    fixture.model.pending_rollback = Some(pending.clone());
    let current = StoredV1Record::for_test(&fixture.root.path, fixture.model.clone()).unwrap();
    let request = BoundObservationRequest::for_test(
        &current,
        V1LifecycleRequest::Abort,
        ObservationKind::RollbackCursor,
    )
    .unwrap();
    let context = crate::operation::OperationContext {
        operation_id: "op_rollback".into(),
        request_id: "req_rollback".into(),
        schema_version: "gwz.protocol/v0".into(),
        action: crate::operation::ActionKind::Merge,
        dry_run: false,
        attribution: None,
    };
    let observation = observe_rollback(&fixture.backend, &context, &current, &request).unwrap();
    match resolve_observation(
        &current,
        V1LifecycleRequest::Abort,
        request,
        observation,
        None,
    )
    .unwrap()
    {
        ResolvedV1Action::Execute(action) => {
            assert_eq!(
                action.kind(),
                &PhysicalActionKind::Rollback(pending.clone())
            )
        }
        _ => panic!("exact rollback before-state did not authorize execution"),
    }

    execute_v1_participant_rollback(
        &fixture.backend,
        &fixture.root.path,
        &fixture.model,
        "mem_a",
        &fixture.model.participants["mem_a"],
        ParticipantRollbackKindV1::ResetIntegrated,
    )
    .unwrap();
    let request = BoundObservationRequest::for_test(
        &current,
        V1LifecycleRequest::Abort,
        ObservationKind::RollbackCursor,
    )
    .unwrap();
    let observation = observe_rollback(&fixture.backend, &context, &current, &request).unwrap();
    assert!(matches!(
        resolve_observation(
            &current,
            V1LifecycleRequest::Abort,
            request,
            observation,
            None,
        )
        .unwrap(),
        ResolvedV1Action::Apply(_)
    ));
}

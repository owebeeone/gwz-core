use super::super::capability::{
    AsciiComponent, CanonicalComponent, CanonicalPathIdentityV1, DurableObjectIdentityV1,
    PathComponentMode,
};
use super::super::namespace::test_support::{
    backend_events, barrier_target, bootstrap_target, installed_component_evidence,
    recording_backend, retained_directory_for, retained_object_for, retired_marker_evidence,
};
use super::super::namespace::{ActionNamespace, NamespaceObjectKind, PublishRoleV1};
use super::super::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ActionDirectoryAdmissionV1,
    ActionDirectoryObservationV1, ActionScheduleV1, BootstrapOrdinalV1, CleanupAliasSetV1,
    CleanupAliasV1, ManagedBootstrapInputV1, ManagedParentBootstrapIntentV1, OwnershipMarkerV1,
    RecordObservationV1, RequestOwnerBindingV1, admit_observed_action,
};

fn identity(value: u8) -> DurableObjectIdentityV1 {
    DurableObjectIdentityV1::linux_ext4([value; 16], 1, vec![value]).unwrap()
}

fn path(value: &[u8]) -> CanonicalPathIdentityV1 {
    CanonicalPathIdentityV1::new(vec![CanonicalComponent::new(
        AsciiComponent::parse(value).unwrap(),
        PathComponentMode::Sensitive,
    )])
    .unwrap()
}

fn reservation(
    action: u8,
    barriers: usize,
    bootstrap_components: &[usize],
    aliases: CleanupAliasSetV1,
) -> ActionCapacityReservationV1 {
    let inputs = bootstrap_components
        .iter()
        .enumerate()
        .map(|(index, count)| ManagedBootstrapInputV1::new([index as u8 + 1; 32], *count).unwrap())
        .collect();
    ActionCapacityReservationV1::new(
        ActionDigestV1::new([action; 32]),
        RequestOwnerBindingV1::new([action + 1; 32]),
        ActionScheduleV1::try_new(barriers, inputs, aliases).unwrap(),
    )
}

fn admitted(
    reservation: &ActionCapacityReservationV1,
    directory: u8,
) -> super::super::protocol::AdmittedActionV1 {
    admit_observed_action(
        &ActionDirectoryAdmissionV1::idle(),
        reservation,
        &ActionDirectoryObservationV1::Missing,
        &ActionDirectoryObservationV1::exact(
            identity(directory),
            RecordObservationV1::Exact(reservation.clone()),
        ),
    )
    .unwrap()
}

#[test]
fn only_role_typed_scheduled_slots_are_issued() {
    let reservation = reservation(3, 1, &[2], CleanupAliasSetV1::all());
    let backend = recording_backend([9; 32], identity(7));
    let target = barrier_target(
        &backend,
        retained_directory_for(&backend, 2, identity(8), path(b"target")),
        AsciiComponent::parse(b"index").unwrap(),
        &reservation,
        0,
    );
    let bootstrap_component = |ordinal: usize| {
        bootstrap_target(
            &backend,
            retained_directory_for(&backend, 3, identity(9), path(b"managed")),
            &reservation,
            ordinal,
            AsciiComponent::parse(format!("final-{ordinal}").as_bytes()).unwrap(),
        )
        .unwrap()
    };
    let component0 = bootstrap_component(0);
    let component1 = bootstrap_component(1);
    let namespace = ActionNamespace::from_admitted(backend, admitted(&reservation, 7));

    let goal = namespace.publish_destination(PublishRoleV1::GoalPayload);
    assert!(goal.leaf().as_bytes().ends_with(b"goal-payload-v1"));
    assert_eq!(goal.reservation_binding(), reservation.record_digest());

    let barrier = namespace
        .barrier_slots(namespace.scheduled_barrier(0).unwrap(), target)
        .unwrap();
    assert_eq!(barrier.ordinal().index(), 0);
    assert!(
        namespace
            .barrier_intent(
                &barrier,
                identity(20),
                identity(21),
                AsciiComponent::parse(b"anchor-home").unwrap(),
            )
            .is_ok()
    );
    assert!(namespace.scheduled_barrier(64).is_err());

    let bootstrap = namespace.bootstrap_slots(0).unwrap();
    let generation = bootstrap.generation(0).unwrap();
    assert!(
        generation
            .active_leaf()
            .as_bytes()
            .ends_with(b"active-00-v1")
    );
    assert!(
        generation
            .retired_leaf()
            .as_bytes()
            .ends_with(b"retired-00-v1")
    );
    assert!(
        generation
            .scratch_leaf()
            .as_bytes()
            .ends_with(b"bootstrap-intent-scratch-v1")
    );
    assert!(bootstrap.generation(4).is_ok());
    assert!(bootstrap.generation(5).is_err());
    let component = bootstrap.component(0, component0).unwrap();
    assert!(
        component
            .staging_leaf()
            .as_bytes()
            .ends_with(b"-00-staging-v1")
    );
    assert_eq!(component.final_leaf().as_bytes(), b"final-0");
    assert!(
        component
            .marker_retired_leaf()
            .as_bytes()
            .ends_with(b"retired-bootstrap-marker-00-v1")
    );
    assert!(bootstrap.component(1, component1).is_ok());
    assert_eq!(bootstrap.component_range_for_test(), 0..2);
    assert!(namespace.bootstrap_slots(8).is_err());
}

#[test]
fn omitted_cleanup_aliases_and_global_schedule_limits_reject() {
    let reservation = reservation(
        3,
        0,
        &[1, 1, 1, 1, 1, 1, 1, 1],
        CleanupAliasSetV1::from_mask(0b001).unwrap(),
    );
    let backend = recording_backend([9; 32], identity(7));
    let component_target = |ordinal: usize| {
        bootstrap_target(
            &backend,
            retained_directory_for(&backend, 3, identity(9), path(b"managed")),
            &reservation,
            ordinal,
            AsciiComponent::parse(format!("final-{ordinal}").as_bytes()).unwrap(),
        )
        .unwrap()
    };
    let component7 = component_target(7);
    let component8_rejected = bootstrap_target(
        &backend,
        retained_directory_for(&backend, 3, identity(9), path(b"managed")),
        &reservation,
        8,
        AsciiComponent::parse(b"final-8").unwrap(),
    )
    .is_err();
    let namespace = ActionNamespace::from_admitted(backend, admitted(&reservation, 7));
    assert!(namespace.cleanup_retirement(CleanupAliasV1::Source).is_ok());
    assert!(namespace.cleanup_retirement(CleanupAliasV1::Goal).is_err());
    assert!(
        namespace
            .cleanup_retirement(CleanupAliasV1::Authority)
            .is_err()
    );
    assert!(namespace.scheduled_barrier(0).is_err());
    let bootstrap = namespace.bootstrap_slots(7).unwrap();
    assert!(bootstrap.generation(23).is_ok());
    assert!(bootstrap.generation(24).is_err());
    assert!(bootstrap.component(7, component7).is_ok());
    assert_eq!(bootstrap.component_range_for_test(), 7..8);
    assert!(component8_rejected);
}

#[test]
fn wrapper_rejects_cross_reservation_and_cross_provider_capabilities() {
    let first_reservation = reservation(3, 1, &[], CleanupAliasSetV1::all());
    let second_reservation = reservation(4, 1, &[], CleanupAliasSetV1::all());
    let first_backend = recording_backend([9; 32], identity(7));
    let second_backend = recording_backend([10; 32], identity(7));
    let first_source = retained_object_for(
        &first_backend,
        1,
        identity(7),
        path(b"action"),
        AsciiComponent::parse(b"source").unwrap(),
        11,
        identity(11),
        NamespaceObjectKind::RegularFile,
    );
    let cross_reservation_barrier_target = barrier_target(
        &first_backend,
        retained_directory_for(&first_backend, 20, identity(20), path(b"barrier")),
        AsciiComponent::parse(b"target").unwrap(),
        &second_reservation,
        0,
    );
    let bound_barrier_target = barrier_target(
        &first_backend,
        retained_directory_for(&first_backend, 20, identity(20), path(b"barrier")),
        AsciiComponent::parse(b"target").unwrap(),
        &first_reservation,
        0,
    );
    let wrong_barrier_parent =
        retained_directory_for(&first_backend, 21, identity(21), path(b"other"));
    let second_source = retained_object_for(
        &second_backend,
        1,
        identity(7),
        path(b"action"),
        AsciiComponent::parse(b"source").unwrap(),
        11,
        identity(11),
        NamespaceObjectKind::RegularFile,
    );
    let mut first = ActionNamespace::from_admitted(first_backend, admitted(&first_reservation, 7));
    let second = ActionNamespace::from_admitted(second_backend, admitted(&second_reservation, 7));
    let first_destination = first.publish_destination(PublishRoleV1::GoalPayload);
    let second_destination = second.publish_destination(PublishRoleV1::GoalPayload);

    assert!(
        first
            .publish_no_replace(&first_source, &first_destination)
            .is_ok()
    );
    assert!(
        first
            .publish_no_replace(&first_source, &second_destination)
            .is_err()
    );
    assert!(
        first
            .publish_no_replace(&second_source, &first_destination)
            .is_err()
    );
    assert!(
        first
            .barrier_slots(
                first.scheduled_barrier(0).unwrap(),
                cross_reservation_barrier_target,
            )
            .is_err()
    );
    let bound_barrier = first
        .barrier_slots(first.scheduled_barrier(0).unwrap(), bound_barrier_target)
        .unwrap();
    assert!(
        first
            .barrier_namespace(&wrong_barrier_parent, &bound_barrier)
            .is_err()
    );
    assert_eq!(backend_events(&first).len(), 1);
    assert!(backend_events(&second).is_empty());
}

#[test]
fn action_directory_is_revalidated_before_every_forwarded_operation() {
    let reservation = reservation(3, 0, &[], CleanupAliasSetV1::all());
    let backend = recording_backend([9; 32], identity(99));
    let source = retained_object_for(
        &backend,
        1,
        identity(7),
        path(b"action"),
        AsciiComponent::parse(b"source").unwrap(),
        11,
        identity(11),
        NamespaceObjectKind::RegularFile,
    );
    let mut namespace = ActionNamespace::from_admitted(backend, admitted(&reservation, 7));
    let destination = namespace.publish_destination(PublishRoleV1::GoalPayload);
    assert!(namespace.publish_no_replace(&source, &destination).is_err());
    assert!(backend_events(&namespace).is_empty());
}

#[test]
fn namespace_source_contains_no_consumer_backend_escape_hatch() {
    let source = include_str!("../namespace/mod.rs");
    assert!(!source.contains("fn implementation("));
    assert!(!source.contains("fn implementation_mut("));
    assert!(!source.contains("pub(super) fn reserve_action_slot"));
    assert!(!source.contains("pub(super) fn reserve_action_retirement_slot"));
    assert!(!source.contains("reserved_target_leaf: AsciiComponent"));
    let evidence = include_str!("../namespace/evidence.rs");
    assert!(!evidence.contains("pub(in crate::checked_artifact) fn new"));
    assert!(!evidence.contains("pub(super) fn new"));
}

#[test]
fn managed_success_evidence_is_schedule_role_and_observation_bound() {
    let other_reservation = reservation(4, 0, &[1], CleanupAliasSetV1::all());
    let reservation = reservation(3, 0, &[1], CleanupAliasSetV1::all());
    let backend = recording_backend([9; 32], identity(7));
    let intent = ManagedParentBootstrapIntentV1::try_initial_for_test(
        &reservation,
        [1; 32],
        BootstrapOrdinalV1::new(0).unwrap(),
        identity(30),
        PathComponentMode::Sensitive,
        path(b"retained"),
        vec![AsciiComponent::parse(b"final").unwrap()],
        [5; 32],
    )
    .unwrap();
    let marker = OwnershipMarkerV1::for_current_component(&intent).unwrap();
    let target = bootstrap_target(
        &backend,
        retained_directory_for(&backend, 3, identity(9), path(b"managed")),
        &reservation,
        0,
        AsciiComponent::parse(b"final").unwrap(),
    )
    .unwrap();
    let slots = ActionNamespace::from_admitted(
        recording_backend([9; 32], identity(7)),
        admitted(&reservation, 7),
    )
    .bootstrap_slots(0)
    .unwrap()
    .component(0, target)
    .unwrap();
    let installed = installed_component_evidence(
        &backend,
        &slots,
        marker.clone(),
        identity(40),
        PathComponentMode::Sensitive,
        path(b"installed"),
    )
    .unwrap();
    assert_eq!(installed.action_digest(), reservation.action_digest());
    assert_eq!(installed.reservation_digest(), reservation.record_digest());
    assert_ne!(
        installed.reservation_digest(),
        other_reservation.record_digest()
    );
    assert_eq!(installed.bootstrap_ordinal().index(), 0);
    assert_eq!(installed.component_ordinal().index(), 0);
    assert_eq!(installed.final_leaf().as_bytes(), b"final");
    assert_eq!(installed.marker().marker_id(), marker.marker_id());
    assert_eq!(installed.installed_identity(), &identity(40));
    assert_eq!(installed.installed_mode(), PathComponentMode::Sensitive);
    assert_eq!(installed.installed_path(), &path(b"installed"));

    let retired = retired_marker_evidence(
        &backend,
        &slots,
        marker.clone(),
        identity(41),
        PathComponentMode::AsciiCaseFold,
        path(b"retired"),
    )
    .unwrap();
    assert_eq!(retired.action_digest(), reservation.action_digest());
    assert_eq!(retired.reservation_digest(), reservation.record_digest());
    assert_eq!(retired.bootstrap_ordinal().index(), 0);
    assert_eq!(retired.component_ordinal().index(), 0);
    assert_eq!(retired.marker().marker_id(), marker.marker_id());
    assert!(
        retired
            .marker_retirement_leaf()
            .as_bytes()
            .ends_with(b"retired-bootstrap-marker-00-v1")
    );
    assert_eq!(retired.retired_marker_identity(), &identity(41));
    assert_eq!(
        retired.retired_parent_mode(),
        PathComponentMode::AsciiCaseFold
    );
    assert_eq!(retired.retired_parent_path(), &path(b"retired"));

    let other_backend = recording_backend([10; 32], identity(7));
    assert!(
        installed_component_evidence(
            &other_backend,
            &slots,
            marker.clone(),
            identity(40),
            PathComponentMode::Sensitive,
            path(b"installed"),
        )
        .is_err()
    );
    assert!(
        retired_marker_evidence(
            &other_backend,
            &slots,
            marker,
            identity(41),
            PathComponentMode::AsciiCaseFold,
            path(b"retired"),
        )
        .is_err()
    );
}

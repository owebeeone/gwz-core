use std::io::Cursor;

use super::super::capability::{
    AsciiComponent, CanonicalComponent, CanonicalPathIdentityV1, DurableObjectIdentityV1,
    PathComponentMode,
};
use super::super::namespace::test_support::{
    bootstrap_target, installed_component_evidence, recording_backend, retained_directory_for,
    retired_marker_evidence,
};
use super::super::namespace::{
    ActionNamespace, InstalledManagedComponentV1, RetiredManagedMarkerV1,
};
use super::super::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ActionDirectoryAdmissionV1,
    ActionDirectoryObservationV1, ActionScheduleV1, BootstrapGenerationV1, BootstrapOrdinalV1,
    CleanupAliasSetV1, ManagedBootstrapInputV1, ManagedBootstrapPhaseV1,
    ManagedParentBootstrapIntentV1, OwnershipMarkerV1, ProtocolRecordKindV1, RecordObservationV1,
    RequestOwnerBindingV1, admit_observed_action, read_and_bind_managed_bootstrap_intent_for_test,
    read_and_bind_ownership_marker, read_bounded_record,
};

fn identity(byte: u8) -> DurableObjectIdentityV1 {
    DurableObjectIdentityV1::linux_ext4([byte; 16], 1, vec![byte; 24]).unwrap()
}

fn path() -> CanonicalPathIdentityV1 {
    CanonicalPathIdentityV1::new(vec![
        CanonicalComponent::try_bound(
            AsciiComponent::parse(b"root").unwrap(),
            PathComponentMode::Sensitive,
            identity(8),
            vec![9; 16],
            vec![10; 16],
        )
        .unwrap(),
    ])
    .unwrap()
}

fn installed_path(
    parent: &CanonicalPathIdentityV1,
    name: &AsciiComponent,
    byte: u8,
) -> CanonicalPathIdentityV1 {
    let mut components = parent.components().to_vec();
    components.push(
        CanonicalComponent::try_bound(
            name.clone(),
            PathComponentMode::Sensitive,
            identity(byte),
            vec![byte; 16],
            vec![byte + 1; 16],
        )
        .unwrap(),
    );
    CanonicalPathIdentityV1::new(components).unwrap()
}

fn reservation(spec: u8, count: usize) -> ActionCapacityReservationV1 {
    reservation_rows(&[(spec, count)])
}

fn reservation_rows(rows: &[(u8, usize)]) -> ActionCapacityReservationV1 {
    ActionCapacityReservationV1::new(
        ActionDigestV1::new([1; 32]),
        RequestOwnerBindingV1::new([2; 32]),
        ActionScheduleV1::try_new(
            0,
            rows.iter()
                .map(|(spec, count)| ManagedBootstrapInputV1::new([*spec; 32], *count).unwrap())
                .collect(),
            CleanupAliasSetV1::all(),
        )
        .unwrap(),
    )
}

fn initial(
    reservation: &ActionCapacityReservationV1,
    spec: u8,
    components: &[&[u8]],
) -> ManagedParentBootstrapIntentV1 {
    initial_at(reservation, spec, 0, components)
}

fn initial_at(
    reservation: &ActionCapacityReservationV1,
    spec: u8,
    bootstrap: usize,
    components: &[&[u8]],
) -> ManagedParentBootstrapIntentV1 {
    ManagedParentBootstrapIntentV1::try_initial_for_test(
        reservation,
        [spec; 32],
        BootstrapOrdinalV1::new(bootstrap).unwrap(),
        identity(3),
        PathComponentMode::Sensitive,
        path(),
        components
            .iter()
            .map(|name| AsciiComponent::parse(name).unwrap())
            .collect(),
        [4; 32],
    )
    .unwrap()
}

fn admitted(reservation: &ActionCapacityReservationV1) -> super::super::protocol::AdmittedActionV1 {
    admit_observed_action(
        &ActionDirectoryAdmissionV1::idle(),
        reservation,
        &ActionDirectoryObservationV1::Missing,
        &ActionDirectoryObservationV1::exact(
            identity(31),
            RecordObservationV1::Exact(reservation.clone()),
        ),
    )
    .unwrap()
}

fn installed_evidence(
    reservation: &ActionCapacityReservationV1,
    intent: &ManagedParentBootstrapIntentV1,
    marker: OwnershipMarkerV1,
    bootstrap: usize,
    local: usize,
    byte: u8,
) -> InstalledManagedComponentV1 {
    let provider = [30; 32];
    let issuer = recording_backend(provider, identity(31));
    let component = &intent.components()[local];
    let global = component.global_component_ordinal().index();
    let target = bootstrap_target(
        &issuer,
        retained_directory_for(&issuer, 1, identity(32), path()),
        reservation,
        global,
        component.final_name().clone(),
    )
    .unwrap();
    let namespace = ActionNamespace::from_admitted(
        recording_backend(provider, identity(31)),
        admitted(reservation),
    );
    let slots = namespace
        .bootstrap_slots(bootstrap)
        .unwrap()
        .component(global, target)
        .unwrap();
    installed_component_evidence(
        &issuer,
        &slots,
        marker,
        identity(byte),
        PathComponentMode::Sensitive,
        installed_path(
            intent.retained_parent_path_for_test(),
            component.final_name(),
            byte,
        ),
    )
    .unwrap()
}

fn retirement_evidence(
    reservation: &ActionCapacityReservationV1,
    intent: &ManagedParentBootstrapIntentV1,
    marker: OwnershipMarkerV1,
    bootstrap: usize,
    local: usize,
    byte: u8,
) -> RetiredManagedMarkerV1 {
    let provider = [30; 32];
    let issuer = recording_backend(provider, identity(31));
    let component = &intent.components()[local];
    let global = component.global_component_ordinal().index();
    let target = bootstrap_target(
        &issuer,
        retained_directory_for(&issuer, 1, identity(32), path()),
        reservation,
        global,
        component.final_name().clone(),
    )
    .unwrap();
    let namespace = ActionNamespace::from_admitted(
        recording_backend(provider, identity(31)),
        admitted(reservation),
    );
    let slots = namespace
        .bootstrap_slots(bootstrap)
        .unwrap()
        .component(global, target)
        .unwrap();
    retired_marker_evidence(
        &issuer,
        &slots,
        marker,
        identity(byte),
        PathComponentMode::Sensitive,
        installed_path(&path(), component.final_name(), byte),
    )
    .unwrap()
}

#[test]
fn component_and_marker_successors_form_one_closed_chain() {
    let reservation = reservation(7, 2);
    let initial = initial(&reservation, 7, &[b"one", b"two"]);
    assert_eq!(initial.phase(), ManagedBootstrapPhaseV1::InstallComponents);
    assert_eq!(initial.cursor(), 0);
    assert_eq!(initial.generation_ordinal().index(), 0);

    let first_marker = OwnershipMarkerV1::for_current_component(&initial).unwrap();
    let first_install = installed_evidence(&reservation, &initial, first_marker.clone(), 0, 0, 5);
    let after_first = initial.successor_after_component(&first_install).unwrap();
    assert_eq!(after_first.cursor(), 1);
    assert_eq!(after_first.generation_ordinal().index(), 1);
    assert_eq!(
        after_first.predecessor_intent_id(),
        Some(initial.intent_id())
    );

    let second_marker = OwnershipMarkerV1::for_current_component(&after_first).unwrap();
    let second_install =
        installed_evidence(&reservation, &after_first, second_marker.clone(), 0, 1, 6);
    let retiring = after_first
        .successor_after_component(&second_install)
        .unwrap();
    assert_eq!(retiring.phase(), ManagedBootstrapPhaseV1::RetireMarkers);
    assert_eq!(retiring.cursor(), 0);
    assert_eq!(retiring.generation_ordinal().index(), 2);

    let first_retirement =
        retirement_evidence(&reservation, &retiring, first_marker.clone(), 0, 0, 7);
    let after_first_retirement = retiring
        .successor_after_marker_retirement(&first_retirement)
        .unwrap();
    assert_eq!(after_first_retirement.cursor(), 1);
    assert_eq!(after_first_retirement.generation_ordinal().index(), 3);
    let second_retirement = retirement_evidence(
        &reservation,
        &after_first_retirement,
        second_marker.clone(),
        0,
        1,
        8,
    );
    let complete = after_first_retirement
        .successor_after_marker_retirement(&second_retirement)
        .unwrap();
    assert!(complete.is_complete());
    assert_eq!(complete.generation_ordinal().index(), 4);

    assert_eq!(
        read_and_bind_managed_bootstrap_intent_for_test(
            Cursor::new(complete.encode_canonical()),
            &reservation,
            BootstrapGenerationV1::new(4).unwrap(),
            complete.predecessor_intent_id(),
        )
        .unwrap()
        .value(),
        &complete
    );
    assert_eq!(
        read_and_bind_ownership_marker(Cursor::new(first_marker.encode_canonical()), &complete, 0,)
            .unwrap()
            .value(),
        &first_marker
    );
}

#[test]
fn substitutions_cannot_advance_or_bind_a_managed_chain() {
    let first_reservation = reservation(7, 1);
    let first = initial(&first_reservation, 7, &[b"one"]);
    let marker = OwnershipMarkerV1::for_current_component(&first).unwrap();
    let other_reservation = reservation(8, 1);
    let other = initial(&other_reservation, 8, &[b"one"]);
    let other_marker = OwnershipMarkerV1::for_current_component(&other).unwrap();
    let other_evidence =
        installed_evidence(&other_reservation, &other, other_marker.clone(), 0, 0, 5);

    assert!(first.successor_after_component(&other_evidence).is_err());
    let premature_retirement =
        retirement_evidence(&first_reservation, &first, marker.clone(), 0, 0, 6);
    assert!(
        first
            .successor_after_marker_retirement(&premature_retirement)
            .is_err()
    );
    assert!(
        read_and_bind_managed_bootstrap_intent_for_test(
            Cursor::new(first.encode_canonical()),
            &other_reservation,
            BootstrapGenerationV1::new(0).unwrap(),
            None,
        )
        .is_err()
    );
    assert!(
        read_and_bind_managed_bootstrap_intent_for_test(
            Cursor::new(first.encode_canonical()),
            &first_reservation,
            BootstrapGenerationV1::new(1).unwrap(),
            None,
        )
        .is_err()
    );
}

#[test]
fn opaque_success_evidence_rejects_component_target_marker_and_bootstrap_substitution() {
    let reservation = reservation(7, 2);
    let first = initial(&reservation, 7, &[b"one", b"two"]);
    let marker = OwnershipMarkerV1::for_current_component(&first).unwrap();

    let alternate = initial(&reservation, 7, &[b"other", b"two"]);
    let alternate_marker = OwnershipMarkerV1::for_current_component(&alternate).unwrap();
    let wrong_marker = installed_evidence(&reservation, &first, alternate_marker.clone(), 0, 0, 10);
    assert!(first.successor_after_component(&wrong_marker).is_err());

    let wrong_target = installed_evidence(&reservation, &alternate, marker.clone(), 0, 0, 10);
    assert!(first.successor_after_component(&wrong_target).is_err());

    let wrong_component = installed_evidence(&reservation, &first, marker.clone(), 0, 1, 10);
    assert!(first.successor_after_component(&wrong_component).is_err());

    let two_bootstraps = reservation_rows(&[(7, 1), (8, 1)]);
    let first_bootstrap = initial_at(&two_bootstraps, 7, 0, &[b"one"]);
    let first_marker = OwnershipMarkerV1::for_current_component(&first_bootstrap).unwrap();
    let second_bootstrap = initial_at(&two_bootstraps, 8, 1, &[b"two"]);
    let wrong_bootstrap =
        installed_evidence(&two_bootstraps, &second_bootstrap, first_marker, 1, 0, 11);
    assert!(
        first_bootstrap
            .successor_after_component(&wrong_bootstrap)
            .is_err()
    );

    let install = installed_evidence(&reservation, &first, marker, 0, 0, 12);
    let after_first = first.successor_after_component(&install).unwrap();
    let second_marker = OwnershipMarkerV1::for_current_component(&after_first).unwrap();
    let second_install = installed_evidence(&reservation, &after_first, second_marker, 0, 1, 13);
    let retiring = after_first
        .successor_after_component(&second_install)
        .unwrap();
    let wrong_retired_marker =
        retirement_evidence(&reservation, &retiring, alternate_marker, 0, 0, 14);
    assert!(
        retiring
            .successor_after_marker_retirement(&wrong_retired_marker)
            .is_err()
    );
}

#[test]
fn maximum_component_chain_fits_bound_and_limit_plus_one_rejects() {
    let reservation = reservation(9, 8);
    let names = [
        b"a".as_slice(),
        b"b".as_slice(),
        b"c".as_slice(),
        b"d".as_slice(),
        b"e".as_slice(),
        b"f".as_slice(),
        b"g".as_slice(),
        b"h".as_slice(),
    ];
    let value = initial(&reservation, 9, &names);
    let bytes = value.encode_canonical();
    assert!(bytes.len() <= ProtocolRecordKindV1::BootstrapIntent.max_bytes());
    assert_eq!(
        read_bounded_record::<ManagedParentBootstrapIntentV1>(Cursor::new(bytes)).unwrap(),
        value
    );
    let limit = ProtocolRecordKindV1::BootstrapIntent.max_bytes();
    assert!(
        read_bounded_record::<ManagedParentBootstrapIntentV1>(Cursor::new(vec![0; limit + 1]))
            .is_err()
    );
    let marker_limit = ProtocolRecordKindV1::Marker.max_bytes();
    assert!(
        read_bounded_record::<OwnershipMarkerV1>(Cursor::new(vec![0; marker_limit + 1])).is_err()
    );
}

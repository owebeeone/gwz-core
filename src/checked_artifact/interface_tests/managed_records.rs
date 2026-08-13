use std::io::Cursor;

use super::super::capability::{
    AsciiComponent, CanonicalComponent, CanonicalPathIdentityV1, CheckedFsError,
    DurableObjectIdentityV1, PathComponentMode,
};
use super::super::namespace::test_support::{
    bootstrap_target, recording_backend, retained_directory_for,
    seed_installed_component_observation, seed_retired_marker_observation,
};
use super::super::namespace::{
    ActionNamespace, InstalledManagedComponentV1, RetiredManagedMarkerV1,
};
use super::super::protocol::generated;
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
    parent_identity: &DurableObjectIdentityV1,
    parent_mode: PathComponentMode,
    name: &AsciiComponent,
    byte: u8,
) -> CanonicalPathIdentityV1 {
    let mut components = parent.components().to_vec();
    components.push(
        CanonicalComponent::try_bound(
            name.clone(),
            parent_mode,
            parent_identity.clone(),
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
) -> Result<InstalledManagedComponentV1, CheckedFsError> {
    installed_evidence_exact(
        reservation,
        intent,
        marker,
        bootstrap,
        local,
        identity(byte + 40),
        identity(byte),
        PathComponentMode::Sensitive,
        installed_path(
            intent.retained_parent_path_for_test(),
            intent.retained_parent_identity_for_test(),
            intent.retained_parent_mode_for_test(),
            intent.components()[local].final_name(),
            byte,
        ),
    )
}

#[allow(clippy::too_many_arguments)]
fn installed_evidence_exact(
    reservation: &ActionCapacityReservationV1,
    intent: &ManagedParentBootstrapIntentV1,
    marker: OwnershipMarkerV1,
    bootstrap: usize,
    local: usize,
    marker_object_identity: DurableObjectIdentityV1,
    installed_identity: DurableObjectIdentityV1,
    installed_mode: PathComponentMode,
    installed_path: CanonicalPathIdentityV1,
) -> Result<InstalledManagedComponentV1, CheckedFsError> {
    let provider = [30; 32];
    let mut backend = recording_backend(provider, identity(31));
    seed_installed_component_observation(
        &mut backend,
        marker,
        marker_object_identity,
        installed_identity,
        installed_mode,
        installed_path,
    );
    let component = &intent.components()[local];
    let global = component.global_component_ordinal().index();
    let target = bootstrap_target(
        &backend,
        retained_directory_for(
            &backend,
            1,
            intent.retained_parent_identity_for_test().clone(),
            intent.retained_parent_path_for_test().clone(),
        ),
        reservation,
        global,
        component.final_name().clone(),
    )
    .unwrap();
    let mut namespace = ActionNamespace::from_admitted(backend, admitted(reservation));
    let slots = namespace
        .bootstrap_slots(bootstrap)
        .unwrap()
        .component(global, target)
        .unwrap();
    namespace.recover_installed_bootstrap_component(intent, &slots)
}

fn retirement_evidence(
    reservation: &ActionCapacityReservationV1,
    intent: &ManagedParentBootstrapIntentV1,
    marker: OwnershipMarkerV1,
    bootstrap: usize,
    local: usize,
) -> Result<RetiredManagedMarkerV1, CheckedFsError> {
    let component = &intent.components()[local];
    retirement_evidence_exact(
        reservation,
        intent,
        marker,
        bootstrap,
        local,
        component
            .ownership_marker_object_identity()
            .unwrap()
            .clone(),
        component.installed_identity().unwrap().clone(),
        component.installed_mode().unwrap(),
        component.installed_path().unwrap().clone(),
    )
}

#[allow(clippy::too_many_arguments)]
fn retirement_evidence_exact(
    reservation: &ActionCapacityReservationV1,
    intent: &ManagedParentBootstrapIntentV1,
    marker: OwnershipMarkerV1,
    bootstrap: usize,
    local: usize,
    retired_marker_identity: DurableObjectIdentityV1,
    installed_parent_identity: DurableObjectIdentityV1,
    installed_parent_mode: PathComponentMode,
    installed_parent_path: CanonicalPathIdentityV1,
) -> Result<RetiredManagedMarkerV1, CheckedFsError> {
    let provider = [30; 32];
    let mut backend = recording_backend(provider, identity(31));
    seed_retired_marker_observation(
        &mut backend,
        marker,
        retired_marker_identity,
        installed_parent_identity,
        installed_parent_mode,
        installed_parent_path,
    );
    let component = &intent.components()[local];
    let global = component.global_component_ordinal().index();
    let (component_parent_identity, component_parent_path) =
        if let Some(installed) = component.installed_path() {
            let installed_component = installed.components().last().unwrap();
            (
                installed_component.parent_durable_identity().clone(),
                CanonicalPathIdentityV1::new(
                    installed.components()[..installed.components().len() - 1].to_vec(),
                )
                .unwrap(),
            )
        } else {
            (
                intent.retained_parent_identity_for_test().clone(),
                intent.retained_parent_path_for_test().clone(),
            )
        };
    let target = bootstrap_target(
        &backend,
        retained_directory_for(
            &backend,
            1,
            component_parent_identity,
            component_parent_path,
        ),
        reservation,
        global,
        component.final_name().clone(),
    )
    .unwrap();
    let mut namespace = ActionNamespace::from_admitted(backend, admitted(reservation));
    let slots = namespace
        .bootstrap_slots(bootstrap)
        .unwrap()
        .component(global, target)
        .unwrap();
    namespace.recover_retired_bootstrap_marker(intent, &slots)
}

#[test]
fn component_and_marker_successors_form_one_closed_chain() {
    let reservation = reservation(7, 2);
    let initial = initial(&reservation, 7, &[b"one", b"two"]);
    assert_eq!(initial.phase(), ManagedBootstrapPhaseV1::InstallComponents);
    assert_eq!(initial.cursor(), 0);
    assert_eq!(initial.generation_ordinal().index(), 0);

    let first_marker = OwnershipMarkerV1::for_current_component(&initial).unwrap();
    let first_install =
        installed_evidence(&reservation, &initial, first_marker.clone(), 0, 0, 5).unwrap();
    let after_first = initial.successor_after_component(&first_install).unwrap();
    assert_eq!(after_first.cursor(), 1);
    assert_eq!(after_first.generation_ordinal().index(), 1);
    assert_eq!(
        after_first.predecessor_intent_id(),
        Some(initial.intent_id())
    );

    let second_marker = OwnershipMarkerV1::for_current_component(&after_first).unwrap();
    let second_install =
        installed_evidence(&reservation, &after_first, second_marker.clone(), 0, 1, 6).unwrap();
    let retiring = after_first
        .successor_after_component(&second_install)
        .unwrap();
    assert_eq!(retiring.phase(), ManagedBootstrapPhaseV1::RetireMarkers);
    assert_eq!(retiring.cursor(), 0);
    assert_eq!(retiring.generation_ordinal().index(), 2);

    let first_retirement =
        retirement_evidence(&reservation, &retiring, first_marker.clone(), 0, 0).unwrap();
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
    )
    .unwrap();
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
        installed_evidence(&other_reservation, &other, other_marker.clone(), 0, 0, 5).unwrap();

    assert!(first.successor_after_component(&other_evidence).is_err());
    let premature_retirement = retirement_evidence_exact(
        &first_reservation,
        &first,
        marker.clone(),
        0,
        0,
        identity(66),
        identity(67),
        PathComponentMode::Sensitive,
        installed_path(
            first.retained_parent_path_for_test(),
            first.retained_parent_identity_for_test(),
            first.retained_parent_mode_for_test(),
            first.components()[0].final_name(),
            68,
        ),
    );
    assert!(premature_retirement.is_err());
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
    assert!(wrong_marker.is_err());

    let wrong_target = installed_evidence(&reservation, &alternate, marker.clone(), 0, 0, 10);
    assert!(wrong_target.is_err());

    let wrong_component = installed_evidence(&reservation, &first, marker.clone(), 0, 1, 10);
    assert!(wrong_component.is_err());

    let two_bootstraps = reservation_rows(&[(7, 1), (8, 1)]);
    let first_bootstrap = initial_at(&two_bootstraps, 7, 0, &[b"one"]);
    let first_marker = OwnershipMarkerV1::for_current_component(&first_bootstrap).unwrap();
    let second_bootstrap = initial_at(&two_bootstraps, 8, 1, &[b"two"]);
    let wrong_bootstrap =
        installed_evidence(&two_bootstraps, &second_bootstrap, first_marker, 1, 0, 11);
    assert!(wrong_bootstrap.is_err());

    let install = installed_evidence(&reservation, &first, marker, 0, 0, 12).unwrap();
    let after_first = first.successor_after_component(&install).unwrap();
    let second_marker = OwnershipMarkerV1::for_current_component(&after_first).unwrap();
    let second_install =
        installed_evidence(&reservation, &after_first, second_marker, 0, 1, 13).unwrap();
    let retiring = after_first
        .successor_after_component(&second_install)
        .unwrap();
    let wrong_retired_marker = retirement_evidence(&reservation, &retiring, alternate_marker, 0, 0);
    assert!(wrong_retired_marker.is_err());
}

#[test]
fn exact_installed_facts_are_durable_and_every_retirement_substitution_rejects() {
    let reservation = reservation(7, 1);
    let initial = initial(&reservation, 7, &[b"one"]);
    let marker = OwnershipMarkerV1::for_current_component(&initial).unwrap();
    let marker_identity = identity(50);
    let installed_identity = identity(51);
    let installed_mode = PathComponentMode::AsciiCaseFold;
    let exact_path = installed_path(
        initial.retained_parent_path_for_test(),
        initial.retained_parent_identity_for_test(),
        initial.retained_parent_mode_for_test(),
        initial.components()[0].final_name(),
        52,
    );
    let install = installed_evidence_exact(
        &reservation,
        &initial,
        marker.clone(),
        0,
        0,
        marker_identity.clone(),
        installed_identity.clone(),
        installed_mode,
        exact_path.clone(),
    )
    .unwrap();
    let retiring = initial.successor_after_component(&install).unwrap();
    let component = &retiring.components()[0];
    assert_eq!(component.installed_identity(), Some(&installed_identity));
    assert_eq!(component.installed_mode(), Some(installed_mode));
    assert_eq!(component.installed_path(), Some(&exact_path));
    assert_eq!(
        component.ownership_marker_object_identity(),
        Some(&marker_identity)
    );
    let recovered = read_bounded_record::<ManagedParentBootstrapIntentV1>(Cursor::new(
        retiring.encode_canonical(),
    ))
    .unwrap();
    assert_eq!(recovered, retiring);

    let exact = retirement_evidence_exact(
        &reservation,
        &retiring,
        marker.clone(),
        0,
        0,
        marker_identity.clone(),
        installed_identity.clone(),
        installed_mode,
        exact_path.clone(),
    )
    .unwrap();
    assert!(retiring.successor_after_marker_retirement(&exact).is_ok());

    for substituted in [
        retirement_evidence_exact(
            &reservation,
            &retiring,
            marker.clone(),
            0,
            0,
            identity(53),
            installed_identity.clone(),
            installed_mode,
            exact_path.clone(),
        ),
        retirement_evidence_exact(
            &reservation,
            &retiring,
            marker.clone(),
            0,
            0,
            marker_identity.clone(),
            identity(54),
            installed_mode,
            exact_path.clone(),
        ),
        retirement_evidence_exact(
            &reservation,
            &retiring,
            marker.clone(),
            0,
            0,
            marker_identity.clone(),
            installed_identity.clone(),
            PathComponentMode::Sensitive,
            exact_path.clone(),
        ),
        retirement_evidence_exact(
            &reservation,
            &retiring,
            marker,
            0,
            0,
            marker_identity,
            installed_identity,
            installed_mode,
            installed_path(
                retiring.retained_parent_path_for_test(),
                retiring.retained_parent_identity_for_test(),
                retiring.retained_parent_mode_for_test(),
                retiring.components()[0].final_name(),
                55,
            ),
        ),
    ] {
        assert!(substituted.is_err());
    }
}

#[test]
fn canonical_record_rejects_mutated_installed_facts_and_marker_object_identity() {
    let reservation = reservation(7, 1);
    let initial = initial(&reservation, 7, &[b"one"]);
    let marker = OwnershipMarkerV1::for_current_component(&initial).unwrap();
    let install = installed_evidence(&reservation, &initial, marker, 0, 0, 70).unwrap();
    let retiring = initial.successor_after_component(&install).unwrap();
    let bytes = retiring.encode_canonical();
    let cbor = crate::cbor::try_decode(&bytes).unwrap();
    let wire = generated::CheckedManagedParentBootstrapIntentV1::from_cbor(&cbor).unwrap();

    let mut mutations = Vec::new();
    let mut changed_identity = wire.clone();
    changed_identity.components[0].installed_identity = Some(identity(71).to_generated());
    mutations.push(changed_identity);
    let mut changed_mode = wire.clone();
    changed_mode.components[0].installed_mode =
        Some(generated::CheckedPathComponentMode::AsciiCaseFold);
    mutations.push(changed_mode);
    let mut changed_path = wire.clone();
    changed_path.components[0].installed_path = Some(
        installed_path(
            retiring.retained_parent_path_for_test(),
            retiring.retained_parent_identity_for_test(),
            retiring.retained_parent_mode_for_test(),
            retiring.components()[0].final_name(),
            72,
        )
        .to_generated(),
    );
    mutations.push(changed_path);
    let mut changed_marker_identity = wire;
    changed_marker_identity.components[0].ownership_marker_object_identity =
        Some(identity(73).to_generated());
    mutations.push(changed_marker_identity);

    for mutation in mutations {
        assert!(
            read_bounded_record::<ManagedParentBootstrapIntentV1>(Cursor::new(
                crate::cbor::encode(&mutation.to_cbor()),
            ))
            .is_err()
        );
    }
}

#[test]
fn installation_rejects_changed_prefix_parent_identity_and_parent_mode() {
    let reservation = reservation(7, 1);
    let initial = initial(&reservation, 7, &[b"one"]);
    let marker = OwnershipMarkerV1::for_current_component(&initial).unwrap();
    let final_name = initial.components()[0].final_name();
    let make = |parent_path: &CanonicalPathIdentityV1,
                parent_identity: DurableObjectIdentityV1,
                parent_mode: PathComponentMode,
                byte: u8| {
        installed_evidence_exact(
            &reservation,
            &initial,
            marker.clone(),
            0,
            0,
            identity(60),
            identity(61),
            PathComponentMode::Sensitive,
            installed_path(parent_path, &parent_identity, parent_mode, final_name, byte),
        )
    };

    let changed_prefix = CanonicalPathIdentityV1::new(vec![
        CanonicalComponent::try_bound(
            AsciiComponent::parse(b"other-root").unwrap(),
            PathComponentMode::Sensitive,
            identity(8),
            vec![9; 16],
            vec![10; 16],
        )
        .unwrap(),
    ])
    .unwrap();
    for substituted in [
        make(
            initial.retained_parent_path_for_test(),
            identity(62),
            initial.retained_parent_mode_for_test(),
            63,
        ),
        make(
            initial.retained_parent_path_for_test(),
            initial.retained_parent_identity_for_test().clone(),
            PathComponentMode::AsciiCaseFold,
            64,
        ),
        make(
            &changed_prefix,
            initial.retained_parent_identity_for_test().clone(),
            initial.retained_parent_mode_for_test(),
            65,
        ),
    ] {
        assert!(substituted.is_err());
    }
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
    let initial_value = initial(&reservation, 9, &names);
    let mut value = initial_value;
    let mut markers = Vec::new();
    for local in 0..names.len() {
        let marker = OwnershipMarkerV1::for_current_component(&value).unwrap();
        let evidence = installed_evidence(
            &reservation,
            &value,
            marker.clone(),
            0,
            local,
            80 + local as u8,
        )
        .unwrap();
        value = value.successor_after_component(&evidence).unwrap();
        markers.push(marker);
    }
    for (local, marker) in markers.into_iter().enumerate() {
        let evidence = retirement_evidence(&reservation, &value, marker, 0, local).unwrap();
        value = value.successor_after_marker_retirement(&evidence).unwrap();
    }
    assert!(value.is_complete());
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

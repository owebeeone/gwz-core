use std::io::Cursor;

use super::super::capability::{
    AsciiComponent, CanonicalComponent, CanonicalPathIdentityV1, DurableCatalogTargetDigestV1,
    DurableObjectIdentityV1, DurablePathV1, HistoricalCollisionDigestV1, PathComponentMode,
    PreCatalogRootKindV1, SupportedFilesystemProfile,
};
use super::super::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ActionScheduleV1,
    CatalogBootstrapOwnershipTokenV1, CatalogBootstrapRecordV1, CheckedAuthorityRecordV1,
    CleanupAliasSetV1, DurableLeafFingerprintV1, ManagedBootstrapInputV1, ProtocolRecordKindV1,
    RequestOwnerBindingV1, read_and_bind_authority_record, read_and_match_catalog_bootstrap_record,
    read_and_match_infrastructure_record, read_bounded_record, synthetic_authority_observation,
    synthetic_authority_observation_owner, synthetic_infrastructure_from_catalog_bootstrap,
};

fn identity(byte: u8) -> DurableObjectIdentityV1 {
    DurableObjectIdentityV1::linux_ext4([byte; 16], 1, vec![byte; 24]).unwrap()
}

fn mac_identity(byte: u8) -> DurableObjectIdentityV1 {
    DurableObjectIdentityV1::mac([byte; 16], [byte; 8]).unwrap()
}

fn path(byte: u8) -> CanonicalPathIdentityV1 {
    CanonicalPathIdentityV1::new(vec![
        CanonicalComponent::try_bound(
            AsciiComponent::parse(&[byte]).unwrap(),
            PathComponentMode::Sensitive,
            identity(byte),
            vec![byte; 16],
            vec![byte; 16],
        )
        .unwrap(),
    ])
    .unwrap()
}

fn reservation() -> ActionCapacityReservationV1 {
    ActionCapacityReservationV1::new(
        ActionDigestV1::new([1; 32]),
        RequestOwnerBindingV1::new([2; 32]),
        ActionScheduleV1::try_new(
            1,
            vec![ManagedBootstrapInputV1::new([3; 32], 1).unwrap()],
            CleanupAliasSetV1::all(),
        )
        .unwrap(),
    )
}

fn authority_observation(
    reservation: &ActionCapacityReservationV1,
    root: CanonicalPathIdentityV1,
    parent: DurableObjectIdentityV1,
    source: DurableLeafFingerprintV1,
    expected: [u8; 32],
    goal: [u8; 32],
) -> super::super::protocol::CheckedAuthorityObservationV1 {
    synthetic_authority_observation(reservation, root, parent, source, expected, goal).unwrap()
}

fn catalog_with_token(
    root_kind: PreCatalogRootKindV1,
    byte: u8,
    token: [u8; 32],
) -> CatalogBootstrapRecordV1 {
    let live_path = path(byte);
    CatalogBootstrapRecordV1::synthetic_for_test(
        root_kind,
        SupportedFilesystemProfile::LinuxExt4FsIocGetFsUuidV1,
        DurableCatalogTargetDigestV1::owner_issue([byte.wrapping_add(1); 32]),
        HistoricalCollisionDigestV1::owner_issue([byte.wrapping_add(2); 32]),
        identity(byte),
        DurablePathV1::from_live(&live_path).unwrap(),
        CatalogBootstrapOwnershipTokenV1::try_from_random_bytes(token).unwrap(),
    )
}

fn catalog(root_kind: PreCatalogRootKindV1, byte: u8) -> CatalogBootstrapRecordV1 {
    catalog_with_token(root_kind, byte, [byte.wrapping_add(64); 32])
}

#[test]
fn every_nontransition_record_has_a_bounded_canonical_adapter() {
    let reservation = reservation();
    let observation = authority_observation(
        &reservation,
        path(b'a'),
        identity(4),
        DurableLeafFingerprintV1::new(identity(5), 9, [6; 32]),
        [7; 32],
        [8; 32],
    );
    let authority = CheckedAuthorityRecordV1::issue(&observation).unwrap();
    let catalog = catalog(PreCatalogRootKindV1::Workspace, 12);
    let infrastructure = synthetic_infrastructure_from_catalog_bootstrap(
        &catalog,
        identity(14),
        identity(15),
        identity(16),
        identity(17),
    )
    .unwrap();

    let authority_bytes = authority.encode_canonical();
    assert_eq!(
        read_and_bind_authority_record(Cursor::new(&authority_bytes), &reservation, &observation)
            .unwrap()
            .value(),
        &authority
    );
    let catalog_bytes = catalog.encode_canonical();
    assert_eq!(
        read_and_match_catalog_bootstrap_record(Cursor::new(&catalog_bytes), &catalog)
            .unwrap()
            .value(),
        &catalog
    );
    let infrastructure_bytes = infrastructure.encode_canonical();
    assert_eq!(
        read_and_match_infrastructure_record(Cursor::new(&infrastructure_bytes), &infrastructure,)
            .unwrap()
            .value(),
        &infrastructure
    );
    assert!(authority_bytes.len() <= ProtocolRecordKindV1::Authority.max_bytes());
    assert!(catalog_bytes.len() <= ProtocolRecordKindV1::CatalogBootstrap.max_bytes());
    assert!(infrastructure_bytes.len() <= ProtocolRecordKindV1::Infrastructure.max_bytes());
}

#[test]
fn authority_id_binds_reservation_path_identity_payload_and_goal() {
    let first_reservation = reservation();
    let other_reservation = ActionCapacityReservationV1::new(
        ActionDigestV1::new([21; 32]),
        RequestOwnerBindingV1::new([2; 32]),
        first_reservation.schedule().clone(),
    );
    let make = |reservation: &ActionCapacityReservationV1, root, parent, source, expected, goal| {
        let observation = authority_observation(reservation, root, parent, source, expected, goal);
        CheckedAuthorityRecordV1::issue(&observation).unwrap()
    };
    let base = make(
        &first_reservation,
        path(b'a'),
        identity(1),
        DurableLeafFingerprintV1::new(identity(2), 3, [4; 32]),
        [5; 32],
        [6; 32],
    );
    for changed in [
        make(
            &other_reservation,
            path(b'a'),
            identity(1),
            DurableLeafFingerprintV1::new(identity(2), 3, [4; 32]),
            [5; 32],
            [6; 32],
        ),
        make(
            &first_reservation,
            path(b'b'),
            identity(1),
            DurableLeafFingerprintV1::new(identity(2), 3, [4; 32]),
            [5; 32],
            [6; 32],
        ),
        make(
            &first_reservation,
            path(b'a'),
            identity(9),
            DurableLeafFingerprintV1::new(identity(2), 3, [4; 32]),
            [5; 32],
            [6; 32],
        ),
        make(
            &first_reservation,
            path(b'a'),
            identity(1),
            DurableLeafFingerprintV1::new(identity(2), 4, [4; 32]),
            [5; 32],
            [6; 32],
        ),
        make(
            &first_reservation,
            path(b'a'),
            identity(1),
            DurableLeafFingerprintV1::new(identity(2), 3, [4; 32]),
            [9; 32],
            [6; 32],
        ),
        make(
            &first_reservation,
            path(b'a'),
            identity(1),
            DurableLeafFingerprintV1::new(identity(2), 3, [4; 32]),
            [5; 32],
            [9; 32],
        ),
    ] {
        assert_ne!(base.record_id(), changed.record_id());
    }
}

#[test]
fn authority_owner_rejects_two_request_substitution_in_both_directions() {
    let first = reservation();
    let second = ActionCapacityReservationV1::new(
        first.action_digest(),
        RequestOwnerBindingV1::new([91; 32]),
        first.schedule().clone(),
    );
    assert_eq!(first.action_digest(), second.action_digest());
    assert_eq!(first.schedule(), second.schedule());
    assert_ne!(
        first.request_owner_binding(),
        second.request_owner_binding()
    );
    let retained_root = path(b'a');
    let retained_parent = identity(1);
    let source = DurableLeafFingerprintV1::new(identity(2), 3, [4; 32]);

    let first_owner = synthetic_authority_observation_owner(
        first.request_owner_binding(),
        retained_root.clone(),
        retained_parent.clone(),
        source.clone(),
        [5; 32],
        [6; 32],
    );
    let first_observation = first_owner.observe(&first).unwrap();
    assert!(first_owner.observe(&second).is_err());

    let second_owner = synthetic_authority_observation_owner(
        second.request_owner_binding(),
        retained_root,
        retained_parent,
        source,
        [7; 32],
        [8; 32],
    );
    let second_observation = second_owner.observe(&second).unwrap();
    assert!(second_owner.observe(&first).is_err());
    assert_ne!(
        CheckedAuthorityRecordV1::issue(&first_observation)
            .unwrap()
            .record_id(),
        CheckedAuthorityRecordV1::issue(&second_observation)
            .unwrap()
            .record_id()
    );
}

#[test]
fn catalog_id_binds_every_pre_catalog_domain() {
    let base = catalog(PreCatalogRootKindV1::Workspace, 1);
    let changed = [
        catalog(PreCatalogRootKindV1::GitDirectory, 1),
        catalog(PreCatalogRootKindV1::Workspace, 2),
    ];
    for value in changed {
        assert_ne!(base.record_id(), value.record_id());
    }
}

#[test]
fn first_catalog_ownership_token_is_nonzero_and_binds_staging_infrastructure() {
    assert!(CatalogBootstrapOwnershipTokenV1::try_from_random_bytes([0; 32]).is_err());

    let token = [91; 32];
    let bootstrap = catalog_with_token(PreCatalogRootKindV1::Workspace, 1, token);
    assert_eq!(bootstrap.bootstrap_ownership_token().as_bytes(), &token);

    let infrastructure = synthetic_infrastructure_from_catalog_bootstrap(
        &bootstrap,
        identity(14),
        identity(15),
        identity(16),
        identity(17),
    )
    .unwrap();
    assert_eq!(
        infrastructure.catalog_bootstrap_record_id(),
        bootstrap.record_id()
    );
    assert_eq!(
        infrastructure.bootstrap_ownership_token(),
        bootstrap.bootstrap_ownership_token()
    );

    let foreign = catalog_with_token(PreCatalogRootKindV1::Workspace, 1, [92; 32]);
    assert_ne!(foreign.record_id(), bootstrap.record_id());
}

#[test]
fn bounded_read_precedes_decode_and_noncanonical_bytes_reject() {
    let limit = ProtocolRecordKindV1::CatalogBootstrap.max_bytes();
    assert!(
        read_bounded_record::<CatalogBootstrapRecordV1>(Cursor::new(vec![0; limit + 1])).is_err()
    );
    let record = catalog(PreCatalogRootKindV1::Workspace, 1);
    let mut trailing = record.encode_canonical();
    trailing.push(0);
    assert!(read_bounded_record::<CatalogBootstrapRecordV1>(Cursor::new(trailing)).is_err());
    let other = catalog(PreCatalogRootKindV1::GitDirectory, 1);
    assert!(
        read_and_match_catalog_bootstrap_record(Cursor::new(record.encode_canonical()), &other,)
            .is_err()
    );
}

#[test]
fn durable_record_roles_cannot_mix_filesystem_profiles() {
    let reservation = reservation();
    assert!(
        synthetic_authority_observation(
            &reservation,
            path(b'a'),
            identity(1),
            DurableLeafFingerprintV1::new(mac_identity(2), 3, [4; 32]),
            [5; 32],
            [6; 32],
        )
        .is_err()
    );
    assert!(
        synthetic_infrastructure_from_catalog_bootstrap(
            &catalog(PreCatalogRootKindV1::Workspace, 5),
            identity(1),
            identity(2),
            mac_identity(3),
            identity(4),
        )
        .is_err()
    );
}

#[test]
fn authority_recovery_binds_the_exact_coherent_observation() {
    let reservation = reservation();
    let observed = authority_observation(
        &reservation,
        path(b'a'),
        identity(1),
        DurableLeafFingerprintV1::new(identity(2), 3, [4; 32]),
        [5; 32],
        [6; 32],
    );
    let authority = CheckedAuthorityRecordV1::issue(&observed).unwrap();
    let bytes = authority.encode_canonical();
    let other_reservation = ActionCapacityReservationV1::new(
        ActionDigestV1::new([19; 32]),
        RequestOwnerBindingV1::new([2; 32]),
        reservation.schedule().clone(),
    );
    assert!(
        read_and_bind_authority_record(Cursor::new(&bytes), &other_reservation, &observed).is_err()
    );

    for substituted in [
        authority_observation(
            &reservation,
            path(b'b'),
            identity(1),
            DurableLeafFingerprintV1::new(identity(2), 3, [4; 32]),
            [5; 32],
            [6; 32],
        ),
        authority_observation(
            &reservation,
            path(b'a'),
            identity(9),
            DurableLeafFingerprintV1::new(identity(2), 3, [4; 32]),
            [5; 32],
            [6; 32],
        ),
        authority_observation(
            &reservation,
            path(b'a'),
            identity(1),
            DurableLeafFingerprintV1::new(identity(2), 4, [4; 32]),
            [5; 32],
            [6; 32],
        ),
        authority_observation(
            &reservation,
            path(b'a'),
            identity(1),
            DurableLeafFingerprintV1::new(identity(2), 3, [4; 32]),
            [9; 32],
            [6; 32],
        ),
        authority_observation(
            &reservation,
            path(b'a'),
            identity(1),
            DurableLeafFingerprintV1::new(identity(2), 3, [4; 32]),
            [5; 32],
            [9; 32],
        ),
    ] {
        assert!(
            read_and_bind_authority_record(Cursor::new(&bytes), &reservation, &substituted)
                .is_err()
        );
    }
    assert!(read_and_bind_authority_record(Cursor::new(bytes), &reservation, &observed).is_ok());
}

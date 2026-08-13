use std::io::Cursor;

use super::super::capability::{
    AsciiComponent, CanonicalComponent, CanonicalPathIdentityV1, DurableObjectIdentityV1,
    PathComponentMode, PreCatalogRootKindV1, SupportedFilesystemProfile,
    synthetic_pre_catalog_permit,
};
use super::super::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ActionScheduleV1, CatalogBootstrapRecordV1,
    CatalogBootstrapRecoveryDecisionV1, CatalogDirectoryObservationV1, CatalogRecordObservationV1,
    CheckedAuthorityRecordV1, CleanupAliasSetV1, DurableLeafFingerprintV1, InfrastructureRecordV1,
    ManagedBootstrapInputV1, ProtocolRecordKindV1, RequestOwnerBindingV1,
    classify_catalog_bootstrap_recovery, read_and_bind_authority_record,
    read_and_match_catalog_bootstrap_record, read_and_match_infrastructure_record,
    read_bounded_record,
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

fn catalog(root_kind: PreCatalogRootKindV1, byte: u8) -> CatalogBootstrapRecordV1 {
    let permit = synthetic_pre_catalog_permit(
        (),
        SupportedFilesystemProfile::LinuxExt4FsIocGetFsUuidV1,
        identity(byte),
        vec![byte; 16],
        vec![byte; 16],
        path(byte),
        [byte; 32],
        [byte.wrapping_add(1); 32],
        root_kind,
    )
    .unwrap();
    CatalogBootstrapRecordV1::from_permit(&permit)
}

#[test]
fn every_nontransition_record_has_a_bounded_canonical_adapter() {
    let reservation = reservation();
    let authority = CheckedAuthorityRecordV1::new(
        &reservation,
        path(b'a'),
        identity(4),
        DurableLeafFingerprintV1::new(identity(5), 9, [6; 32]),
        [7; 32],
        [8; 32],
    )
    .unwrap();
    let catalog = catalog(PreCatalogRootKindV1::Workspace, 12);
    let infrastructure = InfrastructureRecordV1::from_catalog_bootstrap(
        &catalog,
        identity(14),
        identity(15),
        identity(16),
        identity(17),
    )
    .unwrap();

    let authority_bytes = authority.encode_canonical();
    assert_eq!(
        read_and_bind_authority_record(Cursor::new(&authority_bytes), &reservation)
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
        CheckedAuthorityRecordV1::new(reservation, root, parent, source, expected, goal).unwrap()
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
        CheckedAuthorityRecordV1::new(
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
        InfrastructureRecordV1::from_catalog_bootstrap(
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
fn catalog_bootstrap_recovery_table_is_closed() {
    use CatalogBootstrapRecoveryDecisionV1::*;
    use CatalogDirectoryObservationV1 as Directory;
    use CatalogRecordObservationV1 as Record;

    let bootstrap = catalog(PreCatalogRootKindV1::Workspace, 31);
    let infrastructure = InfrastructureRecordV1::from_catalog_bootstrap(
        &bootstrap,
        identity(32),
        identity(33),
        identity(34),
        identity(35),
    )
    .unwrap();

    let records = [
        Record::Missing,
        Record::PartialExpectedPrefix,
        Record::Exact(Box::new(bootstrap.clone())),
        Record::Other,
    ];
    let directories = [
        Directory::Missing,
        Directory::PartialExpectedContents,
        Directory::Exact(Box::new(infrastructure.clone())),
        Directory::Other,
    ];

    let mut accepted = 0;
    for scratch in &records {
        for active in &records {
            for staging in &directories {
                for final_directory in &directories {
                    for retired in &records {
                        let decision = classify_catalog_bootstrap_recovery(
                            &bootstrap,
                            &infrastructure,
                            scratch,
                            active,
                            staging,
                            final_directory,
                            retired,
                        );
                        let expected = match (scratch, active, staging, final_directory, retired) {
                            (
                                Record::Missing | Record::PartialExpectedPrefix,
                                Record::Missing,
                                Directory::Missing,
                                Directory::Missing,
                                Record::Missing,
                            ) => WriteOrRewriteScratch,
                            (
                                Record::Exact(value),
                                Record::Missing,
                                Directory::Missing,
                                Directory::Missing,
                                Record::Missing,
                            ) if value.as_ref() == &bootstrap => PublishActive,
                            (
                                Record::Missing,
                                Record::Exact(value),
                                Directory::Missing | Directory::PartialExpectedContents,
                                Directory::Missing,
                                Record::Missing,
                            ) if value.as_ref() == &bootstrap => PrepareOrRewriteStaging,
                            (
                                Record::Missing,
                                Record::Exact(active_value),
                                Directory::Exact(staging_value),
                                Directory::Missing,
                                Record::Missing,
                            ) if active_value.as_ref() == &bootstrap
                                && staging_value.as_ref() == &infrastructure =>
                            {
                                PublishFinal
                            }
                            (
                                Record::Missing,
                                Record::Exact(active_value),
                                Directory::Missing,
                                Directory::Exact(final_value),
                                Record::Missing,
                            ) if active_value.as_ref() == &bootstrap
                                && final_value.as_ref() == &infrastructure =>
                            {
                                RetireActive
                            }
                            (
                                Record::Missing,
                                Record::Missing,
                                Directory::Missing,
                                Directory::Exact(final_value),
                                Record::Exact(retired_value),
                            ) if final_value.as_ref() == &infrastructure
                                && retired_value.as_ref() == &bootstrap =>
                            {
                                Complete
                            }
                            _ => Ambiguous,
                        };
                        assert_eq!(decision, expected);
                        accepted += usize::from(decision != Ambiguous);
                    }
                }
            }
        }
    }
    assert_eq!(accepted, 8);
}

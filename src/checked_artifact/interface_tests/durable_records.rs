use std::io::Cursor;
use std::path::Path;

use super::super::bootstrap::CatalogBootstrapV1;
use super::super::capability::{
    AsciiComponent, CanonicalComponent, CanonicalPathIdentityV1, CheckedFsError,
    DurableObjectIdentityV1, PathComponentMode, PreCatalogRootKindV1, PrivateControlDomain,
    RevalidatedPreCatalogPermitV1, SupportedFilesystemProfile, synthetic_pre_catalog_owner,
};
use super::super::protocol::{
    ActionCapacityReservationV1, ActionDigestV1, ActionScheduleV1,
    CatalogBootstrapOwnershipTokenV1, CatalogBootstrapRecordV1, CatalogBootstrapRecoveryDecisionV1,
    CatalogDirectoryObservationV1, CatalogRecordObservationV1, CheckedAuthorityRecordV1,
    CleanupAliasSetV1, DurableLeafFingerprintV1, ManagedBootstrapInputV1, ProtocolRecordKindV1,
    RequestOwnerBindingV1, classify_catalog_bootstrap_recovery, read_and_bind_authority_record,
    read_and_match_catalog_bootstrap_record, read_and_match_infrastructure_record,
    read_bounded_record, synthetic_authority_observation,
    synthetic_infrastructure_from_catalog_bootstrap,
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

struct CatalogRecordBuilder {
    token: CatalogBootstrapOwnershipTokenV1,
}

impl CatalogBootstrapV1<()> for CatalogRecordBuilder {
    type Catalog = CatalogBootstrapRecordV1;

    fn recover_or_create(
        &self,
        permit: RevalidatedPreCatalogPermitV1<'_, ()>,
    ) -> Result<Self::Catalog, CheckedFsError> {
        Ok(CatalogBootstrapRecordV1::from_revalidated_permit(
            permit, self.token,
        ))
    }
}

fn catalog_with_token(
    root_kind: PreCatalogRootKindV1,
    byte: u8,
    token: [u8; 32],
) -> CatalogBootstrapRecordV1 {
    let (owner, _) = synthetic_pre_catalog_owner(
        (),
        SupportedFilesystemProfile::LinuxExt4FsIocGetFsUuidV1,
        identity(byte),
        vec![byte; 16],
        vec![byte; 16],
        path(byte),
    );
    owner
        .recover_or_create(
            Path::new("."),
            root_kind,
            [byte.wrapping_add(1); 32],
            &PrivateControlDomain::checked_v1(),
            &[],
            &[],
            &CatalogRecordBuilder {
                token: CatalogBootstrapOwnershipTokenV1::try_from_random_bytes(token).unwrap(),
            },
        )
        .unwrap()
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
fn catalog_recovery_exact_requires_owner_bound_physical_marker_and_staging_identity() {
    let bootstrap = catalog_with_token(PreCatalogRootKindV1::Workspace, 1, [91; 32]);
    let identities = super::super::protocol::ObservedInfrastructureIdentitiesV1::new(
        identity(14),
        identity(15),
        identity(16),
        identity(17),
    );
    let owner = super::super::protocol::synthetic_catalog_infrastructure_owner(
        &bootstrap,
        bootstrap.record_id(),
        *bootstrap.bootstrap_ownership_token().as_bytes(),
        identity(14),
        identities.clone(),
    );
    let exact = owner.recover_or_create(&bootstrap).unwrap();
    let infrastructure = exact.value().clone();

    assert_eq!(
        classify_catalog_bootstrap_recovery(
            &bootstrap,
            &CatalogRecordObservationV1::Missing,
            &CatalogRecordObservationV1::Exact(Box::new(bootstrap.clone())),
            &CatalogDirectoryObservationV1::Exact(Box::new(exact)),
            &CatalogDirectoryObservationV1::Missing,
            &CatalogRecordObservationV1::Missing,
        ),
        CatalogBootstrapRecoveryDecisionV1::PublishFinal
    );

    let foreign = super::super::protocol::synthetic_catalog_infrastructure_owner(
        &bootstrap,
        bootstrap.record_id(),
        [92; 32],
        identity(14),
        identities.clone(),
    );
    assert!(foreign.recover_or_create(&bootstrap).is_err());

    let (missing, writes) =
        super::super::protocol::synthetic_catalog_infrastructure_owner_missing_record(
            &bootstrap,
            identity(14),
            identities.clone(),
        );
    assert!(missing.recover_or_create(&bootstrap).is_ok());
    assert_eq!(writes.writes(), 1);

    let mismatched =
        super::super::protocol::synthetic_catalog_infrastructure_owner_mismatched_record(
            &bootstrap,
            identity(14),
            identities.clone(),
        );
    assert!(mismatched.recover_or_create(&bootstrap).is_err());

    let self_consistent_but_unowned = synthetic_infrastructure_from_catalog_bootstrap(
        &bootstrap,
        identity(14),
        identity(15),
        identity(16),
        identity(17),
    )
    .unwrap();
    assert_eq!(self_consistent_but_unowned, infrastructure);
    assert_eq!(
        classify_catalog_bootstrap_recovery(
            &bootstrap,
            &CatalogRecordObservationV1::Missing,
            &CatalogRecordObservationV1::Exact(Box::new(bootstrap.clone())),
            &CatalogDirectoryObservationV1::Other,
            &CatalogDirectoryObservationV1::Missing,
            &CatalogRecordObservationV1::Missing,
        ),
        CatalogBootstrapRecoveryDecisionV1::Ambiguous
    );
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

#[test]
fn catalog_bootstrap_recovery_table_is_closed() {
    use CatalogBootstrapRecoveryDecisionV1::*;
    use CatalogDirectoryObservationV1 as Directory;
    use CatalogRecordObservationV1 as Record;

    let bootstrap = catalog(PreCatalogRootKindV1::Workspace, 31);
    let exact = super::super::protocol::synthetic_catalog_infrastructure_owner(
        &bootstrap,
        bootstrap.record_id(),
        *bootstrap.bootstrap_ownership_token().as_bytes(),
        identity(14),
        super::super::protocol::ObservedInfrastructureIdentitiesV1::new(
            identity(14),
            identity(33),
            identity(34),
            identity(35),
        ),
    )
    .recover_or_create(&bootstrap)
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
        Directory::Exact(Box::new(exact.clone())),
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
                                && staging_value.as_ref() == &exact =>
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
                                && final_value.as_ref() == &exact =>
                            {
                                RetireActive
                            }
                            (
                                Record::Missing,
                                Record::Missing,
                                Directory::Missing,
                                Directory::Exact(final_value),
                                Record::Exact(retired_value),
                            ) if final_value.as_ref() == &exact
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

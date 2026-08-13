use std::path::Path;

use super::super::bootstrap::CatalogBootstrapV1;
use super::super::capability::{
    AsciiComponent, CanonicalComponent, CanonicalPathIdentityV1, CheckedFsError,
    DurableObjectIdentityV1, PathComponentMode, PreCatalogRootKindV1, PrivateControlDomain,
    RevalidatedPreCatalogPermitV1, SupportedFilesystemProfile, synthetic_pre_catalog_owner,
};
use super::super::catalog_names::{CatalogPrivateNameV1, CatalogPrivateRootV1};
use super::super::policy::CheckedArtifactPolicy;
use super::super::protocol::{
    CatalogBootstrapOwnershipTokenV1, CatalogBootstrapRecordV1, CatalogBootstrapRecoveryDecisionV1,
    CatalogBootstrapRecoveryObservationV1, ObservedInfrastructureIdentitiesV1,
    SyntheticCatalogDirectoryStateV1 as Directory, SyntheticCatalogRecordStateV1 as Record,
    SyntheticCatalogRecoveryLayoutV1, synthetic_catalog_recovery_owner,
};

fn identity(byte: u8) -> DurableObjectIdentityV1 {
    DurableObjectIdentityV1::linux_ext4([byte; 16], 1, vec![byte; 24]).unwrap()
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

struct CatalogRecordBuilder;

impl CatalogBootstrapV1<()> for CatalogRecordBuilder {
    type Catalog = CatalogBootstrapRecordV1;

    fn recover_or_create(
        &self,
        permit: RevalidatedPreCatalogPermitV1<'_, ()>,
    ) -> Result<Self::Catalog, CheckedFsError> {
        Ok(CatalogBootstrapRecordV1::from_revalidated_permit(
            permit,
            CatalogBootstrapOwnershipTokenV1::try_from_random_bytes([91; 32]).unwrap(),
        ))
    }
}

fn catalog() -> CatalogBootstrapRecordV1 {
    let (owner, _) = synthetic_pre_catalog_owner(
        (),
        SupportedFilesystemProfile::LinuxExt4FsIocGetFsUuidV1,
        identity(1),
        vec![1; 16],
        vec![1; 16],
        path(1),
    );
    owner
        .recover_or_create(
            Path::new("."),
            PreCatalogRootKindV1::Workspace,
            [2; 32],
            &PrivateControlDomain::checked_v1(),
            &[],
            &[],
            &CatalogRecordBuilder,
        )
        .unwrap()
}

fn identities() -> ObservedInfrastructureIdentitiesV1 {
    ObservedInfrastructureIdentitiesV1::new(identity(14), identity(15), identity(16), identity(17))
}

fn layout(
    active: Record,
    staging: Directory,
    final_directory: Directory,
    retired: Record,
) -> SyntheticCatalogRecoveryLayoutV1 {
    SyntheticCatalogRecoveryLayoutV1::new(
        Record::Missing,
        active,
        staging,
        final_directory,
        retired,
    )
}

#[test]
fn exact_staging_and_final_results_are_role_specific() {
    let bootstrap = catalog();
    let (staging_owner, staging_probe) = synthetic_catalog_recovery_owner(
        &bootstrap,
        layout(
            Record::Exact,
            Directory::Exact,
            Directory::Missing,
            Record::Missing,
        ),
        None,
        identity(14),
        identities(),
    );
    match staging_owner.recover(&bootstrap).unwrap() {
        CatalogBootstrapRecoveryObservationV1::PublishFinal(exact) => {
            assert_eq!(exact.value().staging_directory_identity(), &identity(14));
        }
        value => panic!("unexpected staging result: {:?}", value.decision()),
    }
    assert_eq!(
        (staging_probe.observations(), staging_probe.writes()),
        (1, 0)
    );

    let (final_owner, final_probe) = synthetic_catalog_recovery_owner(
        &bootstrap,
        layout(
            Record::Exact,
            Directory::Missing,
            Directory::Exact,
            Record::Missing,
        ),
        None,
        identity(14),
        identities(),
    );
    match final_owner.recover(&bootstrap).unwrap() {
        CatalogBootstrapRecoveryObservationV1::RetireActive(exact) => {
            assert_eq!(exact.value().staging_directory_identity(), &identity(14));
        }
        value => panic!("unexpected final result: {:?}", value.decision()),
    }
    assert_eq!((final_probe.observations(), final_probe.writes()), (1, 0));
}

#[test]
fn both_present_substituted_and_final_partial_are_ambiguous_without_mutation() {
    let bootstrap = catalog();
    for current in [
        layout(
            Record::Exact,
            Directory::Exact,
            Directory::Exact,
            Record::Missing,
        ),
        layout(
            Record::Exact,
            Directory::SubstitutedName,
            Directory::Missing,
            Record::Missing,
        ),
        layout(
            Record::Exact,
            Directory::Missing,
            Directory::PartialExpectedContents,
            Record::Missing,
        ),
    ] {
        let (owner, probe) =
            synthetic_catalog_recovery_owner(&bootstrap, current, None, identity(14), identities());
        assert_eq!(
            owner.recover(&bootstrap).unwrap().decision(),
            CatalogBootstrapRecoveryDecisionV1::Ambiguous
        );
        assert_eq!((probe.observations(), probe.writes()), (1, 0));
    }
}

#[test]
fn infrastructure_write_requires_accepted_aggregate_then_reobservation() {
    let bootstrap = catalog();
    let before = layout(
        Record::Exact,
        Directory::OwnedMissingRecord,
        Directory::Missing,
        Record::Missing,
    );
    let after = layout(
        Record::Exact,
        Directory::Exact,
        Directory::Missing,
        Record::Missing,
    );
    let (owner, probe) = synthetic_catalog_recovery_owner(
        &bootstrap,
        before,
        Some(after),
        identity(14),
        identities(),
    );
    assert!(matches!(
        owner.recover(&bootstrap).unwrap(),
        CatalogBootstrapRecoveryObservationV1::PublishFinal(_)
    ));
    assert_eq!((probe.observations(), probe.writes()), (2, 1));
}

#[test]
fn rejected_aggregate_with_missing_record_never_writes() {
    let bootstrap = catalog();
    for current in [
        layout(
            Record::Exact,
            Directory::OwnedMissingRecord,
            Directory::Exact,
            Record::Missing,
        ),
        SyntheticCatalogRecoveryLayoutV1::new(
            Record::Other,
            Record::Exact,
            Directory::OwnedMissingRecord,
            Directory::Missing,
            Record::Missing,
        ),
        layout(
            Record::Exact,
            Directory::Missing,
            Directory::OwnedMissingRecord,
            Record::Missing,
        ),
    ] {
        let (owner, probe) =
            synthetic_catalog_recovery_owner(&bootstrap, current, None, identity(14), identities());
        assert_eq!(
            owner.recover(&bootstrap).unwrap().decision(),
            CatalogBootstrapRecoveryDecisionV1::Ambiguous
        );
        assert_eq!((probe.observations(), probe.writes()), (1, 0));
    }
}

#[test]
fn one_catalog_name_owner_drives_record_collision_and_policy_paths() {
    let bootstrap = catalog();
    assert_eq!(
        bootstrap.staging_name().as_bytes(),
        CatalogPrivateNameV1::BootstrapStaging.leaf_bytes()
    );
    assert_eq!(
        bootstrap.final_name().as_bytes(),
        CatalogPrivateNameV1::Final.leaf_bytes()
    );

    let collision = PrivateControlDomain::checked_v1()
        .members()
        .iter()
        .map(|path| path.as_bytes().to_vec())
        .collect::<Vec<_>>();
    assert_eq!(
        collision,
        CatalogPrivateNameV1::ALL
            .iter()
            .map(|name| name.relative_bytes(CatalogPrivateRootV1::Workspace))
            .collect::<Vec<_>>()
    );

    assert_eq!(
        CheckedArtifactPolicy::workspace(Path::new(".")).private_parent(),
        CatalogPrivateNameV1::Final.relative_path(CatalogPrivateRootV1::Workspace)
    );
    assert_eq!(
        CheckedArtifactPolicy::git_directory(Path::new(".")).private_parent(),
        CatalogPrivateNameV1::Final.relative_path(CatalogPrivateRootV1::GitDirectory)
    );
}

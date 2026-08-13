use std::path::Path;

use super::super::bootstrap::CatalogBootstrapV1;
use super::super::capability::{
    AsciiComponent, CanonicalComponent, CanonicalPathIdentityV1, CheckedFsError,
    DurableObjectIdentityV1, PathComponentMode, PreCatalogRootKindV1, PrivateControlDomain,
    RevalidatedPreCatalogPermitV1, SupportedFilesystemProfile, synthetic_pre_catalog_owner,
    synthetic_pre_catalog_permit,
};
use super::super::catalog_names::CatalogPrivateNameV1;

#[derive(Clone, Debug, Eq, PartialEq)]
struct RetainedRoot(u8);

fn identity(value: u8) -> DurableObjectIdentityV1 {
    DurableObjectIdentityV1::linux_ext4([value; 16], 1, vec![value]).unwrap()
}

fn path(root: u8, invocation: u8, domain: u8) -> CanonicalPathIdentityV1 {
    CanonicalPathIdentityV1::new(vec![
        CanonicalComponent::try_bound(
            AsciiComponent::parse(b".gwz").unwrap(),
            PathComponentMode::Sensitive,
            identity(root),
            vec![invocation],
            vec![domain],
        )
        .unwrap(),
        CanonicalComponent::try_bound(
            AsciiComponent::parse(CatalogPrivateNameV1::Final.leaf_bytes()).unwrap(),
            PathComponentMode::Sensitive,
            identity(root + 1),
            vec![invocation + 1],
            vec![domain],
        )
        .unwrap(),
    ])
    .unwrap()
}

struct Catalog {
    probe: super::super::capability::SyntheticPreCatalogProbeV1,
}

impl CatalogBootstrapV1<RetainedRoot> for Catalog {
    type Catalog = [u8; 32];

    fn recover_or_create(
        &self,
        permit: RevalidatedPreCatalogPermitV1<'_, RetainedRoot>,
    ) -> Result<Self::Catalog, CheckedFsError> {
        self.probe.note_bootstrap();
        Ok(permit.collision_domain_digest())
    }
}

fn owner() -> (
    super::super::capability::PreCatalogOwnerV1<Path, RetainedRoot>,
    super::super::capability::SyntheticPreCatalogProbeV1,
) {
    synthetic_pre_catalog_owner(
        RetainedRoot(1),
        SupportedFilesystemProfile::LinuxExt4FsIocGetFsUuidV1,
        identity(1),
        vec![2],
        vec![3],
        path(1, 2, 3),
    )
}

#[test]
fn owner_structurally_revalidates_immediately_before_catalog_bootstrap() {
    let (owner, probe) = owner();
    let control = PrivateControlDomain::checked_v1();
    for kind in [
        PreCatalogRootKindV1::Workspace,
        PreCatalogRootKindV1::GitDirectory,
    ] {
        probe.clear_events();
        let value = owner
            .recover_or_create(
                Path::new("."),
                kind,
                [9; 32],
                &control,
                &[],
                &[],
                &Catalog {
                    probe: probe.clone(),
                },
            )
            .unwrap();
        assert_eq!(value, control.version_digest());
        assert_eq!(probe.events(), ["observe", "revalidate", "bootstrap"]);
    }
}

#[test]
fn failed_immediate_revalidation_cannot_enter_catalog_bootstrap() {
    let (owner, probe) = owner();
    probe.reject_revalidation();
    assert!(
        owner
            .recover_or_create(
                Path::new("."),
                PreCatalogRootKindV1::Workspace,
                [9; 32],
                &PrivateControlDomain::checked_v1(),
                &[],
                &[],
                &Catalog {
                    probe: probe.clone(),
                },
            )
            .is_err()
    );
    assert_eq!(probe.events(), ["observe", "revalidate"]);
}

#[test]
fn profile_identity_and_path_root_substitution_reject() {
    let wrong_profile = synthetic_pre_catalog_permit(
        RetainedRoot(1),
        SupportedFilesystemProfile::WindowsNtfsFileId128V1,
        identity(1),
        vec![2],
        vec![3],
        path(1, 2, 3),
        [4; 32],
        [5; 32],
        PreCatalogRootKindV1::Workspace,
    );
    assert!(wrong_profile.is_err());

    let wrong_root = synthetic_pre_catalog_permit(
        RetainedRoot(1),
        SupportedFilesystemProfile::LinuxExt4FsIocGetFsUuidV1,
        identity(1),
        vec![2],
        vec![3],
        path(9, 2, 3),
        [4; 32],
        [5; 32],
        PreCatalogRootKindV1::Workspace,
    );
    assert!(wrong_root.is_err());

    let wrong_invocation = synthetic_pre_catalog_permit(
        RetainedRoot(1),
        SupportedFilesystemProfile::LinuxExt4FsIocGetFsUuidV1,
        identity(1),
        vec![2],
        vec![3],
        path(1, 8, 3),
        [4; 32],
        [5; 32],
        PreCatalogRootKindV1::Workspace,
    );
    assert!(wrong_invocation.is_err());
}

#[test]
fn every_intermediate_parent_identity_mode_and_domain_is_persisted() {
    let value = path(1, 2, 3);
    assert_eq!(value.components().len(), 2);
    assert_eq!(
        value.components()[0].parent_durable_identity(),
        &identity(1)
    );
    assert_eq!(
        value.components()[1].parent_durable_identity(),
        &identity(2)
    );
    assert_eq!(value.components()[0].parent_invocation_identity(), [2]);
    assert_eq!(value.components()[1].parent_invocation_identity(), [3]);
    assert_eq!(value.components()[0].rename_domain(), [3]);
    assert_eq!(value.components()[1].rename_domain(), [3]);
}

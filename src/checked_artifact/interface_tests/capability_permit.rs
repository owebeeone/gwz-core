use std::path::Path;

use super::super::bootstrap::CatalogBootstrapV1;
use super::super::capability::{
    AsciiComponent, CanonicalComponent, CanonicalPathIdentityV1, CheckedFsError,
    DurableObjectIdentityV1, LosslessIndexEntry, PathComponentMode, PreCatalogPermitV1,
    PreCatalogPreflightV1, PreCatalogRootKindV1, PrivateControlDomain, SupportedFilesystemProfile,
    TrackedWorktreeEntry, synthetic_pre_catalog_permit,
};

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
            AsciiComponent::parse(b"checked-artifacts").unwrap(),
            PathComponentMode::Sensitive,
            identity(root + 1),
            vec![invocation + 1],
            vec![domain],
        )
        .unwrap(),
    ])
    .unwrap()
}

struct Provider {
    root: u8,
    invocation: u8,
    domain: u8,
}

impl PreCatalogPreflightV1<Path> for Provider {
    type RetainedRoot = RetainedRoot;

    fn inspect_and_scan(
        &self,
        _root: &Path,
        _root_kind: PreCatalogRootKindV1,
        _domain: &PrivateControlDomain,
        _index: &[LosslessIndexEntry],
        _worktree: &[TrackedWorktreeEntry],
    ) -> Result<
        (
            Self::RetainedRoot,
            SupportedFilesystemProfile,
            DurableObjectIdentityV1,
            Vec<u8>,
            Vec<u8>,
            CanonicalPathIdentityV1,
        ),
        CheckedFsError,
    > {
        Ok((
            RetainedRoot(self.root),
            SupportedFilesystemProfile::LinuxExt4FsIocGetFsUuidV1,
            identity(self.root),
            vec![self.invocation],
            vec![self.domain],
            path(self.root, self.invocation, self.domain),
        ))
    }

    fn revalidate(
        &self,
        _root: &Path,
        permit: &PreCatalogPermitV1<Self::RetainedRoot>,
    ) -> Result<(), CheckedFsError> {
        if permit.retained_root() == &RetainedRoot(self.root)
            && permit.root_identity() == &identity(self.root)
            && permit.root_invocation_identity() == [self.invocation]
            && permit.rename_domain() == [self.domain]
        {
            Ok(())
        } else {
            Err(CheckedFsError::ambiguous("retained root", "replaced"))
        }
    }
}

struct Catalog;

impl CatalogBootstrapV1<RetainedRoot> for Catalog {
    type Catalog = [u8; 32];

    fn recover_or_create(
        &self,
        permit: &PreCatalogPermitV1<RetainedRoot>,
    ) -> Result<Self::Catalog, CheckedFsError> {
        Ok(permit.collision_domain_digest())
    }
}

#[test]
fn one_transaction_issues_root_bound_workspace_and_git_directory_permits() {
    let provider = Provider {
        root: 1,
        invocation: 2,
        domain: 3,
    };
    let control = PrivateControlDomain::checked_v1();
    for kind in [
        PreCatalogRootKindV1::Workspace,
        PreCatalogRootKindV1::GitDirectory,
    ] {
        let permit = provider
            .preflight(Path::new("."), kind, [9; 32], &control, &[], &[])
            .unwrap();
        assert_eq!(permit.root_kind(), kind);
        assert_eq!(permit.lease_binding(), [9; 32]);
        assert_eq!(permit.collision_domain_digest(), control.version_digest());
        provider.revalidate(Path::new("."), &permit).unwrap();
        assert_eq!(
            Catalog.recover_or_create(&permit).unwrap(),
            control.version_digest()
        );
    }
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

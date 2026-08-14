//! Owner-private raw pre-catalog provider seam.

use super::*;
use crate::checked_artifact::bootstrap::CatalogLeaseTargetWitnessV1;
use crate::checked_artifact::capability::{
    CheckedFsError, DurableObjectIdentityV1, SupportedFilesystemProfile,
};

mod filesystem;
mod index;
mod namespace;
mod platform;
mod retained;
mod snapshot;

#[allow(
    unused_imports,
    reason = "R2-C1 consumes the sole lease-bound production observation route"
)]
pub(in crate::checked_artifact::capability::pre_catalog) use filesystem::{
    inspect_bound_catalog_target, revalidate_lease_root_binding,
};

pub(in crate::checked_artifact::capability::pre_catalog) fn revalidate_bound_observation(
    bound: &LeaseBoundPreCatalogObservationV1<'_>,
) -> Result<(), CheckedFsError> {
    filesystem::platform_pre_catalog_provider()
        .revalidate_bound_target(&bound.target, &bound.observation)
}
pub(in crate::checked_artifact) use platform::HostPlatform;
pub(in crate::checked_artifact::capability::pre_catalog) use retained::RetainedPlatformRoot;

pub(super) struct RawPreCatalogObservationV1<RetainedRoot> {
    pub(super) retained_root: RetainedRoot,
    pub(super) support_profile: SupportedFilesystemProfile,
    pub(super) root_identity: DurableObjectIdentityV1,
    pub(super) root_invocation_identity: Vec<u8>,
    pub(super) rename_domain: Vec<u8>,
    pub(super) path_profile: CanonicalPathIdentityV1,
    pub(super) collision_snapshot_digest: [u8; 32],
    pub(super) raw_roles: RawCatalogRoleObservationV1,
}

pub(in crate::checked_artifact::capability::pre_catalog) struct LeaseBoundPreCatalogObservationV1<
    'lease,
> {
    pub(super) target: CatalogLeaseTargetWitnessV1<'lease>,
    pub(super) observation: RawPreCatalogObservationV1<RetainedPlatformRoot>,
}

pub(in crate::checked_artifact::capability::pre_catalog) fn has_private_parent(
    bound: &LeaseBoundPreCatalogObservationV1<'_>,
) -> bool {
    bound.observation.retained_root.private_parent().is_some()
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct RawCatalogRoleObservationV1 {
    pub(super) rows: Vec<(Vec<u8>, Vec<u8>)>,
}

pub(super) trait RawPreCatalogProviderV1<Root: ?Sized, RetainedRoot> {
    fn inspect_workspace(
        &self,
        root: &Root,
    ) -> Result<RawPreCatalogObservationV1<RetainedRoot>, CheckedFsError>;

    fn inspect_git_directory(
        &self,
        root: &Root,
    ) -> Result<RawPreCatalogObservationV1<RetainedRoot>, CheckedFsError>;

    fn revalidate_workspace(
        &self,
        root: &Root,
        observation: &RawPreCatalogObservationV1<RetainedRoot>,
    ) -> Result<(), CheckedFsError>;

    fn revalidate_git_directory(
        &self,
        root: &Root,
        observation: &RawPreCatalogObservationV1<RetainedRoot>,
    ) -> Result<(), CheckedFsError>;
}

#[cfg(test)]
mod production_tests;

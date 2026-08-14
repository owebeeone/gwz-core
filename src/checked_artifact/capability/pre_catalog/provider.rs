//! Owner-private raw pre-catalog provider seam.

use super::*;
use crate::checked_artifact::bootstrap::CatalogLeaseTargetWitnessV1;
use crate::checked_artifact::capability::{
    CheckedFsError, DurableObjectIdentityV1, DurablePathV1, SupportedFilesystemProfile,
};
use crate::checked_artifact::catalog::{
    CatalogAttemptBindingV1, CatalogParentObservationV1, CatalogRecognizedNameV1,
};

mod aggregate;
mod digests;
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
    inspect_bound_catalog_target, revalidate_lease_root_binding, revalidate_missing_observation,
    revalidate_ready_observation,
};

pub(in crate::checked_artifact::capability::pre_catalog) fn revalidate_bound_observation(
    bound: &LeaseBoundPreCatalogObservationV1<'_>,
) -> Result<(), CheckedFsError> {
    filesystem::platform_pre_catalog_provider()
        .revalidate_bound_target(&bound.target, &bound.observation)
}
pub(in crate::checked_artifact::capability::pre_catalog) use aggregate::outer_aggregate_facts;
pub(in crate::checked_artifact::capability::pre_catalog) use digests::ReadyObservationDigestsV1;
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
    pub(super) ready_digests: Option<ReadyObservationDigestsV1>,
    pub(super) missing_parent_digest: Option<MissingParentObservationDigestV1>,
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

pub(in crate::checked_artifact::capability::pre_catalog) fn attempt_binding(
    bound: &LeaseBoundPreCatalogObservationV1<'_>,
    durable_target_digest: DurableCatalogTargetDigestV1,
    historical_collision_digest: HistoricalCollisionDigestV1,
) -> Result<CatalogAttemptBindingV1, CheckedFsError> {
    let parent = bound
        .observation
        .retained_root
        .private_parent()
        .ok_or_else(|| {
            CheckedFsError::ambiguous(
                "catalog attempt binding",
                "ready binding requires retained mutation parent",
            )
        })?;
    Ok(CatalogAttemptBindingV1::owner_issue(
        bound.target.facts()?.root_kind(),
        bound.observation.support_profile,
        durable_target_digest,
        historical_collision_digest,
        parent.identity().durable().clone(),
        DurablePathV1::from_live(&bound.observation.path_profile)?,
    ))
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct RawCatalogRoleObservationV1 {
    pub(super) enumeration: CatalogParentObservationV1,
    pub(super) rows: Vec<RawCatalogRoleRowV1>,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct RawCatalogRoleRowV1 {
    pub(super) role: CatalogRecognizedNameV1,
    pub(super) path: Vec<u8>,
    pub(super) fact: RawCatalogEntryFactV1,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum RawCatalogEntryFactV1 {
    Directory {
        identity: Vec<u8>,
        retired: RawCatalogRetiredFactV1,
    },
    RegularFile {
        identity: Vec<u8>,
        bytes: RawCatalogBytesV1,
    },
    Other(Vec<u8>),
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum RawCatalogRetiredFactV1 {
    Missing,
    RegularFile {
        identity: Vec<u8>,
        bytes: RawCatalogBytesV1,
    },
    Other(Vec<u8>),
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum RawCatalogBytesV1 {
    Bounded(Vec<u8>),
    Oversize,
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
mod catalog_tests;
#[cfg(test)]
mod production_tests;

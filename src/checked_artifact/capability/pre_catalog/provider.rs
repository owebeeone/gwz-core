//! Owner-private raw pre-catalog provider seam.

use super::*;
use crate::checked_artifact::bootstrap::CatalogLeaseTargetWitnessV1;
use crate::checked_artifact::capability::{
    CheckedFsError, DurableObjectIdentityV1, DurablePathV1, SupportedFilesystemProfile,
};
use crate::checked_artifact::catalog::{
    CatalogAttemptBindingV1, CatalogParentObservationV1, CatalogRecognizedNameV1,
};

mod admission_mutation;
mod aggregate;
/// R2-D Phase 2 Step 2.4 — the authority parse / streamed proof split.
#[allow(
    dead_code,
    reason = "Step 2.4 lands the binding; plan §4 Step 3.3 wires its production consumer"
)]
mod authority_record_binding;
mod completed;
mod digests;
mod directory_mutation;
mod filesystem;
mod index;
mod interior;
/// R2-D Step 2.1. Its production caller is Step 2.4's
/// `authority_record_binding`; the allow remains because that binding's own
/// consumer is wired by plan §4 Step 3.3.
#[allow(
    dead_code,
    reason = "Step 2.4 binds the observer; plan §4 Step 3.3 wires the binding's consumer"
)]
mod leaf_observation;
mod managed_mutation;
mod mutation;
mod namespace;
mod namespace_mutation;
mod platform;
mod publication;
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
pub(in crate::checked_artifact::capability::pre_catalog) use completed::{
    RetainedCompletedCatalogV1, retain_completed_catalog,
};
pub(in crate::checked_artifact::capability::pre_catalog) use digests::ReadyObservationDigestsV1;
pub(in crate::checked_artifact::capability::pre_catalog) use directory_mutation::{
    prepare_or_rewrite_staging, publish_final_directory, retire_active_record,
};
#[cfg(test)]
pub(in crate::checked_artifact) use managed_mutation::retain_managed_parent_at_for_test;
/// R2-D Phase 2 Step 2.3 — the retained managed-parent capability and the
/// durable facts its two edges observe. Same rule as the row above: what leaves
/// this owner is a capability, never a path and never a raw mutation surface.
///
/// R2-D Phase 3 Step 3.1 adds the managed-prefix observation the provider's
/// `observe_preflight`/`revalidate_plan` are built from, and drops
/// `retain_managed_parent` from this hop: a managed parent can only be retained
/// under a `&Dir`, no `Dir` leaves this owner, and the production route is
/// `managed_mutation::retain_managed_prefix` — so no consumer outside the owner
/// could ever have called the re-exported constructor (Step-3.1 review [P3-1]
/// corrects an earlier claim here that named it `retain_managed_parent`'s
/// caller; it calls `retain_managed_child`).
///
/// R2-D Phase 3 Step 3.1b adds the managed intent record's owner-private
/// lifecycle surface, driven by the `namespace` owner exactly as the E15/E16
/// edges are.
pub(in crate::checked_artifact) use managed_mutation::{
    ManagedInstalledFactsV1, ManagedIntentEdgeV1, ManagedPrefixObservationV1,
    ManagedRetiredFactsV1, ObservedManagedObjectV1, RetainedManagedParentV1,
    observe_managed_intent_row, read_managed_intent_row, write_managed_intent_scratch,
};
pub(in crate::checked_artifact::capability::pre_catalog) use managed_mutation::{
    managed_provider_instance, observe_managed_prefix, retain_managed_prefix,
};
pub(in crate::checked_artifact::capability::pre_catalog) use mutation::{
    create_git_private_parent, finish_ready_edge_root_barrier, publish_active_record,
    write_or_rewrite_scratch,
};
/// R2-D Phase 2 Step 2.2. The retained action namespace is the only namespace
/// capability that leaves this owner, and it carries no path and no raw
/// mutation surface (amendment §7 :576-577).
pub(in crate::checked_artifact) use namespace_mutation::{
    ActionNamespaceEdgeV1, ObservedNamespaceObjectV1, RetainedActionNamespaceV1,
};
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
        durable_identity: DurableObjectIdentityV1,
        interior: RawCatalogInteriorObservationV1,
    },
    RegularFile {
        identity: Vec<u8>,
        bytes: RawCatalogBytesV1,
    },
    Other(Vec<u8>),
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct RawCatalogInteriorObservationV1 {
    pub(super) entry_count: usize,
    pub(super) encoded_name_bytes: usize,
    pub(super) rows: Vec<RawCatalogInteriorRowV1>,
    /// The catalog root's `RootEntryNameV1::ActiveAction` rows, sorted by
    /// digest. C-3 widening (interface freeze §4.4 Class 2): the observer now
    /// records the second arm of the frozen root grammar instead of refusing
    /// it. Only the row's identity is retained here — the admission owner
    /// re-verifies each action directory's interior through its own bounded
    /// observation before every edge.
    pub(super) action_rows: Vec<crate::checked_artifact::protocol::ActionDigestV1>,
    /// The bounded global classification of every child into the
    /// `GwzM5-8R4bR2ConsumerCheckpoint.md` §6 (:199-201) grammar.
    pub(super) census: crate::checked_artifact::protocol::CatalogRootRowCensusV1,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct RawCatalogInteriorRowV1 {
    pub(super) slot: crate::checked_artifact::protocol::InfrastructureSlotV1,
    pub(super) fact: RawCatalogInteriorFactV1,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum RawCatalogInteriorFactV1 {
    EmptyDirectory {
        identity: Vec<u8>,
        durable_identity: DurableObjectIdentityV1,
    },
    RegularFile {
        identity: Vec<u8>,
        durable_identity: DurableObjectIdentityV1,
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
mod directory_mutation_tests;
#[cfg(test)]
mod mutation_tests;
#[cfg(test)]
mod production_tests;
#[cfg(test)]
mod tests_admission_spike;
#[cfg(test)]
mod tests_authority_record;
#[cfg(test)]
mod tests_authority_record_matrix;
#[cfg(test)]
mod tests_leaf_fault_matrix;
#[cfg(test)]
mod tests_leaf_observation;

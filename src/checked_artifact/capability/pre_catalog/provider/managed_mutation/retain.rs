use crate::checked_artifact::capability::{
    AsciiComponent, CanonicalComponent, CanonicalPathIdentityV1, CheckedFsError,
    DurableIdentityProvider, DurableObjectIdentityV1, PathComponentMode, PathEquivalenceProvider,
};
use crate::checked_artifact::protocol::RecordDigestV1;
use crate::filesystem::FsDirectory as Dir;
use std::ffi::OsString;

use super::*;

/// The one retained managed parent a namespace backend owns for the life of a
/// managed component: opened once through an identity-proved no-follow hop from
/// a directory the caller already retained, and held across every managed
/// observation, publication and retirement.
pub(in crate::checked_artifact) struct RetainedManagedParentV1 {
    pub(crate) parent: Dir,
    pub(crate) leaf: OsString,
    pub(crate) handle: Dir,
    pub(crate) identity: DurableObjectIdentityV1,
    pub(crate) path_profile: CanonicalPathIdentityV1,
    pub(crate) parent_mode: PathComponentMode,
    pub(crate) reservation: RecordDigestV1,
}

/// Retains one managed parent directory beneath an already-retained directory.
///
/// This is the managed analogue of `namespace_mutation::retain_action_namespace`
/// and follows the identical rule: one no-follow hop, identity proved at
/// acquisition, and a canonical path profile bound to the enclosing directory's
/// own observed identity — one component here, because this entry point takes
/// the enclosure as the profile root.
///
/// **Call graph, stated exactly** (Step-3.1 review [P3-1] corrects the earlier
/// claim in this comment). This wrapper has **no production caller**: production
/// reaches a managed parent through [`retain_managed_prefix`], which composes a
/// depth-*d* profile and calls [`retain_managed_child`] directly. What is
/// retained here is the *one-component* shape, and its only caller is the
/// `#[cfg(test)]` enclosure door [`retain_managed_parent_at_for_test`], for which
/// it is the shared retainer. It is kept rather than folded into that door so the
/// door stays a two-line ambient-open, and its production visibility is a
/// standing item for the lane owner at the Phase 3 settle.
pub(in crate::checked_artifact) fn retain_managed_parent(
    enclosing: &Dir,
    leaf: &str,
    reservation: RecordDigestV1,
) -> Result<RetainedManagedParentV1, CheckedFsError> {
    retain_managed_child(
        enclosing,
        &[],
        &AsciiComponent::parse(leaf.as_bytes())?,
        reservation,
    )
}

/// The one-hop retainer both managed entry points share.
///
/// `prefix` is the canonical path profile of `enclosing` itself, so a managed
/// parent reached at depth *d* carries a *d*-component profile rather than the
/// one-component profile a single hop would produce. That is not cosmetic: the
/// resident intent binds each component to its parent's profile
/// (`protocol/managed_bootstrap_record.rs` `matches_component_parent`), and the
/// installed component's profile is the parent's plus exactly one component
/// (`installed_path`), so a truncated prefix would refuse every managed
/// component below the first.
pub(crate) fn retain_managed_child(
    enclosing: &Dir,
    prefix: &[CanonicalComponent],
    leaf: &AsciiComponent,
    reservation: RecordDigestV1,
) -> Result<RetainedManagedParentV1, CheckedFsError> {
    let leaf_name = os_name(leaf);
    let handle = enclosing
        .retained_child(&leaf_name)
        .map_err(|source| CheckedFsError::io("open managed parent", source))?;
    let fact = super::super::HostPlatform.dir_identity(&handle)?;
    let mut components = prefix.to_vec();
    components.push(bind_child_component(enclosing, leaf)?);
    let path_profile = CanonicalPathIdentityV1::new(components)?;
    let parent = enclosing
        .clone_handle()
        .map_err(|source| CheckedFsError::io("retain managed parent enclosure", source))?;
    // The mode that governs the *installed* leaf is the managed parent's own,
    // not its enclosure's: the installed component is a child of `handle`.
    let installed_mode = super::super::HostPlatform.parent_mode(&handle)?;
    Ok(RetainedManagedParentV1 {
        parent,
        leaf: leaf_name,
        handle,
        identity: fact.durable().clone(),
        path_profile,
        parent_mode: installed_mode,
        reservation,
    })
}

/// One canonical component bound to the observed facts of its own enclosure.
pub(crate) fn bind_child_component(
    enclosing: &Dir,
    leaf: &AsciiComponent,
) -> Result<CanonicalComponent, CheckedFsError> {
    let enclosing_fact = super::super::HostPlatform.dir_identity(enclosing)?;
    CanonicalComponent::try_bound(
        leaf.clone(),
        super::super::HostPlatform.parent_mode(enclosing)?,
        enclosing_fact.durable().clone(),
        enclosing_fact.invocation().clone(),
        super::super::HostPlatform.rename_domain(enclosing)?,
    )
}

/// The test-only enclosure door.
///
/// Production reaches a managed parent through [`retain_managed_prefix`], which
/// walks from the permit-retained root that plan §4 Step 3.1's
/// `ManagedParentBootstrap` provider drives. This door lets Step 2.3's matrix
/// drive the two real edges against a managed parent placed beside the catalog,
/// and it is the only place in this owner that opens an ambient path.
///
/// *[E4.2 disposition, 2026-09-01, §11.3 item 2(a): SURVIVES. E4.2 gave the
/// managed-parent provider its production caller, and that caller does supply a
/// real retained managed parent — but through `retain_managed_prefix`'s depth-d
/// composition, never this ONE-COMPONENT enclosure shape. The three
/// `namespace/tests_managed.rs` callers are Step 2.3's only route to it, so
/// retiring the door would delete coverage rather than duplication.]*
#[cfg(test)]
pub(in crate::checked_artifact) fn retain_managed_parent_at_for_test(
    enclosing: &std::path::Path,
    leaf: &str,
    reservation: RecordDigestV1,
) -> Result<RetainedManagedParentV1, CheckedFsError> {
    let directory = crate::filesystem::make_filesystem()
        .open_directory(enclosing)
        .map_err(|source| CheckedFsError::io("open managed enclosure", source))?;
    retain_managed_parent(&directory, leaf, reservation)
}

/// R2-D Phase 3 Step 3.1 — the managed-parent provider's instance binding.
///
/// `ManagedParentProviderBindingV1` must be nonzero and must identify *this*
/// provider across the preflight, the admission bind, and the execute of one
/// plan (`bootstrap/managed/owner.rs` `execute`). The retained root's own
/// durable identity is the honest source: it is stable for the life of the
/// retained target, differs between targets, and is already proved. It is
/// hashed rather than exposed so the binding carries no identity bytes.
pub(in crate::checked_artifact::capability::pre_catalog) fn managed_provider_instance(
    root: &super::super::RetainedPlatformRoot,
) -> [u8; 32] {
    use sha2::{Digest, Sha256};

    let mut digest = Sha256::new();
    digest.update(b"gwz-managed-parent-provider-instance-v1\0");
    digest.update(root.root().identity().durable().encode_canonical());
    digest.finalize().into()
}

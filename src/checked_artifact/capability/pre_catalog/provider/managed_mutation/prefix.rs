use crate::checked_artifact::capability::{
    AsciiComponent, CanonicalComponent, CanonicalPathIdentityV1, CheckedFsError,
    DurableIdentityProvider, DurableObjectIdentityV1, PathComponentMode, PathEquivalenceProvider,
};
use crate::checked_artifact::protocol::RecordDigestV1;
use crate::filesystem::FsKind;

use super::*;

/// The durable facts one managed-path prefix depth observed. Nothing here is a
/// handle or a path string: it is the same typed triple the plan row and the
/// resident intent already carry (`bootstrap/managed/plan.rs`,
/// `protocol/managed_bootstrap_record.rs`), so a consumer outside this owner
/// still receives only facts.
pub(in crate::checked_artifact) struct ManagedPrefixDepthV1 {
    pub(crate) identity: DurableObjectIdentityV1,
    pub(crate) mode: PathComponentMode,
    pub(crate) path: CanonicalPathIdentityV1,
}

impl ManagedPrefixDepthV1 {
    pub(in crate::checked_artifact) const fn identity(&self) -> &DurableObjectIdentityV1 {
        &self.identity
    }

    /// The mode governing this directory's *children*, which is the mode the
    /// plan row and the intent both record for a retained managed parent
    /// (`retain_managed_child`'s `installed_mode`).
    pub(in crate::checked_artifact) const fn mode(&self) -> PathComponentMode {
        self.mode
    }

    pub(in crate::checked_artifact) const fn path(&self) -> &CanonicalPathIdentityV1 {
        &self.path
    }
}

/// One bounded observation of a managed-parent path prefix: the facts of every
/// depth that is durably present, in order, stopping at the first absent
/// component. `retained_count()` is exactly the plan's
/// `retained_existing_parent_count`.
pub(in crate::checked_artifact) struct ManagedPrefixObservationV1 {
    pub(crate) depths: Vec<ManagedPrefixDepthV1>,
}

impl ManagedPrefixObservationV1 {
    pub(in crate::checked_artifact) fn retained_count(&self) -> usize {
        self.depths.len()
    }

    /// The facts of the directory reached by `depth` components. `depth` is the
    /// plan's own 1-based count, so `at(retained_count())` is the deepest
    /// retained parent and `at(0)` is deliberately `None` — the enclosing root
    /// is never a managed parent.
    pub(in crate::checked_artifact) fn at(&self, depth: usize) -> Option<&ManagedPrefixDepthV1> {
        depth
            .checked_sub(1)
            .and_then(|index| self.depths.get(index))
    }
}

/// R2-D Phase 3 Step 3.1 — the bounded, read-only managed-parent prefix
/// observation `ManagedParentBootstrap::observe_preflight` and
/// `revalidate_plan` are built from.
///
/// It is primitive family P3 + P4 only (identity and bounded enumeration): one
/// no-follow hop per component from the permit-retained root, each hop's
/// identity proved before the next, and no durable edge anywhere. The walk
/// stops at the first absent component, which is what makes the plan a
/// *missing-suffix* plan rather than a re-plan of live components
/// (`GwzM5-8R4bR2ConsumerCheckpoint.md` §9; the Step-2.3 review's Phase-3
/// caution on populated components).
pub(in crate::checked_artifact::capability::pre_catalog) fn observe_managed_prefix(
    root: &super::super::RetainedPlatformRoot,
    components: &[AsciiComponent],
) -> Result<ManagedPrefixObservationV1, CheckedFsError> {
    require_bounded_prefix(components)?;
    let mut current = clone_root(root)?;
    let mut profile: Vec<CanonicalComponent> = Vec::new();
    let mut depths = Vec::new();
    depths
        .try_reserve_exact(components.len())
        .map_err(|_| prefix_allocation_failure())?;
    for component in components {
        let name = os_name(component);
        match current.entry_metadata(&name) {
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => break,
            Err(source) => return Err(CheckedFsError::io("observe managed parent prefix", source)),
            Ok(metadata)
                if metadata.kind != FsKind::Directory || metadata.kind == FsKind::Symlink =>
            {
                return Err(managed_error(
                    "managed parent prefix component is not a canonical directory",
                ));
            }
            Ok(_) => {}
        }
        profile.push(bind_child_component(&current, component)?);
        let child = current
            .retained_child(&name)
            .map_err(|source| CheckedFsError::io("open managed parent prefix", source))?;
        let fact = super::super::HostPlatform.dir_identity(&child)?;
        depths.push(ManagedPrefixDepthV1 {
            identity: fact.durable().clone(),
            mode: super::super::HostPlatform.parent_mode(&child)?,
            path: CanonicalPathIdentityV1::new(profile.clone())?,
        });
        current = child;
    }
    Ok(ManagedPrefixObservationV1 { depths })
}

/// R2-D Phase 3 Step 3.1 — the production route to a retained managed parent.
///
/// This is the only route to one that exists at all: a managed parent must be
/// retained under an already-retained `&Dir`, and no `Dir` leaves this owner, so
/// the enclosing directory can only be walked here. It composes the depth-*d*
/// profile itself and calls [`retain_managed_child`]; the one-component
/// [`retain_managed_parent`] wrapper is not on this path (Step-3.1 review
/// [P3-1]). `depth` is the plan's own 1-based retained count, so the retained
/// parent is `components[..depth]` and the enclosure is `components[..depth - 1]`.
pub(in crate::checked_artifact::capability::pre_catalog) fn retain_managed_prefix(
    root: &super::super::RetainedPlatformRoot,
    components: &[AsciiComponent],
    depth: usize,
    reservation: RecordDigestV1,
) -> Result<RetainedManagedParentV1, CheckedFsError> {
    require_bounded_prefix(components)?;
    if depth == 0 || depth > components.len() {
        return Err(managed_error(
            "managed parent depth is outside the declared path",
        ));
    }
    let mut current = clone_root(root)?;
    let mut profile: Vec<CanonicalComponent> = Vec::new();
    for component in &components[..depth - 1] {
        profile.push(bind_child_component(&current, component)?);
        current = current
            .retained_child(os_name(component))
            .map_err(|source| CheckedFsError::io("open managed parent enclosure", source))?;
    }
    retain_managed_child(&current, &profile, &components[depth - 1], reservation)
}

//! Owner-private physical managed-component edges for one managed parent.
//!
//! R2-D Phase 2 Step 2.3 (`GwzM5-8R2D-Plan.md` §4): the physical half of the
//! four managed `RawNamespaceBackend` operations. Edge E15 (managed component
//! install, staged directory → final) runs through the sealed source-associated
//! publication primitive with the §4.4 Class 1 **managed source-interior** arm,
//! and edge E16 (ownership-marker retirement) runs through the same primitive
//! with no arm at all — §4.3's E16 annotation makes the destination arm
//! conditional on "the marker retir[ing] as a directory", and it does not: the
//! marker is the frozen regular-file leaf `managed_marker_name()`
//! (`protocol/managed_bootstrap_record.rs:508`), which
//! `namespace/operations.rs:361-362` already pins to
//! `NamespaceObjectKind::RegularFile`. No raw rename is named here
//! (`GwzM5-8R2CCatalogBootstrapAmendment.md` §8.13).
//!
//! Provenance follows the audit's own pattern (`GwzM5-8R2C2PublicationAudit.md`
//! :39-44): the managed parent is reached through exactly one identity-proved
//! no-follow hop from a directory the caller already retained, the installed
//! component is reached through one further no-follow hop from that retained
//! parent, and no ambient path `Dir` ever reaches the primitive. The retirement
//! edge's destination is the retained action directory itself, supplied by
//! `namespace_mutation::RetainedActionNamespaceV1` rather than reopened here.
//!
//! Every name here is frozen managed vocabulary — `managed_staging_name`,
//! `managed_marker_name`, the component's `final_name`, and the schedule's
//! `ActionSlotV1::RetiredBootstrapMarker` row. This file mints no name and no
//! record.
//!
//! R2-D Phase 3 Step 3.1 adds the two halves the provider needs and nothing
//! else: the bounded read-only managed-prefix walk
//! ([`observe_managed_prefix`], [`retain_managed_prefix`]) that is the only
//! route from the permit-retained root to a `Dir` a managed parent can be
//! retained under, and the P2 staged-component writer
//! ([`RetainedManagedParentV1::stage_component`]). Neither adds a namespace
//! edge, so the `CATALOG_PUBLICATION_CALL_COUNTS` companion and the
//! `capability_permit.rs` caller inventory are unchanged by that step.

use std::ffi::{OsStr, OsString};
use std::io::{Read, Seek, SeekFrom, Write};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};

use super::directory_mutation::sync_directory_edge;
use super::interior;
use super::namespace_mutation::RetainedActionNamespaceV1;
use super::publication::{
    DestinationRecheckV1, DirectoryInteriorExpectationV1, DirectoryInteriorRecheckV1,
    PublicationSourceV1, publish_verified_no_replace,
};
use super::retained::encode_identity;
use crate::checked_artifact::capability::{
    AsciiComponent, CanonicalComponent, CanonicalPathIdentityV1, CheckedFsError,
    DurableIdentityProvider, DurableObjectIdentityV1, PathComponentMode, PathEquivalenceProvider,
    PlatformCapability,
};
#[cfg(test)]
use crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1;
use crate::checked_artifact::protocol::{
    OwnershipMarkerV1, ProtocolRecordKindV1, RecordDigestV1, managed_marker_name,
};

/// One exact managed namespace object, observed and closed.
///
/// The observation handle is dropped before it is returned for the reason
/// `namespace_mutation.rs:97-105` records: on Windows the sealed primitive
/// reopens the source with `DELETE` access, which a surviving caller handle
/// opened without `FILE_SHARE_DELETE` would refuse.
pub(in crate::checked_artifact) struct ObservedManagedObjectV1 {
    identity: DurableObjectIdentityV1,
    encoded_identity: Vec<u8>,
    bytes: Vec<u8>,
}

impl ObservedManagedObjectV1 {
    pub(in crate::checked_artifact) const fn identity(&self) -> &DurableObjectIdentityV1 {
        &self.identity
    }
}

/// The durable facts one managed component installation observed.
pub(in crate::checked_artifact) struct ManagedInstalledFactsV1 {
    pub(in crate::checked_artifact) marker_object_identity: DurableObjectIdentityV1,
    pub(in crate::checked_artifact) installed_identity: DurableObjectIdentityV1,
    pub(in crate::checked_artifact) installed_mode: PathComponentMode,
    pub(in crate::checked_artifact) installed_path: CanonicalPathIdentityV1,
}

/// The durable facts one ownership-marker retirement observed.
pub(in crate::checked_artifact) struct ManagedRetiredFactsV1 {
    pub(in crate::checked_artifact) marker_bytes: Vec<u8>,
    pub(in crate::checked_artifact) retired_marker_identity: DurableObjectIdentityV1,
    pub(in crate::checked_artifact) installed_parent_identity: DurableObjectIdentityV1,
    pub(in crate::checked_artifact) installed_parent_mode: PathComponentMode,
    pub(in crate::checked_artifact) installed_parent_path: CanonicalPathIdentityV1,
}

/// The one retained managed parent a namespace backend owns for the life of a
/// managed component: opened once through an identity-proved no-follow hop from
/// a directory the caller already retained, and held across every managed
/// observation, publication and retirement.
pub(in crate::checked_artifact) struct RetainedManagedParentV1 {
    parent: Dir,
    leaf: OsString,
    handle: Dir,
    identity: DurableObjectIdentityV1,
    path_profile: CanonicalPathIdentityV1,
    parent_mode: PathComponentMode,
    reservation: RecordDigestV1,
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
fn retain_managed_child(
    enclosing: &Dir,
    prefix: &[CanonicalComponent],
    leaf: &AsciiComponent,
    reservation: RecordDigestV1,
) -> Result<RetainedManagedParentV1, CheckedFsError> {
    let leaf_name = os_name(leaf);
    let handle = enclosing
        .open_dir_nofollow(&leaf_name)
        .map_err(|source| CheckedFsError::io("open managed parent", source))?;
    let fact = super::HostPlatform.dir_identity(&handle)?;
    let mut components = prefix.to_vec();
    components.push(bind_child_component(enclosing, leaf)?);
    let path_profile = CanonicalPathIdentityV1::new(components)?;
    let parent = enclosing
        .try_clone()
        .map_err(|source| CheckedFsError::io("retain managed parent enclosure", source))?;
    // The mode that governs the *installed* leaf is the managed parent's own,
    // not its enclosure's: the installed component is a child of `handle`.
    let installed_mode = super::HostPlatform.parent_mode(&handle)?;
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
fn bind_child_component(
    enclosing: &Dir,
    leaf: &AsciiComponent,
) -> Result<CanonicalComponent, CheckedFsError> {
    let enclosing_fact = super::HostPlatform.dir_identity(enclosing)?;
    CanonicalComponent::try_bound(
        leaf.clone(),
        super::HostPlatform.parent_mode(enclosing)?,
        enclosing_fact.durable().clone(),
        enclosing_fact.invocation().clone(),
        super::HostPlatform.rename_domain(enclosing)?,
    )
}

/// The durable facts one managed-path prefix depth observed. Nothing here is a
/// handle or a path string: it is the same typed triple the plan row and the
/// resident intent already carry (`bootstrap/managed/plan.rs`,
/// `protocol/managed_bootstrap_record.rs`), so a consumer outside this owner
/// still receives only facts.
pub(in crate::checked_artifact) struct ManagedPrefixDepthV1 {
    identity: DurableObjectIdentityV1,
    mode: PathComponentMode,
    path: CanonicalPathIdentityV1,
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
    depths: Vec<ManagedPrefixDepthV1>,
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
    root: &super::RetainedPlatformRoot,
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
        match current.symlink_metadata(&name) {
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => break,
            Err(source) => return Err(CheckedFsError::io("observe managed parent prefix", source)),
            Ok(metadata) if !metadata.is_dir() || metadata.is_symlink() => {
                return Err(managed_error(
                    "managed parent prefix component is not a canonical directory",
                ));
            }
            Ok(_) => {}
        }
        profile.push(bind_child_component(&current, component)?);
        let child = current
            .open_dir_nofollow(&name)
            .map_err(|source| CheckedFsError::io("open managed parent prefix", source))?;
        let fact = super::HostPlatform.dir_identity(&child)?;
        depths.push(ManagedPrefixDepthV1 {
            identity: fact.durable().clone(),
            mode: super::HostPlatform.parent_mode(&child)?,
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
    root: &super::RetainedPlatformRoot,
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
            .open_dir_nofollow(os_name(component))
            .map_err(|source| CheckedFsError::io("open managed parent enclosure", source))?;
    }
    retain_managed_child(&current, &profile, &components[depth - 1], reservation)
}

fn require_bounded_prefix(components: &[AsciiComponent]) -> Result<(), CheckedFsError> {
    if components.is_empty()
        || components.len() > crate::checked_artifact::protocol::MAX_MANAGED_PARENT_COMPONENTS
    {
        return Err(managed_error(
            "managed parent path is outside the frozen component bound",
        ));
    }
    Ok(())
}

fn clone_root(root: &super::RetainedPlatformRoot) -> Result<Dir, CheckedFsError> {
    root.root()
        .handle()
        .try_clone()
        .map_err(|source| CheckedFsError::io("retain managed parent root", source))
}

fn prefix_allocation_failure() -> CheckedFsError {
    CheckedFsError::unsupported(
        PlatformCapability::ManagedParentBootstrap,
        "managed parent prefix allocation failed",
    )
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
    root: &super::RetainedPlatformRoot,
) -> [u8; 32] {
    use sha2::{Digest, Sha256};

    let mut digest = Sha256::new();
    digest.update(b"gwz-managed-parent-provider-instance-v1\0");
    digest.update(root.root().identity().durable().encode_canonical());
    digest.finalize().into()
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
    let directory = Dir::open_ambient_dir(enclosing, cap_std::ambient_authority())
        .map_err(|source| CheckedFsError::io("open managed enclosure", source))?;
    retain_managed_parent(&directory, leaf, reservation)
}

impl RetainedManagedParentV1 {
    pub(in crate::checked_artifact) const fn identity(&self) -> &DurableObjectIdentityV1 {
        &self.identity
    }

    pub(in crate::checked_artifact) const fn path_profile(&self) -> &CanonicalPathIdentityV1 {
        &self.path_profile
    }

    pub(in crate::checked_artifact) const fn parent_mode(&self) -> PathComponentMode {
        self.parent_mode
    }

    pub(in crate::checked_artifact) const fn reservation(&self) -> RecordDigestV1 {
        self.reservation
    }

    /// Re-proves that the retained handle is still the named managed parent, in
    /// the shape `namespace_mutation.rs:191-219` uses for the action directory.
    pub(in crate::checked_artifact) fn revalidate(
        &self,
        expected_reservation: RecordDigestV1,
    ) -> Result<(), CheckedFsError> {
        if expected_reservation != self.reservation {
            return Err(managed_error(
                "managed parent binding does not match the admitted reservation",
            ));
        }
        let named = self
            .parent
            .open_dir_nofollow(&self.leaf)
            .map_err(|source| CheckedFsError::io("reopen named managed parent", source))?;
        if super::HostPlatform.dir_identity(&named)?.durable() != &self.identity
            || super::HostPlatform.dir_identity(&self.handle)?.durable() != &self.identity
        {
            return Err(managed_error(
                "retained managed parent is no longer the named managed parent",
            ));
        }
        #[cfg(test)]
        crate::checked_artifact::fault_v1::hit(
            CheckedArtifactFaultKeyV1::ManagedBootstrapParentRevalidate,
        );
        Ok(())
    }

    /// Whether a managed row is currently resident. Read-only, and the only
    /// managed question this owner answers without an edge, so a restart can
    /// tell which half of the component sequence it already reached.
    pub(in crate::checked_artifact) fn row_is_resident(&self, leaf: &AsciiComponent) -> bool {
        self.handle.symlink_metadata(os_name(leaf)).is_ok()
    }

    /// R2-D Phase 3 Step 3.1 — the writer half of edge E15's source: the staged
    /// component directory holding exactly its ownership marker.
    ///
    /// Primitive family P2 only (write-through plus flush); no namespace edge is
    /// crossed here, which is why the publication inventory is unchanged. The
    /// marker is written and flushed, then the staged directory and the managed
    /// parent are flushed in that order.
    ///
    /// **Write-or-rewrite scratch, and why it must be.** The staging name is
    /// this admitted action's own deterministic scratch row
    /// (`managed_staging_name(action, ordinal)`), and the marker is derived
    /// deterministically from this action's intent — so every window this writer
    /// can leave behind (directory created, marker absent; marker created, bytes
    /// short) is one *this* drive owns and must converge on, not one it may wedge
    /// on. It therefore follows the catalog owner's own scratch doctrine
    /// (`directory_mutation.rs` `prepare_or_rewrite_staging`): an exact interior
    /// settles with no edge, a non-exact one has its marker written or
    /// rewritten, and the interior is then re-proved. What is *not* adopted is a
    /// staging row carrying anything else — an extra child survives the rewrite,
    /// the re-proof fails, and the sequence is refused. That is the same content
    /// this owner would refuse to publish (§4.4 Class 1), refused earlier.
    ///
    /// The five `managed_bootstrap.*` writer keys these boundaries announce —
    /// `staging_directory_create`, the three `ownership_marker_*`, and
    /// `staging_directory_flush` — were converted as *edges* by Step 3.1 and
    /// activated with their matrix rows by Step 3.2, the step the plan assigns
    /// `managed_bootstrap.*` activation to (freeze §3.5's deferral record and its
    /// Step-3.2 annotation). Their rows are in
    /// `bootstrap/managed/tests_writer_matrix.rs`.
    pub(in crate::checked_artifact) fn stage_component(
        &self,
        staging_leaf: &AsciiComponent,
        marker: &OwnershipMarkerV1,
    ) -> Result<(), CheckedFsError> {
        let name = os_name(staging_leaf);
        let created = match self.handle.symlink_metadata(&name) {
            Ok(metadata) if !metadata.is_dir() || metadata.is_symlink() => {
                return Err(managed_error(
                    "resident managed staging row is not a canonical directory",
                ));
            }
            Ok(_) => false,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                self.handle.create_dir(&name).map_err(|source| {
                    CheckedFsError::io("create managed staging no-replace", source)
                })?;
                #[cfg(test)]
                crate::checked_artifact::fault_v1::hit(
                    CheckedArtifactFaultKeyV1::ManagedBootstrapStagingDirectoryCreate,
                );
                true
            }
            Err(source) => return Err(CheckedFsError::io("observe managed staging row", source)),
        };
        let staged = crate::checked_artifact::platform::open_dir_share_delete(&self.handle, &name)
            .map_err(|source| CheckedFsError::io("open managed staging no-follow", source))?;
        if !interior::observe_managed_component_interior(&staged, marker)?.is_exact() {
            write_or_rewrite_marker(&staged, marker)?;
            if !interior::observe_managed_component_interior(&staged, marker)?.is_exact() {
                return Err(managed_error(
                    "managed staging interior is not the exact ownership marker",
                ));
            }
        }
        if created {
            sync_directory_edge(&self.handle, "flush managed staging creation")?;
            // The second of this key's two boundaries. `staging_directory_flush`
            // names the state "a staging directory flush is durable", and this
            // writer performs two of them — the staged interior's, inside
            // `write_or_rewrite_marker`, and the managed parent's here, which
            // only a creating drive owes. Both are that state, so both announce
            // it rather than minting a second key (§3.5, §6).
            #[cfg(test)]
            crate::checked_artifact::fault_v1::hit(
                CheckedArtifactFaultKeyV1::ManagedBootstrapStagingDirectoryFlush,
            );
        }
        Ok(())
    }

    /// Retains the staged component directory as an exact source: no-follow
    /// open, durable identity, and the §4.4 Class 1 interior expectation proved
    /// once at retention so a caller cannot retain a directory this owner would
    /// refuse to publish.
    pub(in crate::checked_artifact) fn retain_staging_source(
        &self,
        staging_leaf: &AsciiComponent,
        expected_marker: &OwnershipMarkerV1,
    ) -> Result<ObservedManagedObjectV1, CheckedFsError> {
        let name = os_name(staging_leaf);
        let metadata = self
            .handle
            .symlink_metadata(&name)
            .map_err(|source| CheckedFsError::io("observe staged component", source))?;
        if !metadata.is_dir() || metadata.is_symlink() {
            return Err(managed_error(
                "staged managed component is not a canonical directory",
            ));
        }
        // Deliberate conservatism, not a required interlock. This handle is a
        // local dropped before `retain_staging_source` returns — only the
        // identity and the exactness verdict survive — so it is never live at
        // the same time as the DELETE-access reopen the sealed primitive makes
        // at the edge, and no os-error-32 collision is possible on this path.
        // (The collision that genuinely needs the recipe is *inside* the
        // primitive, `publication.rs` source reopen vs. its rename handle, and
        // is documented at `interior.rs` `observe_slot`.) The recipe is used
        // anyway so every directory open in the managed owner shares one
        // sharing doctrine; both arms are no-follow, and on macOS/Linux the
        // helper is byte-identically `open_dir_nofollow`.
        let directory =
            crate::checked_artifact::platform::open_dir_share_delete(&self.handle, &name)
                .map_err(|source| CheckedFsError::io("open staged component no-follow", source))?;
        let fact = super::HostPlatform.dir_identity(&directory)?;
        if !interior::observe_managed_component_interior(&directory, expected_marker)?.is_exact() {
            return Err(managed_error(
                "staged managed component interior is not the exact ownership marker",
            ));
        }
        Ok(ObservedManagedObjectV1 {
            identity: fact.durable().clone(),
            encoded_identity: encode_identity(&fact),
            bytes: Vec::new(),
        })
    }

    /// Retains the installed component's ownership marker as the retirement
    /// source: the frozen marker leaf inside the installed component, read
    /// bounded against the frozen `Marker` record bound.
    pub(in crate::checked_artifact) fn retain_marker_source(
        &self,
        final_leaf: &AsciiComponent,
    ) -> Result<ObservedManagedObjectV1, CheckedFsError> {
        let installed = self.open_installed(final_leaf)?;
        observe_marker(&installed, "retain managed ownership marker")
    }

    /// Edge E15 — the staged component directory published onto its final name
    /// through the sealed primitive, then reopened and reobserved so the
    /// observation the backend returns is durable truth rather than the caller's
    /// expectation.
    pub(in crate::checked_artifact) fn install_component(
        &self,
        staging_leaf: &AsciiComponent,
        final_leaf: &AsciiComponent,
        source: &ObservedManagedObjectV1,
        expected_marker: &OwnershipMarkerV1,
    ) -> Result<ManagedInstalledFactsV1, CheckedFsError> {
        let label = "install managed component";
        let staging_name = os_name(staging_leaf);
        let final_name = os_name(final_leaf);
        self.require_absent(&final_name, label)?;

        publish_verified_no_replace(
            &self.handle,
            &staging_name,
            &self.handle,
            &final_name,
            PublicationSourceV1::directory(
                &source.encoded_identity,
                DirectoryInteriorRecheckV1 {
                    durable_identity: &source.identity,
                    expected: DirectoryInteriorExpectationV1::ManagedStaging(expected_marker),
                },
            ),
            DestinationRecheckV1::None,
            label,
        )?;
        #[cfg(test)]
        crate::checked_artifact::fault_v1::hit(
            CheckedArtifactFaultKeyV1::ManagedBootstrapStagingDirectoryPublish,
        );
        sync_directory_edge(&self.handle, "flush managed component install")?;
        self.observe_installed(final_leaf, expected_marker)
    }

    /// The restart *entry* of the install observation.
    ///
    /// It exists as its own entry point so the boundary "a fresh process chose
    /// the restart path" is announced from this owner rather than from the
    /// `namespace` owner, which is what keeps every `managed_bootstrap.*`
    /// injection site inside the provider — the same rule
    /// `interface_tests/fault_expected_keys.rs` records for `namespace.*`.
    pub(in crate::checked_artifact) fn observe_installed_on_restart(
        &self,
        final_leaf: &AsciiComponent,
        expected_marker: &OwnershipMarkerV1,
    ) -> Result<ManagedInstalledFactsV1, CheckedFsError> {
        #[cfg(test)]
        crate::checked_artifact::fault_v1::hit(
            CheckedArtifactFaultKeyV1::ManagedBootstrapComponentReobserve,
        );
        self.observe_installed(final_leaf, expected_marker)
    }

    /// The restart half of edge E15 (ConsumerCheckpoint §8 :228-231): the same
    /// observation, reached without an edge, so a fresh process that finds the
    /// component already installed reproduces the identical evidence.
    ///
    /// **Phase-scoped, deliberately.** This holds only in the window between the
    /// install and edge E16: once the marker has retired, the component's
    /// interior is empty and the exactness check below refuses — for a component
    /// that *is* installed. That is correct rather than a gap, because the
    /// intent's phase is what a restart consumes (ConsumerCheckpoint §9
    /// :261-262), and in `RetireMarkers` the phase selects
    /// `observe_retired_managed_marker`, which deliberately omits the interior
    /// check. Do not widen this observation to cover the post-retirement window;
    /// widening it would erase exactly the evidence that distinguishes the two
    /// phases.
    pub(in crate::checked_artifact) fn observe_installed(
        &self,
        final_leaf: &AsciiComponent,
        expected_marker: &OwnershipMarkerV1,
    ) -> Result<ManagedInstalledFactsV1, CheckedFsError> {
        let installed = self.open_installed(final_leaf)?;
        let installed_fact = super::HostPlatform.dir_identity(&installed)?;
        if !interior::observe_managed_component_interior(&installed, expected_marker)?.is_exact() {
            return Err(managed_error(
                "installed managed component interior is not the exact ownership marker",
            ));
        }
        let marker = observe_marker(&installed, "observe installed ownership marker")?;
        #[cfg(test)]
        crate::checked_artifact::fault_v1::hit(
            CheckedArtifactFaultKeyV1::ManagedBootstrapFinalDirectoryReobserve,
        );
        Ok(ManagedInstalledFactsV1 {
            marker_object_identity: marker.identity,
            installed_identity: installed_fact.durable().clone(),
            installed_mode: self.parent_mode,
            installed_path: self.installed_path(final_leaf)?,
        })
    }

    /// Edge E16 — the ownership marker retired out of the installed component
    /// and into the action directory's scheduled retirement row. The source is a
    /// regular file, so the primitive verifies it by identity and bytes and no
    /// recheck arm is involved on either side (§4.3 row E16's annotation).
    pub(in crate::checked_artifact) fn retire_marker(
        &self,
        action: &RetainedActionNamespaceV1,
        final_leaf: &AsciiComponent,
        destination_leaf: &AsciiComponent,
        source: &ObservedManagedObjectV1,
    ) -> Result<ManagedRetiredFactsV1, CheckedFsError> {
        let label = "retire managed ownership marker";
        let installed = self.open_installed(final_leaf)?;
        let destination_name = os_name(destination_leaf);
        let marker_name = os_name(&managed_marker_name());
        require_absent_in(action.handle(), &destination_name, label)?;

        let fresh = observe_marker(&installed, label)?;
        if fresh.identity != source.identity
            || fresh.encoded_identity != source.encoded_identity
            || fresh.bytes != source.bytes
        {
            return Err(CheckedFsError::ambiguous(
                label,
                "retained ownership marker changed before the edge",
            ));
        }

        publish_verified_no_replace(
            &installed,
            &marker_name,
            action.handle(),
            &destination_name,
            PublicationSourceV1::regular_file(&source.encoded_identity, &source.bytes),
            DestinationRecheckV1::None,
            label,
        )?;
        #[cfg(test)]
        crate::checked_artifact::fault_v1::hit(
            CheckedArtifactFaultKeyV1::ManagedBootstrapMarkerRetire,
        );
        // **E16 cross-parent atomicity record** (freeze §4.3's E16 annotation;
        // the cross-parent twin of the E4 record at §4.3 :637-701). This is the
        // lane's first *cross-directory* durable edge — every prior edge in this
        // owner family passes one retained handle as both source and destination
        // (`namespace_mutation.rs` `execute_edge`) — so what the recovery path
        // rests on is written down here rather than assumed.
        //
        // The two flushes below order the *observation*, not the atomicity. The
        // commit point is the rename above, and on the closed support table
        // (journaled NTFS/ext4/APFS) a rename — cross-directory included, so
        // long as both parents are one filesystem — is a single metadata
        // transaction that crash recovery replays or discards whole. The three
        // reachable post-crash states are therefore:
        //
        // * rename discarded → the marker is still inside the component and the
        //   retirement row is absent; the next drive re-enters this edge and
        //   re-derives the same scheduled row (matrix rows `component_reobserve`
        //   and `final_directory_reobserve` settle from exactly this state);
        // * rename durable, neither parent flushed → the retirement row is
        //   resident and the drive short-circuits on it (matrix row
        //   `marker_retire`, whose boundary sits precisely here);
        // * rename durable, both parents flushed → same, one boundary later
        //   (matrix rows `marker_retired_reobserve`, `final_identity_reobserve`).
        //
        // Nothing durable changes *between* the two flushes, so that window is
        // bracketed by the `marker_retire` and `marker_retired_reobserve` rows
        // with no namespace transition in between; it gets no key of its own,
        // and minting one would move the frozen 165-key census (§3.5, §6).
        //
        // The state that would wedge the restart — marker absent from the
        // component *and* retirement row absent from the action directory — is
        // not producible on that table, and the two ways it might seem to arise
        // both fail closed instead:
        //
        // * different filesystems: `renameat` returns `EXDEV` before touching
        //   either parent, surfacing as a typed refusal from the sealed
        //   primitive with nothing durable changed, so every retry re-enters the
        //   same pre-edge state idempotently — a deterministic typed refusal,
        //   not a wedge;
        // * foreign removal of the marker: outside the accepted same-user
        //   namespace boundary (§4.4's drift-rejection paragraph).
        //
        // Were that state ever produced anyway — i.e. off the supported table —
        // `observe_installed`'s interior check refuses it permanently and by
        // design: a component whose ownership marker cannot be accounted for
        // must not be adopted. The refusal is typed, never silent.
        sync_directory_edge(&installed, "flush managed marker retirement")?;
        sync_directory_edge(action.handle(), "flush managed marker retirement row")?;
        self.observe_retired_marker(action, final_leaf, destination_leaf)
    }

    /// The restart half of edge E16 (ConsumerCheckpoint §8 :228-231): the retired
    /// row is reobserved in the action directory and the installed component is
    /// reproved, so a fresh process reproduces the identical evidence.
    pub(in crate::checked_artifact) fn observe_retired_marker(
        &self,
        action: &RetainedActionNamespaceV1,
        final_leaf: &AsciiComponent,
        destination_leaf: &AsciiComponent,
    ) -> Result<ManagedRetiredFactsV1, CheckedFsError> {
        let retired = observe_regular_file(
            action.handle(),
            &os_name(destination_leaf),
            "observe retired ownership marker",
            ProtocolRecordKindV1::Marker,
        )?;
        #[cfg(test)]
        crate::checked_artifact::fault_v1::hit(
            CheckedArtifactFaultKeyV1::ManagedBootstrapMarkerRetiredReobserve,
        );
        let installed = self.open_installed(final_leaf)?;
        let installed_fact = super::HostPlatform.dir_identity(&installed)?;
        #[cfg(test)]
        crate::checked_artifact::fault_v1::hit(
            CheckedArtifactFaultKeyV1::ManagedBootstrapFinalIdentityReobserve,
        );
        Ok(ManagedRetiredFactsV1 {
            marker_bytes: retired.bytes,
            retired_marker_identity: retired.identity,
            installed_parent_identity: installed_fact.durable().clone(),
            installed_parent_mode: self.parent_mode,
            installed_parent_path: self.installed_path(final_leaf)?,
        })
    }

    /// The installed component's own durable identity and canonical path, for
    /// the retained-capability the marker-retirement source is issued against.
    pub(in crate::checked_artifact) fn installed_facts(
        &self,
        final_leaf: &AsciiComponent,
    ) -> Result<(DurableObjectIdentityV1, CanonicalPathIdentityV1), CheckedFsError> {
        let installed = self.open_installed(final_leaf)?;
        let fact = super::HostPlatform.dir_identity(&installed)?;
        Ok((fact.durable().clone(), self.installed_path(final_leaf)?))
    }

    /// One identity-proved no-follow hop from the retained managed parent to the
    /// installed component. Every managed observation goes through here, so no
    /// other route to the component exists in this owner.
    fn open_installed(&self, final_leaf: &AsciiComponent) -> Result<Dir, CheckedFsError> {
        let name = os_name(final_leaf);
        let metadata = self
            .handle
            .symlink_metadata(&name)
            .map_err(|source| CheckedFsError::io("observe installed component", source))?;
        if !metadata.is_dir() || metadata.is_symlink() {
            return Err(managed_error(
                "installed managed component is not a canonical directory",
            ));
        }
        // Same deliberate conservatism as the staged open above. Note what
        // `FILE_SHARE_DELETE` does and does not do here: this handle is the
        // *parent* of the renamed marker, and a directory handle never
        // constrains DELETE access to its children, so the sharing mode is not
        // what makes edge E16 legal. It is a relaxation — it permits others to
        // rename or delete this component while the handle is held — not a
        // protection. Phase 3.1 inherits this doctrine: apply it for uniformity
        // of the owner's opens, never as an interlock argument.
        let directory =
            crate::checked_artifact::platform::open_dir_share_delete(&self.handle, &name).map_err(
                |source| CheckedFsError::io("open installed component no-follow", source),
            )?;
        #[cfg(test)]
        crate::checked_artifact::fault_v1::hit(
            CheckedArtifactFaultKeyV1::ManagedBootstrapFinalDirectoryReopen,
        );
        Ok(directory)
    }

    /// The installed component's canonical path: the retained parent's own
    /// profile extended by exactly one bound component. `ManagedInstallRequestV1`
    /// re-checks that shape (`namespace/managed.rs:105-118`), so the two never
    /// disagree without a typed refusal.
    fn installed_path(
        &self,
        final_leaf: &AsciiComponent,
    ) -> Result<CanonicalPathIdentityV1, CheckedFsError> {
        let fact = super::HostPlatform.dir_identity(&self.handle)?;
        let mut components = self.path_profile.components().to_vec();
        components.push(CanonicalComponent::try_bound(
            final_leaf.clone(),
            self.parent_mode,
            fact.durable().clone(),
            fact.invocation().clone(),
            super::HostPlatform.rename_domain(&self.handle)?,
        )?);
        CanonicalPathIdentityV1::new(components)
    }

    fn require_absent(&self, name: &OsStr, label: &'static str) -> Result<(), CheckedFsError> {
        require_absent_in(&self.handle, name, label)
    }
}

/// The deterministic destination row must be free before the edge, stated as a
/// pre-edge expectation so a resident row is a typed refusal rather than an
/// `EEXIST` (`namespace_mutation.rs:317-331`).
fn require_absent_in(
    directory: &Dir,
    name: &OsStr,
    label: &'static str,
) -> Result<(), CheckedFsError> {
    match directory.symlink_metadata(name) {
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(CheckedFsError::io("observe managed destination", source)),
        Ok(_) => Err(CheckedFsError::ambiguous(
            label,
            "managed destination row is already occupied",
        )),
    }
}

/// The staged component's ownership marker, written once or rewritten in place.
///
/// Rewriting is the *scratch* case only: the marker leaf lives inside this
/// action's own deterministic staging row, which carries no authority until the
/// sealed primitive publishes it, and the bytes are re-derived from the same
/// intent every drive derives. The file is opened `create_new` when absent and
/// existing-only when present, so this never creates a marker at an unexpected
/// name and never adopts a symlink or a non-file in its place.
fn write_or_rewrite_marker(staged: &Dir, marker: &OwnershipMarkerV1) -> Result<(), CheckedFsError> {
    let name = os_name(&managed_marker_name());
    let create_new = match staged.symlink_metadata(&name) {
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => true,
        Err(source) => {
            return Err(CheckedFsError::io(
                "observe managed ownership marker",
                source,
            ));
        }
        Ok(metadata) if metadata.is_file() && !metadata.is_symlink() => false,
        Ok(_) => {
            return Err(managed_error(
                "staged ownership marker is not a canonical regular file",
            ));
        }
    };
    let options = super::directory_mutation::durable_write_options(create_new);
    let mut file = staged
        .open_with(&name, &options)
        .map_err(|source| CheckedFsError::io("open managed ownership marker", source))?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(
        CheckedArtifactFaultKeyV1::ManagedBootstrapOwnershipMarkerCreate,
    );
    if !create_new {
        file.set_len(0)
            .map_err(|source| CheckedFsError::io("truncate managed ownership marker", source))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|source| CheckedFsError::io("rewind managed ownership marker", source))?;
    }
    file.write_all(&marker.encode_canonical())
        .map_err(|source| CheckedFsError::io("write managed ownership marker", source))?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(
        CheckedArtifactFaultKeyV1::ManagedBootstrapOwnershipMarkerWrite,
    );
    file.sync_all()
        .map_err(|source| CheckedFsError::io("flush managed ownership marker", source))?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(
        CheckedArtifactFaultKeyV1::ManagedBootstrapOwnershipMarkerFlush,
    );
    drop(file);
    sync_directory_edge(staged, "flush managed staging interior")?;
    // The first of `staging_directory_flush`'s two boundaries; the second is the
    // managed parent's flush in `stage_component`, which only a creating drive
    // reaches. See the note there.
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(
        CheckedArtifactFaultKeyV1::ManagedBootstrapStagingDirectoryFlush,
    );
    Ok(())
}

/// Which generation edge of the managed intent record's durable lifecycle a call
/// belongs to.
///
/// R2-D Phase 3 Step 3.1b (`GwzM5-8R2D-Plan.md` §4 Step 3.1, "durable successor,
/// prior-generation retirement"; `GwzM5-8R2DInterfaceFreeze.md` §4.3 row E17).
/// It selects the stable `managed_bootstrap.*` boundaries and the diagnostic
/// label, exactly as `ActionNamespaceEdgeV1` selects the `namespace.*` ones for
/// the two edges that share one primitive call site.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::checked_artifact) enum ManagedIntentEdgeV1 {
    /// The row's first generation: the intent record that exists before any
    /// component is touched.
    Initial,
    /// Every later generation: the successor an evidence transition derives.
    Successor,
    /// The retirement of the generation a successor supersedes.
    PriorGeneration,
    /// The retirement of the last generation, which is the row's own completion
    /// record.
    FinalRetirement,
}

impl ManagedIntentEdgeV1 {
    const fn label(self) -> &'static str {
        match self {
            Self::Initial => "publish initial managed intent",
            Self::Successor => "publish managed intent successor",
            Self::PriorGeneration => "retire prior managed intent generation",
            Self::FinalRetirement => "retire final managed intent",
        }
    }

    const fn scratch_label(self) -> &'static str {
        match self {
            Self::Initial => "write initial managed intent scratch",
            Self::Successor => "write managed intent successor scratch",
            Self::PriorGeneration | Self::FinalRetirement => {
                "write managed intent retirement scratch"
            }
        }
    }

    /// The create / write / flush boundaries of this edge's scratch record.
    /// Only the two publishing edges have one; a retirement moves a record that
    /// is already durable.
    #[cfg(test)]
    const fn scratch_faults(self) -> Option<[CheckedArtifactFaultKeyV1; 3]> {
        match self {
            Self::Initial => Some([
                CheckedArtifactFaultKeyV1::ManagedBootstrapInitialIntentScratchCreate,
                CheckedArtifactFaultKeyV1::ManagedBootstrapInitialIntentScratchWrite,
                CheckedArtifactFaultKeyV1::ManagedBootstrapInitialIntentScratchFlush,
            ]),
            Self::Successor => Some([
                CheckedArtifactFaultKeyV1::ManagedBootstrapSuccessorScratchCreate,
                CheckedArtifactFaultKeyV1::ManagedBootstrapSuccessorScratchWrite,
                CheckedArtifactFaultKeyV1::ManagedBootstrapSuccessorScratchFlush,
            ]),
            Self::PriorGeneration | Self::FinalRetirement => None,
        }
    }

    /// The post-edge and post-observation boundaries this edge crosses.
    ///
    /// The first names the state "the namespace rename is durable and nothing
    /// has looked at it yet"; the second names "the published row has been read
    /// back and proved". The rename itself is the already-executed
    /// `namespace.publish_no_replace` / `namespace.retire_exact` boundary of
    /// Step 2.2, because this lifecycle deliberately routes through the
    /// role-validated backend rather than opening the sealed primitive again —
    /// so these two are the boundaries this family owns, in the same shape
    /// Step 2.3 used for `staging_directory_publish` and `marker_retire`.
    #[cfg(test)]
    const fn edge_faults(self) -> [CheckedArtifactFaultKeyV1; 2] {
        match self {
            Self::Initial => [
                CheckedArtifactFaultKeyV1::ManagedBootstrapInitialIntentPublish,
                CheckedArtifactFaultKeyV1::ManagedBootstrapInitialIntentReobserve,
            ],
            Self::Successor => [
                CheckedArtifactFaultKeyV1::ManagedBootstrapSuccessorPublish,
                CheckedArtifactFaultKeyV1::ManagedBootstrapSuccessorReobserve,
            ],
            Self::PriorGeneration => [
                CheckedArtifactFaultKeyV1::ManagedBootstrapPriorGenerationRetire,
                CheckedArtifactFaultKeyV1::ManagedBootstrapPriorGenerationReobserve,
            ],
            Self::FinalRetirement => [
                CheckedArtifactFaultKeyV1::ManagedBootstrapFinalIntentRetire,
                CheckedArtifactFaultKeyV1::ManagedBootstrapFinalIntentRetiredReobserve,
            ],
        }
    }
}

/// R2-D Phase 3 Step 3.1b — the managed intent record written into the action
/// directory's scheduled `BootstrapIntentScratch` row.
///
/// Primitive family P2 only (write-through plus flush); the record reaches its
/// active row through the Step-2.2 backend's `publish_bootstrap_generation`,
/// which is why this step adds no publication call site.
///
/// **Write-or-rewrite, for the same reason `stage_component` is.** The scratch
/// slot is one deterministic base slot shared by every generation of every row
/// of this action, and the bytes are re-derived identically on every drive, so a
/// leftover scratch from an interrupted generation is this drive's own residue
/// and must be converged on rather than wedged against. The row carries no
/// authority until the no-replace publish moves it, and the post-write proof
/// below re-reads the named row before any of that happens.
pub(in crate::checked_artifact) fn write_managed_intent_scratch(
    action: &RetainedActionNamespaceV1,
    scratch_leaf: &AsciiComponent,
    bytes: &[u8],
    edge: ManagedIntentEdgeV1,
) -> Result<(), CheckedFsError> {
    let directory = action.handle();
    let name = os_name(scratch_leaf);
    let create_new = match directory.symlink_metadata(&name) {
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => true,
        Err(source) => return Err(CheckedFsError::io("observe managed intent scratch", source)),
        Ok(metadata) if metadata.is_file() && !metadata.is_symlink() => false,
        Ok(_) => {
            return Err(managed_error(
                "managed intent scratch row is not a canonical regular file",
            ));
        }
    };
    if bytes.len() > ProtocolRecordKindV1::BootstrapIntent.max_bytes() {
        return Err(managed_error(
            "managed intent record exceeds its frozen record bound",
        ));
    }
    let options = super::directory_mutation::durable_write_options(create_new);
    let mut file = directory
        .open_with(&name, &options)
        .map_err(|source| CheckedFsError::io("open managed intent scratch", source))?;
    #[cfg(test)]
    if let Some(faults) = edge.scratch_faults() {
        crate::checked_artifact::fault_v1::hit(faults[0]);
    }
    if !create_new {
        file.set_len(0)
            .map_err(|source| CheckedFsError::io("truncate managed intent scratch", source))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|source| CheckedFsError::io("rewind managed intent scratch", source))?;
    }
    file.write_all(bytes)
        .map_err(|source| CheckedFsError::io("write managed intent scratch", source))?;
    #[cfg(test)]
    if let Some(faults) = edge.scratch_faults() {
        crate::checked_artifact::fault_v1::hit(faults[1]);
    }
    file.sync_all()
        .map_err(|source| CheckedFsError::io("flush managed intent scratch", source))?;
    #[cfg(test)]
    if let Some(faults) = edge.scratch_faults() {
        crate::checked_artifact::fault_v1::hit(faults[2]);
    }
    drop(file);
    sync_directory_edge(directory, "flush managed intent scratch row")?;
    let observed = observe_regular_file(
        directory,
        &name,
        edge.scratch_label(),
        ProtocolRecordKindV1::BootstrapIntent,
    )?;
    if observed.bytes != bytes {
        return Err(managed_error(
            "managed intent scratch is not the record just written",
        ));
    }
    // Only the successor half of the frozen vocabulary carries a scratch
    // reobservation key; the initial half's post-write proof runs identically and
    // announces nothing, because minting `initial_intent_scratch_reobserve` would
    // move the frozen 165-key census (§3.5, §6).
    #[cfg(test)]
    if edge == ManagedIntentEdgeV1::Successor {
        crate::checked_artifact::fault_v1::hit(
            CheckedArtifactFaultKeyV1::ManagedBootstrapSuccessorScratchReobserve,
        );
    }
    Ok(())
}

/// R2-D Phase 3 Step 3.1b — the post-edge proof of one scheduled managed intent
/// row, and the two boundaries around it.
///
/// The caller has just moved the row through the Step-2.2 backend; this reads it
/// back bounded and returns its bytes, so the record a drive resumes from is
/// durable truth rather than the caller's own expectation.
pub(in crate::checked_artifact) fn observe_managed_intent_row(
    action: &RetainedActionNamespaceV1,
    leaf: &AsciiComponent,
    edge: ManagedIntentEdgeV1,
) -> Result<Vec<u8>, CheckedFsError> {
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(edge.edge_faults()[0]);
    let observed = read_managed_intent_row(action, leaf, edge.label())?;
    #[cfg(test)]
    crate::checked_artifact::fault_v1::hit(edge.edge_faults()[1]);
    Ok(observed)
}

/// R2-D Phase 3 Step 3.1b — a bounded read of one resident managed intent row.
///
/// This is the resume's own read. It announces no boundary: it crosses no
/// durable edge, and every row it reads already carries the keyed boundaries of
/// the edge that put it there.
pub(in crate::checked_artifact) fn read_managed_intent_row(
    action: &RetainedActionNamespaceV1,
    leaf: &AsciiComponent,
    label: &'static str,
) -> Result<Vec<u8>, CheckedFsError> {
    Ok(observe_regular_file(
        action.handle(),
        &os_name(leaf),
        label,
        ProtocolRecordKindV1::BootstrapIntent,
    )?
    .bytes)
}

fn observe_marker(
    installed: &Dir,
    label: &'static str,
) -> Result<ObservedManagedObjectV1, CheckedFsError> {
    observe_regular_file(
        installed,
        &os_name(&managed_marker_name()),
        label,
        ProtocolRecordKindV1::Marker,
    )
}

/// One exact regular-file managed object, read bounded against the frozen bound
/// of `kind` — never against the file's own length (ConsumerCheckpoint §8
/// :236-237). R2-D Step 3.1b adds the `kind` parameter because the managed
/// intent record is a `BootstrapIntent`, not a `Marker`.
fn observe_regular_file(
    directory: &Dir,
    name: &OsStr,
    label: &'static str,
    kind: ProtocolRecordKindV1,
) -> Result<ObservedManagedObjectV1, CheckedFsError> {
    let metadata = directory
        .symlink_metadata(name)
        .map_err(|source| CheckedFsError::io("observe managed object", source))?;
    if !metadata.is_file() || metadata.is_symlink() {
        return Err(CheckedFsError::ambiguous(
            label,
            "managed object is not a canonical regular file",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let mut file = directory
        .open_with(name, &options)
        .map_err(|source| CheckedFsError::io("open managed object no-follow", source))?;
    let fact = super::HostPlatform.file_identity(&file)?;
    let limit = kind.max_bytes();
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(limit + 1).map_err(|_| {
        CheckedFsError::unsupported(
            PlatformCapability::PrivateNamespaceCollisionScan,
            "managed object read allocation failed",
        )
    })?;
    file.seek(SeekFrom::Start(0))
        .map_err(|source| CheckedFsError::io("rewind managed object", source))?;
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|source| CheckedFsError::io("read managed object", source))?;
    if bytes.len() > limit {
        return Err(CheckedFsError::ambiguous(
            label,
            "managed object exceeds its frozen record bound",
        ));
    }
    Ok(ObservedManagedObjectV1 {
        identity: fact.durable().clone(),
        encoded_identity: encode_identity(&fact),
        bytes,
    })
}

fn managed_error(detail: &'static str) -> CheckedFsError {
    CheckedFsError::ambiguous("managed component", detail)
}

/// Managed names are frozen ASCII, so this conversion is total and needs no
/// platform-specific `OsStr` construction (`namespace_mutation.rs:387-393`).
fn os_name(leaf: &AsciiComponent) -> OsString {
    OsString::from(
        std::str::from_utf8(leaf.as_bytes()).expect("an ASCII component is always valid UTF-8"),
    )
}

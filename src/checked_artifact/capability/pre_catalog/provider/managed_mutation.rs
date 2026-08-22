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

use std::ffi::{OsStr, OsString};
use std::io::{Read, Seek, SeekFrom};

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
/// acquisition, and a one-component canonical path profile bound to the
/// enclosing directory's own observed identity. Plan §4 Step 3.1 supplies the
/// enclosing retained directory in production; this owner never opens a path.
pub(in crate::checked_artifact) fn retain_managed_parent(
    enclosing: &Dir,
    leaf: &str,
    reservation: RecordDigestV1,
) -> Result<RetainedManagedParentV1, CheckedFsError> {
    let leaf_name = OsString::from(leaf);
    let handle = enclosing
        .open_dir_nofollow(&leaf_name)
        .map_err(|source| CheckedFsError::io("open managed parent", source))?;
    let fact = super::HostPlatform.dir_identity(&handle)?;
    let enclosing_fact = super::HostPlatform.dir_identity(enclosing)?;
    let parent_mode = super::HostPlatform.parent_mode(enclosing)?;
    let path_profile = CanonicalPathIdentityV1::new(vec![CanonicalComponent::try_bound(
        AsciiComponent::parse(leaf.as_bytes())?,
        parent_mode,
        enclosing_fact.durable().clone(),
        enclosing_fact.invocation().clone(),
        super::HostPlatform.rename_domain(enclosing)?,
    )?])?;
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

/// The test-only enclosure door.
///
/// Production reaches a managed parent through the retained enclosing directory
/// plan §4 Step 3.1's `ManagedParentBootstrap::execute_bound` returns; that
/// provider is Phase 3 and does not exist yet, exactly as Step 2.2 landed its
/// backend before Step 3.3's consumer. This door lets Step 2.3's matrix drive
/// the two real edges against real durable state in the meantime, and it is the
/// only place in this owner that opens an ambient path.
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

fn observe_marker(
    installed: &Dir,
    label: &'static str,
) -> Result<ObservedManagedObjectV1, CheckedFsError> {
    observe_regular_file(installed, &os_name(&managed_marker_name()), label)
}

fn observe_regular_file(
    directory: &Dir,
    name: &OsStr,
    label: &'static str,
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
    let limit = ProtocolRecordKindV1::Marker.max_bytes();
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

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

#[cfg(test)]
use crate::filesystem::FileSystem;
use crate::filesystem::FsKind;
use std::ffi::OsStr;

use crate::filesystem::FsDirectory as Dir;

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
    DurableIdentityProvider, DurableObjectIdentityV1, PathComponentMode,
};
#[cfg(test)]
use crate::checked_artifact::fault_v1::CheckedArtifactFaultKeyV1;
use crate::checked_artifact::protocol::{
    OwnershipMarkerV1, ProtocolRecordKindV1, RecordDigestV1, managed_marker_name,
};

mod intent;
mod marker;
mod observe;
mod prefix;
mod retain;
mod support;

pub(in crate::checked_artifact) use intent::*;
pub(crate) use marker::*;
pub(crate) use observe::*;
pub(in crate::checked_artifact) use prefix::*;
pub(in crate::checked_artifact::capability::pre_catalog) use prefix::{
    observe_managed_prefix, retain_managed_prefix,
};
pub(crate) use retain::*;
pub(crate) use support::*;

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
            .retained_child(&self.leaf)
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
        self.handle.entry_metadata(os_name(leaf)).is_ok()
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
        let created = match self.handle.entry_metadata(&name) {
            Ok(metadata)
                if metadata.kind != FsKind::Directory || metadata.kind == FsKind::Symlink =>
            {
                return Err(managed_error(
                    "resident managed staging row is not a canonical directory",
                ));
            }
            Ok(_) => false,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                self.handle.create_child(&name).map_err(|source| {
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
        let staged = self
            .handle
            .retained_child(&name)
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
            .entry_metadata(&name)
            .map_err(|source| CheckedFsError::io("observe staged component", source))?;
        if metadata.kind != FsKind::Directory || metadata.kind == FsKind::Symlink {
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
        let directory = self
            .handle
            .retained_child(&name)
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
            .entry_metadata(&name)
            .map_err(|source| CheckedFsError::io("observe installed component", source))?;
        if metadata.kind != FsKind::Directory || metadata.kind == FsKind::Symlink {
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
        let directory = self
            .handle
            .retained_child(&name)
            .map_err(|source| CheckedFsError::io("open installed component no-follow", source))?;
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

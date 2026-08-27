//! Retained exact-completion capability returned by the first-catalog owner.

use std::ffi::OsStr;
use std::io::{Read, Seek, SeekFrom};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::OpenOptions;

use super::directory_mutation::{
    ObservedDirectoryV1, ObservedFileV1, observed_directory, open_observed_directory, row,
    verify_named_file,
};
use super::interior;
use super::{
    RawCatalogBytesV1, RawCatalogInteriorFactV1, RawCatalogRoleObservationV1, RetainedPlatformRoot,
};
use crate::checked_artifact::capability::{
    CheckedFsError, DurableIdentityProvider, ObjectIdentityFact, PlatformCapability,
};
use crate::checked_artifact::catalog::{CatalogRecognizedNameV1, CatalogRecordFactV1};
use crate::checked_artifact::catalog_names::CatalogPrivateNameV1;
use crate::checked_artifact::protocol::{
    ActionAdmissionEdgeV1, ActionAdmissionObservationV1, ActionCapacityReservationV1,
    AdmittedActionV1, CatalogBootstrapRecordV1, InfrastructureSlotV1, RootEntryNameV1,
};

type IdentityV1 =
    ObjectIdentityFact<crate::checked_artifact::capability::DurableObjectIdentityV1, Vec<u8>>;

struct RetainedCatalogFileV1 {
    handle: cap_std::fs::File,
    identity: IdentityV1,
}

struct RetainedCatalogDirectoryV1 {
    handle: cap_std::fs::Dir,
    identity: IdentityV1,
}

/// Exact final catalog plus every fixed authority-bearing interior handle.
pub(in crate::checked_artifact::capability::pre_catalog) struct RetainedCompletedCatalogV1 {
    final_directory: RetainedCatalogDirectoryV1,
    catalog_format: RetainedCatalogFileV1,
    catalog_anchor: RetainedCatalogFileV1,
    roaming_anchor: RetainedCatalogFileV1,
    retired_actions: RetainedCatalogDirectoryV1,
    retired_descriptor: RetainedCatalogFileV1,
    retired_bootstrap: RetainedCatalogFileV1,
    expected_bootstrap: CatalogBootstrapRecordV1,
}

pub(in crate::checked_artifact::capability::pre_catalog) fn retain_completed_catalog(
    retained: &RetainedPlatformRoot,
    raw_roles: &RawCatalogRoleObservationV1,
    expected: &CatalogBootstrapRecordV1,
) -> Result<RetainedCompletedCatalogV1, CheckedFsError> {
    let parent = retained.private_parent().ok_or_else(|| {
        CheckedFsError::ambiguous("completed catalog", "retained private parent is missing")
    })?;
    let observed =
        observed_directory(raw_roles, CatalogRecognizedNameV1::Final)?.ok_or_else(|| {
            CheckedFsError::ambiguous("completed catalog", "final catalog is missing")
        })?;
    if interior::completed_record(observed.durable_identity, observed.interior, expected).is_none()
        || !matches!(
            interior::retired_record(observed.interior),
            CatalogRecordFactV1::Exact(value) if value.as_ref() == expected
        )
    {
        return Err(CheckedFsError::ambiguous(
            "completed catalog",
            "final catalog does not have the exact retired layout",
        ));
    }
    let final_directory = open_observed_directory(
        parent.handle(),
        OsStr::new(private_name(CatalogPrivateNameV1::Final)),
        ObservedDirectoryV1 {
            identity: observed.identity,
            durable_identity: observed.durable_identity,
            interior: observed.interior,
        },
        "completed catalog",
    )?;
    let final_identity = super::HostPlatform.dir_identity(&final_directory)?;
    #[cfg(test)]
    super::directory_mutation::run_fault(
        super::directory_mutation::CatalogDirectoryMutationFaultV1::CompleteAfterFinalOpen,
    );
    let catalog_format = retain_file(
        &final_directory,
        observed.interior,
        InfrastructureSlotV1::CatalogFormat,
    )?;
    let catalog_anchor = retain_file(
        &final_directory,
        observed.interior,
        InfrastructureSlotV1::CatalogAnchorA,
    )?;
    let roaming_anchor = retain_file(
        &final_directory,
        observed.interior,
        InfrastructureSlotV1::RoamingAnchorHome,
    )?;
    let retired_actions = retain_directory(
        &final_directory,
        observed.interior,
        InfrastructureSlotV1::RetiredActions,
    )?;
    let retired_descriptor = retain_file(
        &final_directory,
        observed.interior,
        InfrastructureSlotV1::RetiredActionsDescriptor,
    )?;
    let retired_bootstrap = retain_file(
        &final_directory,
        observed.interior,
        InfrastructureSlotV1::CatalogBootstrapRetired,
    )?;
    let completed = RetainedCompletedCatalogV1 {
        final_directory: RetainedCatalogDirectoryV1 {
            handle: final_directory,
            identity: final_identity,
        },
        catalog_format,
        catalog_anchor,
        roaming_anchor,
        retired_actions,
        retired_descriptor,
        retired_bootstrap,
        expected_bootstrap: expected.clone(),
    };
    completed.revalidate(retained)?;
    Ok(completed)
}

impl RetainedCompletedCatalogV1 {
    /// R2-D Phase 1's read-only admission observation, driven through the
    /// retained final-catalog capability this owner already holds. The caller
    /// receives the typed observation only — never a handle
    /// (`GwzM5-8R2CCatalogBootstrapAmendment.md` §7 :576-577).
    pub(in crate::checked_artifact::capability::pre_catalog) fn observe_admission(
        &self,
        expected: &ActionCapacityReservationV1,
    ) -> Result<ActionAdmissionObservationV1, CheckedFsError> {
        super::admission_mutation::observe(&self.final_directory.handle, expected)
    }

    /// One bounded durable admission edge, executed inside this owner.
    pub(in crate::checked_artifact::capability::pre_catalog) fn execute_admission_edge(
        &self,
        edge: ActionAdmissionEdgeV1<'_>,
        expected: &ActionCapacityReservationV1,
    ) -> Result<(), CheckedFsError> {
        super::admission_mutation::execute(
            &self.final_directory.handle,
            self.final_directory.identity.durable(),
            &self.expected_bootstrap,
            edge,
            expected,
        )
    }

    /// R2-D Phase 2 Step 2.2. The one identity-proved no-follow hop from the
    /// retained completed catalog to the admitted action's deterministic final
    /// directory (`GwzM5-8R2C2PublicationAudit.md` :39-44). The caller receives
    /// the retained namespace capability only — never the catalog handle and
    /// never a path.
    pub(in crate::checked_artifact::capability::pre_catalog) fn retain_action_namespace(
        &self,
        admitted: &AdmittedActionV1,
    ) -> Result<super::RetainedActionNamespaceV1, CheckedFsError> {
        let action_leaf =
            RootEntryNameV1::ActiveAction(admitted.reservation().action_digest()).name();
        super::namespace_mutation::retain_action_namespace(
            &self.final_directory.handle,
            &action_leaf,
            admitted.directory_identity(),
            admitted.reservation().record_digest(),
        )
    }

    /// R2-E E3.1 — the **one** new owner-private forward DECISION T-C′ mints
    /// (`GwzM5-8R2E-SemanticsAmendment-E02b-DRAFT.md` §8): the retired-action
    /// root, handed to the sibling module that owns the catalog root's edges.
    ///
    /// **Where each half of the contract comes from.** The *shape* — a `&Dir`
    /// handed between siblings of `provider/` — is
    /// `RetainedActionNamespaceV1::handle`'s
    /// (`namespace_mutation.rs`, `pub(super)`, consumed by `managed_mutation`);
    /// this accessor is tighter than that precedent, carrying no visibility
    /// modifier at all, so it is private to this file and the handle reaches
    /// `admission_mutation` only as a call argument. The *never a path* half is
    /// [`Self::retain_action_namespace`]'s contract, whose sentence reads: "The
    /// caller receives the retained namespace capability only — never the
    /// catalog handle and never a path." That sentence is about a typed
    /// capability return and does not itself sanction a sibling `&Dir`; the
    /// `handle` precedent does, and E0.2b §8 names "the retired-root **handle**"
    /// in terms. Nothing leaves the sealed pre-catalog provider owner:
    /// `admission_mutation` is a sibling module of this one, not a consumer.
    /// E3.1 mints exactly one forward, and this is it; the family's other five
    /// keys need no forward at all because they read the action directory
    /// through the capability that already owns it.
    const fn retired_root(&self) -> &cap_std::fs::Dir {
        &self.retired_actions.handle
    }

    /// R2-E E3.1 — the admitted action directory's terminal retirement into the
    /// catalog's retired root: freeze §4.3 row E7's Phase-4 half, and the one
    /// composite the whole `terminal.*` family names.
    ///
    /// Ten boundaries in one durable sequence, split by capability per DECISION
    /// T-C′: keys #1-#5 in the action-directory owner, keys #6-#10 here in the
    /// catalog-root owner. The rename inside key #7 is the commit point, so a
    /// restart that finds the row already retired converges by observation
    /// alone.
    pub(in crate::checked_artifact::capability::pre_catalog) fn retire_admitted_action(
        &self,
        retained: &RetainedPlatformRoot,
        admitted: &AdmittedActionV1,
    ) -> Result<(), CheckedFsError> {
        let expected = admitted.reservation();
        let name = RootEntryNameV1::ActiveAction(expected.action_digest()).name();
        let name = OsStr::new(name.as_str());
        let retired_resident = self.retired_root().symlink_metadata(name).is_ok();
        let active_resident = self.final_directory.handle.symlink_metadata(name).is_ok();
        if retired_resident {
            // The corpus's standing convergence idiom: a resumed drive that
            // finds the row already at its destination returns without
            // re-crossing the edge's post-boundaries — here key #8's retired-root
            // flush and key #9's catalog-root barrier. The same shape is
            // `bootstrap/managed/provider.rs`'s `installed_resident` skip, and
            // `namespace/tests_fault_matrix.rs`'s
            // `scheduled_row_is_resident(&retired)` early return. Recorded
            // because the terminal family is the first to make that window span
            // a *cross-parent* rename: a process converging on another
            // process's post-rename, pre-flush state reports the retirement
            // complete on the strength of a rename nobody has flushed. On the
            // closed support table a cross-parent rename is a single metadata
            // transaction, replayed or discarded whole (the E16 cross-parent
            // record, freeze §4.3), so the reachable states are "rename
            // discarded" — this branch is not taken and the drive re-enters the
            // edge — or "rename durable with either or both parents unflushed",
            // which is what this branch converges on. E3 interior review F7
            // names it; it is inherited from the corpus, not introduced here.
            //
            // The rename is atomic and no edge of this family ever restores an
            // active row, so both parents holding the name is not a state this
            // sequence can leave; it is a substituted namespace, refused rather
            // than converged over.
            if active_resident {
                return Err(CheckedFsError::ambiguous(
                    "terminal retirement",
                    "the action row is resident under both the catalog root and the retired root",
                ));
            }
            return Ok(());
        }

        let namespace = self.retain_action_namespace(admitted)?;
        namespace.observe_terminal_preconditions(expected)?;
        namespace.flush_terminal_action_directory()?;
        // Release the action-directory capability before the rename edge, for
        // the reason `admission_mutation::publish_staging_action` states: on
        // Windows a directory rename fails with a sharing violation while any
        // handle into the source tree survives.
        drop(namespace);

        let observed = interior::observe(&self.final_directory.handle, &super::HostPlatform)?;
        let retired_action_dirs = interior::retired_action_dirs(&observed).ok_or_else(|| {
            CheckedFsError::ambiguous(
                "terminal retirement",
                "the catalog's retired-action root is not a bounded retired root",
            )
        })?;
        super::admission_mutation::retire_action_directory(
            &self.final_directory.handle,
            self.final_directory.identity.durable(),
            self.retired_root(),
            &self.expected_bootstrap,
            expected,
            observed.action_rows.len(),
            retired_action_dirs,
            super::admission_mutation::observed_admission_occupancy(&observed),
        )?;
        super::admission_mutation::barrier_catalog_root(
            &self.final_directory.handle,
            self.final_directory.identity.durable(),
            &self.expected_bootstrap,
            || self.revalidate(retained),
        )
    }

    pub(in crate::checked_artifact::capability::pre_catalog) fn revalidate(
        &self,
        retained: &RetainedPlatformRoot,
    ) -> Result<(), CheckedFsError> {
        let parent = retained.private_parent().ok_or_else(|| {
            CheckedFsError::ambiguous("completed catalog", "retained private parent is missing")
        })?;
        let named = parent
            .handle()
            .open_dir_nofollow(OsStr::new(private_name(CatalogPrivateNameV1::Final)))
            .map_err(|source| CheckedFsError::io("reopen named completed catalog", source))?;
        let named_identity = super::HostPlatform.dir_identity(&named)?;
        if named_identity != self.final_directory.identity
            || super::HostPlatform.dir_identity(&self.final_directory.handle)?
                != self.final_directory.identity
        {
            return Err(CheckedFsError::ambiguous(
                "completed catalog",
                "retained final directory is no longer the named catalog",
            ));
        }
        for file in [
            &self.catalog_format,
            &self.catalog_anchor,
            &self.roaming_anchor,
            &self.retired_descriptor,
            &self.retired_bootstrap,
        ] {
            if super::HostPlatform.file_identity(&file.handle)? != file.identity {
                return Err(CheckedFsError::ambiguous(
                    "completed catalog",
                    "retained catalog file identity changed",
                ));
            }
        }
        if super::HostPlatform.dir_identity(&self.retired_actions.handle)?
            != self.retired_actions.identity
        {
            return Err(CheckedFsError::ambiguous(
                "completed catalog",
                "retained retired-action directory identity changed",
            ));
        }
        let observed = interior::observe(&self.final_directory.handle, &super::HostPlatform)?;
        for (slot, file) in [
            (InfrastructureSlotV1::CatalogFormat, &self.catalog_format),
            (InfrastructureSlotV1::CatalogAnchorA, &self.catalog_anchor),
            (
                InfrastructureSlotV1::RoamingAnchorHome,
                &self.roaming_anchor,
            ),
            (
                InfrastructureSlotV1::RetiredActionsDescriptor,
                &self.retired_descriptor,
            ),
            (
                InfrastructureSlotV1::CatalogBootstrapRetired,
                &self.retired_bootstrap,
            ),
        ] {
            require_named_file_identity(&observed, slot, &file.identity)?;
        }
        require_named_directory_identity(
            &observed,
            InfrastructureSlotV1::RetiredActions,
            &self.retired_actions.identity,
        )?;
        if interior::completed_record(
            self.final_directory.identity.durable(),
            &observed,
            &self.expected_bootstrap,
        )
        .is_none()
            || !matches!(
                interior::retired_record(&observed),
                CatalogRecordFactV1::Exact(value) if value.as_ref() == &self.expected_bootstrap
            )
        {
            return Err(CheckedFsError::ambiguous(
                "completed catalog",
                "fresh retained-catalog observation is not exact",
            ));
        }
        Ok(())
    }
}

fn retain_file(
    directory: &cap_std::fs::Dir,
    interior: &super::RawCatalogInteriorObservationV1,
    slot: InfrastructureSlotV1,
) -> Result<RetainedCatalogFileV1, CheckedFsError> {
    let Some(RawCatalogInteriorFactV1::RegularFile {
        identity,
        bytes: RawCatalogBytesV1::Bounded(bytes),
        ..
    }) = row(interior, slot)
    else {
        return Err(CheckedFsError::ambiguous(
            "completed catalog",
            "required retained catalog file is missing or invalid",
        ));
    };
    let expected = ObservedFileV1 { identity, bytes };
    let name = OsStr::new(slot.name());
    verify_named_file(directory, name, expected, "completed catalog")?;
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let mut handle = directory
        .open_with(name, &options)
        .map_err(|source| CheckedFsError::io("retain completed catalog file", source))?;
    verify_open_bytes(&mut handle, expected)?;
    let identity = super::HostPlatform.file_identity(&handle)?;
    if super::retained::encode_identity(&identity) != *expected.identity {
        return Err(CheckedFsError::ambiguous(
            "completed catalog",
            "retained catalog file is not the freshly observed named object",
        ));
    }
    Ok(RetainedCatalogFileV1 { handle, identity })
}

/// T1 widening gate 2 of 3 (`GwzM5-8R2E-SemanticsAmendment-E02b-DRAFT.md`
/// §2.2). This matched `EmptyDirectory` alone, so it refused a
/// retired-root-populated catalog independently of `completed_record` and a
/// `completed_record`-only widening would have left it standing.
///
/// The identity retained is the same one in both arms — the retired root's own
/// — so what the retention proves is unchanged; only the reading of the root
/// widens. The count and infrastructure-row checks stay with
/// `interior::retired_root_identity`, which the predicate this retention runs
/// behind has already applied (`retain_completed_catalog` refuses before it
/// reaches here).
fn retain_directory(
    directory: &cap_std::fs::Dir,
    interior: &super::RawCatalogInteriorObservationV1,
    slot: InfrastructureSlotV1,
) -> Result<RetainedCatalogDirectoryV1, CheckedFsError> {
    let Some(
        RawCatalogInteriorFactV1::EmptyDirectory { identity, .. }
        | RawCatalogInteriorFactV1::RetiredActionRoot { identity, .. },
    ) = row(interior, slot)
    else {
        return Err(CheckedFsError::ambiguous(
            "completed catalog",
            "required retained catalog directory is missing or invalid",
        ));
    };
    let handle = directory
        .open_dir_nofollow(OsStr::new(slot.name()))
        .map_err(|source| CheckedFsError::io("retain completed catalog directory", source))?;
    let opened = super::HostPlatform.dir_identity(&handle)?;
    if super::retained::encode_identity(&opened) != *identity {
        return Err(CheckedFsError::ambiguous(
            "completed catalog",
            "retained catalog directory identity changed",
        ));
    }
    Ok(RetainedCatalogDirectoryV1 {
        handle,
        identity: opened,
    })
}

fn verify_open_bytes(
    file: &mut cap_std::fs::File,
    expected: ObservedFileV1<'_>,
) -> Result<(), CheckedFsError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|source| CheckedFsError::io("rewind retained catalog file", source))?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(expected.bytes.len()).map_err(|_| {
        CheckedFsError::unsupported(
            PlatformCapability::PrivateNamespaceCollisionScan,
            "retained catalog verification allocation failed",
        )
    })?;
    file.take((expected.bytes.len() + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|source| CheckedFsError::io("read retained catalog file", source))?;
    if bytes != expected.bytes {
        return Err(CheckedFsError::ambiguous(
            "completed catalog",
            "retained catalog file bytes changed",
        ));
    }
    Ok(())
}

fn require_named_file_identity(
    interior: &super::RawCatalogInteriorObservationV1,
    slot: InfrastructureSlotV1,
    retained: &IdentityV1,
) -> Result<(), CheckedFsError> {
    let Some(RawCatalogInteriorFactV1::RegularFile { identity, .. }) = row(interior, slot) else {
        return Err(CheckedFsError::ambiguous(
            "completed catalog",
            "retained catalog file is no longer present at its named slot",
        ));
    };
    if *identity != super::retained::encode_identity(retained) {
        return Err(CheckedFsError::ambiguous(
            "completed catalog",
            "retained catalog file is no longer the named slot object",
        ));
    }
    Ok(())
}

/// T1 widening gate 3 of 3 (`GwzM5-8R2E-SemanticsAmendment-E02b-DRAFT.md`
/// §2.2). This runs on **every** `revalidate`, so before the widening the first
/// terminal retirement broke revalidation even for a caller that never
/// consulted `completed_record` — the third independent `EmptyDirectory`
/// requirement the E0.2 draft's single-gate package would have missed.
///
/// What it proves is unchanged: the freshly observed named slot object is still
/// the retained one, compared by identity. `revalidate`'s own handle loops
/// (`:204-225`) `fstat` the retained handles and are unaffected by a child
/// addition, so they were never gates.
fn require_named_directory_identity(
    interior: &super::RawCatalogInteriorObservationV1,
    slot: InfrastructureSlotV1,
    retained: &IdentityV1,
) -> Result<(), CheckedFsError> {
    let Some(
        RawCatalogInteriorFactV1::EmptyDirectory { identity, .. }
        | RawCatalogInteriorFactV1::RetiredActionRoot { identity, .. },
    ) = row(interior, slot)
    else {
        return Err(CheckedFsError::ambiguous(
            "completed catalog",
            "retained catalog directory is no longer present at its named slot",
        ));
    };
    if *identity != super::retained::encode_identity(retained) {
        return Err(CheckedFsError::ambiguous(
            "completed catalog",
            "retained catalog directory is no longer the named slot object",
        ));
    }
    Ok(())
}

fn private_name(name: CatalogPrivateNameV1) -> &'static str {
    std::str::from_utf8(name.leaf_bytes()).expect("fixed catalog names are ASCII")
}

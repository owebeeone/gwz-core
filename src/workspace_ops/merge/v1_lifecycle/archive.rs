use std::fs;
use std::path::Path;

use super::archive_result::ValidatedArchivedMerge;
use super::authority::{
    BoundExactObservation, BoundObservationRequest, V1LifecycleRequest, V1ResponseDisposition,
    observe_archive,
};
use super::checked::StoredV1Record;
use super::reverse::{ReverseRuntime, route_error};
use super::service;
use super::store::CheckedV1Store;
use crate::durable_fs::sync_dir;
use crate::git::MergeAuthorityBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::{OperationContext, WorkspaceMutatorLock};
use crate::workspace_ops::merge::record_wire::{
    CanonicalRecordLeaf, ValidatedArchivedRecord, acquire_canonical_merge_locations,
    decode_archived,
};

pub(super) struct CanonicalArchiveAcquisition {
    destination_bytes: Vec<u8>,
    decoded: ValidatedArchivedRecord,
}

impl CanonicalArchiveAcquisition {
    pub(super) fn into_parts(self) -> (Vec<u8>, ValidatedArchivedRecord) {
        (self.destination_bytes, self.decoded)
    }

    fn acquire(root: &Path, merge_id: &str) -> ModelResult<Self> {
        let locations = acquire_canonical_merge_locations(root, merge_id)?;
        let (_, destination_bytes, _) = locations.archived().exact().ok_or_else(|| {
            ModelError::new(
                ErrorCode::OperationNotFound,
                format!("archived merge record '{merge_id}' was not found"),
            )
        })?;
        let destination_bytes = destination_bytes.as_slice().to_vec();
        let decoded = decode_archived(&destination_bytes, merge_id)?;
        Ok(Self {
            destination_bytes,
            decoded,
        })
    }

    #[cfg(test)]
    fn for_test(destination_bytes: Vec<u8>, expected_merge_id: &str) -> ModelResult<Self> {
        let decoded = decode_archived(&destination_bytes, expected_merge_id)?;
        Ok(Self {
            destination_bytes,
            decoded,
        })
    }
}

/// Acquire one immutable archived record from its canonical, no-follow path.
///
/// The opaque result is the only P4-to-P3 handoff. Projection and cleanup data
/// are decoded once from the exact bytes retained by the result.
pub(super) fn acquire_archived(root: &Path, merge_id: &str) -> ModelResult<ValidatedArchivedMerge> {
    Ok(ValidatedArchivedMerge::from_acquisition(
        CanonicalArchiveAcquisition::acquire(root, merge_id)?,
    ))
}

/// Run the terminal checked archive action, reconcile either crash shape, and
/// return a fresh destination-derived result.
pub(super) fn archive_terminal<B: MergeAuthorityBackend>(
    backend: &B,
    store: &CheckedV1Store,
    root: &Path,
    merge_id: &str,
    context: &OperationContext,
) -> ModelResult<ValidatedArchivedMerge> {
    if open_record_present(root, merge_id)? {
        let mut runtime = ReverseRuntime::new(backend, context);
        let response = service::run(
            store,
            root,
            merge_id,
            V1LifecycleRequest::Archive,
            &mut runtime,
        )?;
        if response.disposition() != V1ResponseDisposition::ArchiveReady {
            return Err(route_error(
                "terminal archive service returned a non-archive disposition",
            ));
        }
        return acquire_archived(root, merge_id);
    }

    // Destination-only is a completed crash shape. Serialize the source
    // absence check with every cooperating workspace mutation before reading
    // the destination authority bytes.
    let _guard = WorkspaceMutatorLock::acquire(root)?;
    if open_record_present(root, merge_id)? {
        return Err(recovery(
            "open merge record appeared during destination-only archive recovery",
        ));
    }
    acquire_archived(root, merge_id)
}

/// Collect only merge-owned backup refs from an immutable archive worklist,
/// then delete that unchanged archive. Stashes and bundles are never inputs.
/// [2026-09-02, E4.4-6-B: RE-REASONED. The terminal archive is CARVED
/// (`GwzM5-8R2E-CapabilityFreeAmendment.md` §3/§5), so this checked family has no
/// consumer to arrive: the route RE-OWNS to DR-1, conditional on (C), and E4.7
/// deletes the family or re-reasons this allowance as permanent-pending-DR-1.]
/// [2026-09-02, E4.7: RE-REASONED, not deleted — the choice, made and costed.
/// The family is SEVEN functions and one struct across TWO files, not the three
/// `GwzM5-8R2E-CapabilityFreeAmendment.md` §5 names: `gc_archived`,
/// `gc_archived_with_hook`, `remove_archive` and `require_same_archive` here,
/// plus `merge/gc.rs`'s `preflight_archived_cleanup`,
/// `delete_preflighted_backup_refs`, `require_backup_refs_absent` and
/// `PreparedArchivedCleanup`, which have no other caller.
/// `acquire_archived` is NOT in it — it has live callers on the
/// archive-recovery path. DELETING the family would drop this file's `sync_dir`
/// count 2 → 0 and fire the O13 inventory's fail-closed SHRINKAGE arm, which
/// §3 makes an amendment-tier event; E4.7 is not that tier, and DR-1 convenes
/// one step later, so the delete option travels there as a named agenda
/// sub-item ("delete the `gc_archived` family, or rebuild it against (C)'s
/// boundary") rather than being taken here. The six `tests/gc.rs` tests keep
/// executing and keep the semantics honest for a DR-1 rebuild.]
#[allow(
    dead_code,
    reason = "PERMANENT PENDING DR-1: the checked archive route this family \
              was built for has no consumer to arrive — the archive is carved \
              out (dev-docs/GwzM5-8R2E-CapabilityFreeAmendment.md §3, ADOPTED \
              2026-09-02) and E4.4 does not start (§7). O8's gc_archived route \
              RE-OWNS to DR-1, conditional on (C) resurrecting the archive \
              conversion (§5). The live GC deletion path is store/gc.rs and \
              store/retention.rs, which this family is NOT."
)]
pub(super) fn gc_archived<B: MergeAuthorityBackend>(
    backend: &B,
    root: &Path,
    merge_id: &str,
) -> ModelResult<ValidatedArchivedMerge> {
    gc_archived_with_hook(backend, root, merge_id, || {})
}

fn gc_archived_with_hook<B: MergeAuthorityBackend, F: FnOnce()>(
    backend: &B,
    root: &Path,
    merge_id: &str,
    after_ref_deletions: F,
) -> ModelResult<ValidatedArchivedMerge> {
    let _guard = WorkspaceMutatorLock::acquire(root)?;
    if any_open_record_present(root)? {
        return Err(ModelError::new(
            ErrorCode::OpenOperation,
            format!("cannot collect archived merge record '{merge_id}' while an open merge exists"),
        ));
    }

    let authority = acquire_archived(root, merge_id)?;
    let prepared =
        super::super::gc::preflight_archived_cleanup(backend, root, merge_id, authority.cleanup())?;
    super::super::gc::delete_preflighted_backup_refs(backend, &prepared)?;
    after_ref_deletions();

    let verified = acquire_archived(root, merge_id)?;
    require_same_archive(&authority, &verified)?;
    super::super::gc::require_backup_refs_absent(backend, &prepared)?;

    // Keep the final identity check adjacent to unlink. The retained mutator
    // lock closes cooperating races across the whole preflight/delete/recheck.
    let final_read = acquire_archived(root, merge_id)?;
    require_same_archive(&authority, &final_read)?;
    remove_archive(root, merge_id, authority.destination_bytes())?;
    Ok(authority)
}

pub(super) fn observe_open<B: MergeAuthorityBackend>(
    _backend: &B,
    _context: &OperationContext,
    current: &StoredV1Record,
    request: &BoundObservationRequest,
) -> ModelResult<BoundExactObservation> {
    observe_archive(current, request)
}

fn open_record_present(root: &Path, merge_id: &str) -> ModelResult<bool> {
    Ok(!matches!(
        acquire_canonical_merge_locations(root, merge_id)?.open(),
        CanonicalRecordLeaf::Absent
    ))
}

fn any_open_record_present(root: &Path) -> ModelResult<bool> {
    let root = root.canonicalize().map_err(io_error)?;
    require_real_directory(&root)?;
    let mut directory = root;
    for component in [".gwz", "merge"] {
        directory.push(component);
        match fs::symlink_metadata(&directory) {
            Ok(_) => require_real_directory(&directory)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(io_error(error)),
        }
    }
    let entries = fs::read_dir(&directory).map_err(io_error)?;
    for entry in entries {
        let path = entry.map_err(io_error)?.path();
        if path.extension().and_then(|value| value.to_str()) == Some("yaml") {
            require_regular_file(&path)?;
            return Ok(true);
        }
    }
    Ok(false)
}

fn remove_archive(root: &Path, merge_id: &str, expected: &[u8]) -> ModelResult<()> {
    let locations = acquire_canonical_merge_locations(root, merge_id)?;
    let (path, actual, _) = locations
        .archived()
        .exact()
        .ok_or_else(|| recovery("validated archive disappeared before its deletion boundary"))?;
    if actual.as_slice() != expected {
        return Err(recovery(
            "validated archive changed before its deletion boundary",
        ));
    }
    let done = path
        .as_path()
        .parent()
        .ok_or_else(|| recovery("validated archive path has no parent"))?;
    // CAPABILITY-FREE EXCEPTION, §10 row `:275`: the DEAD `remove_archive` arm behind the `:108-111` allowance; carved with the rest of the row rather than half-converted (2026-09-02, GwzM5-8R2E-CapabilityFreeAmendment.md §3).
    fs::remove_file(path.as_path()).map_err(io_error)?;
    sync_dir(done).map_err(io_error)
}

fn require_same_archive(
    expected: &ValidatedArchivedMerge,
    actual: &ValidatedArchivedMerge,
) -> ModelResult<()> {
    if expected.destination_bytes() == actual.destination_bytes()
        && expected.destination_sha256() == actual.destination_sha256()
        && expected.source_version() == actual.source_version()
        && expected.projection() == actual.projection()
        && expected.cleanup() == actual.cleanup()
    {
        Ok(())
    } else {
        Err(recovery(
            "archived merge authority changed during checked cleanup",
        ))
    }
}

fn require_real_directory(path: &Path) -> ModelResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if metadata.file_type().is_dir() {
        Ok(())
    } else {
        Err(recovery(format!(
            "'{}' is not a real directory",
            path.display()
        )))
    }
}

fn require_regular_file(path: &Path) -> ModelResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if metadata.file_type().is_file() {
        Ok(())
    } else {
        Err(recovery(format!(
            "record path '{}' is not a regular non-symlink file",
            path.display()
        )))
    }
}

fn recovery(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecoveryRequired, detail)
}

fn io_error(error: impl std::fmt::Display) -> ModelError {
    ModelError::new(ErrorCode::IoError, error.to_string())
}

#[cfg(test)]
pub(super) fn validated_result_for_test(
    destination_bytes: Vec<u8>,
    expected_merge_id: &str,
) -> ModelResult<ValidatedArchivedMerge> {
    Ok(ValidatedArchivedMerge::from_acquisition(
        CanonicalArchiveAcquisition::for_test(destination_bytes, expected_merge_id)?,
    ))
}

#[cfg(test)]
#[path = "tests/archive.rs"]
mod archive_tests;

#[cfg(test)]
#[path = "tests/gc.rs"]
mod gc_tests;

//! `gwz-family-store`: the sole writer of family metadata (lane S).
//!
//! [`YamlFamilyStore`] implements `gwz_family_store_contract::FamilyStore`
//! and its session over the frozen format-1 files (`gwz_family_model`
//! constants): the root index, clone pointers and allocation markers, using
//! same-directory temporary write and rename with checked flushes, the
//! 1 MiB encoded-index limit, and a small OS advisory try-lock on
//! `.gwz/local-family.lock` (`flock` on supported Unix hosts, `LockFileEx`
//! on Windows) released with its handle. It never reaches into gwz-core's
//! private checked-artifact locks or the single-caller pinned verified
//! writer; unsupported locking refuses family mutation.
//!
//! **One resolution of a recorded path.** The three derivations the contract
//! names — [`install_pointer`](FamilySession::install_pointer)'s check of its
//! destination, [`remove_pointer`](FamilySession::remove_pointer), and the
//! `RemoveRow`/`Disband` guard — all go through [`resolve`], which is
//! `std::fs::canonicalize` of `root` joined with the row's recorded path. So
//! they agree by construction: any spelling of the row's path (a `.`, a
//! `..`, a symlinked parent) is one destination, and the pointer
//! `install_pointer` wrote is the one the other two find.
//!
//! **What it does not promise.** Design §3.1: this is best-effort metadata
//! publication, not a power-loss-safe multi-file transaction. Every step is
//! checked and a failure is reported — with [`StoreError::Partial`] when an
//! earlier effect already landed — but nothing is journalled, replayed or
//! repaired, and metadata this store cannot decode is retained for
//! inspection rather than guessed at or removed.

#![forbid(unsafe_code)]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

mod format;
mod lock;
mod publish;

#[cfg(test)]
mod fixture;
#[cfg(test)]
mod tests;

use gwz_family_model::{
    ALLOCATION_MARKER_RELATIVE_PATH, AllocationId, FamilyChange, FamilyId, FamilyView,
    INDEX_RELATIVE_PATH, LOCK_RELATIVE_PATH, MemberName, MemberRow, MemberState,
    POINTER_RELATIVE_PATH, Refusal, check_encoded_size, validate_transition, validate_view,
};
use gwz_family_store_contract::{
    AppliedChange, FamilyLocation, FamilyObservation, FamilySession, FamilySource, FamilyStore,
    MetadataEffect, StoreError, StoreOperation,
};

use crate::format::{MarkerFile, PointerFile};
use crate::lock::{Attempt, FamilyLock};
use crate::publish::FileState;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YamlFamilyStore;

impl YamlFamilyStore {
    pub fn new() -> Self {
        Self
    }
}

impl FamilyStore for YamlFamilyStore {
    type Session = LockedFamilySession;

    fn read_view(&self, location: &FamilyLocation) -> Result<FamilyObservation, StoreError> {
        observe(&location.workspace)
    }

    fn try_lock(&self, location: &FamilyLocation) -> Result<Self::Session, StoreError> {
        // A clone locks its root: locate the family first, then take the one
        // lock that serialises it (design §3.2). Founding is allowed, so a
        // workspace in no family locks itself.
        let root = match locate(&location.workspace)? {
            Some((root, _)) => root,
            None => location.workspace.clone(),
        };
        let directory = metadata_directory(&root);
        publish::ensure_metadata_directory(&directory)
            .map_err(|error| io_error(StoreOperation::Lock, &directory, &error))?;
        let lock_path = root.join(LOCK_RELATIVE_PATH);
        match FamilyLock::try_acquire(&lock_path)
            .map_err(|error| io_error(StoreOperation::Lock, &lock_path, &error))?
        {
            Attempt::Acquired(lock) => Ok(LockedFamilySession { root, _lock: lock }),
            Attempt::Busy => Err(StoreError::Busy { lock_path }),
            Attempt::Unsupported(detail) => Err(StoreError::LockingUnsupported { detail }),
        }
    }
}

/// A held root family lock. Constructed only by [`YamlFamilyStore::try_lock`].
#[derive(Debug)]
pub struct LockedFamilySession {
    root: PathBuf,
    /// Released with this session; never read.
    _lock: FamilyLock,
}

impl FamilySession for LockedFamilySession {
    fn root(&self) -> &Path {
        &self.root
    }

    fn reread(&mut self) -> Result<Option<FamilyView>, StoreError> {
        read_index(&self.root)
    }

    fn found(
        &mut self,
        family_id: FamilyId,
        root_allocation: AllocationId,
    ) -> Result<AppliedChange, StoreError> {
        if self.reread()?.is_some() || self.pointer_state()?.is_present() {
            return Err(StoreError::ConflictingMetadata {
                workspace: self.root.clone(),
            });
        }
        let view = FamilyView::founded(family_id, root_allocation);
        let effect = self.write_index(&view)?;
        Ok(AppliedChange {
            effects: vec![effect],
            view: Some(view),
        })
    }

    fn apply(&mut self, change: &FamilyChange) -> Result<AppliedChange, StoreError> {
        let Some(current) = self.reread()? else {
            // Disband is repeatable: a family that is already gone is the
            // outcome Disband asks for, and nothing else can run without an
            // index to validate against.
            if matches!(change, FamilyChange::Disband) {
                return Ok(AppliedChange {
                    effects: Vec::new(),
                    view: None,
                });
            }
            return Err(StoreError::NoFamily {
                workspace: self.root.clone(),
            });
        };
        if let Some((member, workspace)) = self.stranded_by(&current, change) {
            return Err(StoreError::PointerStillInstalled { member, workspace });
        }
        let validated = validate_transition(&current, change)?;
        if matches!(change, FamilyChange::Disband) {
            let effect = self.remove_index()?;
            return Ok(AppliedChange {
                effects: vec![effect],
                view: None,
            });
        }
        let effect = self.write_index(&validated.next)?;
        Ok(AppliedChange {
            effects: vec![effect],
            view: Some(validated.next),
        })
    }

    fn install_pointer(
        &mut self,
        name: &MemberName,
        destination: &Path,
    ) -> Result<AppliedChange, StoreError> {
        // Refusal order (contract `install_pointer`): no index, unknown row,
        // non-`creating` row, `PathMismatch`, then the destination's own
        // metadata. Nothing above this line writes.
        let view = self.reread()?.ok_or_else(|| StoreError::NoFamily {
            workspace: self.root.clone(),
        })?;
        let row = self.row(&view, name)?;
        if row.state != MemberState::Creating {
            return Err(Refusal::WrongState {
                name: name.clone(),
                expected: MemberState::Creating,
                actual: row.state,
            }
            .into());
        }
        let recorded = self.recorded_workspace(&row);
        let marker_path = destination.join(ALLOCATION_MARKER_RELATIVE_PATH);
        // The orchestrator allocates the destination before the store writes
        // into it, so a destination that does not exist cannot be resolved
        // and fails as the marker write it was about to become.
        let resolved = resolve(destination)
            .map_err(|error| io_error(StoreOperation::WriteMarker, &marker_path, &error))?;
        if resolve(&recorded).ok().as_deref() != Some(resolved.as_path()) {
            return Err(StoreError::PathMismatch {
                member: name.as_str().to_owned(),
                recorded,
                requested: destination.to_path_buf(),
            });
        }
        if index_state(destination)?.is_present() {
            return Err(StoreError::ConflictingMetadata {
                workspace: destination.to_path_buf(),
            });
        }
        if let Some(pointer) = read_destination_pointer(destination)?
            && pointer.family_id != view.family_id.as_str()
        {
            return Err(StoreError::PointerTargetInvalid {
                pointer: destination.join(POINTER_RELATIVE_PATH),
                root: self.root.clone(),
                detail: format!(
                    "the destination already points at family `{}`",
                    pointer.family_id
                ),
            });
        }

        let directory = metadata_directory(destination);
        publish::ensure_metadata_directory(&directory)
            .map_err(|error| io_error(StoreOperation::WriteMarker, &directory, &error))?;
        let marker = format::encode_marker(&view.family_id, &row.allocation_id)
            .map_err(|error| encoding_error(StoreOperation::WriteMarker, &marker_path, &error))?;
        publish::publish(&marker_path, &marker)
            .map_err(|error| io_error(StoreOperation::WriteMarker, &marker_path, &error))?;
        let mut effects = vec![MetadataEffect::MarkerWritten {
            workspace: destination.to_path_buf(),
        }];

        let pointer_path = destination.join(POINTER_RELATIVE_PATH);
        let pointer = self.encode_pointer(&view.family_id, &pointer_path)?;
        publish::publish(&pointer_path, &pointer).map_err(|error| StoreError::Partial {
            operation: StoreOperation::WritePointer,
            completed: effects.clone(),
            path: pointer_path.clone(),
            detail: error.to_string(),
        })?;
        effects.push(MetadataEffect::PointerWritten {
            workspace: destination.to_path_buf(),
        });
        Ok(AppliedChange {
            effects,
            view: Some(view),
        })
    }

    fn remove_pointer(&mut self, name: &MemberName) -> Result<AppliedChange, StoreError> {
        let view = self.reread()?.ok_or_else(|| StoreError::NoFamily {
            workspace: self.root.clone(),
        })?;
        let row = self.row(&view, name)?;
        let workspace = self.recorded_workspace(&row);
        // A recorded path that no longer resolves holds nothing to remove.
        let Ok(resolved) = resolve(&workspace) else {
            return Ok(AppliedChange {
                effects: Vec::new(),
                view: Some(view),
            });
        };
        let mut effects = Vec::new();
        if pointer_of(&resolved, &view) {
            let path = resolved.join(POINTER_RELATIVE_PATH);
            publish::remove(&path)
                .map_err(|error| io_error(StoreOperation::RemovePointer, &path, &error))?;
            effects.push(MetadataEffect::PointerRemoved {
                workspace: workspace.clone(),
            });
        }
        if marker_of(&resolved, &view, &row) {
            let path = resolved.join(ALLOCATION_MARKER_RELATIVE_PATH);
            publish::remove(&path).map_err(|error| {
                partial_or_io(StoreOperation::RemoveMarker, &effects, &path, &error)
            })?;
            effects.push(MetadataEffect::MarkerRemoved { workspace });
        }
        Ok(AppliedChange {
            effects,
            view: Some(view),
        })
    }
}

impl LockedFamilySession {
    /// The row's recorded path joined to the root: the spelling every effect
    /// and refusal of this session carries.
    fn recorded_workspace(&self, row: &MemberRow) -> PathBuf {
        self.root.join(&row.path)
    }

    fn row(&self, view: &FamilyView, name: &MemberName) -> Result<MemberRow, StoreError> {
        view.members
            .get(name)
            .cloned()
            .ok_or_else(|| Refusal::NotFound { name: name.clone() }.into())
    }

    fn pointer_state(&self) -> Result<FileState, StoreError> {
        let path = self.root.join(POINTER_RELATIVE_PATH);
        publish::file_state(&path)
            .map_err(|error| io_error(StoreOperation::ReadPointer, &path, &error))
    }

    /// The row whose pointer this change would strand, if any. Both arms are
    /// derived from the rows, because a filesystem store cannot enumerate
    /// the pointers of a family; `Disband` names the first such row in name
    /// order (`members` is ordered by name).
    fn stranded_by(&self, view: &FamilyView, change: &FamilyChange) -> Option<(String, PathBuf)> {
        let standing = |name: &MemberName, row: &MemberRow| {
            self.standing_pointer(view, row)
                .map(|workspace| (name.as_str().to_owned(), workspace))
        };
        match change {
            FamilyChange::RemoveRow { name, .. } => standing(name, view.members.get(name)?),
            FamilyChange::Disband => view
                .members
                .iter()
                .find_map(|(name, row)| standing(name, row)),
            _ => None,
        }
    }

    /// This family's pointer at the row's recorded path, resolved exactly as
    /// `install_pointer` resolved its destination. A path that no longer
    /// resolves, and metadata that names another family or cannot be
    /// decoded, hold no pointer of *this* family: they are reported by
    /// `gwz local list` and retained, and they neither protect the row nor
    /// are removed by `remove_pointer`.
    fn standing_pointer(&self, view: &FamilyView, row: &MemberRow) -> Option<PathBuf> {
        let recorded = self.recorded_workspace(row);
        let resolved = resolve(&recorded).ok()?;
        pointer_of(&resolved, view).then_some(recorded)
    }

    fn write_index(&self, view: &FamilyView) -> Result<MetadataEffect, StoreError> {
        let path = self.root.join(INDEX_RELATIVE_PATH);
        let encoded = format::encode_index(view)
            .map_err(|error| encoding_error(StoreOperation::WriteIndex, &path, &error))?;
        // The 1 MiB limit is enforced before any mutation (design §3).
        check_encoded_size(encoded.len() as u64).map_err(|oversize| StoreError::Oversize {
            path: path.clone(),
            bytes: oversize.bytes,
            limit: oversize.limit,
        })?;
        let directory = metadata_directory(&self.root);
        publish::ensure_metadata_directory(&directory)
            .map_err(|error| io_error(StoreOperation::WriteIndex, &directory, &error))?;
        publish::publish(&path, &encoded)
            .map_err(|error| io_error(StoreOperation::WriteIndex, &path, &error))?;
        Ok(MetadataEffect::IndexWritten)
    }

    fn remove_index(&self) -> Result<MetadataEffect, StoreError> {
        let path = self.root.join(INDEX_RELATIVE_PATH);
        publish::remove(&path)
            .map_err(|error| io_error(StoreOperation::RemoveIndex, &path, &error))?;
        Ok(MetadataEffect::IndexRemoved)
    }

    fn encode_pointer(
        &self,
        family_id: &FamilyId,
        pointer_path: &Path,
    ) -> Result<Vec<u8>, StoreError> {
        let root = resolve(&self.root).unwrap_or_else(|_| self.root.clone());
        let root = root.to_str().ok_or_else(|| StoreError::Io {
            operation: StoreOperation::WritePointer,
            path: pointer_path.to_path_buf(),
            detail: format!(
                "the registering root {} is not UTF-8, so it cannot be recorded in a pointer",
                root.display()
            ),
        })?;
        format::encode_pointer(family_id, root)
            .map_err(|error| encoding_error(StoreOperation::WritePointer, pointer_path, &error))
    }
}

/// The one canonical resolution shared by every derivation of a recorded
/// path (contract call-order clause, "One resolution of the recorded path").
fn resolve(path: &Path) -> io::Result<PathBuf> {
    fs::canonicalize(path)
}

fn metadata_directory(workspace: &Path) -> PathBuf {
    let relative = Path::new(INDEX_RELATIVE_PATH)
        .parent()
        .expect("the index is inside the workspace's metadata directory");
    workspace.join(relative)
}

/// Which root a workspace belongs to, and how it was reached — without
/// decoding that root's index.
///
/// This is the step `try_lock` needs and `read_view` starts from. Keeping
/// the index *decode* out of it is what makes a family whose index is
/// malformed or oversize still lockable, so `reread` reports the refusal
/// under the lock and an explicit repair or disband can be attempted; a
/// store that refused the lock would leave nothing able to address the
/// family at all. It matches the reference fake, whose `try_lock` resolves
/// the root and whose `reread` carries the refusal.
fn locate(workspace: &Path) -> Result<Option<(PathBuf, FamilySource)>, StoreError> {
    let index = index_state(workspace)?;
    let pointer_path = workspace.join(POINTER_RELATIVE_PATH);
    let pointer = publish::file_state(&pointer_path)
        .map_err(|error| io_error(StoreOperation::ReadPointer, &pointer_path, &error))?;
    if index.is_present() && pointer.is_present() {
        return Err(StoreError::ConflictingMetadata {
            workspace: workspace.to_path_buf(),
        });
    }
    if index.is_present() {
        return Ok(Some((workspace.to_path_buf(), FamilySource::Index)));
    }
    let Some(pointer) = read_pointer(workspace)? else {
        return Ok(None);
    };
    let root = PathBuf::from(&pointer.root_path);
    let invalid = |detail: &str| StoreError::PointerTargetInvalid {
        pointer: pointer_path.clone(),
        root: root.clone(),
        detail: detail.to_owned(),
    };
    // A root that holds no index, or another family's, invalidates the
    // pointer. An index this store cannot decode does not: the pointer
    // still names this root, and the refusal belongs to whoever reads it.
    match read_index(&root) {
        Ok(Some(view)) if view.family_id.as_str() == pointer.family_id => {}
        Ok(Some(_)) => return Err(invalid("the root holds another family's index")),
        Ok(None) => return Err(invalid("the root holds no family index")),
        Err(StoreError::Malformed { .. } | StoreError::Oversize { .. }) => {}
        Err(error) => return Err(error),
    }
    Ok(Some((root, FamilySource::Pointer)))
}

/// Observe a workspace's family without writing anything, the lock file
/// included.
fn observe(workspace: &Path) -> Result<FamilyObservation, StoreError> {
    let Some((root, source)) = locate(workspace)? else {
        return Ok(FamilyObservation::NoFamily);
    };
    let view = read_index(&root)?.ok_or_else(|| StoreError::NoFamily {
        workspace: workspace.to_path_buf(),
    })?;
    Ok(FamilyObservation::Family { root, source, view })
}

fn index_state(workspace: &Path) -> Result<FileState, StoreError> {
    let path = workspace.join(INDEX_RELATIVE_PATH);
    publish::file_state(&path).map_err(|error| io_error(StoreOperation::ReadIndex, &path, &error))
}

/// Read and validate the index at `root`, or `None` when there is none. An
/// index that is oversize, undecodable or not a valid family refuses; it is
/// never repaired or ignored.
fn read_index(root: &Path) -> Result<Option<FamilyView>, StoreError> {
    let path = root.join(INDEX_RELATIVE_PATH);
    match publish::file_state(&path)
        .map_err(|error| io_error(StoreOperation::ReadIndex, &path, &error))?
    {
        FileState::Absent => return Ok(None),
        FileState::Irregular => return Err(irregular(&path)),
        FileState::Regular => {}
    }
    let bytes = publish::encoded_size(&path)
        .map_err(|error| io_error(StoreOperation::ReadIndex, &path, &error))?;
    check_encoded_size(bytes).map_err(|oversize| StoreError::Oversize {
        path: path.clone(),
        bytes: oversize.bytes,
        limit: oversize.limit,
    })?;
    let encoded =
        publish::read(&path).map_err(|error| io_error(StoreOperation::ReadIndex, &path, &error))?;
    let view = format::decode_index(&encoded).map_err(|error| malformed(&path, error.detail()))?;
    validate_view(&view).map_err(|refusal| malformed(&path, &refusal.to_string()))?;
    Ok(Some(view))
}

fn read_pointer(workspace: &Path) -> Result<Option<PointerFile>, StoreError> {
    let path = workspace.join(POINTER_RELATIVE_PATH);
    let Some(encoded) = read_metadata(&path, StoreOperation::ReadPointer)? else {
        return Ok(None);
    };
    format::decode_pointer(&encoded)
        .map(Some)
        .map_err(|error| malformed(&path, error.detail()))
}

/// The pointer an `install_pointer` destination already holds.
///
/// The contract's refusal order names exactly two things the destination's
/// own metadata can refuse: an index (`ConflictingMetadata`) and a pointer
/// to another family (`PointerTargetInvalid`). A *regular* file that cannot
/// be decoded may be another family's corrupted pointer, so it refuses as
/// malformed and is retained rather than replaced. A node that is not a
/// regular file at all is not metadata this store wrote: it is left to the
/// publication, whose rename replaces a symlink itself (never following it)
/// and fails with a checked error on anything it cannot replace.
fn read_destination_pointer(workspace: &Path) -> Result<Option<PointerFile>, StoreError> {
    let path = workspace.join(POINTER_RELATIVE_PATH);
    match publish::file_state(&path)
        .map_err(|error| io_error(StoreOperation::ReadPointer, &path, &error))?
    {
        FileState::Absent | FileState::Irregular => Ok(None),
        FileState::Regular => read_pointer(workspace),
    }
}

fn read_marker(workspace: &Path) -> Result<Option<MarkerFile>, StoreError> {
    let path = workspace.join(ALLOCATION_MARKER_RELATIVE_PATH);
    let Some(encoded) = read_metadata(&path, StoreOperation::ReadMarker)? else {
        return Ok(None);
    };
    format::decode_marker(&encoded)
        .map(Some)
        .map_err(|error| malformed(&path, error.detail()))
}

fn read_metadata(path: &Path, operation: StoreOperation) -> Result<Option<Vec<u8>>, StoreError> {
    match publish::file_state(path).map_err(|error| io_error(operation, path, &error))? {
        FileState::Absent => Ok(None),
        FileState::Irregular => Err(irregular(path)),
        FileState::Regular => publish::read(path)
            .map(Some)
            .map_err(|error| io_error(operation, path, &error)),
    }
}

/// Does this family's pointer stand at `resolved`? Undecodable metadata and
/// another family's pointer are both "no": they are retained, not claimed.
fn pointer_of(resolved: &Path, view: &FamilyView) -> bool {
    read_pointer(resolved)
        .ok()
        .flatten()
        .is_some_and(|pointer| pointer.family_id == view.family_id.as_str())
}

/// Does this row's allocation marker stand at `resolved`? Both the family
/// and the row's allocation must match before the store removes it.
fn marker_of(resolved: &Path, view: &FamilyView, row: &MemberRow) -> bool {
    read_marker(resolved).ok().flatten().is_some_and(|marker| {
        marker.family_id == view.family_id.as_str()
            && marker.allocation_id == row.allocation_id.as_str()
    })
}

fn io_error(operation: StoreOperation, path: &Path, error: &io::Error) -> StoreError {
    StoreError::Io {
        operation,
        path: path.to_path_buf(),
        detail: error.to_string(),
    }
}

fn encoding_error(
    operation: StoreOperation,
    path: &Path,
    error: &format::FormatError,
) -> StoreError {
    StoreError::Io {
        operation,
        path: path.to_path_buf(),
        detail: format!("the metadata could not be encoded: {}", error.detail()),
    }
}

fn partial_or_io(
    operation: StoreOperation,
    completed: &[MetadataEffect],
    path: &Path,
    error: &io::Error,
) -> StoreError {
    if completed.is_empty() {
        return io_error(operation, path, error);
    }
    StoreError::Partial {
        operation,
        completed: completed.to_vec(),
        path: path.to_path_buf(),
        detail: error.to_string(),
    }
}

fn malformed(path: &Path, detail: &str) -> StoreError {
    StoreError::Malformed {
        path: path.to_path_buf(),
        detail: detail.to_owned(),
    }
}

fn irregular(path: &Path) -> StoreError {
    malformed(
        path,
        "a symlink, directory or other node occupies this metadata path; \
         the store neither follows nor replaces it",
    )
}

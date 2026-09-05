//! Family store contract for the GWZ local clone family.
//!
//! The store is the sole writer of the family index, clone pointers and
//! allocation markers (design §3, §3.1, §3.2). This crate owns the
//! location, the read observation, the locked session port, the typed
//! store errors and the partial-effect vocabulary. It contains no YAML, no
//! lock implementation and no filesystem code: `gwz-family-store`
//! implements it; `gwz-workspace-install` and `gwz-local-disposal` consume
//! it through `&mut dyn FamilySession`.
//!
//! Contract (`GwzLocalCloneLibraryBoundaries.md` §3, "Writes, locks and
//! transport"):
//!
//! - [`FamilyStore::read_view`] never creates a lock file, never repairs,
//!   and refuses malformed, oversize or conflicting metadata instead of
//!   pretending no family exists.
//! - [`FamilyStore::try_lock`] takes one OS advisory try-lock on the root's
//!   lock file and refuses busy. The returned session owns that lock until
//!   it is dropped; every mutation goes through the live session.
//! - [`FamilySession::apply`] rereads and validates the index under the lock
//!   (`gwz_family_model::validate_transition`) and writes only the matching
//!   index change. Pointer and marker files are separate session operations
//!   on the destination workspace. There is no multi-file atomicity promise:
//!   a failure between effects reports [`StoreError::Partial`] with what was
//!   completed, and the caller reports it.
//! - Every error names its operation, typed cause and known partial effects.

#![forbid(unsafe_code)]

use std::fmt;
use std::path::{Path, PathBuf};

use gwz_family_model::{AllocationId, FamilyChange, FamilyId, FamilyView, MemberName, Refusal};

#[cfg(any(test, feature = "contract-tests"))]
pub mod contract_tests;

/// The workspace whose family is addressed. It may be the root (holding the
/// index) or a clone (holding a pointer); the store follows the pointer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FamilyLocation {
    pub workspace: PathBuf,
}

impl FamilyLocation {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
        }
    }
}

/// How the family was reached from the addressed workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FamilySource {
    /// The workspace holds the index: it is the root.
    Index,
    /// The workspace holds a pointer to the root.
    Pointer,
}

/// The result of a read: no family, or a valid view plus where it lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FamilyObservation {
    /// Neither an index nor a pointer is present. This is a valid
    /// observation, distinct from every refusal.
    NoFamily,
    Family {
        /// The registering root (the directory holding the index).
        root: PathBuf,
        source: FamilySource,
        view: FamilyView,
    },
}

impl FamilyObservation {
    pub fn view(&self) -> Option<&FamilyView> {
        match self {
            Self::NoFamily => None,
            Self::Family { view, .. } => Some(view),
        }
    }
}

/// The store operation an error or effect belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreOperation {
    ReadIndex,
    ReadPointer,
    ReadMarker,
    Lock,
    WriteIndex,
    RemoveIndex,
    WritePointer,
    RemovePointer,
    WriteMarker,
    RemoveMarker,
}

/// One completed metadata effect. Reported on success and inside
/// [`StoreError::Partial`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MetadataEffect {
    IndexWritten,
    IndexRemoved,
    PointerWritten { workspace: PathBuf },
    PointerRemoved { workspace: PathBuf },
    MarkerWritten { workspace: PathBuf },
    MarkerRemoved { workspace: PathBuf },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreError {
    /// Another family operation holds the lock.
    Busy { lock_path: PathBuf },
    /// The platform offers no supported advisory try-lock; family mutation
    /// refuses.
    LockingUnsupported { detail: String },
    /// A pointer or index could not be decoded. The file is retained.
    Malformed { path: PathBuf, detail: String },
    /// The encoded index exceeds [`gwz_family_model::MAX_ENCODED_INDEX_BYTES`].
    Oversize {
        path: PathBuf,
        bytes: u64,
        limit: u64,
    },
    /// The workspace holds both an index and a pointer.
    ConflictingMetadata { workspace: PathBuf },
    /// A pointer names a root that holds no matching index.
    PointerTargetInvalid {
        pointer: PathBuf,
        root: PathBuf,
        detail: String,
    },
    /// The addressed workspace is in no family, but the operation needs one.
    NoFamily { workspace: PathBuf },
    /// The pure model refused the transition; nothing was written.
    Refused(Refusal),
    /// An I/O failure before any effect of the operation.
    Io {
        operation: StoreOperation,
        path: PathBuf,
        detail: String,
    },
    /// An I/O failure after some effects of the operation completed.
    Partial {
        operation: StoreOperation,
        completed: Vec<MetadataEffect>,
        path: PathBuf,
        detail: String,
    },
    /// This store implements nothing yet.
    Unimplemented { operation: StoreOperation },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy { lock_path } => {
                write!(
                    f,
                    "family lock {} is held by another operation",
                    lock_path.display()
                )
            }
            Self::LockingUnsupported { detail } => {
                write!(f, "family locking unsupported: {detail}")
            }
            Self::Malformed { path, detail } => {
                write!(f, "{} is malformed: {detail}", path.display())
            }
            Self::Oversize { path, bytes, limit } => {
                write!(
                    f,
                    "{} is {bytes} bytes; the limit is {limit}",
                    path.display()
                )
            }
            Self::ConflictingMetadata { workspace } => write!(
                f,
                "{} holds both a family index and a family pointer",
                workspace.display()
            ),
            Self::PointerTargetInvalid {
                pointer,
                root,
                detail,
            } => write!(
                f,
                "{} points at {} which holds no matching index: {detail}",
                pointer.display(),
                root.display()
            ),
            Self::NoFamily { workspace } => {
                write!(f, "{} is in no local family", workspace.display())
            }
            Self::Refused(refusal) => write!(f, "{refusal}"),
            Self::Io {
                operation,
                path,
                detail,
            } => {
                write!(f, "{operation:?} {}: {detail}", path.display())
            }
            Self::Partial {
                operation,
                completed,
                path,
                detail,
            } => write!(
                f,
                "{operation:?} {} failed after {} effect(s): {detail}",
                path.display(),
                completed.len()
            ),
            Self::Unimplemented { operation } => write!(f, "{operation:?} is not implemented"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<Refusal> for StoreError {
    fn from(refusal: Refusal) -> Self {
        Self::Refused(refusal)
    }
}

/// What a successful session operation did, and the index it left behind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppliedChange {
    pub effects: Vec<MetadataEffect>,
    /// The index after the operation (`None` after `Disband` removed it).
    pub view: Option<FamilyView>,
}

/// Read access and lock acquisition.
pub trait FamilyStore {
    type Session: FamilySession;

    /// Observe the family of `location` without writing anything, including
    /// the lock file.
    fn read_view(&self, location: &FamilyLocation) -> Result<FamilyObservation, StoreError>;

    /// Take the root family lock or refuse `Busy`. The session holds the lock
    /// until it is dropped. Founding a family (a root with no index yet) is
    /// allowed: `reread` then reports `None` and `found` writes the first
    /// index.
    fn try_lock(&self, location: &FamilyLocation) -> Result<Self::Session, StoreError>;
}

/// One held family lock. Call order: `reread`/`found`/`apply`/pointer
/// operations in any order while held; dropping the session releases the
/// lock. Each operation rereads and validates before it writes.
pub trait FamilySession {
    /// The registering root whose lock this session holds.
    fn root(&self) -> &Path;

    /// Fresh validated view under the lock, or `None` when no index exists.
    fn reread(&mut self) -> Result<Option<FamilyView>, StoreError>;

    /// Write the first index of a new family. Refuses if an index or pointer
    /// already exists at the root.
    fn found(
        &mut self,
        family_id: FamilyId,
        root_allocation: AllocationId,
    ) -> Result<AppliedChange, StoreError>;

    /// Reread, validate through `gwz_family_model::validate_transition`, and
    /// write the resulting index (or remove it for `Disband`).
    fn apply(&mut self, change: &FamilyChange) -> Result<AppliedChange, StoreError>;

    /// Write the allocation marker, then the pointer, into `destination`'s
    /// `.gwz/` for the `creating` row `name`. Refuses if the destination
    /// holds an index or a pointer to another family.
    fn install_pointer(
        &mut self,
        name: &MemberName,
        destination: &Path,
    ) -> Result<AppliedChange, StoreError>;

    /// Remove the pointer and marker at the row's recorded path when they
    /// match this family. Repeatable; absent files are not errors.
    fn remove_pointer(&mut self, name: &MemberName) -> Result<AppliedChange, StoreError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observation_exposes_the_view_only_for_a_family() {
        assert!(FamilyObservation::NoFamily.view().is_none());
        let view = FamilyView::founded(
            FamilyId::new("fam").unwrap(),
            AllocationId::new("alloc").unwrap(),
        );
        let observation = FamilyObservation::Family {
            root: PathBuf::from("/root"),
            source: FamilySource::Pointer,
            view: view.clone(),
        };
        assert_eq!(observation.view(), Some(&view));
    }

    #[test]
    fn errors_render_operation_and_partial_effect_count() {
        let partial = StoreError::Partial {
            operation: StoreOperation::WritePointer,
            completed: vec![MetadataEffect::MarkerWritten {
                workspace: PathBuf::from("/ws-A"),
            }],
            path: PathBuf::from("/ws-A/.gwz/family-root"),
            detail: "disk full".to_owned(),
        };
        assert!(partial.to_string().contains("after 1 effect(s)"));
        let refused: StoreError = Refusal::NotFound {
            name: MemberName::parse("A").unwrap(),
        }
        .into();
        assert!(matches!(refused, StoreError::Refused(_)));
    }
}

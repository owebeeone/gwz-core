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
/// [`StoreError::Partial`]. `workspace` is the clone workspace as the
/// operation addressed it: the `destination` the caller passed to
/// [`FamilySession::install_pointer`], or the row's recorded path resolved
/// against the root for [`FamilySession::remove_pointer`] and the
/// `RemoveRow`/`Disband` guard (the conformance suite compares them exactly).
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
    /// A member's row cannot be removed (`RemoveRow`) and the family cannot be
    /// disbanded (`Disband`) while a clone pointer this family installed at
    /// the member's recorded path is still present: the pointer must be
    /// removed first, or it would be stranded with no row to reach it through
    /// (LCM1.0c-rem1, State P2-2; design §3.1 recovery order). `member` names
    /// the row whose pointer stands -- for `Disband`, the first such row in
    /// name order -- and `workspace` is its recorded path resolved against
    /// the root. Nothing was written.
    PointerStillInstalled { member: String, workspace: PathBuf },
    /// `install_pointer` was handed a destination that is not the store's own
    /// resolution of the member row's recorded path (LCM1.0c-fu1, State
    /// S2-P3-1). A pointer there would be unreachable by `remove_pointer` and
    /// invisible to the `RemoveRow`/`Disband` guard, so nothing was written.
    /// `recorded` is the row's path resolved against the root as the store
    /// spells it; `requested` is the destination as passed.
    PathMismatch {
        member: String,
        recorded: PathBuf,
        requested: PathBuf,
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
            Self::PointerStillInstalled { member, workspace } => write!(
                f,
                "member `{member}` still has its clone pointer installed at {}; remove the \
                 pointer before removing the row or disbanding the family",
                workspace.display()
            ),
            Self::PathMismatch {
                member,
                recorded,
                requested,
            } => write!(
                f,
                "member `{member}` is recorded at {}; refusing to install its pointer at {}",
                recorded.display(),
                requested.display()
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

/// One held family lock. Dropping the session releases the lock; each
/// operation rereads and validates before it writes.
///
/// **Call order (LCM1.0c-rem1, State P2-2; design §3, §3.1).** These are not
/// free: two independently developed orchestrations (install, disposal,
/// disband) must agree on the write order, and one order is unrecoverable.
///
/// - A pointer or marker is installed only *after* the member's `creating`
///   row exists ([`install_pointer`](FamilySession::install_pointer) refuses a
///   non-`creating` row).
/// - A pointer and its marker are removed *strictly before* the member's row
///   is removed or the family is disbanded. `remove_pointer` is keyed by the
///   member name and needs the row to find the recorded path, so a pointer
///   whose row is already gone is **not reachable through this interface** and
///   must never be produced: the store refuses a `RemoveRow`/`Disband` that
///   would strand an installed pointer, so the safe order is the only order
///   it accepts. The reverse order is crash-recoverable — a repeat after an
///   interrupted pointer removal succeeds (absent files are not errors) —
///   which is why it is the required one.
/// - **One resolution of the recorded path (LCM1.0c-fu1, State S2-P3-1).**
///   Three derivations look at a member's path — `install_pointer`'s check of
///   its `destination`, `remove_pointer`, and the `RemoveRow`/`Disband` guard
///   — and the store resolves all three through one canonical resolution of
///   its own (`root` joined with the row's root-relative `path`: a filesystem
///   store canonicalises through the filesystem, the reference fake
///   lexically), so they agree by construction and the pointer `install_pointer`
///   wrote is the one the other two find. `install_pointer` refuses a
///   destination that resolves anywhere else ([`StoreError::PathMismatch`]),
///   which is what makes the previous bullet's "must never be produced" true
///   of the interface as declared. A store whose derivations disagree fails
///   the conformance suite instead of stranding a pointer.
/// - `reread` and `found` may be interleaved with the above at any point they
///   are individually valid.
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
    /// `.gwz/` for the `creating` row `name`.
    ///
    /// `destination` is not a free argument: it must be the store's own
    /// resolution of the row's recorded path (`root` joined with the row's
    /// root-relative `path`, resolved canonically — see the call-order
    /// clause), and it must already exist, because the orchestrator allocates
    /// the destination before the store writes into it (design §3 step 2);
    /// the store creates nothing above `.gwz/`. Any spelling that resolves to
    /// that directory is accepted (effects name `destination` as passed); a
    /// destination that resolves elsewhere is refused with
    /// [`StoreError::PathMismatch`] before any effect; a destination that does
    /// not exist fails with [`StoreError::Io`] `{ operation: WriteMarker, .. }`.
    /// Refusal order: no index (`NoFamily`), unknown row (`Refused(NotFound)`),
    /// non-`creating` row (`Refused(WrongState)`), `PathMismatch`, then the
    /// destination's own metadata — it holds an index (`ConflictingMetadata`)
    /// or a pointer to another family (`PointerTargetInvalid`). (LCM1.0c-rem1
    /// State P2-2; LCM1.0c-fu1 State S2-P3-1, Code C2-P3-1.)
    fn install_pointer(
        &mut self,
        name: &MemberName,
        destination: &Path,
    ) -> Result<AppliedChange, StoreError>;

    /// Remove the pointer and marker at the row's recorded path — resolved
    /// exactly as `install_pointer` resolves it, so the pointer it installed
    /// is the one found here — when they match this family. Repeatable;
    /// absent files are not errors, and a recorded path that no longer
    /// resolves (the directory is gone) holds nothing to remove. Effects name
    /// the recorded path resolved against the root.
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

    #[test]
    fn ordering_and_path_refusals_name_the_member_and_the_paths() {
        // LCM1.0c-fu1 (Code C2-P3-2): the ordering refusal describes both a
        // row removal and a disband, with no `"*"` sentinel in the message.
        let still = StoreError::PointerStillInstalled {
            member: "A".to_owned(),
            workspace: PathBuf::from("/root/../ws-A"),
        };
        let text = still.to_string();
        assert!(text.contains("member `A`"), "{text}");
        assert!(text.contains("/root/../ws-A"), "{text}");
        assert!(text.contains("disbanding"), "{text}");
        assert!(!text.contains('*'), "{text}");
        // (State S2-P3-1): the mismatch refusal shows both spellings.
        let mismatch = StoreError::PathMismatch {
            member: "A".to_owned(),
            recorded: PathBuf::from("/root/../ws-A"),
            requested: PathBuf::from("/root/../ws-B"),
        };
        let text = mismatch.to_string();
        assert!(text.contains("member `A`"), "{text}");
        assert!(text.contains("/root/../ws-A"), "{text}");
        assert!(text.contains("/root/../ws-B"), "{text}");
    }
}

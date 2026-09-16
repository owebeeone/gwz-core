use super::super::RawCatalogInteriorFactV1;
use super::super::filesystem::PlatformProviderV1;
use super::super::retained::encode_identity;
use crate::checked_artifact::capability::{CheckedFsError, PlatformCapability};
use crate::checked_artifact::catalog::CatalogNameBudgetV1;
use crate::checked_artifact::protocol::{CatalogRootRowClassV1, MAX_RETIRED_ACTION_DIRS};
use crate::filesystem::FsKind;
use crate::filesystem::FsOpenMode;
use std::ffi::OsStr;

use super::*;

pub(crate) fn observe_slot(
    directory: &crate::filesystem::FsDirectory,
    name: &OsStr,
    probe_empty_directory: bool,
    platform: &impl PlatformProviderV1,
) -> Result<RawCatalogInteriorFactV1, CheckedFsError> {
    let metadata = directory
        .entry_metadata(name)
        .map_err(|source| CheckedFsError::io("observe catalog interior slot", source))?;
    if metadata.kind == FsKind::Directory && metadata.kind != FsKind::Symlink {
        // Share-delete open, not a plain no-follow one. This enumeration runs
        // inside the sealed publication's destination recheck, which holds the
        // retained rename-source handle open across the whole edge — and R2-D
        // Phase 1's `PublishStagingAction` is the first publication whose
        // source is a *directory child of the very root being enumerated here*
        // (`ActionAdmissionStaging`). On Windows that handle carries DELETE
        // access, so any later open of the same object that does not itself
        // grant DELETE sharing fails with a sharing violation (os error 32);
        // cap-std's plain directory open omits `FILE_SHARE_DELETE`, so it is
        // exactly such an open. `platform::open_dir_share_delete` is the
        // established recipe for this collision (`platform.rs`, the
        // `FILE_SHARE_DELETE` arm; freeze §4.1 P3 records it as "so the
        // directory open does not collide with the retained rename-source
        // handle"). Dropping the source handle instead is not available: the
        // primitive renames that exact identity-checked handle, so its lifetime
        // is the seam's guarantee.
        //
        // Non-Windows arm is byte-identical to the previous call — the helper
        // is `open_dir_nofollow` there — so macOS and Linux behaviour is
        // unchanged. The sibling regular-file open below needs no counterpart:
        // it inherits std's default share mode, which already includes
        // `FILE_SHARE_DELETE`, which is why only the directory label appeared
        // in the Windows failures.
        let child = directory
            .retained_child(name)
            .map_err(|source| CheckedFsError::io("open catalog interior directory", source))?;
        let identity = platform.dir_identity(&child)?;
        if probe_empty_directory {
            // T1 widening (E0.2b §2, AUTHORIZED; the freeze's own Class 2
            // shape, `:1443-1450` — "what Phase 1 must extend is the
            // *provider's reading* of that vocabulary, not the vocabulary").
            // The retired root is read by its own dedicated single-level
            // reader, which is where the empty case is decided too.
            let retired = read_retired_root(&child)?;
            if retired.is_empty() {
                return Ok(RawCatalogInteriorFactV1::EmptyDirectory {
                    identity: encode_identity(&identity),
                    durable_identity: identity.durable().clone(),
                });
            }
            return Ok(RawCatalogInteriorFactV1::RetiredActionRoot {
                identity: encode_identity(&identity),
                durable_identity: identity.durable().clone(),
                unaccepted_rows: retired.unaccepted_rows,
                retired_action_dirs: retired.retired_action_dirs,
            });
        }
    } else if metadata.kind == FsKind::File && metadata.kind != FsKind::Symlink {
        let options = FsOpenMode::Read;
        let mut file = directory
            .open_file(name, &options)
            .map_err(|source| CheckedFsError::io("open catalog interior file", source))?;
        let identity = platform.file_identity(&file)?;
        return Ok(RawCatalogInteriorFactV1::RegularFile {
            identity: encode_identity(&identity),
            durable_identity: identity.durable().clone(),
            bytes: read_bounded(&mut file)?,
        });
    }
    let mut value = Vec::new();
    value.push(if metadata.kind == FsKind::Symlink {
        3
    } else {
        4
    });
    value.extend_from_slice(&metadata.dev().to_be_bytes());
    value.extend_from_slice(&metadata.ino().to_be_bytes());
    Ok(RawCatalogInteriorFactV1::Other(value))
}

/// The T1 widening's bounded reading of the `RetiredActions` root, one level
/// deep.
pub(crate) struct RetiredRootReadingV1 {
    /// Children that classify `RootEntryNameV1::ActiveAction`.
    pub(crate) retired_action_dirs: usize,
    /// Children that do **not**. Infrastructure-slot names, scheduled-scratch
    /// and retired names, malformed-recognized names, non-ASCII names and
    /// foreign names all land here: the reading accepts *only* action rows, so
    /// one counter of everything else is all the predicate needs, and it is
    /// what makes an infrastructure-slot name planted in the retired root a
    /// refusal rather than a classified row.
    pub(crate) unaccepted_rows: usize,
}

impl RetiredRootReadingV1 {
    pub(crate) const fn is_empty(&self) -> bool {
        self.retired_action_dirs == 0 && self.unaccepted_rows == 0
    }
}

/// Reads the `RetiredActions` root's own children **exactly once, exactly one
/// level deep**, and classifies each name through the frozen
/// [`RootEntryNameV1`] grammar.
///
/// **It deliberately calls neither [`observe`] nor [`observe_slot`], and that
/// is a structural property, not a check.** The first shape of this widening
/// re-entered `observe` on the retired root; `exact_row` is parent-independent,
/// so a `retired-actions-v1` child of the retired root classified as a
/// perfectly good infrastructure row and the pair became mutually recursive
/// with no depth counter. A nested chain then aborted the process on a stack
/// overflow — `SIGABRT`, reproduced at depth 700 — instead of returning the
/// typed refusal this owner's whole discipline promises, and it did so on the
/// path of *every* catalog consumer, since `completed_record` runs in every
/// recovery and every publication acquisition window. There is no self-call
/// here to exceed, so a nested chain of **any** depth is one directory read and
/// a refusal.
///
/// **The bound is checked explicitly and is not inherited.** The entry cap
/// below is `MAX_RETIRED_ACTION_DIRS` itself (`protocol/bounds.rs:2`), not
/// `interior::observe`'s own effective caps — `MAX_INTERIOR_ENTRIES`
/// (= `MAX_ROOT_ENTRIES` = 74) and `MAX_ACTIVE_ACTION_DIRS` (= 64) — and not
/// the name budget's `MAX_CATALOG_PARENT_ENTRIES_V1`. The reused reader was
/// numerically safe only because `bounds.rs:1` and `:2` are both 64, which
/// silently coupled the retired-root bound to the active one; naming the
/// retired constant here is what makes a future edit to either fail closed
/// (E0.2b §3.2 ground 3, Code round-2 [P3-R1]).
pub(crate) fn read_retired_root(
    directory: &crate::filesystem::FsDirectory,
) -> Result<RetiredRootReadingV1, CheckedFsError> {
    let mut budget = CatalogNameBudgetV1::new();
    let mut actions: Vec<crate::checked_artifact::protocol::ActionDigestV1> = Vec::new();
    let mut unaccepted_rows = 0_usize;
    for entry in directory
        .entries()
        .map_err(|source| CheckedFsError::io("enumerate retired-action root", source))?
    {
        let entry =
            entry.map_err(|source| CheckedFsError::io("read retired-action root", source))?;
        let name = entry;
        budget.charge_os_str(&name)?;
        if budget.entry_count() > MAX_RETIRED_ACTION_DIRS {
            return Err(CheckedFsError::unsupported(
                PlatformCapability::PrivateNamespaceCollisionScan,
                "retired-action root exceeds the frozen retired-action bound",
            ));
        }
        match CatalogRootRowClassV1::classify(native_ascii_bytes(&name).unwrap_or(&[])) {
            CatalogRootRowClassV1::ActiveAction(action) => {
                reserve_one(&mut actions)?;
                actions.push(action);
            }
            _ => unaccepted_rows += 1,
        }
    }
    actions.sort_unstable_by_key(|action| action.bytes());
    if actions.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(CheckedFsError::ambiguous(
            "retired-action root",
            "multiple native entries resolve to one retired action row",
        ));
    }
    Ok(RetiredRootReadingV1 {
        retired_action_dirs: actions.len(),
        unaccepted_rows,
    })
}

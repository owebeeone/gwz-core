//! Tree-copy contract for the GWZ local clone family.
//!
//! This crate owns the request, report, error and cancellation values of a
//! whole-tree copy and the [`TreeCopier`] port that performs one. It contains
//! no platform implementation: `gwz-refcopy` implements the port with native
//! copy-on-write plus an ordinary fallback, and `gwz-workspace-install`
//! consumes it through `&dyn TreeCopier`.
//!
//! Contract (gwz-dev `dev-docs/GwzLocalCloneLibraryBoundaries.md` §3,
//! `GwzLocalCloneImplementationArchitecture.md` §3):
//!
//! - The destination is a new path or an admitted empty directory. A copier
//!   never hardlinks source files; native copy-on-write may share physical
//!   blocks, but later writes are independent.
//! - Exclusions are applied while traversing, before an entry is copied; an
//!   excluded entry is never written and then removed.
//! - Cancellation is checked between bounded work units (directory entries
//!   and buffered writes). It does not promise preemption of a blocking OS
//!   call. A cancelled or failed copy retains whatever was written; the
//!   error carries the partial report and the failed path.
//! - An unsupported or cross-device native attempt is classified and falls
//!   back to ordinary copying; other native failures are errors.
//! - A successful report says nothing about family readiness or crash
//!   durability. It counts what was copied and how.
//!
//! Every value here is owned plain data (paths, counts, strings). No OS
//! handles, `git2` types, core model errors or protocol types cross this
//! boundary.

#![forbid(unsafe_code)]

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(feature = "contract-tests")]
pub mod contract_tests;

/// Cooperative cancellation port.
///
/// Implementations are polled between bounded work units. Returning `true`
/// makes the copier stop before its next unit and report
/// [`CopyErrorCategory::Cancelled`] with the partial report.
pub trait Cancellation {
    fn is_cancelled(&self) -> bool;
}

/// A cancellation port that never cancels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NeverCancelled;

impl Cancellation for NeverCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// A host-owned cancellation flag; `cancel()` is observed by the next poll.
#[derive(Debug, Default)]
pub struct CancelFlag {
    cancelled: AtomicBool,
}

impl CancelFlag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

impl Cancellation for CancelFlag {
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// Which copy mechanisms the copier may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyMode {
    /// Try the platform's native copy-on-write path per file and fall back
    /// to ordinary copying on a classified unsupported result.
    Auto,
    /// Ordinary read/write copying only; the report's `native_files` is 0.
    OrdinaryOnly,
}

/// One entry excluded from the copy, resolved against the source root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Exclusion {
    /// A source-root-relative path. A directory excludes its whole subtree.
    RelativePath(PathBuf),
}

impl Exclusion {
    /// Whether `relative` (a source-root-relative entry path) is excluded.
    pub fn matches(&self, relative: &Path) -> bool {
        match self {
            Self::RelativePath(excluded) => relative.starts_with(excluded),
        }
    }
}

/// One whole-tree copy. Invocation-local; not a reusable authorization.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyRequest {
    /// Existing directory to copy. Never modified by the copier.
    pub source: PathBuf,
    /// New path, or an existing empty directory admitted by the caller.
    pub destination: PathBuf,
    /// Entries skipped during traversal.
    pub exclusions: Vec<Exclusion>,
    pub mode: CopyMode,
}

impl CopyRequest {
    pub fn is_excluded(&self, relative: &Path) -> bool {
        self.exclusions
            .iter()
            .any(|exclusion| exclusion.matches(relative))
    }
}

/// What a copy did. Counts cover entries actually written to the destination.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CopyReport {
    /// Regular files copied through a native copy-on-write path.
    pub native_files: u64,
    /// Regular files copied by ordinary read/write.
    pub ordinary_files: u64,
    /// Directories created, excluding the destination root itself.
    pub directories: u64,
    /// Symbolic links recreated with their original target.
    pub symlinks: u64,
    /// Logical bytes of every regular file copied (sparse files count their
    /// logical length).
    pub logical_bytes: u64,
    pub warnings: Vec<CopyWarning>,
}

impl CopyReport {
    /// Regular files copied by either mechanism.
    pub fn files(&self) -> u64 {
        self.native_files + self.ordinary_files
    }
}

/// A non-fatal observation about one entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyWarning {
    /// Source-root-relative entry path.
    pub path: PathBuf,
    pub kind: CopyWarningKind,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyWarningKind {
    /// A native attempt was classified unsupported and the entry was copied
    /// by ordinary read/write instead.
    NativeUnsupportedFellBack,
    /// Ancillary metadata (ACLs, extended attributes, alternate streams) was
    /// not copied for this entry.
    AncillaryMetadataUnsupported,
}

/// Why a copy stopped. The category is the machine-readable part; `detail`
/// is diagnostic text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyErrorCategory {
    /// The cancellation port reported cancellation between work units.
    Cancelled,
    /// The source path does not exist.
    SourceMissing,
    /// The source or one of its entries could not be read.
    SourceUnreadable,
    /// The destination exists and is not an empty directory.
    DestinationNotEmpty,
    /// The destination or one of its entries could not be created/written.
    DestinationUnwritable,
    /// An entry type the copier does not copy (FIFO, socket, device, other).
    UnsupportedEntry,
    /// A write returned fewer bytes than requested and could not complete.
    ShortWrite,
    /// Required metadata (mode, symlink target) could not be applied.
    MetadataFailed,
    /// Another I/O failure.
    Io,
    /// This copier performs no copies; nothing was written.
    Unimplemented,
}

/// A failed copy. The destination retains the entries counted in `partial`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyError {
    /// The entry that failed (source-root-relative), or the source/destination
    /// root for request-level refusals.
    pub failed_path: PathBuf,
    pub category: CopyErrorCategory,
    pub detail: String,
    /// Work completed before the failure.
    pub partial: CopyReport,
}

impl CopyError {
    /// A request-level refusal that wrote nothing.
    pub fn refused(
        path: impl Into<PathBuf>,
        category: CopyErrorCategory,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            failed_path: path.into(),
            category,
            detail: detail.into(),
            partial: CopyReport::default(),
        }
    }
}

impl fmt::Display for CopyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "copy failed at {}: {:?}: {}",
            self.failed_path.display(),
            self.category,
            self.detail
        )
    }
}

impl std::error::Error for CopyError {}

/// The tree-copy port.
///
/// Call order: one call per copy; the copier owns no state across calls.
/// Resource bounds: the copier holds at most one open source and one open
/// destination file plus a bounded buffer per in-flight entry. Error
/// mapping: every failure is a [`CopyError`] whose `partial` report is
/// accurate for the destination's contents at return.
pub trait TreeCopier {
    fn copy_tree(
        &self,
        request: &CopyRequest,
        cancellation: &dyn Cancellation,
    ) -> Result<CopyReport, CopyError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusion_matches_the_entry_and_its_subtree_only() {
        let exclusion = Exclusion::RelativePath(PathBuf::from(".gwz/merge"));
        assert!(exclusion.matches(Path::new(".gwz/merge")));
        assert!(exclusion.matches(Path::new(".gwz/merge/open")));
        assert!(!exclusion.matches(Path::new(".gwz/merge-notes")));
        assert!(!exclusion.matches(Path::new(".gwz")));
    }

    #[test]
    fn cancel_flag_is_observed_after_cancel() {
        let flag = CancelFlag::new();
        assert!(!flag.is_cancelled());
        flag.cancel();
        assert!(flag.is_cancelled());
        assert!(!NeverCancelled.is_cancelled());
    }

    #[test]
    fn refused_error_carries_an_empty_partial_report() {
        let error = CopyError::refused("dest", CopyErrorCategory::DestinationNotEmpty, "x");
        assert_eq!(error.partial, CopyReport::default());
        assert_eq!(error.partial.files(), 0);
        assert!(error.to_string().contains("DestinationNotEmpty"));
    }
}

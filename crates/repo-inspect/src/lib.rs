//! `gwz-repo-inspect`: local Git/filesystem reads (lane I).
//!
//! [`LocalRepoInspector`] implements `gwz_repo_contract::RepoInspector` for
//! one admitted repository path, and [`LocalObjectReader`] implements
//! `gwz_repo_contract::ObjectReader` over that repository's object store,
//! following gwz-dev `dev-docs/GwzLocalCloneImplementationArchitecture.md`
//! §4: typed object ids in the repository's format, design §4.0 layout
//! hazards (gitfiles, alternates, external common dirs, escaping metadata
//! and configuration including relative hook paths), physical observation
//! of status-suppressed paths, and no implicit fetch, index rewrite, flag
//! clearing or maintenance.
//!
//! # What "read-only" means here
//!
//! Every call opens the repository with libgit2, reads, and closes it.
//! Nothing writes: status is taken with the index refresh and the index
//! update both disabled, physical comparison hashes worktree bytes without
//! storing them, and no ref, reflog or object is created. The crate's own
//! `read_only` test walks the whole fixture tree before and after a full
//! round of calls and compares contents and modification times.
//!
//! # Spellings this crate fixes
//!
//! The contract leaves two spellings open; this implementation pins them and
//! documents them here rather than in a comment on one call site.
//!
//! - `RepositoryInfo::path`, `git_dir` and `common_dir` are **resolved**
//!   paths (`std::fs::canonicalize`), so a boundary comparison and an
//!   equality comparison mean the same thing on a platform where the
//!   temporary directory is itself a symlink.
//! - `HeadState::{Attached, Unborn}::branch` carries the **full** reference
//!   name (`refs/heads/main`), matching `RootSource::Ref { name }`.
//!
//! # Not implemented here
//!
//! Enumerating the nested repositories inside a tree (architecture §4, "scan
//! **all included repositories**") is the caller's traversal: the contract's
//! `inspect_layout` takes one path. This crate answers "is *this* repository
//! copyable", once per repository the caller finds.

#![forbid(unsafe_code)]

use std::cell::RefCell;
use std::fmt;
use std::path::{Path, PathBuf};

use gwz_repo_contract::{
    LayoutError, ObjectFormat, ObjectId, ObjectReader, ObjectRecord, Observation, ProtectedRoot,
    ProtectedRoots, ReadError, ReadLimits, RepoInspector, RepositoryInfo, WorkObservation,
};

mod config_scan;
mod environment;
mod history;
mod layout;
mod objects;
mod oid;
mod paths;
mod work;

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod tests;

pub use environment::Environment;

/// Inspects repositories on the local filesystem.
///
/// The inspector carries two pieces of caller context that the contract's
/// method signatures do not:
///
/// - the [`Environment`] view used for the design §4.0 environment-override
///   refusal, so a test (and a caller inspecting on behalf of a different
///   invocation) states it instead of mutating process-global state;
/// - the object ids named by decoded GWZ coordination records, which core
///   decodes and hands over (see
///   [`LocalRepoInspector::with_coordination_roots`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocalRepoInspector {
    environment: Environment,
    coordination_roots: Vec<CoordinationRoot>,
}

/// One Git object a decoded GWZ coordination record names (design §5.1: "GWZ
/// stash coordination records additionally need surviving interpretable
/// copies and their referenced Git objects; otherwise refuse").
///
/// This crate does not decode records — core does, and hands the ids over.
/// `record` names the record (`"stash gwz_stash_0007"`) and `object` the role
/// the id plays in it (`"base"`, `"index"`, `"worktree"`, `"untracked"`),
/// exactly as `RootSource::CoordinationRecord` spells them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoordinationRoot {
    pub record: String,
    pub object: String,
    pub oid: ObjectId,
}

impl LocalRepoInspector {
    /// An inspector that reads the process environment for the design §4.0
    /// redirecting variables.
    pub fn new() -> Self {
        Self::default()
    }

    /// An inspector with an explicit environment view.
    pub fn with_environment(environment: Environment) -> Self {
        Self {
            environment,
            coordination_roots: Vec::new(),
        }
    }

    /// The Git objects decoded GWZ coordination records name in this
    /// repository. `inventory_history` reports each as a
    /// `RootSource::CoordinationRecord` root, so a history check verifies it
    /// like any other named root.
    pub fn with_coordination_roots(
        mut self,
        roots: impl IntoIterator<Item = CoordinationRoot>,
    ) -> Self {
        self.coordination_roots = roots.into_iter().collect();
        self
    }
}

impl RepoInspector for LocalRepoInspector {
    fn inspect_layout(&self, path: &Path) -> Result<RepositoryInfo, LayoutError> {
        layout::inspect_layout(&self.environment, path)
    }

    fn observe_work(&self, repository: &RepositoryInfo) -> Observation<WorkObservation> {
        work::observe_work(repository)
    }

    fn inventory_history(&self, repository: &RepositoryInfo) -> Observation<ProtectedRoots> {
        history::inventory_history(repository, &self.coordination_roots)
    }
}

/// Bounded reads of one local repository's object store.
///
/// The repository handle is opened on first use and kept for the reader's
/// lifetime: `gwz-history-check` walks object graphs one `read_object` at a
/// time, and reopening the repository per object would dominate the walk.
/// [`Clone`] yields an equivalent, not-yet-opened reader; equality is over
/// the repository the reader serves, not over the cached handle.
pub struct LocalObjectReader {
    repository: PathBuf,
    git_dir: PathBuf,
    object_format: ObjectFormat,
    opened: RefCell<Option<git2::Repository>>,
}

impl LocalObjectReader {
    /// `repository` is an admitted repository path (from `inspect_layout`).
    pub fn open(repository: &RepositoryInfo) -> Self {
        Self {
            repository: repository.path.clone(),
            git_dir: repository.git_dir.clone(),
            object_format: repository.object_format,
            opened: RefCell::new(None),
        }
    }

    pub fn repository(&self) -> &Path {
        &self.repository
    }
}

impl Clone for LocalObjectReader {
    fn clone(&self) -> Self {
        Self {
            repository: self.repository.clone(),
            git_dir: self.git_dir.clone(),
            object_format: self.object_format,
            opened: RefCell::new(None),
        }
    }
}

impl fmt::Debug for LocalObjectReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalObjectReader")
            .field("repository", &self.repository)
            .field("git_dir", &self.git_dir)
            .field("object_format", &self.object_format)
            .finish_non_exhaustive()
    }
}

impl PartialEq for LocalObjectReader {
    fn eq(&self, other: &Self) -> bool {
        self.repository == other.repository
            && self.git_dir == other.git_dir
            && self.object_format == other.object_format
    }
}

impl Eq for LocalObjectReader {}

impl ObjectReader for LocalObjectReader {
    fn retained_roots(&self) -> Result<ProtectedRoots, ReadError> {
        self.with_repository(|repository| {
            history::retained_roots(repository, self.object_format)
                .map_err(|detail| ReadError::ReadFailed { detail })
        })
    }

    fn read_object(&self, oid: &ObjectId, limits: &ReadLimits) -> Result<ObjectRecord, ReadError> {
        if oid.format() != self.object_format {
            return Err(ReadError::ReadFailed {
                detail: format!(
                    "{oid:?} is not in this repository's {:?} object format",
                    self.object_format
                ),
            });
        }
        self.with_repository(|repository| {
            objects::read_object(repository, self.object_format, oid, limits)
        })
    }
}

impl LocalObjectReader {
    fn with_repository<T>(
        &self,
        action: impl FnOnce(&git2::Repository) -> Result<T, ReadError>,
    ) -> Result<T, ReadError> {
        let mut slot = self.opened.borrow_mut();
        if slot.is_none() {
            let repository = git2::Repository::open_ext(
                &self.git_dir,
                git2::RepositoryOpenFlags::NO_SEARCH | git2::RepositoryOpenFlags::NO_DOTGIT,
                std::iter::empty::<&std::ffi::OsStr>(),
            )
            .map_err(|error| ReadError::ReadFailed {
                detail: format!("{}: {}", self.git_dir.display(), error.message()),
            })?;
            *slot = Some(repository);
        }
        action(slot.as_ref().expect("just opened"))
    }
}

/// Sort and de-duplicate protected roots so two observations of one
/// repository compare equal regardless of the order Git enumerated its refs.
pub(crate) fn normalise_roots(mut roots: Vec<ProtectedRoot>) -> ProtectedRoots {
    roots.sort_by(|left, right| {
        source_key(&left.source)
            .cmp(&source_key(&right.source))
            .then_with(|| left.oid.cmp(&right.oid))
    });
    roots.dedup();
    // The contract's per-root channel (`unknown`, I-2, LCM1.0c-fu3) stays
    // empty here: `history.rs` still reports an unreadable root as a
    // whole-observation `Unknown` until the consumers honour an incomplete
    // inventory (checkpoint §12).
    ProtectedRoots {
        roots,
        unknown: Vec::new(),
    }
}

fn source_key(source: &gwz_repo_contract::RootSource) -> (u8, String, u64) {
    use gwz_repo_contract::RootSource as Source;
    match source {
        Source::Head => (0, String::new(), 0),
        Source::Ref { name } => (1, name.clone(), 0),
        Source::AnnotatedTag { name } => (2, name.clone(), 0),
        Source::Stash { index } => (3, String::new(), *index),
        Source::Reflog { reference, index } => (4, reference.clone(), *index),
        Source::CoordinationRecord { record, object } => (5, format!("{record}\u{0}{object}"), 0),
        Source::Other { detail } => (6, detail.clone(), 0),
    }
}

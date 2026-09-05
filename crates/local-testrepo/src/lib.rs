//! `gwz-local-testrepo`: tiny real Git and temporary-tree fixtures (lane T).
//!
//! **Development only.** This crate exists so the real-I/O adapters of the
//! local clone family — `gwz-repo-inspect`'s inspector and object reader,
//! `gwz-family-store`'s filesystem store, `gwz-refcopy`'s copier, the
//! anonymous local transport — can be tested against small *real*
//! repositories instead of against a mock of Git. It is classified `harness`
//! in `scripts/checks/local_clone_inventory.json`, and the boundary gate
//! refuses a normal (non-dev) dependency on a harness package, so nothing
//! here can become a production dependency of any crate. Do not reach for it
//! from library code, and do not add product behaviour to it.
//!
//! Scope (gwz-dev `dev-docs/GwzLocalCloneLibraryBoundaries.md` §2):
//! "`local-testrepo` uses Git library calls, fixed identities/timestamps and
//! small files. Only real-I/O adapter tests use it. Pure-library fast tests
//! use plain values and in-memory graphs, not this I/O harness." Every
//! fixture here is a handful of files and at most a handful of commits, so a
//! consuming suite stays inside its Tier A budget.
//!
//! # Determinism
//!
//! Every commit, tag and stash is written with the fixed identity
//! [`FIXTURE_NAME`] / [`FIXTURE_EMAIL`] at the fixed time
//! [`FIXTURE_TIME_SECONDS`] (offset [`FIXTURE_TIME_OFFSET_MINUTES`]), so the
//! same fixture built twice yields the same object ids and a test may assert
//! on an id. Object ids are [`gwz_repo_contract::ObjectId`] values that carry
//! their [`ObjectFormat`]; both SHA-1 and SHA-256 repositories are
//! constructible ([`RepoSpec::sha256`]).
//!
//! Each repository is initialised with `external_template(false)` and
//! immediately given the local configuration that would otherwise be read
//! from the developer's `~/.gitconfig` (`core.autocrlf`, `core.fileMode`,
//! `core.excludesFile`, `core.attributesFile`, `commit.gpgsign`, `gc.auto`),
//! so an ambient global configuration cannot change a fixture's object ids.
//! Isolation is per repository and safe: this crate never mutates libgit2's
//! process-global configuration search path, so it cannot disturb a
//! consumer's other tests. A global setting this list does not name — a
//! global `include.path`, say — is still visible to a reader that resolves
//! *effective* configuration; a test that cares must assert on the repository
//! keys it set.
//!
//! # Shape
//!
//! - [`TempTree`] owns a `tempfile::TempDir` and hands out paths, files,
//!   directories and repositories inside it. Drop it and the tree is gone.
//! - [`TestRepo`] is a *handle* to one repository on disk: a path, a
//!   [`RepoSpec`] and nothing else. Its methods open the repository, change
//!   the filesystem and close it again, so they take `&self` — the value
//!   itself is immutable and several handles to the same tree are fine.
//! - [`TestWorkspace`] is a root repository plus members at given relative
//!   paths, keyed by [`gwz_repo_contract::RepoKey`]: the multi-member layout
//!   the create path copies (`dev-docs/GwzLocalCloneDesign.md` §4).
//! - `hazard_*` methods build one design §4.0 source-layout hazard each;
//!   `work_*` methods build one design §5.1 work state each. The name says
//!   which.
//!
//! Every path this crate hands out is **canonical** — fully resolved, with no
//! trailing separator — because that is the form libgit2 reports. So
//! `repo.git_dir() == repo.path().join(".git")` holds, and a consumer can
//! compare a `RepositoryInfo`'s paths against the fixture's own without a
//! platform-specific dance (on macOS a temporary directory is reached through
//! the `/var` symlink, and `/private/var` is what Git says).
//!
//! # Failure
//!
//! A fixture that cannot be built **panics** with the failing operation and
//! path. Fixtures are test scaffolding: a broken one is a defect in the test,
//! not a condition under test, and returning a `Result` from every builder
//! would only move the `unwrap` to the caller.

#![forbid(unsafe_code)]
// Ordinary file I/O in a *test fixture* builder: gwz-core's disallowed writers
// (gwz-core/clippy.toml) route **merge artifact** mutation through checked
// entries. This crate is outside that boundary — it never writes a checked
// artifact, and it is never linked into a product build at all.
#![allow(clippy::disallowed_methods)]

mod hazards;
mod repo;
mod work;
mod workspace;

#[cfg(test)]
mod tests;

use std::fs;
use std::path::{Path, PathBuf};

pub use gwz_repo_contract::{NativeOperation, ObjectFormat, RepoKey, SuppressionFlag};
pub use repo::{RepoSpec, TestRepo, modes};
pub use workspace::TestWorkspace;

#[cfg(any(test, feature = "contract-tests"))]
pub use gwz_repo_contract::contract_tests::GraphFixture;

/// The one author/committer name every fixture commit carries.
pub const FIXTURE_NAME: &str = "GWZ Fixture";
/// The one author/committer address every fixture commit carries. `.invalid`
/// is reserved by RFC 2606 and can never be routed.
pub const FIXTURE_EMAIL: &str = "fixture@example.invalid";
/// Seconds since the epoch stamped on every fixture commit, tag and stash.
pub const FIXTURE_TIME_SECONDS: i64 = 1_700_000_000;
/// Timezone offset in minutes stamped on every fixture commit (UTC).
pub const FIXTURE_TIME_OFFSET_MINUTES: i32 = 0;
/// The branch a fixture repository starts on unless [`RepoSpec::branch`] says
/// otherwise.
pub const FIXTURE_BRANCH: &str = "main";
/// The retained import namespace of `dev-docs/GwzLocalCloneDesign.md` §6.2;
/// [`TestRepo::import_ref`] fetches into `<this>/<transfer-id>`.
pub const IMPORT_REF_PREFIX: &str = "refs/gwz/local-imports";

/// The fixed signature every fixture commit, tag and stash is written with.
///
/// Exposed because a consuming real-I/O test that drives `git2` itself needs
/// the *same* identity to keep its own commits deterministic. `git2` types
/// appear in this crate's API only here and in [`TestRepo::open`]; that is a
/// dev-harness convenience, not a contract surface — the contract crates
/// expose no `git2` type (boundaries §2).
pub fn fixture_signature() -> git2::Signature<'static> {
    git2::Signature::new(
        FIXTURE_NAME,
        FIXTURE_EMAIL,
        &git2::Time::new(FIXTURE_TIME_SECONDS, FIXTURE_TIME_OFFSET_MINUTES),
    )
    .expect("fixed fixture signature is well formed")
}

/// A private temporary directory that removes itself when dropped.
///
/// Everything a fixture writes lives under [`TempTree::path`]; a test that
/// wants a path *outside* the copied boundary (an alternates object store, an
/// escaping hook directory) makes it with [`TempTree::dir`] under a sibling
/// name and passes it to the hazard constructor.
#[derive(Debug)]
pub struct TempTree {
    inner: tempfile::TempDir,
    path: PathBuf,
}

impl TempTree {
    /// A fresh temporary tree whose directory name carries `label`, so a
    /// leaked directory says which test made it.
    pub fn new(label: &str) -> Self {
        let inner = tempfile::Builder::new()
            .prefix(&format!("gwz-testrepo-{label}-"))
            .tempdir()
            .expect("create temporary tree");
        // Canonical from the start: libgit2 reports canonical paths, and a
        // fixture whose own paths did not match them would make every
        // `git_dir == path.join(".git")` assertion in a consuming test fail
        // for a reason that has nothing to do with the code under test.
        let path = canonical(inner.path());
        Self { inner, path }
    }

    /// The root of the temporary tree, canonicalised.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `relative` resolved against the tree root. Creates nothing.
    pub fn join(&self, relative: &str) -> PathBuf {
        self.path().join(relative)
    }

    /// Create (including parents) and return a directory in the tree.
    pub fn dir(&self, relative: &str) -> PathBuf {
        let path = self.join(relative);
        create_dir_all(&path);
        path
    }

    /// Create (including parents) and return a file in the tree.
    pub fn file(&self, relative: &str, contents: &[u8]) -> PathBuf {
        let path = self.join(relative);
        write_file(&path, contents);
        path
    }

    /// A non-bare SHA-1 repository at `relative` with no commit yet.
    pub fn repo(&self, relative: &str) -> TestRepo {
        TestRepo::init(&self.join(relative), &RepoSpec::new())
    }

    /// A bare SHA-1 repository at `relative` — the design's `--bare` hub
    /// shape (`dev-docs/GwzLocalCloneDesign.md` §4.3).
    pub fn bare_repo(&self, relative: &str) -> TestRepo {
        TestRepo::init(&self.join(relative), &RepoSpec::new().bare())
    }

    /// A repository at `relative` built to `spec`.
    pub fn repo_with(&self, relative: &str, spec: &RepoSpec) -> TestRepo {
        TestRepo::init(&self.join(relative), spec)
    }

    /// A workspace at `relative`: a root repository plus one member
    /// repository per relative path in `members`.
    pub fn workspace(&self, relative: &str, members: &[&str]) -> TestWorkspace {
        TestWorkspace::init(&self.join(relative), members, &RepoSpec::new())
    }

    /// Keep the tree on disk and return its path, for debugging a failing
    /// fixture. Nothing removes it afterwards.
    pub fn keep(self) -> PathBuf {
        self.inner.keep()
    }
}

/// `fs::create_dir_all` with the failing path in the panic message.
pub(crate) fn create_dir_all(path: &Path) {
    fs::create_dir_all(path).unwrap_or_else(|error| {
        panic!("create directory {}: {error}", path.display());
    });
}

/// `fs::write`, creating parents, with the failing path in the panic message.
pub(crate) fn write_file(path: &Path, contents: &[u8]) {
    if let Some(parent) = path.parent() {
        create_dir_all(parent);
    }
    fs::write(path, contents).unwrap_or_else(|error| {
        panic!("write {}: {error}", path.display());
    });
}

/// A path in the form libgit2 reports: fully resolved, with no trailing
/// separator and no `.` component, so a fixture's own paths compare equal to
/// the ones a repository hands back.
pub(crate) fn canonical(path: &Path) -> PathBuf {
    let resolved = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    resolved.components().collect()
}

/// `fs::read` with the failing path in the panic message.
pub(crate) fn read_file(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

/// The `git2` object format for a contract [`ObjectFormat`].
pub(crate) fn git_format(format: ObjectFormat) -> git2::ObjectFormat {
    match format {
        ObjectFormat::Sha1 => git2::ObjectFormat::Sha1,
        ObjectFormat::Sha256 => git2::ObjectFormat::Sha256,
    }
}

/// The contract [`ObjectFormat`] for a `git2` object format.
pub(crate) fn contract_format(format: git2::ObjectFormat) -> ObjectFormat {
    match format {
        git2::ObjectFormat::Sha1 => ObjectFormat::Sha1,
        git2::ObjectFormat::Sha256 => ObjectFormat::Sha256,
    }
}

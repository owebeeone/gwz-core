//! Small real repositories built with `git2` and `tempfile`, for this crate's
//! own tests.
//!
//! These are deliberately in-crate and dev-only. Lane T owns
//! `crates/local-testrepo`; the builders here that other lanes will also need
//! (a checkout, a bare repository, both object formats, a commit, a raw
//! configuration append and the symlink helpers) are the ones to migrate
//! there once the workspace lock may change. Nothing in this module is
//! compiled into the library.

#![cfg(test)]
// Fixture setup builds real repositories; the merge-path writer boundary
// (gwz-core `clippy.toml`) governs production merge code, not test scaffolding.
#![allow(clippy::disallowed_methods)]

use std::fs;
use std::path::{Path, PathBuf};

use git2::{ObjectFormat, Oid, Repository, RepositoryInitOptions, Signature};
use tempfile::TempDir;

/// A repository under a private temporary directory. `root` is the worktree
/// root of a checkout, or the Git directory of a bare repository — the path
/// `inspect_layout` is given.
pub(crate) struct Fixture {
    temp: TempDir,
    root: PathBuf,
    bare: bool,
}

impl Fixture {
    /// A checkout at `<temp>/work` with one commit on `refs/heads/main`.
    pub(crate) fn checkout(format: ObjectFormat) -> Self {
        let fixture = Self::empty_checkout(format);
        fixture.write("tracked.txt", b"one\n");
        fixture.commit("initial");
        fixture
    }

    /// A checkout with no commit yet: `HEAD` is unborn.
    pub(crate) fn empty_checkout(format: ObjectFormat) -> Self {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("work");
        let mut options = RepositoryInitOptions::new();
        options
            .initial_head("main")
            .object_format(format)
            .mkpath(true);
        Repository::init_opts(&root, &options).expect("init checkout");
        Self {
            temp,
            root,
            bare: false,
        }
    }

    /// A bare repository at `<temp>/hub.git`, empty by default.
    pub(crate) fn bare(format: ObjectFormat) -> Self {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("hub.git");
        let mut options = RepositoryInitOptions::new();
        options
            .bare(true)
            .initial_head("main")
            .object_format(format)
            .mkpath(true);
        Repository::init_opts(&root, &options).expect("init bare");
        Self {
            temp,
            root,
            bare: true,
        }
    }

    /// The path handed to `inspect_layout`.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// The canonical form of [`Fixture::root`], which `RepositoryInfo::path`
    /// carries.
    pub(crate) fn real_root(&self) -> PathBuf {
        fs::canonicalize(&self.root).expect("canonical root")
    }

    /// The temporary directory containing (but outside) the repository. Used
    /// to build the "outside the copied boundary" side of a hazard.
    pub(crate) fn outside(&self) -> &Path {
        self.temp.path()
    }

    pub(crate) fn git_dir(&self) -> PathBuf {
        if self.bare {
            self.real_root()
        } else {
            self.real_root().join(".git")
        }
    }

    pub(crate) fn open(&self) -> Repository {
        Repository::open(&self.root).expect("open fixture")
    }

    pub(crate) fn write(&self, relative: &str, bytes: &[u8]) -> PathBuf {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent");
        }
        fs::write(&path, bytes).expect("write");
        path
    }

    pub(crate) fn mkdir(&self, relative: &str) -> PathBuf {
        let path = self.root.join(relative);
        fs::create_dir_all(&path).expect("mkdir");
        path
    }

    /// Append raw text to the repository's own `config`, so the test controls
    /// the exact spelling a user would have written.
    pub(crate) fn append_config(&self, text: &str) {
        let config = self.git_dir().join("config");
        let mut existing = fs::read_to_string(&config).unwrap_or_default();
        existing.push_str(text);
        if !existing.ends_with('\n') {
            existing.push('\n');
        }
        fs::write(&config, existing).expect("append config");
    }

    /// Stage every worktree change and commit it on the current branch.
    pub(crate) fn commit(&self, message: &str) -> Oid {
        let repository = self.open();
        let mut index = repository.index().expect("index");
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .expect("add all");
        index.write().expect("write index");
        let tree_id = index.write_tree().expect("write tree");
        let tree = repository.find_tree(tree_id).expect("tree");
        let who = Signature::now("Fixture", "fixture@example.invalid").expect("signature");
        let parents: Vec<_> = match repository.head() {
            Ok(head) => vec![head.peel_to_commit().expect("head commit")],
            Err(_) => Vec::new(),
        };
        let borrowed: Vec<_> = parents.iter().collect();
        repository
            .commit(Some("HEAD"), &who, &who, message, &tree, &borrowed)
            .expect("commit")
    }

    /// The commit `HEAD` resolves to.
    pub(crate) fn head_commit(&self) -> Oid {
        self.open()
            .head()
            .expect("HEAD")
            .peel_to_commit()
            .expect("head commit")
            .id()
    }

    /// Detach `HEAD` at its current commit.
    pub(crate) fn detach_head(&self) {
        let commit = self.head_commit();
        self.open().set_head_detached(commit).expect("detach HEAD");
    }

    /// A sibling directory whose `.git` is a **file** naming this
    /// repository's Git directory — the gitfile / linked-worktree shape
    /// design §4.0 refuses.
    pub(crate) fn linked_gitfile(&self, name: &str) -> PathBuf {
        let linked = self.temp.path().join(name);
        fs::create_dir_all(&linked).expect("linked directory");
        fs::write(
            linked.join(".git"),
            format!("gitdir: {}\n", self.git_dir().display()),
        )
        .expect("gitfile");
        linked
    }
}

#[cfg(unix)]
pub(crate) fn symlink(target: &Path, link: &Path) {
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent).expect("link parent");
    }
    std::os::unix::fs::symlink(target, link).expect("symlink");
}

/// A symlink that points at itself: resolution is impossible, not merely
/// absent.
#[cfg(unix)]
pub(crate) fn symlink_loop(link: &Path) {
    symlink(link, link);
}

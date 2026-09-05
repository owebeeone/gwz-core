//! The multi-member workspace layout the create path copies.
//!
//! `dev-docs/GwzLocalCloneDesign.md` §4 copies a whole workspace: the root
//! repository, every materialized member at its relative path, and any
//! unmanaged or ignored nested repository found on the way — §4.0 requires
//! *every* one of them to be inventoried before reservation. This is that
//! shape, and nothing more: it builds the **Git** side of a workspace, not
//! GWZ's own `gwz.conf/` or `.gwz/` metadata, which core's own handlers own
//! and a fixture must not shadow.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gwz_repo_contract::{ObjectId, RepoKey};

use crate::{RepoSpec, TestRepo, create_dir_all};

/// A workspace root repository plus members at given relative paths, keyed by
/// [`RepoKey`] — the identity the family pairs repositories by, never a path.
#[derive(Debug)]
pub struct TestWorkspace {
    path: PathBuf,
    repos: BTreeMap<RepoKey, TestRepo>,
    nested: Vec<(String, TestRepo)>,
}

impl TestWorkspace {
    /// Build the workspace at `path`: a root repository there, and one member
    /// repository per relative path in `members`, all to `spec`.
    pub fn init(path: &Path, members: &[&str], spec: &RepoSpec) -> Self {
        create_dir_all(path);
        let mut repos = BTreeMap::new();
        repos.insert(RepoKey::Root, TestRepo::init(path, spec));
        for member in members {
            repos.insert(
                RepoKey::Member {
                    id: (*member).to_owned(),
                },
                TestRepo::init(&path.join(member), spec),
            );
        }
        Self {
            path: path.to_path_buf(),
            repos,
            nested: Vec::new(),
        }
    }

    /// The workspace root directory.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The root repository.
    pub fn root(&self) -> &TestRepo {
        &self.repos[&RepoKey::Root]
    }

    /// The member repository at `relative`.
    pub fn member(&self, relative: &str) -> &TestRepo {
        self.repos
            .get(&RepoKey::Member {
                id: relative.to_owned(),
            })
            .unwrap_or_else(|| panic!("{relative} is not a member of this workspace"))
    }

    /// The repository for `key`, if the workspace has one.
    pub fn get(&self, key: &RepoKey) -> Option<&TestRepo> {
        self.repos.get(key)
    }

    /// Every managed repository, root first then members in key order.
    pub fn repos(&self) -> impl Iterator<Item = (&RepoKey, &TestRepo)> {
        self.repos.iter()
    }

    /// Every member's relative path, in key order.
    pub fn member_names(&self) -> Vec<String> {
        self.repos
            .keys()
            .filter_map(|key| match key {
                RepoKey::Member { id } => Some(id.clone()),
                RepoKey::Root => None,
            })
            .collect()
    }

    /// An **unmanaged nested repository** at `relative`: a real repository
    /// inside the copied tree that the workspace does not manage. It is not a
    /// member and gets no [`RepoKey`]; design §4.0 still inventories it.
    pub fn nested_repo(&mut self, relative: &str, spec: &RepoSpec) -> &TestRepo {
        let repo = TestRepo::init(&self.path.join(relative), spec);
        self.nested.push((relative.to_owned(), repo));
        &self.nested.last().expect("just pushed").1
    }

    /// Every unmanaged nested repository, in creation order.
    pub fn nested_repos(&self) -> impl Iterator<Item = (&str, &TestRepo)> {
        self.nested
            .iter()
            .map(|(relative, repo)| (relative.as_str(), repo))
    }

    /// Create (including parents) and return a directory in the workspace —
    /// a `target/` the copy is expected to exclude, say.
    pub fn dir(&self, relative: &str) -> PathBuf {
        let path = self.path.join(relative);
        create_dir_all(&path);
        path
    }

    /// Create (including parents) and return a file in the workspace.
    pub fn file(&self, relative: &str, contents: &[u8]) -> PathBuf {
        let path = self.path.join(relative);
        crate::write_file(&path, contents);
        path
    }

    /// Give every managed repository one commit holding a single `README`
    /// naming its key, so each repository has distinct — and deterministic —
    /// history. Returns the commit id per key.
    pub fn commit_all(&self, message: &str) -> BTreeMap<RepoKey, ObjectId> {
        self.repos
            .iter()
            .map(|(key, repo)| {
                let contents = format!("fixture {key}\n");
                let commit = repo.commit_files(message, &[("README", contents.as_bytes())]);
                (key.clone(), commit)
            })
            .collect()
    }
}

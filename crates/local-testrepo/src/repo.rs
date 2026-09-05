//! One repository fixture: construction, content, refs, transfer, observation.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use git2::{ObjectType, Repository, RepositoryInitOptions};
use gwz_repo_contract::{
    HeadState, ObjectFormat, ObjectId, ObjectKind, ObjectRecord, ProtectedRoot, ProtectedRoots,
    RootSource,
};

use crate::{
    FIXTURE_BRANCH, IMPORT_REF_PREFIX, canonical, contract_format, create_dir_all,
    fixture_signature, git_format,
};

/// Git's mode for an ordinary file; the tree entry mode a fixture writes
/// unless it is deliberately building a mode or link change.
pub(crate) const FILE_MODE: u32 = 0o100_644;
/// Git's mode for an executable file.
pub(crate) const EXECUTABLE_MODE: u32 = 0o100_755;
/// Git's mode for a symbolic link.
pub(crate) const LINK_MODE: u32 = 0o120_000;

/// What kind of repository [`TestRepo::init`] builds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoSpec {
    /// A bare repository (the design's `--bare` hub, §4.3): no worktree, and
    /// the repository path *is* the Git directory.
    pub bare: bool,
    /// The object store's hash function. Both are constructible; a SHA-256
    /// fixture is what proves a consumer never assumed 40 hex characters.
    pub format: ObjectFormat,
    /// The branch `HEAD` starts on.
    pub branch: String,
}

impl RepoSpec {
    /// A non-bare SHA-1 repository on [`FIXTURE_BRANCH`].
    pub fn new() -> Self {
        Self {
            bare: false,
            format: ObjectFormat::Sha1,
            branch: FIXTURE_BRANCH.to_owned(),
        }
    }

    /// Build a bare repository.
    pub fn bare(mut self) -> Self {
        self.bare = true;
        self
    }

    /// Build a SHA-256 repository.
    pub fn sha256(mut self) -> Self {
        self.format = ObjectFormat::Sha256;
        self
    }

    /// Build a repository with `format`'s object store.
    pub fn format(mut self, format: ObjectFormat) -> Self {
        self.format = format;
        self
    }

    /// Start `HEAD` on `name` instead of [`FIXTURE_BRANCH`].
    pub fn branch(mut self, name: &str) -> Self {
        self.branch = name.to_owned();
        self
    }
}

impl Default for RepoSpec {
    fn default() -> Self {
        Self::new()
    }
}

/// A handle to one repository fixture on disk.
///
/// The value holds a path and a [`RepoSpec`] and nothing else: every method
/// opens the repository, does its work and closes it again. That is why the
/// building methods take `&self` — what they change is the filesystem, not
/// this value — and why a handle stays usable after a `hazard_*` method has
/// rearranged the repository's layout underneath it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestRepo {
    path: PathBuf,
    spec: RepoSpec,
}

impl TestRepo {
    /// Initialise a repository at `path` (creating it and its parents).
    ///
    /// The repository is hermetic by construction: no template directory is
    /// read, and the configuration keys that would otherwise come from the
    /// developer's global Git configuration and change a fixture's object ids
    /// are written locally.
    pub fn init(path: &Path, spec: &RepoSpec) -> Self {
        create_dir_all(path);
        let mut options = RepositoryInitOptions::new();
        options
            .bare(spec.bare)
            .no_reinit(true)
            .mkpath(true)
            .external_template(false)
            .initial_head(&spec.branch)
            .object_format(git_format(spec.format));
        let repository = Repository::init_opts(path, &options)
            .unwrap_or_else(|error| panic!("init repository {}: {error}", path.display()));
        let repo = Self {
            path: canonical(path),
            spec: spec.clone(),
        };
        repo.isolate_configuration(&repository);
        repo
    }

    fn isolate_configuration(&self, repository: &Repository) {
        let neutral = repository
            .path()
            .join("info")
            .join("gwz-fixture-absent")
            .to_string_lossy()
            .into_owned();
        let mut config = repository.config().expect("repository configuration");
        let mut set = |key: &str, value: &str| {
            config
                .set_str(key, value)
                .unwrap_or_else(|error| panic!("set {key}: {error}"));
        };
        set("core.autocrlf", "false");
        set("core.fileMode", if cfg!(unix) { "true" } else { "false" });
        // Absent paths, so a global excludes/attributes file cannot reach a
        // fixture's ignore or filter decisions.
        set("core.excludesFile", &neutral);
        set("core.attributesFile", &neutral);
        set("commit.gpgsign", "false");
        set("tag.gpgsign", "false");
        set("gc.auto", "0");
    }

    // ---- identity -------------------------------------------------------

    /// The path a consumer inspects: the worktree root, or the Git directory
    /// of a bare repository (`RepositoryInfo::path`'s meaning).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// How this repository was built.
    pub fn spec(&self) -> &RepoSpec {
        &self.spec
    }

    /// Whether this is a bare repository.
    pub fn is_bare(&self) -> bool {
        self.spec.bare
    }

    /// The object store's hash function, as the repository reports it.
    pub fn object_format(&self) -> ObjectFormat {
        contract_format(self.open().object_format())
    }

    /// The Git directory, as the repository reports it (so a `hazard_git_file`
    /// fixture reports the relocated directory).
    pub fn git_dir(&self) -> PathBuf {
        canonical(self.open().path())
    }

    /// The common directory, as the repository reports it.
    pub fn common_dir(&self) -> PathBuf {
        canonical(self.open().commondir())
    }

    /// The worktree root, or `None` for a bare repository.
    pub fn workdir(&self) -> Option<PathBuf> {
        self.open().workdir().map(canonical)
    }

    /// Open the repository with `git2`.
    ///
    /// The escape hatch for a consuming real-I/O test that needs a Git
    /// operation this harness does not wrap. `git2` types appear in this
    /// crate's API only here and in [`crate::fixture_signature`]; that is a
    /// dev-harness convenience, not a contract surface.
    ///
    /// Panics if the repository cannot be opened — which a `hazard_*`
    /// constructor may well have arranged on purpose. Use [`Self::try_open`]
    /// on a deliberately broken layout.
    pub fn open(&self) -> Repository {
        self.try_open()
            .unwrap_or_else(|error| panic!("open repository {}: {error}", self.path.display()))
    }

    /// [`Self::open`] without the panic, for a layout a hazard made
    /// unopenable.
    pub fn try_open(&self) -> Result<Repository, git2::Error> {
        Repository::open(&self.path)
    }

    // ---- worktree content ----------------------------------------------

    /// The absolute path of a repository-relative worktree path.
    pub fn worktree_path(&self, relative: &str) -> PathBuf {
        assert!(
            !self.spec.bare,
            "{} is bare: it has no worktree path for {relative}",
            self.path.display()
        );
        self.path.join(relative)
    }

    /// Write a worktree file, creating parent directories.
    pub fn write_file(&self, relative: &str, contents: &[u8]) -> PathBuf {
        let path = self.worktree_path(relative);
        crate::write_file(&path, contents);
        path
    }

    /// Read a worktree file back.
    pub fn read_file(&self, relative: &str) -> Vec<u8> {
        crate::read_file(&self.worktree_path(relative))
    }

    /// Remove a worktree file, leaving the index alone.
    pub fn remove_file(&self, relative: &str) {
        let path = self.worktree_path(relative);
        std::fs::remove_file(&path)
            .unwrap_or_else(|error| panic!("remove {}: {error}", path.display()));
    }

    /// Replace a worktree path with a symbolic link to `target`.
    #[cfg(unix)]
    pub fn write_symlink(&self, relative: &str, target: &Path) -> PathBuf {
        let path = self.worktree_path(relative);
        if let Some(parent) = path.parent() {
            create_dir_all(parent);
        }
        if path.symlink_metadata().is_ok() {
            std::fs::remove_file(&path)
                .unwrap_or_else(|error| panic!("remove {}: {error}", path.display()));
        }
        std::os::unix::fs::symlink(target, &path)
            .unwrap_or_else(|error| panic!("symlink {}: {error}", path.display()));
        path
    }

    /// Set or clear a worktree file's executable bit.
    #[cfg(unix)]
    pub fn set_executable(&self, relative: &str, executable: bool) {
        use std::os::unix::fs::PermissionsExt;
        let path = self.worktree_path(relative);
        let mode = if executable { 0o755 } else { 0o644 };
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
            .unwrap_or_else(|error| panic!("chmod {}: {error}", path.display()));
    }

    // ---- index and commits ---------------------------------------------

    /// Stage one worktree path into the on-disk index.
    pub fn stage(&self, relative: &str) {
        let repository = self.open();
        let mut index = repository.index().expect("repository index");
        index
            .add_path(Path::new(relative))
            .unwrap_or_else(|error| panic!("stage {relative}: {error}"));
        index.write().expect("write index");
    }

    /// Stage every worktree change, respecting ignore rules.
    pub fn stage_all(&self) {
        let repository = self.open();
        let mut index = repository.index().expect("repository index");
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .expect("stage all");
        index.write().expect("write index");
    }

    /// Commit the current on-disk index onto `HEAD`, leaving the worktree
    /// exactly as it is (so deliberate unstaged work survives).
    pub fn commit_index(&self, message: &str) -> ObjectId {
        let repository = self.open();
        let tree = {
            let mut index = repository.index().expect("repository index");
            index.write_tree().expect("write tree from index")
        };
        let parent = head_commit(&repository);
        self.commit_tree(&repository, message, tree, parent, false)
    }

    /// Commit `files` on top of `HEAD`'s tree and refresh the worktree from
    /// the result. Works for bare and non-bare repositories alike, so this is
    /// the way to give a bare hub some history.
    ///
    /// Call it *before* building work states: it force-checks-out the new
    /// commit in a non-bare repository.
    pub fn commit_files(&self, message: &str, files: &[(&str, &[u8])]) -> ObjectId {
        let repository = self.open();
        let parent = head_commit(&repository);
        let base = parent
            .as_ref()
            .map(|commit| commit.tree().expect("parent tree"));
        let tree = self.build_tree(&repository, base.as_ref(), files, &[]);
        self.commit_tree(&repository, message, tree, parent, true)
    }

    /// Commit `HEAD`'s tree with `files` removed from it.
    pub fn commit_removing(&self, message: &str, files: &[&str]) -> ObjectId {
        let repository = self.open();
        let parent = head_commit(&repository);
        let base = parent
            .as_ref()
            .map(|commit| commit.tree().expect("parent tree"));
        let tree = self.build_tree(&repository, base.as_ref(), &[], files);
        self.commit_tree(&repository, message, tree, parent, true)
    }

    fn build_tree(
        &self,
        repository: &Repository,
        base: Option<&git2::Tree<'_>>,
        files: &[(&str, &[u8])],
        removals: &[&str],
    ) -> git2::Oid {
        let mut index =
            git2::Index::new_ext(git_format(self.spec.format)).expect("in-memory index");
        if let Some(tree) = base {
            index.read_tree(tree).expect("read base tree");
        }
        for (path, contents) in files {
            let blob = repository.blob(contents).expect("write blob");
            index
                .add(&index_entry(path, FILE_MODE, blob, contents.len()))
                .unwrap_or_else(|error| panic!("add tree entry {path}: {error}"));
        }
        for path in removals {
            index
                .remove_path(Path::new(path))
                .unwrap_or_else(|error| panic!("remove tree entry {path}: {error}"));
        }
        index.write_tree_to(repository).expect("write tree")
    }

    fn commit_tree(
        &self,
        repository: &Repository,
        message: &str,
        tree: git2::Oid,
        parent: Option<git2::Commit<'_>>,
        checkout: bool,
    ) -> ObjectId {
        let signature = fixture_signature();
        let tree = repository.find_tree(tree).expect("fixture tree");
        let parents: Vec<&git2::Commit<'_>> = parent.iter().collect();
        let commit = repository
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &parents,
            )
            .unwrap_or_else(|error| panic!("commit {message:?}: {error}"));
        if checkout && !self.spec.bare {
            repository
                .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
                .expect("check out the new commit");
        }
        self.oid(commit)
    }

    // ---- refs -----------------------------------------------------------

    /// `HEAD`'s commit id. Panics on an unborn `HEAD`.
    pub fn head_id(&self) -> ObjectId {
        match self.head_state() {
            HeadState::Attached { target, .. } | HeadState::Detached { target } => target,
            HeadState::Unborn { branch } => {
                panic!("{} has an unborn HEAD on {branch}", self.path.display())
            }
        }
    }

    /// What `HEAD` points at, as the contract describes it.
    pub fn head_state(&self) -> HeadState {
        let repository = self.open();
        match repository.head() {
            Ok(reference) => {
                let target = self.oid(reference.target().expect("HEAD resolves to an id"));
                if repository
                    .head_detached()
                    .expect("HEAD detachment is readable")
                {
                    HeadState::Detached { target }
                } else {
                    HeadState::Attached {
                        branch: reference
                            .shorthand()
                            .expect("attached HEAD names a branch")
                            .to_owned(),
                        target,
                    }
                }
            }
            Err(_) => {
                let head = repository.find_reference("HEAD").expect("HEAD exists");
                let branch = head
                    .symbolic_target()
                    .ok()
                    .flatten()
                    .expect("unborn HEAD is symbolic")
                    .trim_start_matches("refs/heads/")
                    .to_owned();
                HeadState::Unborn { branch }
            }
        }
    }

    /// Create or move `refs/heads/<name>` to `target`; returns the full ref
    /// name.
    pub fn branch(&self, name: &str, target: &ObjectId) -> String {
        let repository = self.open();
        let full = format!("refs/heads/{name}");
        // `Repository::reference`, not `Repository::branch`: the latter
        // refuses to force-move the branch `HEAD` is on, which is exactly the
        // move `reset_branch` needs. The reflog message is fixed, like every
        // other identity in this crate.
        repository
            .reference(
                &full,
                self.git_oid(target),
                true,
                &format!("fixture branch {name}"),
            )
            .unwrap_or_else(|error| panic!("branch {name}: {error}"));
        full
    }

    /// Point `HEAD` at `refs/heads/<name>` and check it out.
    pub fn checkout_branch(&self, name: &str) {
        let repository = self.open();
        repository
            .set_head(&format!("refs/heads/{name}"))
            .unwrap_or_else(|error| panic!("set HEAD to {name}: {error}"));
        if !self.spec.bare {
            repository
                .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
                .expect("check out the branch");
        }
    }

    /// Move `refs/heads/<name>` to `target`, leaving the previous tip in the
    /// branch's reflog — the design §5.1 "retained reflog root" shape.
    pub fn reset_branch(&self, name: &str, target: &ObjectId) {
        self.branch(name, target);
        if matches!(self.head_state(), HeadState::Attached { branch, .. } if branch == name) {
            self.checkout_branch(name);
        }
    }

    /// Detach `HEAD` onto `target` (design §5.1 protects a detached `HEAD`).
    pub fn detach_head(&self, target: &ObjectId) {
        let repository = self.open();
        repository
            .set_head_detached(self.git_oid(target))
            .unwrap_or_else(|error| panic!("detach HEAD onto {target}: {error}"));
        if !self.spec.bare {
            repository
                .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
                .expect("check out the detached commit");
        }
    }

    /// Point `HEAD` at an unborn branch, discarding no objects.
    pub fn unborn_head(&self, branch: &str) {
        let repository = self.open();
        repository
            .set_head(&format!("refs/heads/{branch}"))
            .unwrap_or_else(|error| panic!("set HEAD to {branch}: {error}"));
    }

    /// A lightweight tag at `target`; returns the id the tag names (the
    /// target's own id — a lightweight tag is not an object).
    pub fn lightweight_tag(&self, name: &str, target: &ObjectId) -> ObjectId {
        let repository = self.open();
        let object = repository
            .find_object(self.git_oid(target), None)
            .unwrap_or_else(|error| panic!("find {target}: {error}"));
        let id = repository
            .tag_lightweight(name, &object, true)
            .unwrap_or_else(|error| panic!("lightweight tag {name}: {error}"));
        self.oid(id)
    }

    /// An annotated tag at `target`; returns the **tag object's** id, which
    /// design §5.1 protects separately from its target. The tag message is
    /// fixed (`fixture tag <name>`) so the tag object id is deterministic.
    pub fn annotated_tag(&self, name: &str, target: &ObjectId) -> ObjectId {
        let repository = self.open();
        let object = repository
            .find_object(self.git_oid(target), None)
            .unwrap_or_else(|error| panic!("find {target}: {error}"));
        let id = repository
            .tag(
                name,
                &object,
                &fixture_signature(),
                &format!("fixture tag {name}"),
                true,
            )
            .unwrap_or_else(|error| panic!("annotated tag {name}: {error}"));
        self.oid(id)
    }

    /// Append a reflog entry naming `target` to `reference`; returns the
    /// reflog's new length. A commit named by no ref but held by a reflog
    /// entry is a protected root (design §5.1).
    pub fn reflog_entry(&self, reference: &str, target: &ObjectId, message: &str) -> usize {
        let repository = self.open();
        let mut reflog = repository
            .reflog(reference)
            .unwrap_or_else(|error| panic!("reflog {reference}: {error}"));
        reflog
            .append(self.git_oid(target), &fixture_signature(), Some(message))
            .unwrap_or_else(|error| panic!("append reflog {reference}: {error}"));
        reflog.write().expect("write reflog");
        repository.reflog(reference).expect("reread reflog").len()
    }

    /// The id `name` (a full ref name) points at, if it exists.
    pub fn ref_target(&self, name: &str) -> Option<ObjectId> {
        let repository = self.open();
        repository
            .find_reference(name)
            .ok()
            .and_then(|reference| reference.target())
            .map(|id| self.oid(id))
    }

    /// Every full ref name in the repository, sorted.
    pub fn ref_names(&self) -> Vec<String> {
        let repository = self.open();
        let mut names: Vec<String> = repository
            .references()
            .expect("references are readable")
            .filter_map(|reference| {
                reference
                    .ok()
                    .and_then(|reference| reference.name().ok().map(str::to_owned))
            })
            .collect();
        names.sort();
        names
    }

    // ---- remotes and transfer -------------------------------------------

    /// Persist a named remote pointing at `target` (design §6.2 forbids
    /// persisting *family* remotes; an ordinary one is a fixture, not a
    /// family remote).
    pub fn remote(&self, name: &str, target: &TestRepo) {
        let repository = self.open();
        repository
            .remote(name, &path_url(target.path()))
            .unwrap_or_else(|error| panic!("add remote {name}: {error}"));
    }

    /// Remove a named remote — the "and none" case, so a test can assert a
    /// missing-remote refusal.
    pub fn remove_remote(&self, name: &str) {
        let repository = self.open();
        repository
            .remote_delete(name)
            .unwrap_or_else(|error| panic!("delete remote {name}: {error}"));
    }

    /// Every remote name, sorted.
    pub fn remote_names(&self) -> Vec<String> {
        let repository = self.open();
        let mut names: Vec<String> = repository
            .remotes()
            .expect("remotes are readable")
            .iter()
            .filter_map(|name| name.ok().flatten().map(str::to_owned))
            .collect();
        names.sort();
        names
    }

    /// Fetch `refspecs` from `source` over the **anonymous** local transport
    /// — the port shape design §6.2 requires, with no persisted remote.
    pub fn fetch(&self, source: &TestRepo, refspecs: &[&str]) {
        let repository = self.open();
        let mut remote = repository
            .remote_anonymous(&path_url(source.path()))
            .expect("anonymous remote");
        remote
            .fetch(refspecs, None, None)
            .unwrap_or_else(|error| panic!("fetch {refspecs:?}: {error}"));
    }

    /// Push `refspecs` into `target` over the anonymous local transport.
    pub fn push(&self, target: &TestRepo, refspecs: &[&str]) {
        let repository = self.open();
        let mut remote = repository
            .remote_anonymous(&path_url(target.path()))
            .expect("anonymous remote");
        remote
            .push(refspecs, None)
            .unwrap_or_else(|error| panic!("push {refspecs:?}: {error}"));
    }

    /// Import `source_ref` from `source` under the retained import name
    /// `refs/gwz/local-imports/<transfer_id>` (design §6.2); returns the
    /// received id.
    pub fn import_ref(&self, source: &TestRepo, transfer_id: &str, source_ref: &str) -> ObjectId {
        let name = format!("{IMPORT_REF_PREFIX}/{transfer_id}");
        self.fetch(source, &[&format!("+{source_ref}:{name}")]);
        self.ref_target(&name)
            .unwrap_or_else(|| panic!("import {name} was not created"))
    }

    // ---- observation ----------------------------------------------------

    /// The contract id for a `git2` id, in this repository's format.
    pub(crate) fn oid(&self, id: git2::Oid) -> ObjectId {
        ObjectId::from_bytes(self.spec.format, id.as_bytes())
            .unwrap_or_else(|error| panic!("object id {id}: {error}"))
    }

    pub(crate) fn git_oid(&self, id: &ObjectId) -> git2::Oid {
        git2::Oid::from_str_ext(&id.to_hex(), git_format(self.spec.format))
            .unwrap_or_else(|error| panic!("object id {id}: {error}"))
    }

    /// Whether the object store holds `oid`.
    pub fn contains_object(&self, oid: &ObjectId) -> bool {
        self.open()
            .odb()
            .expect("object database")
            .exists(self.git_oid(oid))
    }

    /// An id in this repository's format that the object store does not hold
    /// — the `GraphFixture::missing` case.
    pub fn absent_object_id(&self) -> ObjectId {
        ObjectId::from_bytes(self.spec.format, &vec![0xEE; self.spec.format.digest_len()])
            .expect("digest length matches the format")
    }

    /// One object's kind, size and outgoing edges, exactly as the
    /// `ObjectReader` contract defines them: a commit's edges are its tree
    /// then its parents, a tree's are its entries in tree order, a tag's is
    /// its target, a blob has none.
    pub fn object_record(&self, oid: &ObjectId) -> ObjectRecord {
        let repository = self.open();
        let id = self.git_oid(oid);
        let (size, kind) = repository
            .odb()
            .expect("object database")
            .read_header(id)
            .unwrap_or_else(|error| panic!("read header of {oid}: {error}"));
        let object = repository
            .find_object(id, None)
            .unwrap_or_else(|error| panic!("find {oid}: {error}"));
        let (kind, edges) = match kind {
            ObjectType::Commit => {
                let commit = object.as_commit().expect("commit object");
                let mut edges = vec![self.oid(commit.tree_id())];
                edges.extend(commit.parent_ids().map(|parent| self.oid(parent)));
                (ObjectKind::Commit, edges)
            }
            ObjectType::Tree => {
                let tree = object.as_tree().expect("tree object");
                (
                    ObjectKind::Tree,
                    tree.iter().map(|entry| self.oid(entry.id())).collect(),
                )
            }
            ObjectType::Blob => (ObjectKind::Blob, Vec::new()),
            ObjectType::Tag => {
                let tag = object.as_tag().expect("tag object");
                (ObjectKind::Tag, vec![self.oid(tag.target_id())])
            }
            other => panic!("object {oid} has unexpected kind {other:?}"),
        };
        ObjectRecord {
            oid: oid.clone(),
            kind,
            size: size as u64,
            edges,
        }
    }

    /// Every object reachable from `HEAD`, oldest commit first and, within a
    /// commit, children before their tree and the tree before the commit —
    /// the order `GraphFixture::small` uses (blob, tree, commit). Deduplicated
    /// on first appearance, so the sequence is deterministic.
    pub fn reachable_objects(&self) -> Vec<ObjectRecord> {
        let repository = self.open();
        let mut seen: BTreeSet<ObjectId> = BTreeSet::new();
        let mut records: Vec<ObjectRecord> = Vec::new();
        let Ok(head) = repository.head() else {
            return records;
        };
        let mut walk = repository.revwalk().expect("revwalk");
        walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::REVERSE)
            .expect("revwalk sorting");
        walk.push(head.target().expect("HEAD resolves to an id"))
            .expect("push HEAD");
        for commit in walk {
            let commit = repository
                .find_commit(commit.expect("revwalk entry"))
                .expect("commit object");
            let tree = commit.tree().expect("commit tree");
            self.collect_tree(&repository, &tree, &mut seen, &mut records);
            let oid = self.oid(commit.id());
            if seen.insert(oid.clone()) {
                records.push(self.object_record(&oid));
            }
        }
        records
    }

    fn collect_tree(
        &self,
        repository: &Repository,
        tree: &git2::Tree<'_>,
        seen: &mut BTreeSet<ObjectId>,
        records: &mut Vec<ObjectRecord>,
    ) {
        for entry in tree {
            let oid = self.oid(entry.id());
            if entry.kind() == Some(ObjectType::Tree) {
                let subtree = repository.find_tree(entry.id()).expect("subtree");
                self.collect_tree(repository, &subtree, seen, records);
            }
            if seen.insert(oid.clone()) {
                records.push(self.object_record(&oid));
            }
        }
        let oid = self.oid(tree.id());
        if seen.insert(oid.clone()) {
            records.push(self.object_record(&oid));
        }
    }

    /// The roots this fixture created: `HEAD` first (when born), then every
    /// reference by full name in sorted order, each with the id the reference
    /// points at directly. A `refs/tags/` reference whose direct object is a
    /// tag object is reported once, as `RootSource::AnnotatedTag` at the tag
    /// object's id, never also as `Ref` (contract T-1, LCM1.0c-fu3); a
    /// lightweight tag stays a `Ref`.
    ///
    /// Reflog and stash roots are **not** included: a reader that reports them
    /// has more roots than this, which the contract's
    /// `object_reader_conformance_allowing` (T-2) admits by kind, and a
    /// conformance fixture is a public-field struct precisely so the consumer
    /// can widen or narrow it.
    pub fn protected_roots(&self) -> ProtectedRoots {
        let repository = self.open();
        let mut roots = Vec::new();
        if let Ok(head) = repository.head()
            && let Some(target) = head.target()
        {
            roots.push(ProtectedRoot {
                source: RootSource::Head,
                oid: self.oid(target),
            });
        }
        for name in self.ref_names() {
            if let Some(oid) = self.ref_target(&name) {
                let annotated = name.starts_with("refs/tags/")
                    && repository.find_tag(self.git_oid(&oid)).is_ok();
                let source = if annotated {
                    RootSource::AnnotatedTag { name }
                } else {
                    RootSource::Ref { name }
                };
                roots.push(ProtectedRoot { source, oid });
            }
        }
        ProtectedRoots {
            roots,
            unknown: Vec::new(),
        }
    }

    /// What an `ObjectReader` over this repository is expected to contain:
    /// [`Self::reachable_objects`], [`Self::protected_roots`] and
    /// [`Self::absent_object_id`], in the shape
    /// `gwz_repo_contract::contract_tests::object_reader_conformance` takes.
    #[cfg(any(test, feature = "contract-tests"))]
    pub fn graph_fixture(&self) -> crate::GraphFixture {
        crate::GraphFixture {
            objects: self.reachable_objects(),
            roots: self.protected_roots(),
            missing: self.absent_object_id(),
        }
    }
}

/// The parent commit for the next commit, or `None` on an unborn `HEAD`.
pub(crate) fn head_commit(repository: &Repository) -> Option<git2::Commit<'_>> {
    repository
        .head()
        .ok()
        .map(|head| head.peel_to_commit().expect("HEAD peels to a commit"))
}

/// One in-memory index entry with zeroed stat data, so a tree built from it
/// depends on content and mode alone.
pub(crate) fn index_entry(path: &str, mode: u32, id: git2::Oid, size: usize) -> git2::IndexEntry {
    git2::IndexEntry {
        ctime: git2::IndexTime::new(0, 0),
        mtime: git2::IndexTime::new(0, 0),
        dev: 0,
        ino: 0,
        mode,
        uid: 0,
        gid: 0,
        file_size: size as u32,
        id,
        flags: 0,
        flags_extended: 0,
        path: path.as_bytes().to_vec(),
    }
}

/// A local path as a transport URL. Fixture paths come from `tempfile`, so
/// they are UTF-8.
pub(crate) fn path_url(path: &Path) -> String {
    path.to_str()
        .unwrap_or_else(|| panic!("fixture path {} is not UTF-8", path.display()))
        .to_owned()
}

/// Modes a work-state fixture uses, re-exported for a consumer asserting on
/// index entries.
pub mod modes {
    /// `100644`.
    pub const FILE: u32 = super::FILE_MODE;
    /// `100755`.
    pub const EXECUTABLE: u32 = super::EXECUTABLE_MODE;
    /// `120000`.
    pub const LINK: u32 = super::LINK_MODE;
}

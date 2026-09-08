//! Shared stateful Git substitute. No native repositories or Git processes.
//! Operations not implemented here return UnsupportedOperation, never pretend success.
use super::*;
use crate::filesystem::{FileSystem, FsKind, make_filesystem};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

mod fixture;
mod root;

type FileTree = BTreeMap<String, Vec<u8>>;

#[derive(Clone)]
pub(crate) struct FakeGitRepository {
    filesystem: Arc<dyn FileSystem>,
    repositories: Arc<Mutex<BTreeMap<PathBuf, RepositoryState>>>,
}
impl Default for FakeGitRepository {
    fn default() -> Self {
        Self::with_filesystem(Arc::new(make_filesystem()))
    }
}
impl FakeGitRepository {
    pub(crate) fn with_filesystem(filesystem: Arc<dyn FileSystem>) -> Self {
        Self {
            filesystem,
            repositories: Default::default(),
        }
    }

    /// Factory-created handles share repository state, just as native handles
    /// opening the same path see the same repository. Direct Default fixtures
    /// retain isolated state for adapter contract tests.
    pub(super) fn shared() -> Self {
        static REPOSITORIES: OnceLock<Arc<Mutex<BTreeMap<PathBuf, RepositoryState>>>> =
            OnceLock::new();
        Self {
            filesystem: Arc::new(make_filesystem()),
            repositories: Arc::clone(REPOSITORIES.get_or_init(Default::default)),
        }
    }
}
#[derive(Default)]
struct RepositoryState {
    index: BTreeMap<String, Vec<u8>>,
    index_override: Option<Vec<TestIndexEntry>>,
    blobs: BTreeMap<String, Vec<u8>>,
    metadata: BTreeMap<String, TestCommit>,
    config: BTreeMap<String, Vec<String>>,
    attached_ref: Option<String>,
    sha256: bool,
    detached: bool,
    commits: BTreeMap<String, BTreeMap<String, Vec<u8>>>,
    head: Option<String>,
    refs: BTreeMap<String, String>,
    symbolic_refs: BTreeMap<String, String>,
    parents: BTreeMap<String, Option<String>>,
    stashes: BTreeMap<String, GitPreservationStashEvidence>,
    stash_snapshots: BTreeMap<String, (FileTree, FileTree)>,
    repository_state: Option<GitRepositoryState>,
    merge_head: Option<String>,
    merge_conflict_snapshot: Option<GitMergeConflictSnapshot>,
    merge_index: Option<Vec<TestIndexEntry>>,
}
fn unsupported<T>(operation: &str) -> ModelResult<T> {
    Err(ModelError::new(
        ErrorCode::UnsupportedOperation,
        format!("fake Git operation not implemented: {operation}"),
    ))
}
#[allow(unused_variables)]
impl GitRepository for FakeGitRepository {
    fn test_init_repo(&self, repo: &Path, spec: &TestRepoSpec) -> ModelResult<()> {
        fixture::test_init_repo(self, repo, spec)
    }
    fn test_create_commit(&self, repo: &Path, spec: &TestCommitSpec) -> ModelResult<String> {
        fixture::test_create_commit(self, repo, spec)
    }
    fn test_read_commit(&self, repo: &Path, oid: &str) -> ModelResult<TestCommit> {
        fixture::test_read_commit(self, repo, oid)
    }
    fn test_set_ref(
        &self,
        repo: &Path,
        name: &str,
        target: Option<&TestRefTarget>,
    ) -> ModelResult<()> {
        fixture::test_set_ref(self, repo, name, target)
    }
    fn test_set_head(&self, repo: &Path, state: &TestHead) -> ModelResult<()> {
        fixture::test_set_head(self, repo, state)
    }
    fn test_replace_index(&self, repo: &Path, entries: &[TestIndexEntry]) -> ModelResult<()> {
        fixture::test_replace_index(self, repo, entries)
    }
    fn test_read_index(&self, repo: &Path) -> ModelResult<Vec<TestIndexEntry>> {
        fixture::test_read_index(self, repo)
    }
    fn test_set_config(&self, repo: &Path, key: &str, values: &[String]) -> ModelResult<()> {
        fixture::test_set_config(self, repo, key, values)
    }
    fn test_read_config(&self, repo: &Path, key: &str) -> ModelResult<Vec<String>> {
        fixture::test_read_config(self, repo, key)
    }
    fn test_force_checkout(&self, repo: &Path, commit: &str) -> ModelResult<()> {
        fixture::test_force_checkout(self, repo, commit)
    }
    fn test_reset_mixed(&self, repo: &Path, commit: &str) -> ModelResult<()> {
        fixture::test_reset_mixed(self, repo, commit)
    }
    fn test_set_repository_state(
        &self,
        repo: &Path,
        state: GitRepositoryState,
        merge_head: Option<&str>,
    ) -> ModelResult<()> {
        fixture::test_set_repository_state(self, repo, state, merge_head)
    }
    fn test_seed_merge_conflict(
        &self,
        repo: &Path,
        before: &str,
        source: &str,
    ) -> ModelResult<GitMergeConflictSnapshot> {
        fixture::test_seed_merge_conflict(self, repo, before, source)
    }
    fn test_create_commit_from_parent(
        &self,
        repo: &Path,
        parent: &str,
        message: &str,
        edits: &[TestCommitFileEdit],
    ) -> ModelResult<String> {
        fixture::test_create_commit_from_parent(self, repo, parent, message, edits)
    }

    fn merge_state(&self, path: &Path) -> ModelResult<Option<GitNativeMergeState>> {
        let repositories = self.repositories.lock().unwrap();
        let repo = repositories
            .get(&repository_key(self.filesystem.as_ref(), path))
            .ok_or_else(|| failed("repository missing"))?;
        Ok(repo.merge_head.as_ref().map(|merge_head| {
            let conflict_paths: Vec<String> = repo
                .merge_conflict_snapshot
                .as_ref()
                .map(|snapshot| {
                    snapshot
                        .files
                        .iter()
                        .map(|file| file.path.clone())
                        .collect()
                })
                .unwrap_or_default();
            GitNativeMergeState {
                merge_head: merge_head.clone(),
                unresolved_entries: conflict_paths.len(),
                conflict_paths,
            }
        }))
    }

    fn validate_merge_recovery_state(
        &self,
        path: &Path,
        expected_before: &str,
        expected_merge_head: &str,
        _require_resolved: bool,
    ) -> ModelResult<()> {
        let snapshot = self.merge_conflict_snapshot(path, expected_before, expected_merge_head)?;
        if snapshot.files.is_empty() {
            return Err(ModelError::new(
                ErrorCode::MergeRecoveryRequired,
                "merge conflict snapshot is empty",
            ));
        }
        Ok(())
    }

    fn merge_conflict_snapshot(
        &self,
        path: &Path,
        expected_before: &str,
        expected_merge_head: &str,
    ) -> ModelResult<GitMergeConflictSnapshot> {
        let repositories = self.repositories.lock().unwrap();
        let repo = repositories
            .get(&repository_key(self.filesystem.as_ref(), path))
            .ok_or_else(|| failed("repository missing"))?;
        if repo.head.as_deref() != Some(expected_before)
            || repo.merge_head.as_deref() != Some(expected_merge_head)
            || repo.repository_state != Some(GitRepositoryState::Merge)
            || repo.index_override.as_ref() != repo.merge_index.as_ref()
        {
            return Err(ModelError::new(
                ErrorCode::MergeRecoveryRequired,
                "merge recovery state differs from its seeded form",
            ));
        }
        let snapshot = repo
            .merge_conflict_snapshot
            .clone()
            .ok_or_else(|| failed("merge conflict snapshot missing"))?;
        drop(repositories);
        for file in &snapshot.files {
            let bytes = self
                .filesystem
                .as_ref()
                .read(&path.join(&file.path))
                .map_err(|error| failed(error.to_string()))?;
            if format!("{:x}", Sha256::digest(bytes)) != file.sha256 {
                return Err(ModelError::new(
                    ErrorCode::MergeRecoveryRequired,
                    "merge conflict worktree differs from its seeded form",
                ));
            }
        }
        Ok(snapshot)
    }

    fn abort_merge(
        &self,
        path: &Path,
        expected_before: &str,
        expected_merge_head: &str,
    ) -> ModelResult<()> {
        if self.repository_state(path)? == GitRepositoryState::Clean {
            return Ok(());
        }
        self.merge_conflict_snapshot(path, expected_before, expected_merge_head)?;
        self.test_force_checkout(path, expected_before)?;
        let mut repositories = self.repositories.lock().unwrap();
        let repo = repositories
            .get_mut(&repository_key(self.filesystem.as_ref(), path))
            .unwrap();
        repo.repository_state = None;
        repo.merge_head = None;
        repo.merge_conflict_snapshot = None;
        repo.merge_index = None;
        Ok(())
    }

    fn repository_state(&self, path: &Path) -> ModelResult<GitRepositoryState> {
        if self.is_repository(path)? {
            Ok(self
                .repositories
                .lock()
                .unwrap()
                .get(&repository_key(self.filesystem.as_ref(), path))
                .and_then(|repo| repo.repository_state)
                .unwrap_or(GitRepositoryState::Clean))
        } else {
            Err(failed("repository missing"))
        }
    }
    fn root_preservation_image(
        &self,
        path: &Path,
        clean: &GitRootManagedForm,
        excluded: &[String],
    ) -> ModelResult<GitPreservationImage> {
        root::capture(self, path, Some(clean), excluded)
    }
    fn validate_root_preservation_spec(
        &self,
        path: &Path,
        spec: &GitRootPreservationSpec,
    ) -> ModelResult<()> {
        root::validate(self, path, spec)
    }
    fn root_managed_index_matches(
        &self,
        path: &Path,
        form: &GitRootManagedIndexForm,
    ) -> ModelResult<bool> {
        root::index_matches(self, path, form)
    }
    fn rewrite_root_managed_index_checked(
        &self,
        path: &Path,
        form: &GitRootManagedIndexForm,
    ) -> ModelResult<()> {
        root::rewrite(self, path, form)
    }
    fn prepare_root_preservation_stash(
        &self,
        path: &Path,
        spec: &GitRootPreservationSpec,
    ) -> ModelResult<GitPreparedRootStash> {
        preservation_root::prepare_root_preservation_stash(
            self.filesystem.as_ref(),
            self,
            path,
            spec,
        )
    }
    fn observe_root_preservation_step(
        &self,
        path: &Path,
        spec: &GitRootPreservationSpec,
        step: &GitRootPreservationPhysicalStep,
        guard: &GitRootPreservationGuard,
    ) -> ModelResult<GitRootPreservationStepObservation> {
        preservation_root::observe_root_preservation_step(
            self.filesystem.as_ref(),
            self,
            path,
            spec,
            step,
            guard,
        )
    }
    fn execute_root_preservation_step_checked(
        &self,
        path: &Path,
        spec: &GitRootPreservationSpec,
        step: &GitRootPreservationPhysicalStep,
        guard: &GitRootPreservationGuard,
    ) -> ModelResult<GitCheckedPreservationMutation> {
        preservation_root::execute_root_preservation_step_checked(
            self.filesystem.as_ref(),
            self,
            path,
            spec,
            step,
            guard,
        )
    }
    fn checkout_matches_commit(
        &self,
        path: &Path,
        branch: &str,
        commit: &str,
    ) -> ModelResult<bool> {
        let head = self.head(path)?;
        Ok(head.branch.as_deref() == Some(branch)
            && head.commit.as_deref() == Some(commit)
            && self.checkout_matches_commit_except(path, commit, &[])?)
    }
    fn checkout_matches_commit_except(
        &self,
        path: &Path,
        commit: &str,
        allowed: &[String],
    ) -> ModelResult<bool> {
        root::checkout_matches(
            self,
            path,
            commit,
            &GitCheckoutOverlay {
                worktree_paths: allowed.to_vec(),
                index_paths: allowed.to_vec(),
            },
        )
    }
    fn checkout_matches_commit_with_overlay(
        &self,
        path: &Path,
        commit: &str,
        overlay: &GitCheckoutOverlay,
    ) -> ModelResult<bool> {
        root::checkout_matches(self, path, commit, overlay)
    }
    fn index_entries_match_candidate_files(
        &self,
        path: &Path,
        files: &[GitCandidateFile],
        absent: &[String],
    ) -> ModelResult<bool> {
        root::candidate_matches(self, path, files, absent)
    }
    fn index_matches_candidate_files(
        &self,
        path: &Path,
        files: &[GitCandidateFile],
        absent: &[String],
    ) -> ModelResult<bool> {
        Ok(root::candidate_matches(self, path, files, absent)?
            && files.iter().all(|file| {
                matches!(
                    self.filesystem.as_ref().kind(&path.join(&file.path)),
                    Ok(FsKind::File)
                )
            }))
    }
    fn commit_gwz_paths_checked(
        &self,
        path: &Path,
        expected: Option<&str>,
        files: &[GitCandidateFile],
        message: &str,
    ) -> ModelResult<GitScopedCommitResult> {
        root::scoped_commit(self, path, expected, files, message)
    }
    fn verify_gwz_paths_commit(
        &self,
        path: &Path,
        commit: &str,
        parent: Option<&str>,
        files: &[GitCandidateFile],
        message: &str,
    ) -> ModelResult<GitScopedCommitResult> {
        root::verify_scoped(self, path, commit, parent, files, message)
    }
    fn rollback_gwz_paths_commit_checked(
        &self,
        path: &Path,
        branch: &str,
        commit: &str,
        parent: Option<&str>,
        files: &[GitCandidateFile],
        message: &str,
    ) -> ModelResult<()> {
        root::verify_scoped(self, path, commit, parent, files, message)?;
        let head = self.head(path)?;
        if head.branch.as_deref() != Some(branch)
            || (head.commit.as_deref() != Some(commit) && head.commit.as_deref() != parent)
        {
            return Err(ModelError::new(
                ErrorCode::MergeDrift,
                "scoped rollback HEAD differs",
            ));
        }
        self.test_set_ref(
            path,
            &format!("refs/heads/{branch}"),
            parent.map(|oid| TestRefTarget::Direct(oid.into())).as_ref(),
        )
    }
    fn repository_index(&self, path: &Path) -> ModelResult<GitIndexSnapshot> {
        let entries = self
            .test_read_index(path)?
            .into_iter()
            .map(|e| {
                Ok(GitIndexEntry {
                    path: e.path,
                    object_id: git2::Oid::from_str(&e.object_id)
                        .map_err(git_error)?
                        .as_bytes()
                        .to_vec(),
                    mode: e.mode,
                    flags: (u16::from(e.stage) << 12) | if e.assume_valid { 0x8000 } else { 0 },
                    flags_extended: (if e.skip_worktree { 0x4000 } else { 0 })
                        | if e.intent_to_add { 0x2000 } else { 0 },
                    ctime: (0, 0),
                    mtime: (0, 0),
                    stat: [0; 5],
                })
            })
            .collect::<ModelResult<Vec<_>>>()?;
        Ok(GitIndexSnapshot {
            path: Some(path.join(".git/index")),
            entries,
        })
    }
    fn repository_paths(&self, path: &Path) -> ModelResult<GitRepositoryPaths> {
        if !self.is_repository(path)? {
            return Err(failed("repository missing"));
        }
        Ok(GitRepositoryPaths {
            worktree: Some(repository_key(self.filesystem.as_ref(), path)),
            git_dir: repository_key(self.filesystem.as_ref(), path).join(".git"),
            common_dir: repository_key(self.filesystem.as_ref(), path).join(".git"),
        })
    }
    fn is_repository(&self, path: &Path) -> ModelResult<bool> {
        Ok(self
            .repositories
            .lock()
            .unwrap()
            .contains_key(&repository_key(self.filesystem.as_ref(), path)))
    }
    fn create_repo(&self, path: &Path) -> ModelResult<GitCreateResult> {
        let mut repositories = self.repositories.lock().unwrap();
        if repositories.contains_key(&repository_key(self.filesystem.as_ref(), path)) {
            return Err(failed("repository already exists"));
        }
        self.filesystem
            .as_ref()
            .create_directories(path)
            .map_err(|e| failed(e.to_string()))?;
        repositories.insert(
            repository_key(self.filesystem.as_ref(), path),
            RepositoryState::default(),
        );
        Ok(GitCreateResult {
            path: path.to_path_buf(),
        })
    }
    fn clone_repo(&self, url: &str, path: &Path) -> ModelResult<GitCloneResult> {
        unsupported("clone_repo")
    }
    fn fetch(&self, path: &Path, remote: &str) -> ModelResult<GitFetchResult> {
        unsupported("fetch")
    }
    fn ls_remote(&self, path: &Path, remote: &str) -> ModelResult<Vec<GitRemoteRef>> {
        unsupported("ls_remote")
    }
    fn fast_forward(
        &self,
        path: &Path,
        branch: &str,
        upstream_ref: &str,
    ) -> ModelResult<GitUpdateResult> {
        unsupported("fast_forward")
    }
    fn merge_upstream(
        &self,
        path: &Path,
        branch: &str,
        upstream_ref: &str,
    ) -> ModelResult<GitIntegrateResult> {
        unsupported("merge_upstream")
    }
    fn rebase_onto(
        &self,
        path: &Path,
        branch: &str,
        upstream_ref: &str,
    ) -> ModelResult<GitIntegrateResult> {
        unsupported("rebase_onto")
    }
    fn reset_hard(
        &self,
        path: &Path,
        branch: &str,
        upstream_ref: &str,
    ) -> ModelResult<GitUpdateResult> {
        unsupported("reset_hard")
    }
    fn checkout_commit(&self, path: &Path, commit: &str) -> ModelResult<GitUpdateResult> {
        unsupported("checkout_commit")
    }
    fn checkout_branch(
        &self,
        path: &Path,
        branch: &str,
        commit: &str,
    ) -> ModelResult<GitUpdateResult> {
        unsupported("checkout_branch")
    }
    fn status(&self, path: &Path) -> ModelResult<GitStatus> {
        let repositories = self.repositories.lock().unwrap();
        let repo = repositories
            .get(&repository_key(self.filesystem.as_ref(), path))
            .ok_or_else(|| failed("repository missing"))?;
        let empty = BTreeMap::new();
        let committed = repo
            .head
            .as_ref()
            .and_then(|oid| repo.commits.get(oid))
            .unwrap_or(&empty);
        let worktree = root::worktree(self, repo, path)?;
        let paths: BTreeSet<_> = committed
            .keys()
            .chain(repo.index.keys())
            .chain(worktree.keys())
            .collect();
        let mut result = GitStatus::clean();
        for name in paths {
            let before = committed.get(name);
            let index = repo.index.get(name);
            let disk = worktree.get(name);
            let untracked = before.is_none() && index.is_none() && disk.is_some();
            let staged = before != index;
            let hidden_by_index_flag = repo.index_override.as_ref().is_some_and(|entries| {
                entries.iter().any(|entry| {
                    entry.stage == 0
                        && entry.path == name.as_bytes()
                        && (entry.assume_valid || entry.skip_worktree)
                })
            });
            let unstaged = !untracked && index != disk && !hidden_by_index_flag;
            result.staged += usize::from(staged);
            result.unstaged += usize::from(unstaged);
            result.untracked += usize::from(untracked);
            if staged || unstaged || untracked {
                result.files.push(GitFileStatus {
                    path: name.clone(),
                    index_status: if untracked {
                        "?"
                    } else if !staged {
                        " "
                    } else if index.is_none() {
                        "D"
                    } else if before.is_none() {
                        "A"
                    } else {
                        "M"
                    }
                    .into(),
                    worktree_status: if untracked {
                        "?"
                    } else if !unstaged {
                        " "
                    } else if disk.is_none() {
                        "D"
                    } else {
                        "M"
                    }
                    .into(),
                    original_path: None,
                });
            }
        }
        result.is_dirty = !result.files.is_empty();
        Ok(result)
    }
    fn head(&self, path: &Path) -> ModelResult<GitHeadState> {
        let repositories = self.repositories.lock().unwrap();
        let repo = repositories
            .get(&repository_key(self.filesystem.as_ref(), path))
            .ok_or_else(|| failed("repository missing"))?;
        Ok(GitHeadState {
            branch: if repo.detached {
                None
            } else {
                Some(
                    repo.attached_ref
                        .as_deref()
                        .unwrap_or("refs/heads/main")
                        .trim_start_matches("refs/heads/")
                        .into(),
                )
            },
            commit: repo.head.clone(),
            is_detached: repo.detached,
        })
    }
    fn remotes(&self, path: &Path) -> ModelResult<Vec<GitRemote>> {
        unsupported("remotes")
    }
    fn add_remote(&self, path: &Path, name: &str, url: &str) -> ModelResult<GitRemoteResult> {
        unsupported("add_remote")
    }
    fn push(&self, path: &Path, remote: &str, refspec: &str) -> ModelResult<GitPushResult> {
        unsupported("push")
    }
    fn fetch_anonymous(
        &self,
        path: &Path,
        url: &str,
        refspecs: &[&str],
    ) -> ModelResult<GitFetchResult> {
        unsupported("fetch_anonymous")
    }
    fn push_anonymous(&self, path: &Path, url: &str, refspec: &str) -> ModelResult<GitPushResult> {
        unsupported("push_anonymous")
    }
    fn read_ref(&self, path: &Path, ref_spec: &str) -> ModelResult<Option<String>> {
        let repositories = self.repositories.lock().unwrap();
        let repo = repositories
            .get(&repository_key(self.filesystem.as_ref(), path))
            .ok_or_else(|| failed("repository missing"))?;
        if ref_spec == "HEAD" {
            return Ok(repo.head.clone());
        }
        let name = if ref_spec.starts_with("refs/") {
            ref_spec.to_owned()
        } else {
            format!("refs/heads/{ref_spec}")
        };
        fixture::resolve_ref(repo, &name)
    }

    fn is_ancestor(&self, path: &Path, ancestor: &str, descendant: &str) -> ModelResult<bool> {
        let repositories = self.repositories.lock().unwrap();
        let repo = repositories
            .get(&repository_key(self.filesystem.as_ref(), path))
            .ok_or_else(|| failed("repository missing"))?;
        if !repo.commits.contains_key(ancestor) || !repo.commits.contains_key(descendant) {
            return Err(failed("commit missing"));
        }
        let mut pending = vec![descendant];
        let mut visited = BTreeSet::new();
        while let Some(oid) = pending.pop() {
            if oid == ancestor {
                return Ok(true);
            }
            if visited.insert(oid)
                && let Some(commit) = repo.metadata.get(oid)
            {
                pending.extend(commit.parents.iter().map(String::as_str));
            }
        }
        Ok(false)
    }
    fn stage_paths(&self, path: &Path, pathspecs: &[&str]) -> ModelResult<GitStageResult> {
        let mut repositories = self.repositories.lock().unwrap();
        let repo = repositories
            .get_mut(&repository_key(self.filesystem.as_ref(), path))
            .ok_or_else(|| failed("repository missing"))?;
        let mut staged = 0;
        for spec in pathspecs {
            // Explicit file paths are sufficient for these shared contracts.
            // Reject glob/directory semantics until they have their own contract.
            if spec.contains('*')
                || matches!(
                    self.filesystem.as_ref().kind(&path.join(spec)),
                    Ok(FsKind::Directory)
                )
            {
                return unsupported("stage pathspec");
            }
            match self.filesystem.as_ref().read(&path.join(spec)) {
                Ok(bytes) => {
                    repo.index.insert((*spec).to_owned(), bytes);
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::NotFound
                        && repo.index.contains_key(*spec) =>
                {
                    repo.index.remove(*spec);
                }
                Err(e) => return Err(failed(e.to_string())),
            }
            repo.index_override = None;
            staged += 1;
        }
        Ok(GitStageResult { staged })
    }
    fn commit(&self, path: &Path, message: &str, all: bool) -> ModelResult<GitCommitResult> {
        if all {
            return unsupported("commit --all");
        }
        let mut repositories = self.repositories.lock().unwrap();
        let repo = repositories
            .get_mut(&repository_key(self.filesystem.as_ref(), path))
            .ok_or_else(|| failed("repository missing"))?;
        if repo.head.as_ref().and_then(|oid| repo.commits.get(oid)) == Some(&repo.index) {
            return Err(failed("nothing to commit"));
        }
        let spec = TestCommitSpec::from_index(message, repo.head.clone().into_iter().collect());
        let oid = fixture::store_commit(repo, repo.index.clone(), &spec)?;
        repo.head = Some(oid.clone());
        if !repo.detached {
            repo.refs.insert(
                repo.attached_ref
                    .clone()
                    .unwrap_or_else(|| "refs/heads/main".into()),
                oid.clone(),
            );
        }
        Ok(GitCommitResult { commit: oid })
    }
    fn tag_create(
        &self,
        path: &Path,
        name: &str,
        message: Option<&str>,
        signed: bool,
    ) -> ModelResult<GitTagResult> {
        unsupported("tag_create")
    }
    fn tag_list(&self, path: &Path) -> ModelResult<Vec<String>> {
        unsupported("tag_list")
    }
    fn tag_delete(&self, path: &Path, name: &str) -> ModelResult<()> {
        unsupported("tag_delete")
    }
    fn tag_fetch(&self, path: &Path, remote: &str) -> ModelResult<GitFetchResult> {
        unsupported("tag_fetch")
    }
    fn observe_direct_ref(&self, path: &Path, name: &str) -> ModelResult<GitDirectRefObservation> {
        if self
            .repositories
            .lock()
            .unwrap()
            .get(&repository_key(self.filesystem.as_ref(), path))
            .is_some_and(|repo| repo.symbolic_refs.contains_key(name))
        {
            return Ok(GitDirectRefObservation::NonDirect);
        }
        Ok(match self.read_ref(path, name)? {
            Some(target) => GitDirectRefObservation::Direct { target },
            None => GitDirectRefObservation::Absent,
        })
    }
    fn create_backup_ref(
        &self,
        path: &Path,
        name: &str,
        target: &str,
    ) -> ModelResult<GitBackupRefResult> {
        preservation::validate_backup_ref_name(name)?;
        let mut repositories = self.repositories.lock().unwrap();
        let repo = repositories
            .get_mut(&repository_key(self.filesystem.as_ref(), path))
            .ok_or_else(|| failed("repository missing"))?;
        if !repo.commits.contains_key(target) {
            return Err(failed("commit missing"));
        }
        if let Some(actual) = repo.refs.get(name) {
            if actual != target {
                return Err(ModelError::new(
                    ErrorCode::MergeDrift,
                    format!("preservation ref '{name}' points to '{actual}' instead of '{target}'"),
                ));
            }
        } else {
            repo.refs.insert(name.into(), target.into());
        }
        Ok(GitBackupRefResult {
            name: name.into(),
            target: target.into(),
        })
    }
    fn create_backup_ref_checked(
        &self,
        path: &Path,
        branch: &str,
        expected_head: &str,
        name: &str,
        target: &str,
    ) -> ModelResult<GitBackupRefResult> {
        preservation::validate_backup_ref_name(name)?;
        let mut repositories = self.repositories.lock().unwrap();
        let repo = repositories
            .get_mut(&repository_key(self.filesystem.as_ref(), path))
            .ok_or_else(|| failed("repository missing"))?;
        if target != expected_head
            || branch != "main"
            || repo.head.as_deref() != Some(expected_head)
        {
            return Err(ModelError::new(
                ErrorCode::PreservationEvidenceMismatch,
                "attached HEAD differs from expected backup target",
            ));
        }
        if repo.refs.get(name).is_some_and(|actual| actual != target) {
            return Err(ModelError::new(
                ErrorCode::PreservationEvidenceMismatch,
                "backup ref differs from expected target",
            ));
        }
        repo.refs.insert(name.into(), target.into());
        Ok(GitBackupRefResult {
            name: name.into(),
            target: target.into(),
        })
    }
    fn delete_backup_ref_checked(
        &self,
        path: &Path,
        name: &str,
        expected_target: &str,
    ) -> ModelResult<()> {
        preservation::validate_backup_ref_name(name)?;
        if expected_target.len() != 40 || !expected_target.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ModelError::new(
                ErrorCode::InvalidRequest,
                "invalid preservation target",
            ));
        }
        let mut repositories = self.repositories.lock().unwrap();
        let repo = repositories
            .get_mut(&repository_key(self.filesystem.as_ref(), path))
            .ok_or_else(|| failed("repository missing"))?;
        if repo
            .refs
            .get(name)
            .is_some_and(|actual| actual != expected_target)
        {
            return Err(ModelError::new(ErrorCode::MergeDrift, "backup ref changed"));
        }
        repo.refs.remove(name);
        Ok(())
    }
    fn set_branch_target_checked(
        &self,
        path: &Path,
        branch: &str,
        expected_current: &str,
        target: &str,
    ) -> ModelResult<GitUpdateResult> {
        if self.status(path)?.is_dirty {
            return Err(ModelError::new(ErrorCode::DirtyMember, "dirty worktree"));
        }
        let mut repositories = self.repositories.lock().unwrap();
        let repo = repositories
            .get_mut(&repository_key(self.filesystem.as_ref(), path))
            .ok_or_else(|| failed("repository missing"))?;
        if !repo.commits.contains_key(expected_current) || !repo.commits.contains_key(target) {
            return Err(failed("commit missing"));
        }
        if branch != "main" {
            return unsupported("non-main branch reset");
        }
        if repo.head.as_deref() == Some(target) {
            return Ok(GitUpdateResult {
                updated: false,
                commit: Some(target.into()),
            });
        }
        if repo.head.as_deref() != Some(expected_current) {
            return Err(ModelError::new(ErrorCode::MergeDrift, "branch changed"));
        }
        let tree = repo.commits[target].clone();
        for name in repo.index.keys().filter(|name| !tree.contains_key(*name)) {
            remove_worktree_file(self.filesystem.as_ref(), &path.join(name))?;
        }
        for (name, bytes) in &tree {
            let file = path.join(name);
            write_worktree_file(self.filesystem.as_ref(), &file, bytes)?;
        }
        repo.index = tree;
        repo.index_override = None;
        repo.head = Some(target.into());
        repo.refs
            .insert(format!("refs/heads/{branch}"), target.into());
        Ok(GitUpdateResult {
            updated: true,
            commit: Some(target.into()),
        })
    }
    fn preservation_image(
        &self,
        path: &Path,
        include_untracked: bool,
    ) -> ModelResult<GitPreservationImage> {
        if !include_untracked {
            return unsupported("tracked-only preservation image");
        }
        root::capture(self, path, None, &[])
    }

    fn preservation_stashes(
        &self,
        path: &Path,
        merge_id: &str,
    ) -> ModelResult<Vec<GitPreservationStashEvidence>> {
        let repositories = self.repositories.lock().unwrap();
        let repo = repositories
            .get(&repository_key(self.filesystem.as_ref(), path))
            .ok_or_else(|| failed("repository missing"))?;
        Ok(repo.stashes.get(merge_id).cloned().into_iter().collect())
    }
    fn stash_list(&self, path: &Path) -> ModelResult<Vec<GitStashEntry>> {
        let repositories = self.repositories.lock().unwrap();
        let repo = repositories
            .get(&repository_key(self.filesystem.as_ref(), path))
            .ok_or_else(|| failed("repository missing"))?;
        Ok(repo
            .stashes
            .values()
            .enumerate()
            .map(|(index, stash)| GitStashEntry {
                index,
                object_id: stash.object_id.clone(),
                message: stash.message.clone(),
            })
            .collect())
    }
    fn stash_for_merge_preservation(
        &self,
        path: &Path,
        merge_id: &str,
        include_untracked: bool,
    ) -> ModelResult<GitStashPushResult> {
        let head = self.head(path)?;
        let image = self.preservation_image(path, include_untracked)?;
        self.stash_for_merge_preservation_checked(
            path,
            head.branch
                .as_deref()
                .ok_or_else(|| failed("detached stash HEAD"))?,
            head.commit
                .as_deref()
                .ok_or_else(|| failed("unborn stash HEAD"))?,
            &image.preimage_sha256,
            merge_id,
            include_untracked,
        )
    }
    fn stash_for_merge_preservation_checked(
        &self,
        path: &Path,
        branch: &str,
        expected_head: &str,
        expected_preimage_sha256: &str,
        merge_id: &str,
        include_untracked: bool,
    ) -> ModelResult<GitStashPushResult> {
        if merge_id.is_empty()
            || !merge_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
        {
            return Err(ModelError::new(
                ErrorCode::InvalidRequest,
                "invalid merge id",
            ));
        }
        let image = self.preservation_image(path, include_untracked)?;
        let mut repositories = self.repositories.lock().unwrap();
        let repo = repositories
            .get_mut(&repository_key(self.filesystem.as_ref(), path))
            .ok_or_else(|| failed("repository missing"))?;
        let mismatch = || {
            ModelError::new(
                ErrorCode::PreservationEvidenceMismatch,
                "preservation stash state changed",
            )
        };
        if branch != "main" || repo.head.as_deref() != Some(expected_head) {
            return Err(mismatch());
        }
        if let Some(stash) = repo.stashes.get(merge_id) {
            if stash.head_commit != expected_head
                || stash.image.preimage_sha256 != expected_preimage_sha256
                || image.dirty != GitPreservationDirtySummary::default()
            {
                return Err(mismatch());
            }
            return Ok(GitStashPushResult {
                object_id: stash.object_id.clone(),
                message: stash.message.clone(),
            });
        }
        if image.preimage_sha256 != expected_preimage_sha256
            || image.dirty == GitPreservationDirtySummary::default()
        {
            return Err(mismatch());
        }
        let message = format!("gwz:stash_{merge_id}: merge preservation");
        let object_id = format!(
            "{:x}",
            Sha256::digest(format!(
                "{expected_head}{expected_preimage_sha256}{message}"
            ))
        )[..40]
            .to_owned();
        let worktree = root::worktree(self, repo, path)?;
        let tree = repo.commits[expected_head].clone();
        for name in worktree.keys().filter(|name| !tree.contains_key(*name)) {
            remove_worktree_file(self.filesystem.as_ref(), &path.join(name))?;
        }
        for (name, bytes) in &tree {
            let file = path.join(name);
            write_worktree_file(self.filesystem.as_ref(), &file, bytes)?;
        }
        repo.stash_snapshots
            .insert(object_id.clone(), (repo.index.clone(), worktree));
        repo.index = tree;
        repo.index_override = None;
        repo.stashes.insert(
            merge_id.into(),
            GitPreservationStashEvidence {
                object_id: object_id.clone(),
                message: message.clone(),
                head_commit: expected_head.into(),
                image,
            },
        );
        Ok(GitStashPushResult { object_id, message })
    }
    fn stash_apply(
        &self,
        path: &Path,
        target: &GitStashTarget,
        options: GitStashRestoreOptions,
    ) -> ModelResult<()> {
        if !options.preserve_index || target.object_id.is_none() {
            return unsupported("stash apply without exact object and index restoration");
        }
        if self.status(path)?.is_dirty {
            return unsupported("stash apply over dirty work");
        }
        let mut repositories = self.repositories.lock().unwrap();
        let repo = repositories
            .get_mut(&repository_key(self.filesystem.as_ref(), path))
            .ok_or_else(|| failed("repository missing"))?;
        let oid = target.object_id.as_ref().unwrap();
        let evidence = repo
            .stashes
            .values()
            .find(|stash| &stash.object_id == oid)
            .ok_or_else(|| failed("stash missing"))?;
        if repo.head.as_deref() != Some(&evidence.head_commit) {
            return unsupported("stash apply onto changed HEAD");
        }
        let (index, worktree) = repo
            .stash_snapshots
            .get(oid)
            .ok_or_else(|| failed("stash snapshot missing"))?
            .clone();
        for name in repo
            .index
            .keys()
            .filter(|name| !worktree.contains_key(*name))
        {
            remove_worktree_file(self.filesystem.as_ref(), &path.join(name))?;
        }
        for (name, bytes) in worktree {
            let file = path.join(name);
            write_worktree_file(self.filesystem.as_ref(), &file, &bytes)?;
        }
        repo.index = index;
        repo.index_override = None;
        Ok(())
    }
    fn commit_exists(&self, path: &Path, oid: &str) -> ModelResult<bool> {
        let repositories = self.repositories.lock().unwrap();
        let repo = repositories
            .get(&repository_key(self.filesystem.as_ref(), path))
            .ok_or_else(|| failed("repository missing"))?;
        Ok(repo.commits.contains_key(oid))
    }
    fn read_file_at_commit(
        &self,
        path: &Path,
        commit: &str,
        relative_path: &str,
    ) -> ModelResult<Option<Vec<u8>>> {
        let relative = Path::new(relative_path);
        if relative.is_absolute()
            || relative
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err(ModelError::new(
                ErrorCode::InvalidRequest,
                "committed path must be normalized and relative",
            ));
        }
        let repositories = self.repositories.lock().unwrap();
        let repo = repositories
            .get(&repository_key(self.filesystem.as_ref(), path))
            .ok_or_else(|| failed("repository missing"))?;
        let tree = repo
            .commits
            .get(commit)
            .ok_or_else(|| failed("commit missing"))?;
        Ok(tree.get(relative_path).cloned())
    }
}

fn failed(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::GitCommandFailed, message)
}
fn read_worktree(
    filesystem: &dyn FileSystem,
    root: &Path,
    ignored: &[String],
    tracked: &FileTree,
) -> ModelResult<FileTree> {
    fn visit(
        filesystem: &dyn FileSystem,
        root: &Path,
        directory: &Path,
        files: &mut FileTree,
        ignored: &[String],
        tracked: &FileTree,
    ) -> ModelResult<()> {
        for entry in filesystem
            .read_directory(directory)
            .map_err(|e| failed(e.to_string()))?
        {
            if entry.name == ".git" {
                continue;
            }
            let path = directory.join(&entry.name);
            let name = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let prefix = format!("{name}/");
            let excluded = ignored
                .iter()
                .any(|rule| name == rule.trim_end_matches('/') || name.starts_with(rule));
            if excluded
                && !tracked.contains_key(&name)
                && !tracked.keys().any(|key| key.starts_with(&prefix))
            {
                continue;
            }
            if entry.kind == FsKind::Directory {
                visit(filesystem, root, &path, files, ignored, tracked)?;
            } else if entry.kind == FsKind::File {
                files.insert(
                    name,
                    filesystem.read(&path).map_err(|e| failed(e.to_string()))?,
                );
            } else {
                return unsupported("non-regular worktree entry");
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    visit(filesystem, root, root, &mut files, ignored, tracked)?;
    Ok(files)
}

fn repository_key(filesystem: &dyn FileSystem, path: &Path) -> PathBuf {
    let canonical = filesystem
        .canonical_path(path)
        .unwrap_or_else(|_| path.to_path_buf());
    if canonical.file_name().is_some_and(|name| name == ".git") {
        canonical.parent().unwrap().to_path_buf()
    } else {
        canonical
    }
}

fn write_worktree_file(filesystem: &dyn FileSystem, path: &Path, bytes: &[u8]) -> ModelResult<()> {
    filesystem
        .create_directories(
            path.parent()
                .ok_or_else(|| failed("worktree file has no parent"))?,
        )
        .map_err(|e| failed(e.to_string()))?;
    match filesystem.remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(failed(error.to_string())),
    }
    let file = filesystem
        .create_file(path)
        .map_err(|e| failed(e.to_string()))?;
    filesystem
        .write_all(&file, bytes)
        .map_err(|e| failed(e.to_string()))
}

fn remove_worktree_file(filesystem: &dyn FileSystem, path: &Path) -> ModelResult<()> {
    filesystem
        .remove_file(path)
        .map_err(|e| failed(e.to_string()))
}

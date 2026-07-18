use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::model::{ErrorCode, ModelError, ModelResult};

use super::*;

pub trait GitBackend {
    fn is_repository(&self, path: &Path) -> ModelResult<bool>;
    /// Return whether `oid` exists locally and resolves to a commit object.
    /// This never fetches and returns `false` for malformed, missing, or
    /// non-commit object ids.
    fn commit_exists(&self, _path: &Path, _oid: &str) -> ModelResult<bool> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "commit_exists is not implemented by this GitBackend",
        ))
    }
    fn create_repo(&self, path: &Path) -> ModelResult<GitCreateResult>;
    fn clone_repo(&self, url: &str, path: &Path) -> ModelResult<GitCloneResult>;
    /// Clone, forwarding libgit2 transfer progress to `progress`. The default
    /// ignores progress; backends that support it override this.
    fn clone_repo_with_progress(
        &self,
        url: &str,
        path: &Path,
        _progress: &dyn Fn(crate::GitTransferProgress),
    ) -> ModelResult<GitCloneResult> {
        self.clone_repo(url, path)
    }
    fn fetch(&self, path: &Path, remote: &str) -> ModelResult<GitFetchResult>;
    /// List the refs a remote advertises WITHOUT fetching objects (porcelain
    /// `git ls-remote`): connect, read the advertised refs, disconnect. Non-mutating
    /// — used to plan a selection before any fetch (Q1).
    fn ls_remote(&self, path: &Path, remote: &str) -> ModelResult<Vec<GitRemoteRef>>;
    fn fast_forward(
        &self,
        path: &Path,
        branch: &str,
        upstream_ref: &str,
    ) -> ModelResult<GitUpdateResult>;
    /// Integrate `upstream_ref` into `branch` by **merge** (porcelain `git merge`):
    /// fast-forward when the branch is strictly behind, else record a two-parent merge
    /// commit. On conflicts, leave the worktree mid-merge — `MERGE_HEAD` recorded so
    /// `git merge --continue` works — and return the conflicted paths instead of erroring;
    /// a conflict is an expected, developer-resolved outcome, not a failure. Self-verifies.
    fn merge_upstream(
        &self,
        path: &Path,
        branch: &str,
        upstream_ref: &str,
    ) -> ModelResult<GitIntegrateResult>;
    /// Integrate one exact source commit only while `branch` still points at
    /// `expected_before`. The implementation holds the branch ref lock across
    /// revalidation and mutation, uses `message` verbatim for a merge commit,
    /// and honors request-provided author and committer identities independently.
    fn merge_upstream_checked(
        &self,
        _path: &Path,
        _branch: &str,
        _expected_before: &str,
        _source_commit: &str,
        _message: &str,
        _attribution: Option<&crate::model::OperationAttribution>,
    ) -> ModelResult<GitIntegrateResult> {
        unsupported_backend("merge_upstream_checked")
    }
    /// Resolve source/target commits and classify the merge without mutation.
    /// Resolution is repository-local, performs no fetch, requires both sides
    /// to peel to commits, and rejects any native integration already in progress.
    /// `prediction_complete` is false only for a divergent true merge because
    /// this primitive deliberately does not modify or simulate the index.
    fn merge_analysis(
        &self,
        _path: &Path,
        _target_branch: &str,
        _source: &str,
    ) -> ModelResult<GitMergeAnalysis> {
        unsupported_backend("merge_analysis")
    }
    /// Optional M4 in-memory tree merge; never writes an index or worktree.
    fn merge_simulate(
        &self,
        _path: &Path,
        _target_commit: &str,
        _source_commit: &str,
    ) -> ModelResult<GitMergeSimulation> {
        unsupported_backend("merge_simulate")
    }
    /// Observe native merge metadata, including the exact MERGE_HEAD.
    fn merge_state(&self, _path: &Path) -> ModelResult<Option<GitNativeMergeState>> {
        unsupported_backend("merge_state")
    }
    /// Verify the exact recorded native merge and its index/worktree without
    /// mutating it. `require_resolved` selects continue safety; otherwise the
    /// check permits expected conflict-path work needed by native abort.
    fn validate_merge_recovery_state(
        &self,
        _path: &Path,
        _expected_before: &str,
        _expected_merge_head: &str,
        _require_resolved: bool,
    ) -> ModelResult<()> {
        unsupported_backend("validate_merge_recovery_state")
    }
    /// Abort only the expected native merge and verify restoration to before.
    fn abort_merge(
        &self,
        _path: &Path,
        _expected_before: &str,
        _expected_merge_head: &str,
    ) -> ModelResult<()> {
        unsupported_backend("abort_merge")
    }
    /// Move an attached branch only when its ref still equals expected_current.
    fn set_branch_target_checked(
        &self,
        _path: &Path,
        _branch: &str,
        _expected_current: &str,
        _target: &str,
    ) -> ModelResult<GitUpdateResult> {
        unsupported_backend("set_branch_target_checked")
    }
    /// Create and verify an exact local preservation ref.
    fn create_backup_ref(
        &self,
        _path: &Path,
        _name: &str,
        _target: &str,
    ) -> ModelResult<GitBackupRefResult> {
        unsupported_backend("create_backup_ref")
    }
    /// Save staged, unstaged, and optionally untracked preservation work.
    fn stash_for_merge_preservation(
        &self,
        _path: &Path,
        _merge_id: &str,
        _include_untracked: bool,
    ) -> ModelResult<GitStashPushResult> {
        unsupported_backend("stash_for_merge_preservation")
    }
    /// Commit only the supplied GWZ-owned candidate files through an isolated
    /// index and checked root ref update. `expected_head=None` means the root
    /// ref must be unborn and the first tree contains only candidate files.
    fn commit_gwz_paths_checked(
        &self,
        _root: &Path,
        _expected_head: Option<&str>,
        _candidate_files: &[GitCandidateFile],
        _message: &str,
    ) -> ModelResult<GitScopedCommitResult> {
        unsupported_backend("commit_gwz_paths_checked")
    }
    /// Integrate `upstream_ref` into `branch` by **rebase** (porcelain `git rebase`):
    /// replay the branch's commits onto the upstream tip. Fast-forwards when strictly
    /// behind. On conflict, leave `.git/rebase-merge/` in place (do NOT abort) so the
    /// developer can resolve and `git rebase --continue`, and return the conflicted
    /// paths instead of erroring. Self-verifies HEAD is reattached and based on upstream.
    fn rebase_onto(
        &self,
        path: &Path,
        branch: &str,
        upstream_ref: &str,
    ) -> ModelResult<GitIntegrateResult>;
    /// Snap `branch` to `upstream_ref` by **hard reset** (porcelain `git reset --hard`):
    /// discard local commits AND uncommitted changes, moving the branch onto upstream.
    /// Destructive and conflict-free; the caller gates it on `policy.destructive`.
    /// Self-verifies the branch (not a detached HEAD) is at the upstream commit, clean.
    fn reset_hard(
        &self,
        path: &Path,
        branch: &str,
        upstream_ref: &str,
    ) -> ModelResult<GitUpdateResult>;
    fn checkout_commit(&self, path: &Path, commit: &str) -> ModelResult<GitUpdateResult>;
    /// Put HEAD on `branch` at `commit` — create the branch if missing, checkout if it
    /// is already there. Per AD3(c)'s orphan-safety rule, REFUSE (`DivergedMember`) if
    /// the branch exists at a different commit — never silently reset it. Self-verifies
    /// HEAD is on the branch at the commit with a clean worktree.
    fn checkout_branch(
        &self,
        path: &Path,
        branch: &str,
        commit: &str,
    ) -> ModelResult<GitUpdateResult>;
    /// List local branches, sorted by branch name.
    fn branch_list(&self, _path: &Path) -> ModelResult<Vec<GitBranch>> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "branch_list is not implemented by this GitBackend",
        ))
    }
    /// Create local `branch` at `start_ref`. Existing branch at the same commit
    /// is a no-op success; existing branch at a different commit is refused.
    fn branch_create(
        &self,
        _path: &Path,
        _branch: &str,
        _start_ref: &str,
    ) -> ModelResult<GitBranchCreateResult> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "branch_create is not implemented by this GitBackend",
        ))
    }
    /// Delete a local branch. Refuses to delete the currently checked-out branch.
    fn branch_delete(&self, _path: &Path, _branch: &str) -> ModelResult<()> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "branch_delete is not implemented by this GitBackend",
        ))
    }
    /// Check out an existing branch without moving it. Self-verifies HEAD is
    /// attached to the requested branch.
    fn switch_branch(&self, _path: &Path, _branch: &str) -> ModelResult<GitUpdateResult> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "switch_branch is not implemented by this GitBackend",
        ))
    }
    /// Save local changes to the native stash stack. The default options are tracked-only.
    fn stash_push(
        &self,
        _path: &Path,
        _message: &str,
        _options: GitStashPushOptions,
    ) -> ModelResult<GitStashPushResult> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "stash_push is not implemented by this GitBackend",
        ))
    }
    /// List native stash entries in stack order (`stash@{0}` first).
    fn stash_list(&self, _path: &Path) -> ModelResult<Vec<GitStashEntry>> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "stash_list is not implemented by this GitBackend",
        ))
    }
    /// Apply a native stash without dropping it.
    fn stash_apply(
        &self,
        _path: &Path,
        _target: &GitStashTarget,
        _options: GitStashRestoreOptions,
    ) -> ModelResult<()> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "stash_apply is not implemented by this GitBackend",
        ))
    }
    /// Apply a native stash and drop it only if application succeeds.
    fn stash_pop(
        &self,
        _path: &Path,
        _target: &GitStashTarget,
        _options: GitStashRestoreOptions,
    ) -> ModelResult<()> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "stash_pop is not implemented by this GitBackend",
        ))
    }
    /// Drop a native stash entry without applying it.
    fn stash_drop(&self, _path: &Path, _target: &GitStashTarget) -> ModelResult<()> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "stash_drop is not implemented by this GitBackend",
        ))
    }
    fn status(&self, path: &Path) -> ModelResult<GitStatus>;
    fn status_with_options(
        &self,
        path: &Path,
        _options: GitStatusOptions,
    ) -> ModelResult<GitStatus> {
        self.status(path)
    }
    fn head(&self, path: &Path) -> ModelResult<GitHeadState>;
    fn remotes(&self, path: &Path) -> ModelResult<Vec<GitRemote>>;
    fn add_remote(&self, path: &Path, name: &str, url: &str) -> ModelResult<GitRemoteResult>;
    fn push(&self, path: &Path, remote: &str, refspec: &str) -> ModelResult<GitPushResult>;
    fn read_ref(&self, path: &Path, ref_spec: &str) -> ModelResult<Option<String>>;
    fn is_ancestor(&self, path: &Path, ancestor: &str, descendant: &str) -> ModelResult<bool>;
    /// Return the best merge base for two commits, when one exists.
    fn merge_base(&self, _path: &Path, _left: &str, _right: &str) -> ModelResult<Option<String>> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "merge_base is not implemented by this GitBackend",
        ))
    }
    /// List paths whose tree entries differ between two commits.
    fn changed_paths_between(
        &self,
        _path: &Path,
        _old_commit: &str,
        _new_commit: &str,
    ) -> ModelResult<Vec<String>> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "changed_paths_between is not implemented by this GitBackend",
        ))
    }
    /// Diff a **single** repository (the workspace root or one materialized
    /// member) into a repo-scoped changed-file manifest. This is the D1 Git
    /// backend primitive: it resolves the requested comparison to libgit2 tree
    /// sides, runs the matching libgit2 diff, applies rename detection, and
    /// reports per-file status/mode/binary/similarity/line-stats with
    /// repo-relative paths. Workspace projection (scopes, member-prefix
    /// rewriting, root/member ordering, `gwz.conf` exclusion) is the D2 planner's
    /// job, not this primitive's. Paths in `comparison`/`options` are already
    /// repo-relative. See [`crate::diff::diff_repo`].
    fn diff_manifest(
        &self,
        _path: &Path,
        _comparison: &crate::diff::RepoDiffComparison,
        _options: &crate::diff::RepoDiffOptions,
    ) -> ModelResult<crate::diff::RepoDiffManifest> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "diff_manifest is not implemented by this GitBackend",
        ))
    }
    /// Resolve a per-repo comparison from raw revision tokens to concrete
    /// libgit2 tree sides (peeling refs/commits to trees, `HEAD`/unborn-HEAD to a
    /// tree or the empty tree, and a `A...B` merge-base old side). Snapshot
    /// operand resolution and candidate selection are D2; this handles only the
    /// per-repo revision → oid step of the primitive. See
    /// [`crate::diff::resolve_comparison`].
    fn resolve_comparison(
        &self,
        _path: &Path,
        _spec: &crate::diff::ComparisonSpec,
    ) -> ModelResult<crate::diff::RepoDiffComparison> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "resolve_comparison is not implemented by this GitBackend",
        ))
    }
    /// Stage `pathspecs` into the index — `git add` semantics: add new/modified
    /// files, remove deleted ones, honor `.gitignore`. Self-verifies the index
    /// persisted with the requested files staged before returning success.
    /// Content parity with porcelain `git add` is proven by contract test.
    fn stage_paths(&self, path: &Path, pathspecs: &[&str]) -> ModelResult<GitStageResult>;
    /// Stage resolved paths while unrelated conflicts remain in the index.
    fn stage_paths_allowing_other_conflicts(
        &self,
        path: &Path,
        pathspecs: &[&str],
    ) -> ModelResult<GitStageResult> {
        self.stage_paths(path, pathspecs)
    }
    /// Commit staged changes (or, with `all`, stage tracked modifications first —
    /// `git commit -a`) via the `git` CLI, so hooks, signing, and committer config are
    /// honored (AD1 per-primitive CLI fallback — libgit2's commit bypasses all of them).
    /// Returns the new commit oid. Self-verifies HEAD advanced to a new commit before
    /// returning. The caller must ensure there is something to commit (no empty commits).
    fn commit(&self, path: &Path, message: &str, all: bool) -> ModelResult<GitCommitResult>;
    /// Commit an in-progress merge after the caller has resolved and staged conflicts.
    /// The default fallback uses porcelain `git commit`; Git2Backend overrides this so
    /// gwz-created merge resolutions also work without user git identity config.
    fn commit_merge_resolution(&self, path: &Path, message: &str) -> ModelResult<GitCommitResult> {
        self.commit(path, message, false)
    }
    /// Commit a resolved merge under a checked parent/ref safety boundary.
    fn commit_merge_resolution_checked(
        &self,
        _path: &Path,
        _expected_before: &str,
        _expected_merge_head: &str,
        _message: &str,
    ) -> ModelResult<GitCommitResult> {
        unsupported_backend("commit_merge_resolution_checked")
    }
    /// Create tag `name` at the current HEAD via the `git` CLI (AD1 per-primitive CLI
    /// fallback — so hooks, signing, and tagger config are honored). Annotated when
    /// `message` is set; signed when `signed` (signing requires a message + GPG config).
    /// Self-verifies the tag exists, returning its peeled target commit oid.
    fn tag_create(
        &self,
        path: &Path,
        name: &str,
        message: Option<&str>,
        signed: bool,
    ) -> ModelResult<GitTagResult>;
    /// All tag names in the repo, sorted.
    fn tag_list(&self, path: &Path) -> ModelResult<Vec<String>>;
    /// Delete tag `name`. Self-verifies it no longer exists before returning.
    fn tag_delete(&self, path: &Path, name: &str) -> ModelResult<()>;

    /// Fetch tags from a remote into local refs (force-updating local copies).
    fn tag_fetch(&self, path: &Path, remote: &str) -> ModelResult<GitFetchResult>;
}

fn unsupported_backend<T>(method: &str) -> ModelResult<T> {
    Err(ModelError::new(
        ErrorCode::UnsupportedOperation,
        format!("{method} is not implemented by this GitBackend"),
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialHelperPolicy {
    Disabled,
    AllowConfigured,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Git2Backend {
    pub(crate) credential_helpers: CredentialHelperPolicy,
}

impl Git2Backend {
    pub fn new() -> Self {
        Self {
            credential_helpers: CredentialHelperPolicy::AllowConfigured,
        }
    }

    pub fn without_credential_helpers() -> Self {
        Self {
            credential_helpers: CredentialHelperPolicy::Disabled,
        }
    }
}

impl Default for Git2Backend {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod merge_interface_tests {
    use super::*;

    #[test]
    fn deferred_merge_primitive_has_typed_unsupported_default() {
        let backend = Git2Backend::new();
        let error = backend
            .merge_simulate(Path::new("missing"), "before", "source")
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedOperation);
    }

    #[test]
    fn status_contract_distinguishes_recovery_relevant_dirt() {
        let status = GitStatus {
            staged: 1,
            unstaged: 2,
            untracked: 3,
            ignored: 4,
            unresolved: 5,
            ..GitStatus::default()
        };
        assert_eq!(
            (status.staged, status.unstaged, status.untracked),
            (1, 2, 3)
        );
        assert_eq!((status.ignored, status.unresolved), (4, 5));
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitCreateResult {
    pub path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitCloneResult {
    pub path: PathBuf,
    pub head: GitHeadState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitFetchResult {
    pub remote: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRemoteRef {
    /// Full ref name as advertised by the remote (e.g. `refs/heads/main`, `HEAD`).
    pub name: String,
    /// Object id the ref points at, as a hex string.
    pub target: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitUpdateResult {
    pub updated: bool,
    pub commit: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitBranch {
    pub name: String,
    pub commit: String,
    pub is_current: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitBranchCreateResult {
    pub branch: GitBranch,
    pub created: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GitStashPushOptions {
    pub include_untracked: bool,
    pub include_ignored: bool,
    /// Preserve staged index entries in the worktree after pushing, matching
    /// `git stash push --keep-index`.
    pub preserve_index: bool,
}

impl GitStashPushOptions {
    pub fn tracked_only() -> Self {
        Self::default()
    }

    pub fn include_untracked() -> Self {
        Self {
            include_untracked: true,
            ..Self::default()
        }
    }

    pub fn include_ignored() -> Self {
        Self {
            include_untracked: true,
            include_ignored: true,
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GitStashRestoreOptions {
    /// Default restore attempts to reinstate the index (`git stash apply --index`).
    pub preserve_index: bool,
}

impl Default for GitStashRestoreOptions {
    fn default() -> Self {
        Self {
            preserve_index: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitStashPushResult {
    pub object_id: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitStashEntry {
    pub index: usize,
    pub object_id: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GitStashTarget {
    /// Exact native stash object id. This may target any stash, including non-GWZ stashes.
    pub object_id: Option<String>,
    /// GWZ message prefix fallback, e.g. `gwz:stash_123:`. Prefix fallback is
    /// intentionally restricted to `gwz:` messages so non-GWZ stashes are never
    /// mutated by fuzzy identity after native indices move.
    pub gwz_message_prefix: Option<String>,
}

impl GitStashTarget {
    pub fn object_id(object_id: impl Into<String>) -> Self {
        Self {
            object_id: Some(object_id.into()),
            gwz_message_prefix: None,
        }
    }

    pub fn gwz_message_prefix(prefix: impl Into<String>) -> Self {
        Self {
            object_id: None,
            gwz_message_prefix: Some(prefix.into()),
        }
    }
}

/// Outcome of a merge/rebase integration. A conflict is reported, not errored:
/// `conflicts` names the paths and `commit` is `None`, with the worktree left
/// mid-integration for the developer to resolve — exactly as porcelain git leaves it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitIntegrateResult {
    /// New HEAD commit when the integration completed cleanly; `None` on conflict.
    pub commit: Option<String>,
    /// Conflicted paths; empty iff the integration completed cleanly.
    pub conflicts: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitMergeAnalysisKind {
    UpToDate,
    FastForward,
    TrueMerge,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitMergeAnalysis {
    pub target_branch: String,
    pub target_commit: String,
    pub source_commit: String,
    pub kind: GitMergeAnalysisKind,
    pub commit_identity_required: bool,
    /// False for a true merge that has not run the optional simulation seam.
    pub prediction_complete: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GitMergeSimulation {
    Clean,
    Conflicts(Vec<String>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitNativeMergeState {
    pub merge_head: String,
    pub conflict_paths: Vec<String>,
    pub unresolved_entries: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitBackupRefResult {
    pub name: String,
    pub target: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitCandidateFile {
    pub path: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitCandidateHash {
    pub path: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitScopedCommitResult {
    pub commit: String,
    pub tree: String,
    pub candidate_hashes: Vec<GitCandidateHash>,
}

impl GitIntegrateResult {
    pub(crate) fn clean(commit: String) -> Self {
        Self {
            commit: Some(commit),
            conflicts: Vec::new(),
        }
    }

    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRemoteResult {
    pub remote: GitRemote,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitPushResult {
    pub remote: String,
    pub refspec: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitStageResult {
    /// Top-level *file* pathspecs confirmed present in the index by the self-verify
    /// pass. Directory pathspecs are staged but not counted here.
    pub staged: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitCommitResult {
    /// The new commit oid created by this commit.
    pub commit: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitTagResult {
    /// The tag name created.
    pub name: String,
    /// The commit oid the tag points at (peeled through annotated tags).
    pub commit: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GitStatus {
    pub is_dirty: bool,
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
    pub ignored: usize,
    pub unresolved: usize,
    pub files: Vec<GitFileStatus>,
}

impl GitStatus {
    pub fn clean() -> Self {
        Self::default()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GitStatusOptions {
    pub include_ignored: bool,
}

impl GitStatusOptions {
    pub fn include_ignored() -> Self {
        Self {
            include_ignored: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitFileStatus {
    pub path: String,
    pub index_status: String,
    pub worktree_status: String,
    pub original_path: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitHeadState {
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub is_detached: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRemote {
    pub name: String,
    pub url: Option<String>,
    pub push_url: Option<String>,
}

impl GitBackend for Git2Backend {
    fn is_repository(&self, path: &Path) -> ModelResult<bool> {
        match git2::Repository::open(path) {
            Ok(_) => Ok(true),
            Err(err) if err.code() == git2::ErrorCode::NotFound => Ok(false),
            Err(err) => Err(git_error(err)),
        }
    }

    fn commit_exists(&self, path: &Path, oid: &str) -> ModelResult<bool> {
        let Ok(oid) = git2::Oid::from_str(oid) else {
            return Ok(false);
        };
        let repo = open_repo(path)?;
        let object = match repo.find_object(oid, None) {
            Ok(object) => object,
            Err(error) if error.code() == git2::ErrorCode::NotFound => return Ok(false),
            Err(error) => return Err(git_error(error)),
        };
        Ok(object.peel_to_commit().is_ok())
    }

    fn create_repo(&self, path: &Path) -> ModelResult<GitCreateResult> {
        let mut opts = git2::RepositoryInitOptions::new();
        opts.bare(false).no_reinit(true).initial_head("main");
        git2::Repository::init_opts(path, &opts).map_err(git_error)?;
        Ok(GitCreateResult {
            path: path.to_path_buf(),
        })
    }

    fn clone_repo(&self, url: &str, path: &Path) -> ModelResult<GitCloneResult> {
        self.clone_repo_with_progress(url, path, &|_progress| {})
    }

    fn clone_repo_with_progress(
        &self,
        url: &str,
        path: &Path,
        progress: &dyn Fn(crate::GitTransferProgress),
    ) -> ModelResult<GitCloneResult> {
        ensure_clone_target_is_empty(path)?;
        let mut builder = git2::build::RepoBuilder::new();
        builder.fetch_options(fetch_options_with_progress(
            self.credential_helpers,
            Some(progress),
        ));
        builder.clone(url, path).map_err(git_error)?;
        Ok(GitCloneResult {
            path: path.to_path_buf(),
            head: self.head(path)?,
        })
    }

    fn fetch(&self, path: &Path, remote: &str) -> ModelResult<GitFetchResult> {
        let repo = open_repo(path)?;
        let mut remote_handle = find_remote(&repo, remote)?;
        let refspecs: [&str; 0] = [];
        remote_handle
            .fetch(
                &refspecs,
                Some(&mut remote_fetch_options(self.credential_helpers)),
                Some("gwz fetch"),
            )
            .map_err(git_error)?;
        Ok(GitFetchResult {
            remote: remote.to_owned(),
        })
    }

    fn tag_fetch(&self, path: &Path, remote: &str) -> ModelResult<GitFetchResult> {
        let repo = open_repo(path)?;
        let mut remote_handle = find_remote(&repo, remote)?;
        // Fetch every tag, force-updating local copies.
        let refspec = "+refs/tags/*:refs/tags/*";
        remote_handle
            .fetch(
                &[refspec],
                Some(&mut remote_fetch_options(self.credential_helpers)),
                Some("gwz tag fetch"),
            )
            .map_err(git_error)?;
        Ok(GitFetchResult {
            remote: remote.to_owned(),
        })
    }

    fn ls_remote(&self, path: &Path, remote: &str) -> ModelResult<Vec<GitRemoteRef>> {
        let repo = open_repo(path)?;
        let mut remote_handle = find_remote(&repo, remote)?;
        let connection = remote_handle
            .connect_auth(
                git2::Direction::Fetch,
                Some(remote_callbacks(self.credential_helpers)),
                None,
            )
            .map_err(git_error)?;
        let refs = connection
            .list()
            .map_err(git_error)?
            .iter()
            .map(|head| GitRemoteRef {
                name: head.name().to_owned(),
                target: head.oid().to_string(),
            })
            .collect::<Vec<_>>();
        // `connection` disconnects on drop.
        Ok(refs)
    }

    fn fast_forward(
        &self,
        path: &Path,
        branch: &str,
        upstream_ref: &str,
    ) -> ModelResult<GitUpdateResult> {
        let repo = open_repo(path)?;
        let target = repo.revparse_single(upstream_ref).map_err(git_error)?.id();
        let annotated = repo.find_annotated_commit(target).map_err(git_error)?;
        let (analysis, _) = repo.merge_analysis(&[&annotated]).map_err(git_error)?;

        if analysis.is_up_to_date() {
            return Ok(GitUpdateResult {
                updated: false,
                commit: Some(target.to_string()),
            });
        }
        if !analysis.is_fast_forward() {
            return Err(ModelError::new(
                ErrorCode::DivergedMember,
                "branch cannot be fast-forwarded",
            ));
        }

        let local_ref_name = format!("refs/heads/{branch}");
        let mut local_ref = repo.find_reference(&local_ref_name).map_err(git_error)?;
        let target_object = repo.find_object(target, None).map_err(git_error)?;
        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.safe();
        repo.checkout_tree(&target_object, Some(&mut checkout))
            .map_err(git_error)?;
        local_ref
            .set_target(target, "gwz fast-forward")
            .map_err(git_error)?;
        repo.set_head(&local_ref_name).map_err(git_error)?;
        verify_checkout_state(path, target)?;
        Ok(GitUpdateResult {
            updated: true,
            commit: Some(target.to_string()),
        })
    }

    fn merge_upstream(
        &self,
        path: &Path,
        branch: &str,
        upstream_ref: &str,
    ) -> ModelResult<GitIntegrateResult> {
        let status = self.status(path)?;
        if status.is_dirty {
            return Err(ModelError::new(
                ErrorCode::DirtyMember,
                "merge requires a clean index and worktree",
            ));
        }
        let plan = self.merge_analysis(path, branch, upstream_ref)?;
        self.merge_upstream_checked(
            path,
            branch,
            &plan.target_commit,
            &plan.source_commit,
            &format!("Merge {upstream_ref} into {branch}"),
            None,
        )
    }

    fn merge_upstream_checked(
        &self,
        path: &Path,
        branch: &str,
        expected_before: &str,
        source_commit: &str,
        message: &str,
        attribution: Option<&crate::model::OperationAttribution>,
    ) -> ModelResult<GitIntegrateResult> {
        let expected = git2::Oid::from_str(expected_before).map_err(git_error)?;
        let source = git2::Oid::from_str(source_commit).map_err(git_error)?;
        let expected_text = expected.to_string();
        let source_text = source.to_string();
        if message.contains('\0') {
            return Err(ModelError::new(
                ErrorCode::InvalidRequest,
                "merge commit message contains a NUL byte",
            ));
        }

        let repo = open_repo(path)?;
        let local_ref_name = branch_ref_name(branch);
        let mut transaction = repo.transaction().map_err(git_error)?;
        transaction.lock_ref(&local_ref_name).map_err(git_error)?;

        ensure_no_integration_in_progress(&repo)?;
        let status = self.status(path)?;
        if status.is_dirty {
            return Err(ModelError::new(
                ErrorCode::DirtyMember,
                "merge requires a clean index and worktree",
            ));
        }
        let target = repo
            .find_reference(&local_ref_name)
            .and_then(|reference| reference.peel_to_commit())
            .map_err(git_error)?
            .id();
        let observed = repo_head(&repo)?;
        if target != expected
            || observed.branch.as_deref() != Some(branch)
            || observed.commit.as_deref() != Some(expected_text.as_str())
        {
            return Err(ModelError::new(
                ErrorCode::MergeDrift,
                format!(
                    "target branch '{branch}' changed before merge mutation; expected {expected_before}"
                ),
            ));
        }
        let source_object = repo.find_commit(source).map_err(git_error)?;
        let kind = classify_merge(&repo, expected, source)?;

        if kind == GitMergeAnalysisKind::UpToDate {
            drop(source_object);
            drop(transaction);
            verify_merge_result(self, path, branch, &expected_text)?;
            return Ok(GitIntegrateResult::clean(expected_text));
        }
        if kind == GitMergeAnalysisKind::FastForward {
            let target_object = repo.find_object(source, None).map_err(git_error)?;
            let mut checkout = git2::build::CheckoutBuilder::new();
            checkout.safe();
            repo.checkout_tree(&target_object, Some(&mut checkout))
                .map_err(git_error)?;
            transaction
                .set_target(&local_ref_name, source, None, "gwz fast-forward")
                .map_err(git_error)?;
            transaction.commit().map_err(git_error)?;
            verify_merge_result(self, path, branch, &source_text)?;
            return Ok(GitIntegrateResult::clean(source_text));
        }

        let (author, committer) = merge_signatures(&repo, attribution)?;
        let annotated = repo.find_annotated_commit(source).map_err(git_error)?;

        // True three-way merge: git2 stages the result into the index + worktree.
        repo.merge(&[&annotated], None, None).map_err(git_error)?;
        let mut index = repo.index().map_err(git_error)?;
        if index.has_conflicts() {
            // Faithful to porcelain: leave the conflict in the worktree and record
            // MERGE_HEAD so the developer can resolve and `git merge --continue`.
            std::fs::write(repo.path().join("MERGE_HEAD"), format!("{source}\n"))
                .map_err(|err| ModelError::new(ErrorCode::GitCommandFailed, err.to_string()))?;
            let conflicts = conflict_paths(&index)?;
            // AD1 self-verify: the conflict state actually persisted on disk.
            let state = self.merge_state(path)?;
            let conflict_head = self.head(path)?;
            if conflicts.is_empty()
                || conflict_head.commit.as_deref() != Some(expected_text.as_str())
                || state.as_ref().is_none_or(|state| {
                    state.merge_head != source.to_string() || state.conflict_paths != conflicts
                })
            {
                return Err(ModelError::new(
                    ErrorCode::GitCommandFailed,
                    "merge conflict state did not persist with the expected MERGE_HEAD",
                ));
            }
            return Ok(GitIntegrateResult {
                commit: None,
                conflicts,
            });
        }

        // Clean merge: write the merged tree and record the two-parent merge commit.
        let tree_oid = index.write_tree().map_err(git_error)?;
        let tree = repo.find_tree(tree_oid).map_err(git_error)?;
        let head_commit = repo.find_commit(expected).map_err(git_error)?;
        let merge_oid = repo
            .commit(
                None,
                &author,
                &committer,
                message,
                &tree,
                &[&head_commit, &source_object],
            )
            .map_err(git_error)?;
        let committed = repo.find_commit(merge_oid).map_err(git_error)?;
        if committed.parent_count() != 2
            || committed.parent_id(0).map_err(git_error)? != head_commit.id()
            || committed.parent_id(1).map_err(git_error)? != source
            || committed.message_bytes() != message.as_bytes()
            || !same_signature(&committed.author(), &author)
            || !same_signature(&committed.committer(), &committer)
        {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                "post-merge commit metadata does not match the checked merge plan",
            ));
        }
        transaction
            .set_target(&local_ref_name, merge_oid, Some(&committer), "gwz merge")
            .map_err(git_error)?;
        transaction.commit().map_err(git_error)?;
        repo.cleanup_state().map_err(git_error)?;
        verify_merge_result(self, path, branch, &merge_oid.to_string())?;
        Ok(GitIntegrateResult::clean(merge_oid.to_string()))
    }

    fn merge_analysis(
        &self,
        path: &Path,
        target_branch: &str,
        source: &str,
    ) -> ModelResult<GitMergeAnalysis> {
        let repo = open_repo(path)?;
        ensure_no_integration_in_progress(&repo)?;
        let target_commit = repo
            .find_reference(&branch_ref_name(target_branch))
            .and_then(|reference| reference.peel_to_commit())
            .map_err(git_error)?
            .id();
        let source_commit = resolve_commit_oid(&repo, source)?;
        let kind = classify_merge(&repo, target_commit, source_commit)?;
        Ok(GitMergeAnalysis {
            target_branch: target_branch.to_owned(),
            target_commit: target_commit.to_string(),
            source_commit: source_commit.to_string(),
            kind,
            commit_identity_required: kind == GitMergeAnalysisKind::TrueMerge,
            prediction_complete: kind != GitMergeAnalysisKind::TrueMerge,
        })
    }

    fn merge_state(&self, path: &Path) -> ModelResult<Option<GitNativeMergeState>> {
        let repo = open_repo(path)?;
        if repo.state() != git2::RepositoryState::Merge {
            return Ok(None);
        }
        let merge_head =
            std::fs::read_to_string(repo.path().join("MERGE_HEAD")).map_err(|err| {
                ModelError::new(
                    ErrorCode::GitCommandFailed,
                    format!("failed to read MERGE_HEAD: {err}"),
                )
            })?;
        let merge_oid = resolve_commit_oid(&repo, merge_head.trim())?;
        let index = repo.index().map_err(git_error)?;
        let conflict_paths = conflict_paths(&index)?;
        Ok(Some(GitNativeMergeState {
            merge_head: merge_oid.to_string(),
            unresolved_entries: conflict_paths.len(),
            conflict_paths,
        }))
    }

    fn validate_merge_recovery_state(
        &self,
        path: &Path,
        expected_before: &str,
        expected_merge_head: &str,
        require_resolved: bool,
    ) -> ModelResult<()> {
        let repo = open_repo(path)?;
        let before = parse_existing_commit(&repo, expected_before)?;
        let merge_head = parse_existing_commit(&repo, expected_merge_head)?;
        validate_expected_native_merge(&repo, before, merge_head)?;
        if require_resolved {
            validate_resolution_index_and_worktree(self, path, &repo, before, merge_head)
        } else {
            validate_abort_index_and_worktree(self, path, &repo, before, merge_head)
        }
    }

    fn abort_merge(
        &self,
        path: &Path,
        expected_before: &str,
        expected_merge_head: &str,
    ) -> ModelResult<()> {
        let repo = open_repo(path)?;
        let before = parse_existing_commit(&repo, expected_before)?;
        let merge_head = parse_existing_commit(&repo, expected_merge_head)?;

        if repo.state() == git2::RepositoryState::Clean {
            verify_restored_merge_state(self, path, before)?;
            return Ok(());
        }
        let ref_name = attached_head_ref(&repo)?;
        let mut transaction = repo.transaction().map_err(git_error)?;
        transaction.lock_ref(&ref_name).map_err(git_error)?;
        validate_expected_native_merge(&repo, before, merge_head)?;
        validate_abort_index_and_worktree(self, path, &repo, before, merge_head)?;

        let target = repo.find_commit(before).map_err(git_error)?;
        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout
            .force()
            .remove_untracked(false)
            .remove_ignored(false);
        repo.checkout_tree(target.as_object(), Some(&mut checkout))
            .map_err(git_error)?;
        let target_tree = target.tree().map_err(git_error)?;
        let mut index = repo.index().map_err(git_error)?;
        index.read_tree(&target_tree).map_err(git_error)?;
        index.write().map_err(git_error)?;
        repo.cleanup_state().map_err(git_error)?;
        drop(transaction);
        verify_restored_merge_state(self, path, before)
    }

    fn set_branch_target_checked(
        &self,
        path: &Path,
        branch: &str,
        expected_current: &str,
        target: &str,
    ) -> ModelResult<GitUpdateResult> {
        let repo = open_repo(path)?;
        let expected = parse_existing_commit(&repo, expected_current)?;
        let target = parse_existing_commit(&repo, target)?;
        let ref_name = branch_ref_name(branch);
        let mut transaction = repo.transaction().map_err(git_error)?;
        transaction.lock_ref(&ref_name).map_err(git_error)?;

        ensure_clean_recovery_state(self, path, &repo, branch)?;
        let current = repo
            .find_reference(&ref_name)
            .and_then(|reference| reference.peel_to_commit())
            .map_err(git_error)?
            .id();
        if current == target {
            drop(transaction);
            verify_merge_result(self, path, branch, &target.to_string())?;
            return Ok(GitUpdateResult {
                updated: false,
                commit: Some(target.to_string()),
            });
        }
        if current != expected {
            return Err(ModelError::new(
                ErrorCode::MergeDrift,
                format!(
                    "branch '{branch}' changed before rollback; expected {expected}, observed {current}"
                ),
            ));
        }

        let target_object = repo.find_object(target, None).map_err(git_error)?;
        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.safe();
        repo.checkout_tree(&target_object, Some(&mut checkout))
            .map_err(git_error)?;
        transaction
            .set_target(&ref_name, target, None, "gwz checked merge rollback")
            .map_err(git_error)?;
        transaction.commit().map_err(git_error)?;
        verify_merge_result(self, path, branch, &target.to_string())?;
        Ok(GitUpdateResult {
            updated: true,
            commit: Some(target.to_string()),
        })
    }

    fn rebase_onto(
        &self,
        path: &Path,
        branch: &str,
        upstream_ref: &str,
    ) -> ModelResult<GitIntegrateResult> {
        let repo = open_repo(path)?;
        let upstream_oid = repo.revparse_single(upstream_ref).map_err(git_error)?.id();
        let upstream_annotated = repo
            .find_annotated_commit(upstream_oid)
            .map_err(git_error)?;
        let (analysis, _) = repo
            .merge_analysis(&[&upstream_annotated])
            .map_err(git_error)?;
        if analysis.is_up_to_date() {
            return Ok(GitIntegrateResult::clean(upstream_oid.to_string()));
        }
        if analysis.is_fast_forward() {
            // Nothing to replay: `git rebase` of a strictly-behind branch fast-forwards.
            let ff = self.fast_forward(path, branch, upstream_ref)?;
            return Ok(GitIntegrateResult {
                commit: ff.commit,
                conflicts: Vec::new(),
            });
        }

        let signature = merge_signature(&repo)?;
        let mut rebase = repo
            .rebase(None, Some(&upstream_annotated), None, None)
            .map_err(git_error)?;
        // Replay each commit; git2 patches it into the index + worktree on `next()`. The
        // operation handle is dropped within the loop condition so the body can re-borrow.
        while rebase.next().transpose().map_err(git_error)?.is_some() {
            let index = repo.index().map_err(git_error)?;
            if index.has_conflicts() {
                // Faithful to porcelain: leave the rebase in progress for the developer
                // to resolve and `git rebase --continue`. Dropping `rebase` frees the
                // in-memory handle but leaves `.git/rebase-merge/` on disk.
                return Ok(GitIntegrateResult {
                    commit: None,
                    conflicts: conflict_paths(&index)?,
                });
            }
            rebase.commit(None, &signature, None).map_err(git_error)?;
        }
        rebase.finish(Some(&signature)).map_err(git_error)?;

        // AD1 self-verify: HEAD reattached to the branch and now descends from upstream.
        let observed = self.head(path)?;
        let Some(new_head) = observed.commit.clone() else {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                "post-rebase HEAD is unborn",
            ));
        };
        if observed.is_detached || observed.branch.as_deref() != Some(branch) {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                format!("post-rebase HEAD is not on branch '{branch}'"),
            ));
        }
        if !self.is_ancestor(path, &upstream_oid.to_string(), &new_head)? {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                "post-rebase HEAD is not based on upstream",
            ));
        }
        Ok(GitIntegrateResult::clean(new_head))
    }

    fn reset_hard(
        &self,
        path: &Path,
        branch: &str,
        upstream_ref: &str,
    ) -> ModelResult<GitUpdateResult> {
        let repo = open_repo(path)?;
        let target = repo.revparse_single(upstream_ref).map_err(git_error)?.id();
        let target_object = repo.find_object(target, None).map_err(git_error)?;
        repo.reset(&target_object, git2::ResetType::Hard, None)
            .map_err(git_error)?;
        verify_checkout_state(path, target)?;
        // AD1 self-verify: the branch (not a detached HEAD) now points at upstream.
        let observed = self.head(path)?;
        if observed.is_detached || observed.branch.as_deref() != Some(branch) {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                format!("post-reset HEAD is not on branch '{branch}'"),
            ));
        }
        Ok(GitUpdateResult {
            updated: true,
            commit: Some(target.to_string()),
        })
    }

    fn checkout_commit(&self, path: &Path, commit: &str) -> ModelResult<GitUpdateResult> {
        let repo = open_repo(path)?;
        let oid = git2::Oid::from_str(commit).map_err(git_error)?;
        let object = repo.find_object(oid, None).map_err(git_error)?;
        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.safe();
        repo.checkout_tree(&object, Some(&mut checkout))
            .map_err(git_error)?;
        repo.set_head_detached(oid).map_err(git_error)?;
        verify_checkout_state(path, oid)?;
        Ok(GitUpdateResult {
            updated: true,
            commit: Some(oid.to_string()),
        })
    }

    fn checkout_branch(
        &self,
        path: &Path,
        branch: &str,
        commit: &str,
    ) -> ModelResult<GitUpdateResult> {
        let repo = open_repo(path)?;
        let oid = git2::Oid::from_str(commit).map_err(git_error)?;
        let ref_name = branch_ref_name(branch);
        // AD3(c) orphan-safety: shared with branch_create. Create if missing; refuse
        // if it already exists at a different commit (that would orphan work).
        ensure_branch_at_commit(&repo, branch, oid)?;
        let object = repo.find_object(oid, None).map_err(git_error)?;
        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.safe();
        repo.checkout_tree(&object, Some(&mut checkout))
            .map_err(git_error)?;
        repo.set_head(&ref_name).map_err(git_error)?;
        verify_checkout_state(path, oid)?;
        // AD1 self-verify: HEAD is attached to the branch, not detached.
        let observed = self.head(path)?;
        if observed.is_detached || observed.branch.as_deref() != Some(branch) {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                format!("post-checkout HEAD is not on branch '{branch}'"),
            ));
        }
        Ok(GitUpdateResult {
            updated: true,
            commit: Some(oid.to_string()),
        })
    }

    fn branch_list(&self, path: &Path) -> ModelResult<Vec<GitBranch>> {
        let repo = open_repo(path)?;
        let current = repo_head(&repo)?.branch;
        let mut branches = Vec::new();
        for entry in repo
            .branches(Some(git2::BranchType::Local))
            .map_err(git_error)?
        {
            let (branch, _) = entry.map_err(git_error)?;
            branches.push(git_branch_record(&branch, current.as_deref())?);
        }
        branches.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(branches)
    }

    fn branch_create(
        &self,
        path: &Path,
        branch: &str,
        start_ref: &str,
    ) -> ModelResult<GitBranchCreateResult> {
        let repo = open_repo(path)?;
        let oid = resolve_commit_oid(&repo, start_ref)?;
        let created = ensure_branch_at_commit(&repo, branch, oid)?;
        let current = repo_head(&repo)?.branch;
        Ok(GitBranchCreateResult {
            branch: GitBranch {
                name: branch.to_owned(),
                commit: oid.to_string(),
                is_current: current.as_deref() == Some(branch),
            },
            created,
        })
    }

    fn branch_delete(&self, path: &Path, branch: &str) -> ModelResult<()> {
        let repo = open_repo(path)?;
        let current = repo_head(&repo)?.branch;
        if current.as_deref() == Some(branch) {
            return Err(ModelError::new(
                ErrorCode::InvalidRequest,
                format!("cannot delete current branch '{branch}'"),
            ));
        }
        repo.find_branch(branch, git2::BranchType::Local)
            .map_err(git_error)?
            .delete()
            .map_err(git_error)?;
        match repo.find_branch(branch, git2::BranchType::Local) {
            Ok(_) => Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                format!("branch '{branch}' still present after delete"),
            )),
            Err(err) if err.code() == git2::ErrorCode::NotFound => Ok(()),
            Err(err) => Err(git_error(err)),
        }
    }

    fn switch_branch(&self, path: &Path, branch: &str) -> ModelResult<GitUpdateResult> {
        let repo = open_repo(path)?;
        let local_branch = repo
            .find_branch(branch, git2::BranchType::Local)
            .map_err(git_error)?;
        let oid = local_branch.get().peel_to_commit().map_err(git_error)?.id();
        let object = repo.find_object(oid, None).map_err(git_error)?;
        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.safe();
        repo.checkout_tree(&object, Some(&mut checkout))
            .map_err(git_error)?;
        let ref_name = branch_ref_name(branch);
        repo.set_head(&ref_name).map_err(git_error)?;
        verify_checkout_state(path, oid)?;
        let observed = self.head(path)?;
        if observed.is_detached || observed.branch.as_deref() != Some(branch) {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                format!("post-switch HEAD is not on branch '{branch}'"),
            ));
        }
        Ok(GitUpdateResult {
            updated: true,
            commit: Some(oid.to_string()),
        })
    }

    fn stash_push(
        &self,
        path: &Path,
        message: &str,
        options: GitStashPushOptions,
    ) -> ModelResult<GitStashPushResult> {
        let mut repo = open_repo(path)?;
        let signature = merge_signature(&repo)?;
        let object_id = repo
            .stash_save(&signature, message, Some(stash_push_flags(options)))
            .map_err(git_error)?;
        Ok(GitStashPushResult {
            object_id: object_id.to_string(),
            message: message.to_owned(),
        })
    }

    fn stash_list(&self, path: &Path) -> ModelResult<Vec<GitStashEntry>> {
        let mut repo = open_repo(path)?;
        stash_entries(&mut repo)
    }

    fn stash_apply(
        &self,
        path: &Path,
        target: &GitStashTarget,
        options: GitStashRestoreOptions,
    ) -> ModelResult<()> {
        let mut repo = open_repo(path)?;
        let index = resolve_stash_index(&mut repo, target)?;
        let mut apply_options = stash_restore_options(options);
        // libgit2 applies through its merge/checkout machinery and can return
        // Conflict without writing porcelain-style conflict markers. Until GWZ
        // has stash-specific protocol errors, callers should treat GitCommandFailed
        // from this path as "native stash remains pending; inspect before retry".
        repo.stash_apply(index, Some(&mut apply_options))
            .map_err(stash_restore_error)
    }

    fn stash_pop(
        &self,
        path: &Path,
        target: &GitStashTarget,
        options: GitStashRestoreOptions,
    ) -> ModelResult<()> {
        let mut repo = open_repo(path)?;
        let index = resolve_stash_index(&mut repo, target)?;
        let mut apply_options = stash_restore_options(options);
        // Same conflict caveat as stash_apply: git2 does not guarantee porcelain
        // conflict-marker behavior. git_stash_pop drops only after a successful apply.
        repo.stash_pop(index, Some(&mut apply_options))
            .map_err(stash_restore_error)
    }

    fn stash_drop(&self, path: &Path, target: &GitStashTarget) -> ModelResult<()> {
        let mut repo = open_repo(path)?;
        let index = resolve_stash_index(&mut repo, target)?;
        repo.stash_drop(index).map_err(git_error)
    }

    fn status(&self, path: &Path) -> ModelResult<GitStatus> {
        self.status_with_options(path, GitStatusOptions::default())
    }

    fn status_with_options(
        &self,
        path: &Path,
        options: GitStatusOptions,
    ) -> ModelResult<GitStatus> {
        let repo = open_repo(path)?;
        let mut opts = git2::StatusOptions::new();
        opts.include_untracked(true)
            .include_ignored(options.include_ignored)
            .recurse_untracked_dirs(true)
            // F17: detect renames so a `git mv` reports `R` + original_path (the status
            // model already carries `original_path`) instead of an unrelated delete+add.
            .renames_head_to_index(true)
            .renames_index_to_workdir(true);
        let statuses = repo.statuses(Some(&mut opts)).map_err(git_error)?;
        let mut out = GitStatus::default();
        for entry in statuses.iter() {
            let status = entry.status();
            if status.intersects(staged_statuses()) {
                out.staged += 1;
            }
            if status.intersects(unstaged_statuses()) {
                out.unstaged += 1;
            }
            if status.contains(git2::Status::WT_NEW) {
                out.untracked += 1;
            }
            if status.contains(git2::Status::IGNORED) {
                out.ignored += 1;
            }
            if status.contains(git2::Status::CONFLICTED) {
                out.unresolved += 1;
            }
            if let Some(file) = git_file_status(&entry) {
                out.files.push(file);
            }
        }
        out.is_dirty =
            out.staged > 0 || out.unstaged > 0 || out.untracked > 0 || out.unresolved > 0;
        Ok(out)
    }

    fn head(&self, path: &Path) -> ModelResult<GitHeadState> {
        let repo = open_repo(path)?;
        repo_head(&repo)
    }

    fn remotes(&self, path: &Path) -> ModelResult<Vec<GitRemote>> {
        let repo = open_repo(path)?;
        let names = repo.remotes().map_err(git_error)?;
        let mut remotes = Vec::new();
        for name in names.iter() {
            let Some(name) = name.map_err(git_error)? else {
                continue;
            };
            let remote = find_remote(&repo, name)?;
            remotes.push(GitRemote {
                name: name.to_owned(),
                url: Some(remote.url().map_err(git_error)?.to_owned()),
                push_url: remote.pushurl().map_err(git_error)?.map(ToOwned::to_owned),
            });
        }
        Ok(remotes)
    }

    fn add_remote(&self, path: &Path, name: &str, url: &str) -> ModelResult<GitRemoteResult> {
        let repo = open_repo(path)?;
        let remote = repo.remote(name, url).map_err(git_error)?;
        Ok(GitRemoteResult {
            remote: GitRemote {
                name: name.to_owned(),
                url: Some(remote.url().map_err(git_error)?.to_owned()),
                push_url: remote.pushurl().map_err(git_error)?.map(ToOwned::to_owned),
            },
        })
    }

    fn push(&self, path: &Path, remote: &str, refspec: &str) -> ModelResult<GitPushResult> {
        let repo = open_repo(path)?;
        let mut remote_handle = find_remote(&repo, remote)?;
        remote_handle
            .push(
                &[refspec],
                Some(&mut remote_push_options(self.credential_helpers)),
            )
            .map_err(git_error)?;
        Ok(GitPushResult {
            remote: remote.to_owned(),
            refspec: refspec.to_owned(),
        })
    }

    fn stage_paths(&self, path: &Path, pathspecs: &[&str]) -> ModelResult<GitStageResult> {
        let repo = open_repo(path)?;
        let mut index = repo.index().map_err(git_error)?;
        index
            .add_all(
                pathspecs.iter().copied(),
                git2::IndexAddOption::DEFAULT,
                None,
            )
            .map_err(git_error)?;
        index.write().map_err(git_error)?;

        // AD1 self-verify: re-open the repo so the index is read fresh from disk,
        // and confirm every requested *file* persisted into the index. Directory
        // pathspecs are covered by the fresh read; full content parity with
        // porcelain `git add` is proven by the contract test, not asserted here.
        let verify = open_repo(path)?.index().map_err(git_error)?;
        if verify.has_conflicts() {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                "index has conflicts after staging",
            ));
        }
        let mut staged = 0usize;
        for spec in pathspecs {
            if path.join(spec).is_file() {
                if verify.get_path(Path::new(spec), 0).is_none() {
                    return Err(ModelError::new(
                        ErrorCode::GitCommandFailed,
                        format!("staged path missing from index after write: {spec}"),
                    ));
                }
                staged += 1;
            }
        }
        Ok(GitStageResult { staged })
    }

    fn stage_paths_allowing_other_conflicts(
        &self,
        path: &Path,
        pathspecs: &[&str],
    ) -> ModelResult<GitStageResult> {
        let repo = open_repo(path)?;
        let mut index = repo.index().map_err(git_error)?;
        let mut staged = 0usize;
        for spec in pathspecs {
            let relative = Path::new(spec);
            match index.conflict_remove(relative) {
                Ok(()) => {}
                Err(err) if err.code() == git2::ErrorCode::NotFound => {}
                Err(err) => return Err(git_error(err)),
            }
            if path.join(relative).exists() {
                index.add_path(relative).map_err(git_error)?;
                staged += 1;
            } else {
                match index.remove_path(relative) {
                    Ok(()) => staged += 1,
                    Err(err) if err.code() == git2::ErrorCode::NotFound => {}
                    Err(err) => return Err(git_error(err)),
                }
            }
        }
        index.write().map_err(git_error)?;

        let verify = open_repo(path)?.index().map_err(git_error)?;
        for spec in pathspecs {
            match verify.conflict_get(Path::new(spec)) {
                Ok(_) => {
                    return Err(ModelError::new(
                        ErrorCode::GitCommandFailed,
                        format!("staged path still has conflicts after write: {spec}"),
                    ));
                }
                Err(err) if err.code() == git2::ErrorCode::NotFound => {}
                Err(err) => return Err(git_error(err)),
            }
        }
        Ok(GitStageResult { staged })
    }

    fn commit(&self, path: &Path, message: &str, all: bool) -> ModelResult<GitCommitResult> {
        // AD1 CLI fallback: run porcelain `git commit` so hooks / signing / committer
        // config are honored (libgit2's commit honors none of them).
        let before = self.head(path)?.commit;
        let mut command = std::process::Command::new("git");
        command.arg("-C").arg(path).arg("commit");
        if all {
            command.arg("-a");
        }
        command.arg("-m").arg(message);
        let output = command.output().map_err(|err| {
            ModelError::new(
                ErrorCode::GitCommandFailed,
                format!("failed to run git commit: {err}"),
            )
        })?;
        if !output.status.success() {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                format!(
                    "git commit failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            ));
        }
        // AD1 self-verify: HEAD advanced to a new commit (read fresh).
        let after = self.head(path)?.commit.ok_or_else(|| {
            ModelError::new(ErrorCode::GitCommandFailed, "HEAD is unborn after commit")
        })?;
        if Some(&after) == before.as_ref() {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                "git commit did not advance HEAD",
            ));
        }
        Ok(GitCommitResult { commit: after })
    }

    fn commit_merge_resolution(&self, path: &Path, message: &str) -> ModelResult<GitCommitResult> {
        let repo = open_repo(path)?;
        let mut index = repo.index().map_err(git_error)?;
        if index.has_conflicts() {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                "cannot commit merge resolution while index has conflicts",
            ));
        }
        let merge_head =
            std::fs::read_to_string(repo.path().join("MERGE_HEAD")).map_err(|err| {
                ModelError::new(
                    ErrorCode::GitCommandFailed,
                    format!("failed to read MERGE_HEAD: {err}"),
                )
            })?;
        let merge_oids = merge_head
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| git2::Oid::from_str(line.trim()).map_err(git_error))
            .collect::<ModelResult<Vec<_>>>()?;
        if merge_oids.is_empty() {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                "MERGE_HEAD is empty",
            ));
        }

        let head_commit = repo
            .head()
            .map_err(git_error)?
            .peel_to_commit()
            .map_err(git_error)?;
        let merge_commits = merge_oids
            .iter()
            .map(|oid| repo.find_commit(*oid).map_err(git_error))
            .collect::<ModelResult<Vec<_>>>()?;
        let tree_oid = index.write_tree().map_err(git_error)?;
        let tree = repo.find_tree(tree_oid).map_err(git_error)?;
        let signature = merge_signature(&repo)?;
        let mut parents = Vec::with_capacity(1 + merge_commits.len());
        parents.push(&head_commit);
        parents.extend(merge_commits.iter());
        let oid = repo
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &parents,
            )
            .map_err(git_error)?;
        repo.cleanup_state().map_err(git_error)?;
        let observed = self.head(path)?;
        if observed.commit.as_deref() != Some(oid.to_string().as_str()) {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                "post-merge-resolution HEAD is not the merge commit",
            ));
        }
        Ok(GitCommitResult {
            commit: oid.to_string(),
        })
    }

    fn commit_merge_resolution_checked(
        &self,
        path: &Path,
        expected_before: &str,
        expected_merge_head: &str,
        message: &str,
    ) -> ModelResult<GitCommitResult> {
        let repo = open_repo(path)?;
        let before = parse_existing_commit(&repo, expected_before)?;
        let merge_head = parse_existing_commit(&repo, expected_merge_head)?;
        let ref_name = attached_head_ref(&repo)?;
        let mut transaction = repo.transaction().map_err(git_error)?;
        transaction.lock_ref(&ref_name).map_err(git_error)?;
        validate_expected_native_merge(&repo, before, merge_head)?;
        validate_resolution_index_and_worktree(self, path, &repo, before, merge_head)?;

        let mut index = repo.index().map_err(git_error)?;
        let tree_oid = index.write_tree().map_err(git_error)?;
        let tree = repo.find_tree(tree_oid).map_err(git_error)?;
        let before_commit = repo.find_commit(before).map_err(git_error)?;
        let merge_commit = repo.find_commit(merge_head).map_err(git_error)?;
        let signature = merge_signature(&repo)?;
        let oid = repo
            .commit(
                None,
                &signature,
                &signature,
                message,
                &tree,
                &[&before_commit, &merge_commit],
            )
            .map_err(git_error)?;
        let committed = repo.find_commit(oid).map_err(git_error)?;
        if committed.parent_count() != 2
            || committed.parent_id(0).map_err(git_error)? != before
            || committed.parent_id(1).map_err(git_error)? != merge_head
            || committed.message_bytes() != message.as_bytes()
        {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                "merge resolution commit does not match its checked parents or message",
            ));
        }
        transaction
            .set_target(&ref_name, oid, Some(&signature), "gwz merge resolution")
            .map_err(git_error)?;
        transaction.commit().map_err(git_error)?;
        repo.cleanup_state().map_err(git_error)?;
        let branch = ref_name.trim_start_matches("refs/heads/");
        verify_merge_result(self, path, branch, &oid.to_string())?;
        Ok(GitCommitResult {
            commit: oid.to_string(),
        })
    }

    fn tag_create(
        &self,
        path: &Path,
        name: &str,
        message: Option<&str>,
        signed: bool,
    ) -> ModelResult<GitTagResult> {
        // AD1 CLI fallback: `git tag` so hooks / signing / tagger config are honored.
        let mut command = std::process::Command::new("git");
        command.arg("-C").arg(path).arg("tag");
        if signed {
            command.arg("-s");
        } else if message.is_some() {
            command.arg("-a");
        }
        if let Some(message) = message {
            command.arg("-m").arg(message);
        }
        command.arg(name);
        let output = command.output().map_err(|err| {
            ModelError::new(
                ErrorCode::GitCommandFailed,
                format!("failed to run git tag: {err}"),
            )
        })?;
        if !output.status.success() {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                format!(
                    "git tag failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            ));
        }
        // AD1 self-verify: the tag exists (read fresh) and resolves to a commit.
        if !self.tag_list(path)?.iter().any(|tag| tag == name) {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                format!("tag '{name}' missing after creation"),
            ));
        }
        let commit = self
            .read_ref(path, &format!("refs/tags/{name}^{{commit}}"))?
            .ok_or_else(|| {
                ModelError::new(
                    ErrorCode::GitCommandFailed,
                    format!("tag '{name}' did not resolve"),
                )
            })?;
        Ok(GitTagResult {
            name: name.to_owned(),
            commit,
        })
    }

    fn tag_list(&self, path: &Path) -> ModelResult<Vec<String>> {
        let repo = open_repo(path)?;
        let names = repo.tag_names(None).map_err(git_error)?;
        let mut tags = Vec::new();
        for entry in names.iter() {
            if let Some(name) = entry.map_err(git_error)? {
                tags.push(name.to_owned());
            }
        }
        tags.sort();
        Ok(tags)
    }

    fn tag_delete(&self, path: &Path, name: &str) -> ModelResult<()> {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("tag")
            .arg("-d")
            .arg(name)
            .output()
            .map_err(|err| {
                ModelError::new(
                    ErrorCode::GitCommandFailed,
                    format!("failed to run git tag -d: {err}"),
                )
            })?;
        if !output.status.success() {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                format!(
                    "git tag -d failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            ));
        }
        // AD1 self-verify: the tag is gone.
        if self.tag_list(path)?.iter().any(|tag| tag == name) {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                format!("tag '{name}' still present after delete"),
            ));
        }
        Ok(())
    }

    fn read_ref(&self, path: &Path, ref_spec: &str) -> ModelResult<Option<String>> {
        let repo = open_repo(path)?;
        match repo.revparse_single(ref_spec) {
            Ok(object) => Ok(Some(object.id().to_string())),
            Err(err)
                if matches!(
                    err.code(),
                    git2::ErrorCode::NotFound | git2::ErrorCode::UnbornBranch
                ) =>
            {
                Ok(None)
            }
            Err(err) => Err(git_error(err)),
        }
    }

    fn is_ancestor(&self, path: &Path, ancestor: &str, descendant: &str) -> ModelResult<bool> {
        let repo = open_repo(path)?;
        let ancestor = git2::Oid::from_str(ancestor).map_err(git_error)?;
        let descendant = git2::Oid::from_str(descendant).map_err(git_error)?;
        repo.graph_descendant_of(descendant, ancestor)
            .map_err(git_error)
    }

    fn merge_base(&self, path: &Path, left: &str, right: &str) -> ModelResult<Option<String>> {
        let repo = open_repo(path)?;
        let left = git2::Oid::from_str(left).map_err(git_error)?;
        let right = git2::Oid::from_str(right).map_err(git_error)?;
        match repo.merge_base(left, right) {
            Ok(oid) => Ok(Some(oid.to_string())),
            Err(err) if err.code() == git2::ErrorCode::NotFound => Ok(None),
            Err(err) => Err(git_error(err)),
        }
    }

    fn changed_paths_between(
        &self,
        path: &Path,
        old_commit: &str,
        new_commit: &str,
    ) -> ModelResult<Vec<String>> {
        let repo = open_repo(path)?;
        let old = repo
            .find_commit(git2::Oid::from_str(old_commit).map_err(git_error)?)
            .map_err(git_error)?;
        let new = repo
            .find_commit(git2::Oid::from_str(new_commit).map_err(git_error)?)
            .map_err(git_error)?;
        let old_tree = old.tree().map_err(git_error)?;
        let new_tree = new.tree().map_err(git_error)?;
        let diff = repo
            .diff_tree_to_tree(Some(&old_tree), Some(&new_tree), None)
            .map_err(git_error)?;
        let mut paths = Vec::new();
        for delta in diff.deltas() {
            for file in [delta.old_file(), delta.new_file()] {
                if let Some(path) = file.path() {
                    paths.push(path.to_string_lossy().into_owned());
                }
            }
        }
        paths.sort();
        paths.dedup();
        Ok(paths)
    }

    fn diff_manifest(
        &self,
        path: &Path,
        comparison: &crate::diff::RepoDiffComparison,
        options: &crate::diff::RepoDiffOptions,
    ) -> ModelResult<crate::diff::RepoDiffManifest> {
        let repo = open_repo(path)?;
        crate::diff::diff_repo(&repo, comparison, options)
    }

    fn resolve_comparison(
        &self,
        path: &Path,
        spec: &crate::diff::ComparisonSpec,
    ) -> ModelResult<crate::diff::RepoDiffComparison> {
        let repo = open_repo(path)?;
        crate::diff::resolve_comparison(&repo, spec)
    }
}

pub(crate) fn open_repo(path: &Path) -> ModelResult<git2::Repository> {
    git2::Repository::open(path).map_err(git_error)
}

pub(crate) fn branch_ref_name(branch: &str) -> String {
    format!("refs/heads/{branch}")
}

pub(crate) fn resolve_commit_oid(
    repo: &git2::Repository,
    ref_spec: &str,
) -> ModelResult<git2::Oid> {
    repo.revparse_single(ref_spec)
        .and_then(|object| object.peel_to_commit())
        .map(|commit| commit.id())
        .map_err(git_error)
}

pub(crate) fn ensure_no_integration_in_progress(repo: &git2::Repository) -> ModelResult<()> {
    let state = repo.state();
    if state != git2::RepositoryState::Clean {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            format!("repository has an integration operation in progress: {state:?}"),
        ));
    }
    Ok(())
}

fn parse_existing_commit(repo: &git2::Repository, value: &str) -> ModelResult<git2::Oid> {
    let oid = git2::Oid::from_str(value).map_err(git_error)?;
    repo.find_commit(oid).map_err(git_error)?;
    Ok(oid)
}

fn recovery_drift(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeDrift, message)
}

fn recovery_dirty(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::DirtyMember, message)
}

fn attached_head_ref(repo: &git2::Repository) -> ModelResult<String> {
    let head = repo.head().map_err(git_error)?;
    if !head.is_branch() {
        return Err(recovery_drift(
            "merge recovery requires an attached local branch",
        ));
    }
    let name = head.name().map_err(git_error)?;
    Ok(name.to_owned())
}

fn validate_expected_native_merge(
    repo: &git2::Repository,
    before: git2::Oid,
    expected_merge_head: git2::Oid,
) -> ModelResult<()> {
    if repo.state() != git2::RepositoryState::Merge {
        return Err(recovery_drift(format!(
            "expected native merge state, observed {:?}",
            repo.state()
        )));
    }
    let head = repo.head().map_err(git_error)?;
    let observed = head.peel_to_commit().map_err(git_error)?.id();
    if !head.is_branch() || observed != before {
        return Err(recovery_drift(format!(
            "merge target changed; expected {before}, observed {observed}"
        )));
    }
    let value = std::fs::read_to_string(repo.path().join("MERGE_HEAD"))
        .map_err(|error| recovery_drift(format!("failed to read expected MERGE_HEAD: {error}")))?;
    let mut heads = value.lines().filter(|line| !line.trim().is_empty());
    let observed_merge_head = heads
        .next()
        .and_then(|line| git2::Oid::from_str(line.trim()).ok());
    if heads.next().is_some() || observed_merge_head != Some(expected_merge_head) {
        return Err(recovery_drift(format!(
            "MERGE_HEAD changed; expected {expected_merge_head}"
        )));
    }
    Ok(())
}

fn expected_conflicts_and_index(
    repo: &git2::Repository,
    before: git2::Oid,
    merge_head: git2::Oid,
) -> ModelResult<(BTreeSet<Vec<u8>>, git2::Index)> {
    let before = repo.find_commit(before).map_err(git_error)?;
    let merge_head = repo.find_commit(merge_head).map_err(git_error)?;
    let index = repo
        .merge_commits(&before, &merge_head, None)
        .map_err(git_error)?;
    let mut conflicts = BTreeSet::new();
    for conflict in index.conflicts().map_err(git_error)? {
        let conflict = conflict.map_err(git_error)?;
        if let Some(entry) = conflict.our.or(conflict.their).or(conflict.ancestor) {
            conflicts.insert(entry.path);
        }
    }
    Ok((conflicts, index))
}

fn comparable_index_entries(
    index: &git2::Index,
    excluded: &BTreeSet<Vec<u8>>,
) -> Vec<(Vec<u8>, u32, git2::Oid, u16)> {
    index
        .iter()
        .filter(|entry| !excluded.contains(&entry.path))
        .map(|entry| (entry.path, entry.mode, entry.id, (entry.flags >> 12) & 3))
        .collect()
}

fn validate_recovery_index(
    repo: &git2::Repository,
    before: git2::Oid,
    merge_head: git2::Oid,
) -> ModelResult<BTreeSet<Vec<u8>>> {
    let (conflicts, expected) = expected_conflicts_and_index(repo, before, merge_head)?;
    let current = repo.index().map_err(git_error)?;
    if comparable_index_entries(&current, &conflicts)
        != comparable_index_entries(&expected, &conflicts)
    {
        return Err(recovery_dirty(
            "merge index contains changes outside the expected conflict paths",
        ));
    }
    Ok(conflicts)
}

fn validate_abort_index_and_worktree(
    backend: &impl GitBackend,
    path: &Path,
    repo: &git2::Repository,
    before: git2::Oid,
    merge_head: git2::Oid,
) -> ModelResult<()> {
    let conflicts = validate_recovery_index(repo, before, merge_head)?;
    let status = backend.status(path)?;
    let unexpected_worktree_change = status
        .files
        .iter()
        .any(|file| file.worktree_status != " " && !conflicts.contains(file.path.as_bytes()));
    if status.untracked > 0 || unexpected_worktree_change {
        return Err(recovery_dirty(
            "merge abort would overwrite work outside the expected conflict paths",
        ));
    }
    Ok(())
}

fn validate_resolution_index_and_worktree(
    backend: &impl GitBackend,
    path: &Path,
    repo: &git2::Repository,
    before: git2::Oid,
    merge_head: git2::Oid,
) -> ModelResult<()> {
    validate_recovery_index(repo, before, merge_head)?;
    let status = backend.status(path)?;
    if status.unresolved > 0 || status.unstaged > 0 || status.untracked > 0 {
        return Err(recovery_dirty(
            "merge resolution must be fully resolved and staged with no unrelated worktree changes",
        ));
    }
    Ok(())
}

fn ensure_clean_recovery_state(
    backend: &impl GitBackend,
    path: &Path,
    repo: &git2::Repository,
    branch: &str,
) -> ModelResult<()> {
    if repo.state() != git2::RepositoryState::Clean {
        return Err(recovery_drift(format!(
            "rollback found integration state {:?}",
            repo.state()
        )));
    }
    let head = repo_head(repo)?;
    if head.is_detached || head.branch.as_deref() != Some(branch) {
        return Err(recovery_drift(format!(
            "rollback target is not the attached branch '{branch}'"
        )));
    }
    if backend.status(path)?.is_dirty {
        return Err(recovery_dirty(
            "rollback requires a clean index and worktree",
        ));
    }
    Ok(())
}

fn verify_restored_merge_state(
    backend: &impl GitBackend,
    path: &Path,
    before: git2::Oid,
) -> ModelResult<()> {
    let branch = backend.head(path)?.branch.ok_or_else(|| {
        recovery_drift("repository is detached after restoring the pre-merge state")
    })?;
    verify_merge_result(backend, path, &branch, &before.to_string())
}

fn classify_merge(
    repo: &git2::Repository,
    target_commit: git2::Oid,
    source_commit: git2::Oid,
) -> ModelResult<GitMergeAnalysisKind> {
    if target_commit == source_commit
        || repo
            .graph_descendant_of(target_commit, source_commit)
            .map_err(git_error)?
    {
        return Ok(GitMergeAnalysisKind::UpToDate);
    }
    if repo
        .graph_descendant_of(source_commit, target_commit)
        .map_err(git_error)?
    {
        return Ok(GitMergeAnalysisKind::FastForward);
    }
    match repo.merge_base(target_commit, source_commit) {
        Ok(_) => Ok(GitMergeAnalysisKind::TrueMerge),
        Err(err) if err.code() == git2::ErrorCode::NotFound => Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "target and source do not share a merge base",
        )),
        Err(err) => Err(git_error(err)),
    }
}

pub(crate) fn verify_merge_result(
    backend: &impl GitBackend,
    path: &Path,
    branch: &str,
    expected_commit: &str,
) -> ModelResult<()> {
    let observed = backend.head(path)?;
    let status = backend.status(path)?;
    if observed.branch.as_deref() != Some(branch)
        || observed.commit.as_deref() != Some(expected_commit)
        || status.is_dirty
        || open_repo(path)?.state() != git2::RepositoryState::Clean
    {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "post-merge branch, HEAD, worktree, or native state did not match",
        ));
    }
    Ok(())
}

/// Create `branch` at `oid` when missing. If it exists, require it already points
/// at `oid`; refusing to move an existing branch preserves the checkout_branch
/// orphan-safety behavior used by materialize branch restore.
pub(crate) fn ensure_branch_at_commit(
    repo: &git2::Repository,
    branch: &str,
    oid: git2::Oid,
) -> ModelResult<bool> {
    let ref_name = branch_ref_name(branch);
    match repo.find_reference(&ref_name) {
        Ok(existing) => {
            let existing_oid = existing.peel_to_commit().map_err(git_error)?.id();
            if existing_oid != oid {
                return Err(ModelError::new(
                    ErrorCode::DivergedMember,
                    format!(
                        "branch '{branch}' is at {existing_oid}, not the target {oid}; refusing to move it"
                    ),
                ));
            }
            Ok(false)
        }
        Err(err) if err.code() == git2::ErrorCode::NotFound => {
            let target = repo.find_commit(oid).map_err(git_error)?;
            repo.branch(branch, &target, false).map_err(git_error)?;
            Ok(true)
        }
        Err(err) => Err(git_error(err)),
    }
}

pub(crate) fn git_branch_record(
    branch: &git2::Branch<'_>,
    current: Option<&str>,
) -> ModelResult<GitBranch> {
    let name = branch
        .name()
        .map_err(git_error)?
        .ok_or_else(|| ModelError::new(ErrorCode::GitCommandFailed, "branch name is not UTF-8"))?
        .to_owned();
    let commit = branch
        .get()
        .peel_to_commit()
        .map_err(git_error)?
        .id()
        .to_string();
    Ok(GitBranch {
        is_current: current == Some(name.as_str()),
        name,
        commit,
    })
}

pub(crate) fn stash_push_flags(options: GitStashPushOptions) -> git2::StashFlags {
    let mut flags = git2::StashFlags::empty();
    if options.preserve_index {
        flags |= git2::StashFlags::KEEP_INDEX;
    }
    if options.include_untracked {
        flags |= git2::StashFlags::INCLUDE_UNTRACKED;
    }
    if options.include_ignored {
        flags |= git2::StashFlags::INCLUDE_UNTRACKED;
        flags |= git2::StashFlags::INCLUDE_IGNORED;
    }
    flags
}

pub(crate) fn stash_restore_options(
    options: GitStashRestoreOptions,
) -> git2::StashApplyOptions<'static> {
    let mut apply_options = git2::StashApplyOptions::new();
    if options.preserve_index {
        apply_options.reinstantiate_index();
    }
    apply_options
}

pub(crate) fn stash_entries(repo: &mut git2::Repository) -> ModelResult<Vec<GitStashEntry>> {
    let mut entries = Vec::new();
    repo.stash_foreach(|index, message, oid| {
        entries.push(GitStashEntry {
            index,
            object_id: oid.to_string(),
            message: message.to_owned(),
        });
        true
    })
    .map_err(git_error)?;
    Ok(entries)
}

pub(crate) fn resolve_stash_index(
    repo: &mut git2::Repository,
    target: &GitStashTarget,
) -> ModelResult<usize> {
    let entries = stash_entries(repo)?;
    if let Some(object_id) = target.object_id.as_deref() {
        let oid = git2::Oid::from_str(object_id).map_err(git_error)?;
        if let Some(entry) = entries
            .iter()
            .find(|entry| entry.object_id == oid.to_string())
        {
            return Ok(entry.index);
        }
    }

    if let Some(prefix) = target.gwz_message_prefix.as_deref() {
        if !prefix.starts_with("gwz:") {
            return Err(ModelError::new(
                ErrorCode::InvalidRequest,
                "stash message prefix fallback is restricted to gwz: prefixes",
            ));
        }
        if let Some(entry) = entries
            .iter()
            .find(|entry| stash_message_matches_gwz_prefix(&entry.message, prefix))
        {
            return Ok(entry.index);
        }
    }

    Err(ModelError::new(
        ErrorCode::GitCommandFailed,
        "stash entry not found",
    ))
}

pub(crate) fn stash_restore_error(error: git2::Error) -> ModelError {
    match error.code() {
        git2::ErrorCode::Conflict => ModelError::new(
            ErrorCode::GitCommandFailed,
            format!("stash restore conflict: {}", error.message()),
        ),
        _ => git_error(error),
    }
}

pub(crate) fn stash_message_matches_gwz_prefix(message: &str, prefix: &str) -> bool {
    message.starts_with(prefix)
        || message
            .split_once(": ")
            .is_some_and(|(_, suffix)| suffix.starts_with(prefix))
}

/// Set libgit2's server (SSH/network) read timeout, process-wide, in milliseconds.
/// libssh2/libgit2 default to NO timeout, so a stalled SSH handshake — an empty ssh-agent
/// or an unreachable host — hangs forever; a positive value makes it a fast `Timeout`
/// error (libgit2 feeds it to `libssh2_session_set_timeout`). `0` disables it. Call ONCE
/// at startup before any network op spawns threads (mutates a libgit2 global without
/// synchronization).
pub fn set_server_timeout_ms(ms: i32) {
    // SAFETY: invoked once from CLI startup, before any backend operation / thread spawn.
    unsafe {
        let _ = git2::opts::set_server_timeout_in_milliseconds(ms);
    }
}

/// Conflicted paths in `index`, sorted and de-duplicated. A conflict carries up to
/// three stages (ancestor/our/their); any one supplies the path.
pub(crate) fn conflict_paths(index: &git2::Index) -> ModelResult<Vec<String>> {
    let mut paths = Vec::new();
    for conflict in index.conflicts().map_err(git_error)? {
        let conflict = conflict.map_err(git_error)?;
        if let Some(entry) = conflict.our.or(conflict.their).or(conflict.ancestor)
            && let Ok(path) = std::str::from_utf8(&entry.path)
        {
            paths.push(path.to_owned());
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

/// Author/committer for gwz-created merge commits: the repo's configured identity
/// when present, else a stable gwz fallback so an unconfigured repo can still merge.
pub(crate) fn merge_signature(repo: &git2::Repository) -> ModelResult<git2::Signature<'static>> {
    if let Ok(signature) = repo.signature() {
        return Ok(signature);
    }
    git2::Signature::now("gwz", "gwz@localhost").map_err(git_error)
}

fn merge_signatures(
    repo: &git2::Repository,
    attribution: Option<&crate::model::OperationAttribution>,
) -> ModelResult<(git2::Signature<'static>, git2::Signature<'static>)> {
    if let Some(attribution) = attribution {
        attribution.validate()?;
    }
    let author = match attribution.and_then(|value| value.git_author.as_ref()) {
        Some(identity) => signature_from_identity(identity)?,
        None => merge_signature(repo)?,
    };
    let committer = match attribution.and_then(|value| value.git_committer.as_ref()) {
        Some(identity) => signature_from_identity(identity)?,
        None => merge_signature(repo)?,
    };
    Ok((author, committer))
}

fn signature_from_identity(
    identity: &crate::model::GitObjectIdentity,
) -> ModelResult<git2::Signature<'static>> {
    identity.validate()?;
    if identity.time_ms.is_none() && identity.timezone_offset_minutes.is_none() {
        return git2::Signature::now(&identity.name, &identity.email).map_err(git_error);
    }
    let seconds = match identity.time_ms {
        Some(value) => value.0.div_euclid(1_000),
        None => {
            let elapsed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| ModelError::new(ErrorCode::InternalError, error.to_string()))?;
            i64::try_from(elapsed.as_secs()).map_err(|_| {
                ModelError::new(ErrorCode::InternalError, "system time is out of Git range")
            })?
        }
    };
    let offset = i32::try_from(identity.timezone_offset_minutes.unwrap_or(0)).map_err(|_| {
        ModelError::new(
            ErrorCode::InvalidRequest,
            "git identity timezone offset is out of range",
        )
    })?;
    git2::Signature::new(
        &identity.name,
        &identity.email,
        &git2::Time::new(seconds, offset),
    )
    .map_err(git_error)
}

fn same_signature(left: &git2::Signature<'_>, right: &git2::Signature<'_>) -> bool {
    left.name_bytes() == right.name_bytes()
        && left.email_bytes() == right.email_bytes()
        && left.when().seconds() == right.when().seconds()
        && left.when().offset_minutes() == right.when().offset_minutes()
}

pub(crate) fn remote_fetch_options(
    credential_helpers: CredentialHelperPolicy,
) -> git2::FetchOptions<'static> {
    fetch_options_with_progress(credential_helpers, None)
}

pub(crate) fn fetch_options_with_progress<'a>(
    credential_helpers: CredentialHelperPolicy,
    progress: Option<&'a dyn Fn(crate::GitTransferProgress)>,
) -> git2::FetchOptions<'a> {
    let mut callbacks = remote_callbacks(credential_helpers);
    if let Some(progress) = progress {
        callbacks.transfer_progress(move |stats| {
            progress(git_transfer_progress(&stats));
            true
        });
    }
    let mut options = git2::FetchOptions::new();
    options.remote_callbacks(callbacks);
    options
}

pub(crate) fn remote_push_options(
    credential_helpers: CredentialHelperPolicy,
) -> git2::PushOptions<'static> {
    let mut options = git2::PushOptions::new();
    options.remote_callbacks(remote_callbacks(credential_helpers));
    options
}

pub(crate) fn remote_callbacks<'a>(
    credential_helpers: CredentialHelperPolicy,
) -> git2::RemoteCallbacks<'a> {
    let mut callbacks = git2::RemoteCallbacks::new();
    // libgit2 re-invokes this after each auth rejection; track SSH attempts so we offer
    // the agent once and then fail, instead of re-offering a dead credential forever.
    let mut ssh_attempts = 0u32;
    callbacks.credentials(move |url, username_from_url, allowed_types| {
        remote_credential(
            url,
            username_from_url,
            allowed_types,
            credential_helpers,
            &mut ssh_attempts,
        )
    });
    callbacks
}

pub(crate) fn remote_credential(
    url: &str,
    username_from_url: Option<&str>,
    allowed_types: git2::CredentialType,
    credential_helpers: CredentialHelperPolicy,
    ssh_attempts: &mut u32,
) -> Result<git2::Cred, git2::Error> {
    let username = username_from_url.unwrap_or("git");
    if allowed_types.is_ssh_key() {
        // Offer the ssh-agent once. If libgit2 asks again, that attempt was rejected and
        // we have nothing else — return an error so it stops rather than looping forever.
        *ssh_attempts += 1;
        if *ssh_attempts > 1 {
            return Err(git2::Error::from_str(
                "SSH key authentication failed (no usable identity in the ssh-agent); \
                 run `ssh-add` or check your SSH setup",
            ));
        }
        return git2::Cred::ssh_key_from_agent(username);
    }
    if allowed_types.is_username() {
        return git2::Cred::username(username);
    }
    if allowed_types.is_user_pass_plaintext()
        && credential_helpers == CredentialHelperPolicy::AllowConfigured
        && let Ok(config) = git2::Config::open_default()
        && let Ok(credential) = git2::Cred::credential_helper(&config, url, username_from_url)
    {
        return Ok(credential);
    }
    if allowed_types.is_default() {
        return git2::Cred::default();
    }
    Err(git2::Error::from_str(
        "GWZ could not acquire credentials for the requested remote",
    ))
}

pub(crate) fn git_file_status(entry: &git2::StatusEntry<'_>) -> Option<GitFileStatus> {
    let status = entry.status();
    // git2 reports a rename entry under the OLD path; model it the way `git status` does —
    // current path = the new path, `original_path` = where it came from.
    let (path, original_path) = match rename_delta(entry, status) {
        Some((old, new)) => (new, Some(old)),
        None => (entry.path().ok()?.to_owned(), None),
    };
    Some(GitFileStatus {
        path,
        index_status: index_status_char(status).to_owned(),
        worktree_status: worktree_status_char(status).to_owned(),
        original_path,
    })
}

/// `(old_path, new_path)` when `entry` is a rename (staged or unstaged), else `None`.
pub(crate) fn rename_delta(
    entry: &git2::StatusEntry<'_>,
    status: git2::Status,
) -> Option<(String, String)> {
    if !status.intersects(git2::Status::INDEX_RENAMED | git2::Status::WT_RENAMED) {
        return None;
    }
    let delta = entry.head_to_index().or_else(|| entry.index_to_workdir())?;
    let old = delta.old_file().path()?.to_str()?.to_owned();
    let new = delta.new_file().path()?.to_str()?.to_owned();
    Some((old, new))
}

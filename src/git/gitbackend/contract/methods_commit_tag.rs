// One method group in the existing trait/impl; not a new interface.
#[rustfmt::skip]
macro_rules! repository_contract_commit_tag {
    () => {
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
    /// Commit a resolved merge under an exact target-branch/parent/ref safety
    /// boundary.
    fn commit_merge_resolution_checked(
        &self,
        _path: &Path,
        _target_branch: &str,
        _expected_before: &str,
        _expected_merge_head: &str,
        _message: &str,
        _attribution: Option<&crate::model::OperationAttribution>,
    ) -> ModelResult<GitCommitResult> {
        unsupported_backend("commit_merge_resolution_checked")
    }
    /// Freeze the resolved index tree and complete commit signatures only
    /// while attached to the exact target branch, without changing refs, HEAD,
    /// index/worktree bytes, or native merge state.
    fn prepare_merge_resolution_checked(
        &self,
        _path: &Path,
        _target_branch: &str,
        _expected_before: &str,
        _expected_merge_head: &str,
        _attribution: Option<&crate::model::OperationAttribution>,
    ) -> ModelResult<GitPreparedCommit> {
        unsupported_backend("prepare_merge_resolution_checked")
    }
    /// Commit a native merge resolution only when its attached target branch
    /// and resolved tree still match the frozen specification, using the
    /// frozen signatures.
    fn commit_prepared_merge_resolution_checked(
        &self,
        _path: &Path,
        _target_branch: &str,
        _expected_before: &str,
        _expected_merge_head: &str,
        _message: &str,
        _prepared: &GitPreparedCommit,
    ) -> ModelResult<GitCommitResult> {
        unsupported_backend("commit_prepared_merge_resolution_checked")
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
    };
}

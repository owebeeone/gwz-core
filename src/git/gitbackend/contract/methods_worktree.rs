// One method group in the existing trait/impl; not a new interface.
#[rustfmt::skip]
macro_rules! repository_contract_worktree {
    () => {
    /// Verify exact stage-0 regular-file entries for `expected_files` and the
    /// complete absence of every `absent_path`.
    ///
    /// Implementations compare blob identity, mode, stage, and worktree file
    /// type without writing objects, the index, or the worktree. Unrelated
    /// index entries are deliberately ignored.
    fn index_matches_candidate_files(
        &self,
        _path: &Path,
        _expected_files: &[GitCandidateFile],
        _absent_paths: &[String],
    ) -> ModelResult<bool> {
        unsupported_backend("index_matches_candidate_files")
    }
    /// Verify only the raw index entries for exact stage-0 regular files and
    /// exact index absence. Worktree existence and bytes are deliberately not
    /// inspected, so callers can classify ordered write-then-stage protocols.
    /// Unrelated index entries are ignored and this method never mutates Git.
    fn index_entries_match_candidate_files(
        &self,
        _path: &Path,
        _expected_files: &[GitCandidateFile],
        _absent_paths: &[String],
    ) -> ModelResult<bool> {
        unsupported_backend("index_entries_match_candidate_files")
    }
    /// Commit only the supplied GWZ-owned candidate files through an isolated
    /// index and checked attached-root-ref update.
    ///
    /// Candidate paths must be unique, normalized repository-relative files
    /// below `gwz.conf/`. The candidate tree starts from `expected_head`, or
    /// from an empty tree when `expected_head=None` requires an unborn ref.
    /// The real index and worktree are never read as candidate content and are
    /// left byte-for-byte unchanged. The returned candidate hashes are sorted
    /// by path and cover every supplied file for later recovery verification.
    fn commit_gwz_paths_checked(
        &self,
        _root: &Path,
        _expected_head: Option<&str>,
        _candidate_files: &[GitCandidateFile],
        _message: &str,
    ) -> ModelResult<GitScopedCommitResult> {
        unsupported_backend("commit_gwz_paths_checked")
    }
    /// Commit staged configuration and named bootstrap outputs only. Uses an
    /// isolated index, preserves unrelated staged work, and returns None for no change.
    fn commit_bootstrap_paths_checked(
        &self,
        _root: &Path,
        _expected_head: Option<&str>,
        _paths: &[&str],
        _message: &str,
    ) -> ModelResult<Option<GitScopedCommitResult>> {
        unsupported_backend("commit_bootstrap_paths_checked")
    }
    /// Verify and recover an already-published scoped commit from its exact
    /// parent, message, candidate paths, and candidate bytes.
    fn verify_gwz_paths_commit(
        &self,
        _root: &Path,
        _commit: &str,
        _expected_parent: Option<&str>,
        _candidate_files: &[GitCandidateFile],
        _message: &str,
    ) -> ModelResult<GitScopedCommitResult> {
        unsupported_backend("verify_gwz_paths_commit")
    }
    /// Roll back an exact scoped GWZ evidence commit by moving its attached
    /// branch to `expected_parent`, or deleting the branch when the evidence
    /// commit was the unborn root's first commit.
    ///
    /// The real index and worktree are deliberately not checked out or
    /// rewritten. Callers restore only GWZ-owned candidate paths afterwards so
    /// unrelated user state is preserved.
    fn rollback_gwz_paths_commit_checked(
        &self,
        _root: &Path,
        _branch: &str,
        _commit: &str,
        _expected_parent: Option<&str>,
        _candidate_files: &[GitCandidateFile],
        _message: &str,
    ) -> ModelResult<()> {
        unsupported_backend("rollback_gwz_paths_commit_checked")
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
    /// Check out an existing branch without moving it. When the branch already
    /// points at the current HEAD, attach HEAD without touching the index or
    /// worktree so pending changes survive. Self-verifies HEAD is attached to
    /// the requested branch.
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
    };
}

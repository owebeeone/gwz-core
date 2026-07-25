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
    /// Read one repository-relative file from the exact committed tree.
    ///
    /// This is read-only, never resolves a symbolic revision, and returns
    /// `None` only when the path is absent from the specified commit.
    fn read_file_at_commit(
        &self,
        _path: &Path,
        _commit: &str,
        _relative_path: &str,
    ) -> ModelResult<Option<Vec<u8>>> {
        unsupported_backend("read_file_at_commit")
    }
    /// Return whether `commit` is an exact two-parent merge commit with the
    /// supplied ordered parents and byte-exact message. This is read-only and
    /// never resolves an abbreviation or fetches a missing object.
    fn commit_matches_merge(
        &self,
        _path: &Path,
        _commit: &str,
        _first_parent: &str,
        _second_parent: &str,
        _message: &str,
    ) -> ModelResult<bool> {
        unsupported_backend("commit_matches_merge")
    }
    /// Return whether `commit` exactly matches a prepared two-parent merge,
    /// including its tree and complete author/committer signatures.
    fn commit_matches_prepared_merge(
        &self,
        _path: &Path,
        _commit: &str,
        _first_parent: &str,
        _second_parent: &str,
        _message: &str,
        _prepared: &GitPreparedCommit,
    ) -> ModelResult<bool> {
        unsupported_backend("commit_matches_prepared_merge")
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
    /// Freeze the exact result of a checked merge without moving a ref or
    /// changing HEAD, the repository index/worktree, or native operation state.
    fn prepare_merge_upstream_checked(
        &self,
        _path: &Path,
        _branch: &str,
        _expected_before: &str,
        _source_commit: &str,
        _attribution: Option<&crate::model::OperationAttribution>,
    ) -> ModelResult<GitPreparedMerge> {
        unsupported_backend("prepare_merge_upstream_checked")
    }
    /// Read-only verification that a prepared merge still exactly matches the
    /// attached branch, before/source commits, result class, and (for a clean
    /// true merge) existing tree and frozen signatures. Implementations must
    /// not create an object or change refs, HEAD, index/worktree, or native
    /// repository state.
    fn validate_prepared_merge_upstream_state(
        &self,
        _path: &Path,
        _branch: &str,
        _expected_before: &str,
        _source_commit: &str,
        _prepared: &GitPreparedMerge,
    ) -> ModelResult<()> {
        unsupported_backend("validate_prepared_merge_upstream_state")
    }
    /// Execute a merge using only its already frozen content and signatures.
    fn execute_prepared_merge_upstream_checked(
        &self,
        _path: &Path,
        _branch: &str,
        _expected_before: &str,
        _source_commit: &str,
        _message: &str,
        _prepared: &GitPreparedMerge,
    ) -> ModelResult<GitIntegrateResult> {
        unsupported_backend("execute_prepared_merge_upstream_checked")
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
    /// Observe the complete native repository operation state. Status,
    /// continue, abort, and checked recovery actions consume this same value so
    /// preflight cannot accept a foreign sequencer state rejected only later.
    fn repository_state(&self, _path: &Path) -> ModelResult<GitRepositoryState> {
        unsupported_backend("repository_state")
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
    /// Read-only verification that a resolved native merge is still attached
    /// to the exact target branch and has the exact index tree frozen in
    /// `prepared`. This must not write a tree object, change the
    /// index/worktree, move a ref, or clean up native state.
    fn validate_prepared_merge_resolution_state(
        &self,
        _path: &Path,
        _target_branch: &str,
        _expected_before: &str,
        _expected_merge_head: &str,
        _prepared: &GitPreparedCommit,
    ) -> ModelResult<()> {
        unsupported_backend("validate_prepared_merge_resolution_state")
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
    /// Delete an attached branch only when it still equals `expected_current`,
    /// leaving symbolic HEAD attached to the now-unborn branch.
    fn delete_branch_target_checked(
        &self,
        _path: &Path,
        _branch: &str,
        _expected_current: &str,
    ) -> ModelResult<()> {
        unsupported_backend("delete_branch_target_checked")
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
    /// Delete an exact local preservation ref when it still has the recorded target.
    fn delete_backup_ref_checked(
        &self,
        _path: &Path,
        _name: &str,
        _expected_target: &str,
    ) -> ModelResult<()> {
        unsupported_backend("delete_backup_ref_checked")
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
}

fn unsupported_backend<T>(method: &str) -> ModelResult<T> {
    Err(ModelError::new(
        ErrorCode::UnsupportedOperation,
        format!("{method} is not implemented by this GitBackend"),
    ))
}

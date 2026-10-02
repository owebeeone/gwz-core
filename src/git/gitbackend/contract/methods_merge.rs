// One method group in the existing trait/impl; not a new interface.
#[rustfmt::skip]
macro_rules! repository_contract_merge {
    () => {
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
    /// Mode-aware form of `prepare_merge_upstream_checked`. Backends that do
    /// not implement forced merge commits retain the released normal behavior.
    fn prepare_merge_upstream_mode_checked(
        &self,
        path: &Path,
        branch: &str,
        expected_before: &str,
        source_commit: &str,
        mode: GitPreparedMergeMode,
        attribution: Option<&crate::model::OperationAttribution>,
    ) -> ModelResult<GitPreparedMerge> {
        match mode {
            GitPreparedMergeMode::AllowFastForward => self.prepare_merge_upstream_checked(
                path,
                branch,
                expected_before,
                source_commit,
                attribution,
            ),
            GitPreparedMergeMode::ForceMergeCommit => {
                unsupported_backend("prepare_merge_upstream_mode_checked")
            }
        }
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
    /// The prepared variant is the authority for whether a fast-forward graph
    /// advances directly or publishes the frozen two-parent commit.
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
    /// M4 in-memory tree merge; never writes refs, HEAD, index, or worktree.
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
    /// Capture an exact pristine native-conflict snapshot. The current index
    /// must still equal the deterministic merge index, including conflict
    /// stages, and the returned hashes cover the conflict-marker worktree
    /// files. This is read-only.
    fn merge_conflict_snapshot(
        &self,
        _path: &Path,
        _expected_before: &str,
        _expected_merge_head: &str,
    ) -> ModelResult<GitMergeConflictSnapshot> {
        unsupported_backend("merge_conflict_snapshot")
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
    };
}

// One method group in the existing trait/impl; not a new interface.
#[rustfmt::skip]
macro_rules! repository_contract_preservation {
    () => {
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
    /// Create an exact preservation ref only while HEAD remains attached to
    /// `branch` at the persisted `expected_head`.
    fn create_backup_ref_checked(
        &self,
        _path: &Path,
        _branch: &str,
        _expected_head: &str,
        _name: &str,
        _target: &str,
    ) -> ModelResult<GitBackupRefResult> {
        unsupported_backend("create_backup_ref_checked")
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
    /// Save work only while the attached branch, HEAD commit, and canonical
    /// complete preimage still equal the persisted action values.
    fn stash_for_merge_preservation_checked(
        &self,
        _path: &Path,
        _branch: &str,
        _expected_head: &str,
        _expected_preimage_sha256: &str,
        _merge_id: &str,
        _include_untracked: bool,
    ) -> ModelResult<GitStashPushResult> {
        unsupported_backend("stash_for_merge_preservation_checked")
    }
    /// Capture the canonical I2 index/worktree image without decoding Git
    /// paths as UTF-8. Forbidden index flags are rejected rather than omitted.
    fn preservation_image(
        &self,
        _path: &Path,
        _include_untracked: bool,
    ) -> ModelResult<GitPreservationImage> {
        unsupported_backend("preservation_image")
    }
    /// Inspect the named reference itself without peeling or resolving it.
    fn observe_direct_ref(
        &self,
        _path: &Path,
        _name: &str,
    ) -> ModelResult<GitDirectRefObservation> {
        unsupported_backend("observe_direct_ref")
    }
    /// Require an attached branch and exact clean index/worktree at `commit`.
    fn checkout_matches_commit(
        &self,
        _path: &Path,
        _branch: &str,
        _commit: &str,
    ) -> ModelResult<bool> {
        unsupported_backend("checkout_matches_commit")
    }
    /// Compare the complete index and worktree with `commit`, excluding only
    /// the exact canonical paths owned by a higher-level pending action.
    fn checkout_matches_commit_except(
        &self,
        _path: &Path,
        _commit: &str,
        _allowed_paths: &[String],
    ) -> ModelResult<bool> {
        unsupported_backend("checkout_matches_commit_except")
    }
    /// Compare the complete checkout while delegating only the named
    /// worktree or index facts to another exact observer.
    fn checkout_matches_commit_with_overlay(
        &self,
        path: &Path,
        commit: &str,
        overlay: &GitCheckoutOverlay,
    ) -> ModelResult<bool> {
        if overlay.worktree_paths == overlay.index_paths {
            self.checkout_matches_commit_except(path, commit, &overlay.worktree_paths)
        } else {
            unsupported_backend("checkout_matches_commit_with_overlay")
        }
    }
    /// Capture a preimage with the supplied managed files normalized to their clean form.
    /// Excluded paths are control state; this observation must not rewrite the checkout.
    fn root_preservation_image(
        &self,
        _root: &Path,
        _clean: &GitRootManagedForm,
        _excluded: &[String],
    ) -> ModelResult<GitPreservationImage> {
        unsupported_backend("root_preservation_image")
    }
    /// Validate managed paths, index facts, and clean forms against their exact commits.
    fn validate_root_preservation_spec(
        &self,
        _root: &Path,
        _spec: &GitRootPreservationSpec,
    ) -> ModelResult<()> {
        unsupported_backend("validate_root_preservation_spec")
    }
    /// Observe the exact managed index entries and marker namespace, without mutation.
    fn root_managed_index_matches(
        &self,
        _root: &Path,
        _form: &GitRootManagedIndexForm,
    ) -> ModelResult<bool> {
        unsupported_backend("root_managed_index_matches")
    }
    /// Replace only the managed index entries, preserving unrelated entries and
    /// checking the resulting index. The shared protocol supplies the precondition.
    fn rewrite_root_managed_index_checked(
        &self,
        _root: &Path,
        _form: &GitRootManagedIndexForm,
    ) -> ModelResult<()> {
        unsupported_backend("rewrite_root_managed_index_checked")
    }
    fn prepare_root_preservation_stash(
        &self,
        _root: &Path,
        _spec: &GitRootPreservationSpec,
    ) -> ModelResult<GitPreparedRootStash> {
        unsupported_backend("prepare_root_preservation_stash")
    }
    fn observe_root_preservation_step(
        &self,
        _root: &Path,
        _spec: &GitRootPreservationSpec,
        _step: &GitRootPreservationPhysicalStep,
        _guard: &GitRootPreservationGuard,
    ) -> ModelResult<GitRootPreservationStepObservation> {
        unsupported_backend("observe_root_preservation_step")
    }
    fn execute_root_preservation_step_checked(
        &self,
        _root: &Path,
        _spec: &GitRootPreservationSpec,
        _step: &GitRootPreservationPhysicalStep,
        _guard: &GitRootPreservationGuard,
    ) -> ModelResult<GitCheckedPreservationMutation> {
        unsupported_backend("execute_root_preservation_step_checked")
    }
    };
}

// One method group in the existing trait/impl; not a new interface.
#[rustfmt::skip]
macro_rules! fake_repository_root {
    () => {
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
    };
}

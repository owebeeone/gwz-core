// One method group in the existing trait/impl; not a new interface.
#[rustfmt::skip]
macro_rules! fake_repository_merge {
    () => {
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
    };
}

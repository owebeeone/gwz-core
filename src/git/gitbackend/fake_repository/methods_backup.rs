// One method group in the existing trait/impl; not a new interface.
#[rustfmt::skip]
macro_rules! fake_repository_backup {
    () => {
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
    };
}

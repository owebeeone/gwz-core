// One method group in the existing trait/impl; not a new interface.
#[rustfmt::skip]
macro_rules! fake_repository_stash {
    () => {
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
    };
}

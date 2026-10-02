// One method group in the existing trait/impl; not a new interface.
#[rustfmt::skip]
macro_rules! fake_repository_repository {
    () => {
    fn repository_index(&self, path: &Path) -> ModelResult<GitIndexSnapshot> {
        let entries = self
            .test_read_index(path)?
            .into_iter()
            .map(|e| {
                Ok(GitIndexEntry {
                    path: e.path,
                    object_id: git2::Oid::from_str(&e.object_id)
                        .map_err(git_error)?
                        .as_bytes()
                        .to_vec(),
                    mode: e.mode,
                    flags: (u16::from(e.stage) << 12) | if e.assume_valid { 0x8000 } else { 0 },
                    flags_extended: (if e.skip_worktree { 0x4000 } else { 0 })
                        | if e.intent_to_add { 0x2000 } else { 0 },
                    ctime: (0, 0),
                    mtime: (0, 0),
                    stat: [0; 5],
                })
            })
            .collect::<ModelResult<Vec<_>>>()?;
        Ok(GitIndexSnapshot {
            path: Some(path.join(".git/index")),
            entries,
        })
    }
    fn repository_paths(&self, path: &Path) -> ModelResult<GitRepositoryPaths> {
        if !self.is_repository(path)? {
            return Err(failed("repository missing"));
        }
        Ok(GitRepositoryPaths {
            worktree: Some(repository_key(self.filesystem.as_ref(), path)),
            git_dir: repository_key(self.filesystem.as_ref(), path).join(".git"),
            common_dir: repository_key(self.filesystem.as_ref(), path).join(".git"),
        })
    }
    fn is_repository(&self, path: &Path) -> ModelResult<bool> {
        Ok(self
            .repositories
            .lock()
            .unwrap()
            .contains_key(&repository_key(self.filesystem.as_ref(), path)))
    }
    fn create_repo(&self, path: &Path) -> ModelResult<GitCreateResult> {
        let mut repositories = self.repositories.lock().unwrap();
        if repositories.contains_key(&repository_key(self.filesystem.as_ref(), path)) {
            return Err(failed("repository already exists"));
        }
        self.filesystem
            .as_ref()
            .create_directories(path)
            .map_err(|e| failed(e.to_string()))?;
        repositories.insert(
            repository_key(self.filesystem.as_ref(), path),
            RepositoryState::default(),
        );
        Ok(GitCreateResult {
            path: path.to_path_buf(),
        })
    }
    fn clone_repo(&self, url: &str, path: &Path) -> ModelResult<GitCloneResult> {
        unsupported("clone_repo")
    }
    fn fetch(&self, path: &Path, remote: &str) -> ModelResult<GitFetchResult> {
        unsupported("fetch")
    }
    fn ls_remote(&self, path: &Path, remote: &str) -> ModelResult<Vec<GitRemoteRef>> {
        unsupported("ls_remote")
    }
    fn fast_forward(
        &self,
        path: &Path,
        branch: &str,
        upstream_ref: &str,
    ) -> ModelResult<GitUpdateResult> {
        unsupported("fast_forward")
    }
    fn merge_upstream(
        &self,
        path: &Path,
        branch: &str,
        upstream_ref: &str,
    ) -> ModelResult<GitIntegrateResult> {
        unsupported("merge_upstream")
    }
    fn rebase_onto(
        &self,
        path: &Path,
        branch: &str,
        upstream_ref: &str,
    ) -> ModelResult<GitIntegrateResult> {
        unsupported("rebase_onto")
    }
    fn reset_hard(
        &self,
        path: &Path,
        branch: &str,
        upstream_ref: &str,
    ) -> ModelResult<GitUpdateResult> {
        unsupported("reset_hard")
    }
    fn checkout_commit(&self, path: &Path, commit: &str) -> ModelResult<GitUpdateResult> {
        unsupported("checkout_commit")
    }
    fn checkout_branch(
        &self,
        path: &Path,
        branch: &str,
        commit: &str,
    ) -> ModelResult<GitUpdateResult> {
        unsupported("checkout_branch")
    }
    fn status(&self, path: &Path) -> ModelResult<GitStatus> {
        let repositories = self.repositories.lock().unwrap();
        let repo = repositories
            .get(&repository_key(self.filesystem.as_ref(), path))
            .ok_or_else(|| failed("repository missing"))?;
        let empty = BTreeMap::new();
        let committed = repo
            .head
            .as_ref()
            .and_then(|oid| repo.commits.get(oid))
            .unwrap_or(&empty);
        let worktree = root::worktree(self, repo, path)?;
        let paths: BTreeSet<_> = committed
            .keys()
            .chain(repo.index.keys())
            .chain(worktree.keys())
            .collect();
        let mut result = GitStatus::clean();
        for name in paths {
            let before = committed.get(name);
            let index = repo.index.get(name);
            let disk = worktree.get(name);
            let untracked = before.is_none() && index.is_none() && disk.is_some();
            let staged = before != index;
            let hidden_by_index_flag = repo.index_override.as_ref().is_some_and(|entries| {
                entries.iter().any(|entry| {
                    entry.stage == 0
                        && entry.path == name.as_bytes()
                        && (entry.assume_valid || entry.skip_worktree)
                })
            });
            let unstaged = !untracked && index != disk && !hidden_by_index_flag;
            result.staged += usize::from(staged);
            result.unstaged += usize::from(unstaged);
            result.untracked += usize::from(untracked);
            if staged || unstaged || untracked {
                result.files.push(GitFileStatus {
                    path: name.clone(),
                    index_status: if untracked {
                        "?"
                    } else if !staged {
                        " "
                    } else if index.is_none() {
                        "D"
                    } else if before.is_none() {
                        "A"
                    } else {
                        "M"
                    }
                    .into(),
                    worktree_status: if untracked {
                        "?"
                    } else if !unstaged {
                        " "
                    } else if disk.is_none() {
                        "D"
                    } else {
                        "M"
                    }
                    .into(),
                    original_path: None,
                });
            }
        }
        result.is_dirty = !result.files.is_empty();
        Ok(result)
    }
    fn head(&self, path: &Path) -> ModelResult<GitHeadState> {
        let repositories = self.repositories.lock().unwrap();
        let repo = repositories
            .get(&repository_key(self.filesystem.as_ref(), path))
            .ok_or_else(|| failed("repository missing"))?;
        Ok(GitHeadState {
            branch: if repo.detached {
                None
            } else {
                Some(
                    repo.attached_ref
                        .as_deref()
                        .unwrap_or("refs/heads/main")
                        .trim_start_matches("refs/heads/")
                        .into(),
                )
            },
            commit: repo.head.clone(),
            is_detached: repo.detached,
        })
    }
    };
}

// One method group in the existing trait/impl; not a new interface.
#[rustfmt::skip]
macro_rules! fake_repository_worktree {
    () => {
    fn stage_paths(&self, path: &Path, pathspecs: &[&str]) -> ModelResult<GitStageResult> {
        let mut repositories = self.repositories.lock().unwrap();
        let repo = repositories
            .get_mut(&repository_key(self.filesystem.as_ref(), path))
            .ok_or_else(|| failed("repository missing"))?;
        let mut staged = 0;
        for spec in pathspecs {
            // Explicit file paths are sufficient for these shared contracts.
            // Reject glob/directory semantics until they have their own contract.
            if spec.contains('*')
                || matches!(
                    self.filesystem.as_ref().kind(&path.join(spec)),
                    Ok(FsKind::Directory)
                )
            {
                return unsupported("stage pathspec");
            }
            match self.filesystem.as_ref().read(&path.join(spec)) {
                Ok(bytes) => {
                    repo.index.insert((*spec).to_owned(), bytes);
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::NotFound
                        && repo.index.contains_key(*spec) =>
                {
                    repo.index.remove(*spec);
                }
                Err(e) => return Err(failed(e.to_string())),
            }
            repo.index_override = None;
            staged += 1;
        }
        Ok(GitStageResult { staged })
    }
    fn commit(&self, path: &Path, message: &str, all: bool) -> ModelResult<GitCommitResult> {
        if all {
            return unsupported("commit --all");
        }
        let mut repositories = self.repositories.lock().unwrap();
        let repo = repositories
            .get_mut(&repository_key(self.filesystem.as_ref(), path))
            .ok_or_else(|| failed("repository missing"))?;
        if repo.head.as_ref().and_then(|oid| repo.commits.get(oid)) == Some(&repo.index) {
            return Err(failed("nothing to commit"));
        }
        let spec = TestCommitSpec::from_index(message, repo.head.clone().into_iter().collect());
        let oid = fixture::store_commit(repo, repo.index.clone(), &spec)?;
        repo.head = Some(oid.clone());
        if !repo.detached {
            repo.refs.insert(
                repo.attached_ref
                    .clone()
                    .unwrap_or_else(|| "refs/heads/main".into()),
                oid.clone(),
            );
        }
        Ok(GitCommitResult { commit: oid })
    }
    fn tag_create(
        &self,
        path: &Path,
        name: &str,
        message: Option<&str>,
        signed: bool,
    ) -> ModelResult<GitTagResult> {
        unsupported("tag_create")
    }
    fn tag_list(&self, path: &Path) -> ModelResult<Vec<String>> {
        unsupported("tag_list")
    }
    fn tag_delete(&self, path: &Path, name: &str) -> ModelResult<()> {
        unsupported("tag_delete")
    }
    fn tag_fetch(&self, path: &Path, remote: &str) -> ModelResult<GitFetchResult> {
        unsupported("tag_fetch")
    }
    };
}

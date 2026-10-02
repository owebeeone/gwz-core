// One method group in the existing trait/impl; not a new interface.
#[rustfmt::skip]
macro_rules! fake_repository_refs {
    () => {
    fn remotes(&self, path: &Path) -> ModelResult<Vec<GitRemote>> {
        unsupported("remotes")
    }
    fn add_remote(&self, path: &Path, name: &str, url: &str) -> ModelResult<GitRemoteResult> {
        unsupported("add_remote")
    }
    fn push(&self, path: &Path, remote: &str, refspec: &str) -> ModelResult<GitPushResult> {
        unsupported("push")
    }
    fn fetch_anonymous(
        &self,
        path: &Path,
        url: &str,
        refspecs: &[&str],
    ) -> ModelResult<GitFetchResult> {
        unsupported("fetch_anonymous")
    }
    fn push_anonymous(&self, path: &Path, url: &str, refspec: &str) -> ModelResult<GitPushResult> {
        unsupported("push_anonymous")
    }
    fn read_ref(&self, path: &Path, ref_spec: &str) -> ModelResult<Option<String>> {
        let repositories = self.repositories.lock().unwrap();
        let repo = repositories
            .get(&repository_key(self.filesystem.as_ref(), path))
            .ok_or_else(|| failed("repository missing"))?;
        if ref_spec == "HEAD" {
            return Ok(repo.head.clone());
        }
        let name = if ref_spec.starts_with("refs/") {
            ref_spec.to_owned()
        } else {
            format!("refs/heads/{ref_spec}")
        };
        fixture::resolve_ref(repo, &name)
    }

    fn is_ancestor(&self, path: &Path, ancestor: &str, descendant: &str) -> ModelResult<bool> {
        let repositories = self.repositories.lock().unwrap();
        let repo = repositories
            .get(&repository_key(self.filesystem.as_ref(), path))
            .ok_or_else(|| failed("repository missing"))?;
        if !repo.commits.contains_key(ancestor) || !repo.commits.contains_key(descendant) {
            return Err(failed("commit missing"));
        }
        let mut pending = vec![descendant];
        let mut visited = BTreeSet::new();
        while let Some(oid) = pending.pop() {
            if oid == ancestor {
                return Ok(true);
            }
            if visited.insert(oid)
                && let Some(commit) = repo.metadata.get(oid)
            {
                pending.extend(commit.parents.iter().map(String::as_str));
            }
        }
        Ok(false)
    }
    };
}

// One method group in the existing trait/impl; not a new interface.
#[rustfmt::skip]
macro_rules! fake_repository_fixtures {
    () => {
    fn test_init_repo(&self, repo: &Path, spec: &TestRepoSpec) -> ModelResult<()> {
        fixture::test_init_repo(self, repo, spec)
    }
    fn test_create_commit(&self, repo: &Path, spec: &TestCommitSpec) -> ModelResult<String> {
        fixture::test_create_commit(self, repo, spec)
    }
    fn test_read_commit(&self, repo: &Path, oid: &str) -> ModelResult<TestCommit> {
        fixture::test_read_commit(self, repo, oid)
    }
    fn test_set_ref(
        &self,
        repo: &Path,
        name: &str,
        target: Option<&TestRefTarget>,
    ) -> ModelResult<()> {
        fixture::test_set_ref(self, repo, name, target)
    }
    fn test_set_head(&self, repo: &Path, state: &TestHead) -> ModelResult<()> {
        fixture::test_set_head(self, repo, state)
    }
    fn test_replace_index(&self, repo: &Path, entries: &[TestIndexEntry]) -> ModelResult<()> {
        fixture::test_replace_index(self, repo, entries)
    }
    fn test_read_index(&self, repo: &Path) -> ModelResult<Vec<TestIndexEntry>> {
        fixture::test_read_index(self, repo)
    }
    fn test_set_config(&self, repo: &Path, key: &str, values: &[String]) -> ModelResult<()> {
        fixture::test_set_config(self, repo, key, values)
    }
    fn test_read_config(&self, repo: &Path, key: &str) -> ModelResult<Vec<String>> {
        fixture::test_read_config(self, repo, key)
    }
    fn test_force_checkout(&self, repo: &Path, commit: &str) -> ModelResult<()> {
        fixture::test_force_checkout(self, repo, commit)
    }
    fn test_reset_mixed(&self, repo: &Path, commit: &str) -> ModelResult<()> {
        fixture::test_reset_mixed(self, repo, commit)
    }
    fn test_set_repository_state(
        &self,
        repo: &Path,
        state: GitRepositoryState,
        merge_head: Option<&str>,
    ) -> ModelResult<()> {
        fixture::test_set_repository_state(self, repo, state, merge_head)
    }
    fn test_seed_merge_conflict(
        &self,
        repo: &Path,
        before: &str,
        source: &str,
    ) -> ModelResult<GitMergeConflictSnapshot> {
        fixture::test_seed_merge_conflict(self, repo, before, source)
    }
    fn test_create_commit_from_parent(
        &self,
        repo: &Path,
        parent: &str,
        message: &str,
        edits: &[TestCommitFileEdit],
    ) -> ModelResult<String> {
        fixture::test_create_commit_from_parent(self, repo, parent, message, edits)
    }

    };
}

// One method group in the existing trait/impl; not a new interface.
#[rustfmt::skip]
macro_rules! repository_contract_repository {
    () => {
    /// Read lossless index facts and the actual index path without retaining a native handle.
    fn repository_index(&self, _path: &Path) -> ModelResult<GitIndexSnapshot> {
        unsupported_backend("repository_index")
    }
    /// Resolve worktree, Git directory, and common storage locations.
    fn repository_paths(&self, _path: &Path) -> ModelResult<GitRepositoryPaths> {
        unsupported_backend("repository_paths")
    }
    fn is_repository(&self, path: &Path) -> ModelResult<bool>;
    /// Return whether `oid` exists locally and resolves to a commit object.
    /// This never fetches and returns `false` for malformed, missing, or
    /// non-commit object ids.
    fn commit_exists(&self, _path: &Path, _oid: &str) -> ModelResult<bool> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "commit_exists is not implemented by this GitBackend",
        ))
    }
    /// Read one repository-relative file from the exact committed tree.
    ///
    /// This is read-only, never resolves a symbolic revision, and returns
    /// `None` only when the path is absent from the specified commit.
    fn read_file_at_commit(
        &self,
        _path: &Path,
        _commit: &str,
        _relative_path: &str,
    ) -> ModelResult<Option<Vec<u8>>> {
        unsupported_backend("read_file_at_commit")
    }
    /// Return whether `commit` is an exact two-parent merge commit with the
    /// supplied ordered parents and byte-exact message. This is read-only and
    /// never resolves an abbreviation or fetches a missing object.
    fn commit_matches_merge(
        &self,
        _path: &Path,
        _commit: &str,
        _first_parent: &str,
        _second_parent: &str,
        _message: &str,
    ) -> ModelResult<bool> {
        unsupported_backend("commit_matches_merge")
    }
    /// Return whether `commit` exactly matches a prepared two-parent merge,
    /// including its tree and complete author/committer signatures.
    fn commit_matches_prepared_merge(
        &self,
        _path: &Path,
        _commit: &str,
        _first_parent: &str,
        _second_parent: &str,
        _message: &str,
        _prepared: &GitPreparedCommit,
    ) -> ModelResult<bool> {
        unsupported_backend("commit_matches_prepared_merge")
    }
    };
}

// One method group in the existing trait/impl; not a new interface.
#[rustfmt::skip]
macro_rules! repository_contract_inspection {
    () => {
    fn status(&self, path: &Path) -> ModelResult<GitStatus>;
    fn status_with_options(
        &self,
        path: &Path,
        _options: GitStatusOptions,
    ) -> ModelResult<GitStatus> {
        self.status(path)
    }
    fn head(&self, path: &Path) -> ModelResult<GitHeadState>;
    fn remotes(&self, path: &Path) -> ModelResult<Vec<GitRemote>>;
    fn add_remote(&self, path: &Path, name: &str, url: &str) -> ModelResult<GitRemoteResult>;
    fn push(&self, path: &Path, remote: &str, refspec: &str) -> ModelResult<GitPushResult>;
    fn prepare_push(
        &self,
        _path: &Path,
        _remote: &str,
        _refspec: &str,
    ) -> ModelResult<GitPreparedPush> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "captured publication is not implemented by this GitBackend",
        ))
    }
    fn push_prepared(&self, _path: &Path, _plan: &GitPreparedPush) -> ModelResult<GitPushResult> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "captured publication is not implemented by this GitBackend",
        ))
    }
    /// The object `destination` (`refs/heads/<branch>`) had on `remote`'s
    /// repository when this repository last recorded it, in the
    /// remote-tracking ref that fetches, and pushes through the named remote,
    /// write. Local only: it never contacts the remote. A push uses it only to
    /// decide whether to contact a destination (gwz-dev
    /// `dev-docs/GwzUrlSchemePushPlan.md` §3.5 rule 2), so an implementation
    /// answers `None` whenever the ref may not stand for that repository. The
    /// default answers `None`, and the push contacts the remote.
    fn last_known_ref(
        &self,
        _path: &Path,
        _remote: &str,
        _destination: &str,
    ) -> ModelResult<Option<String>> {
        Ok(None)
    }
    /// Anonymous local fetch (LCM1.0c, local clone family; gwz-dev
    /// `dev-docs/GwzLocalCloneDesign.md` §6.2). `url` must be an existing
    /// local repository path, never a URL (the backend hands libgit2 its
    /// canonical `file://` form so the local transport is selected whatever
    /// the path contains); `refspecs` are explicit and required. The backend
    /// creates an anonymous in-memory remote, attaches no credential or
    /// network helper, persists no remote name, updates no `origin`
    /// tracking ref, follows no tags and writes no fetch record. Measured
    /// (LCM1.0c-rem1, `local_clone::tests::transport`): libgit2 still
    /// truncates the receiver's `FETCH_HEAD` to empty on every fetch,
    /// creating it when absent, so a prior fetch record in the receiver does
    /// not survive this port; `FETCH_HEAD` is outside the port's promise and
    /// no gwz reader consumes it. Refuses before any effect on a non-local
    /// peer or an empty refspec list.
    fn fetch_anonymous(
        &self,
        path: &Path,
        url: &str,
        refspecs: &[&str],
    ) -> ModelResult<GitFetchResult>;
    /// Anonymous local push with one explicit refspec, same peer rules as
    /// [`GitBackend::fetch_anonymous`]. A per-ref rejection reported by the
    /// receiving side (for example a non-fast-forward update without `+`)
    /// is a typed `remote_rejected` error, never a silent success.
    fn push_anonymous(&self, path: &Path, url: &str, refspec: &str) -> ModelResult<GitPushResult>;
    fn read_ref(&self, path: &Path, ref_spec: &str) -> ModelResult<Option<String>>;
    fn is_ancestor(&self, path: &Path, ancestor: &str, descendant: &str) -> ModelResult<bool>;
    /// Count how far `local` is ahead of and behind `upstream`: the commits
    /// reachable from one and not the other, in that order. `is_ancestor`
    /// answers the boolean; this answers `git status -sb`'s `+A -B`
    /// (gwz-cli dev-docs/GwzFetchPlan.md D6). Both operands are hex object
    /// ids. A default so no existing backend implementation changes.
    fn ahead_behind(
        &self,
        _path: &Path,
        _local: &str,
        _upstream: &str,
    ) -> ModelResult<GitAheadBehind> {
        unsupported_backend("ahead_behind")
    }
    /// Return the best merge base for two commits, when one exists.
    fn merge_base(&self, _path: &Path, _left: &str, _right: &str) -> ModelResult<Option<String>> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "merge_base is not implemented by this GitBackend",
        ))
    }
    /// List paths whose tree entries differ between two commits.
    fn changed_paths_between(
        &self,
        _path: &Path,
        _old_commit: &str,
        _new_commit: &str,
    ) -> ModelResult<Vec<String>> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "changed_paths_between is not implemented by this GitBackend",
        ))
    }
    /// Diff a **single** repository (the workspace root or one materialized
    /// member) into a repo-scoped changed-file manifest. This is the D1 Git
    /// backend primitive: it resolves the requested comparison to libgit2 tree
    /// sides, runs the matching libgit2 diff, applies rename detection, and
    /// reports per-file status/mode/binary/similarity/line-stats with
    /// repo-relative paths. Workspace projection (scopes, member-prefix
    /// rewriting, root/member ordering, `gwz.conf` exclusion) is the D2 planner's
    /// job, not this primitive's. Paths in `comparison`/`options` are already
    /// repo-relative. See [`crate::diff::diff_repo`].
    fn diff_manifest(
        &self,
        _path: &Path,
        _comparison: &crate::diff::RepoDiffComparison,
        _options: &crate::diff::RepoDiffOptions,
    ) -> ModelResult<crate::diff::RepoDiffManifest> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "diff_manifest is not implemented by this GitBackend",
        ))
    }
    /// Resolve a per-repo comparison from raw revision tokens to concrete
    /// libgit2 tree sides (peeling refs/commits to trees, `HEAD`/unborn-HEAD to a
    /// tree or the empty tree, and a `A...B` merge-base old side). Snapshot
    /// operand resolution and candidate selection are D2; this handles only the
    /// per-repo revision → oid step of the primitive. See
    /// [`crate::diff::resolve_comparison`].
    fn resolve_comparison(
        &self,
        _path: &Path,
        _spec: &crate::diff::ComparisonSpec,
    ) -> ModelResult<crate::diff::RepoDiffComparison> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "resolve_comparison is not implemented by this GitBackend",
        ))
    }
    };
}

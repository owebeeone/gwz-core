// One method group in the existing trait/impl; not a new interface.
#[rustfmt::skip]
macro_rules! repository_contract_fetch {
    () => {
    fn create_repo(&self, path: &Path) -> ModelResult<GitCreateResult>;
    fn clone_repo(&self, url: &str, path: &Path) -> ModelResult<GitCloneResult>;
    /// Clone, forwarding libgit2 transfer progress to `progress`. The default
    /// ignores progress; backends that support it override this.
    fn clone_repo_with_progress(
        &self,
        url: &str,
        path: &Path,
        _progress: &dyn Fn(crate::GitTransferProgress),
    ) -> ModelResult<GitCloneResult> {
        self.clone_repo(url, path)
    }
    /// Clone using the source's declared fetch remote, including its identity.
    fn clone_repo_named(
        &self,
        url: &str,
        path: &Path,
        remote: &str,
        progress: &dyn Fn(crate::GitTransferProgress),
    ) -> ModelResult<GitCloneResult> {
        if remote != "origin" {
            return unsupported_backend("clone_repo_named");
        }
        self.clone_repo_with_progress(url, path, progress)
    }
    fn fetch(&self, path: &Path, remote: &str) -> ModelResult<GitFetchResult>;
    /// List the refs a remote advertises WITHOUT fetching objects (porcelain
    /// `git ls-remote`): connect, read the advertised refs, disconnect. Non-mutating
    /// — used to plan a selection before any fetch (Q1).
    fn ls_remote(&self, path: &Path, remote: &str) -> ModelResult<Vec<GitRemoteRef>>;
    /// Read advertised refs from an exact URL without persisting a remote or
    /// fetching objects. Used to prove committed-lock publication dependencies.
    fn ls_remote_url(
        &self,
        _path: &Path,
        _url: &str,
        _remote_name: &str,
        _identity_repo: Option<&Path>,
    ) -> ModelResult<Vec<GitRemoteRef>> {
        unsupported_backend("ls_remote_url")
    }
    };
}

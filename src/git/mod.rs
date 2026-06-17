use std::fs;
use std::path::{Path, PathBuf};

use crate::model::{ErrorCode, ModelError, ModelResult};

pub trait GitBackend {
    fn is_repository(&self, path: &Path) -> ModelResult<bool>;
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
    fn fetch(&self, path: &Path, remote: &str) -> ModelResult<GitFetchResult>;
    fn fast_forward(
        &self,
        path: &Path,
        branch: &str,
        upstream_ref: &str,
    ) -> ModelResult<GitUpdateResult>;
    fn checkout_commit(&self, path: &Path, commit: &str) -> ModelResult<GitUpdateResult>;
    fn status(&self, path: &Path) -> ModelResult<GitStatus>;
    fn head(&self, path: &Path) -> ModelResult<GitHeadState>;
    fn remotes(&self, path: &Path) -> ModelResult<Vec<GitRemote>>;
    fn add_remote(&self, path: &Path, name: &str, url: &str) -> ModelResult<GitRemoteResult>;
    fn push(&self, path: &Path, remote: &str, refspec: &str) -> ModelResult<GitPushResult>;
    fn read_ref(&self, path: &Path, ref_spec: &str) -> ModelResult<Option<String>>;
    fn is_ancestor(&self, path: &Path, ancestor: &str, descendant: &str) -> ModelResult<bool>;
    /// Stage `pathspecs` into the index — `git add` semantics: add new/modified
    /// files, remove deleted ones, honor `.gitignore`. Self-verifies the index
    /// persisted with the requested files staged before returning success.
    /// Content parity with porcelain `git add` is proven by contract test.
    fn stage_paths(&self, path: &Path, pathspecs: &[&str]) -> ModelResult<GitStageResult>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialHelperPolicy {
    Disabled,
    AllowConfigured,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Git2Backend {
    credential_helpers: CredentialHelperPolicy,
}

impl Git2Backend {
    pub fn new() -> Self {
        Self {
            credential_helpers: CredentialHelperPolicy::AllowConfigured,
        }
    }

    pub fn without_credential_helpers() -> Self {
        Self {
            credential_helpers: CredentialHelperPolicy::Disabled,
        }
    }
}

impl Default for Git2Backend {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitCreateResult {
    pub path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitCloneResult {
    pub path: PathBuf,
    pub head: GitHeadState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitFetchResult {
    pub remote: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitUpdateResult {
    pub updated: bool,
    pub commit: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRemoteResult {
    pub remote: GitRemote,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitPushResult {
    pub remote: String,
    pub refspec: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitStageResult {
    /// Top-level *file* pathspecs confirmed present in the index by the self-verify
    /// pass. Directory pathspecs are staged but not counted here.
    pub staged: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GitStatus {
    pub is_dirty: bool,
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
    pub files: Vec<GitFileStatus>,
}

impl GitStatus {
    pub fn clean() -> Self {
        Self::default()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitFileStatus {
    pub path: String,
    pub index_status: String,
    pub worktree_status: String,
    pub original_path: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitHeadState {
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub is_detached: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRemote {
    pub name: String,
    pub url: Option<String>,
    pub push_url: Option<String>,
}

impl GitBackend for Git2Backend {
    fn is_repository(&self, path: &Path) -> ModelResult<bool> {
        match git2::Repository::open(path) {
            Ok(_) => Ok(true),
            Err(err) if err.code() == git2::ErrorCode::NotFound => Ok(false),
            Err(err) => Err(git_error(err)),
        }
    }

    fn create_repo(&self, path: &Path) -> ModelResult<GitCreateResult> {
        let mut opts = git2::RepositoryInitOptions::new();
        opts.bare(false).no_reinit(true).initial_head("main");
        git2::Repository::init_opts(path, &opts).map_err(git_error)?;
        Ok(GitCreateResult {
            path: path.to_path_buf(),
        })
    }

    fn clone_repo(&self, url: &str, path: &Path) -> ModelResult<GitCloneResult> {
        self.clone_repo_with_progress(url, path, &|_progress| {})
    }

    fn clone_repo_with_progress(
        &self,
        url: &str,
        path: &Path,
        progress: &dyn Fn(crate::GitTransferProgress),
    ) -> ModelResult<GitCloneResult> {
        ensure_clone_target_is_empty(path)?;
        let mut builder = git2::build::RepoBuilder::new();
        builder.fetch_options(fetch_options_with_progress(
            self.credential_helpers,
            Some(progress),
        ));
        builder.clone(url, path).map_err(git_error)?;
        Ok(GitCloneResult {
            path: path.to_path_buf(),
            head: self.head(path)?,
        })
    }

    fn fetch(&self, path: &Path, remote: &str) -> ModelResult<GitFetchResult> {
        let repo = open_repo(path)?;
        let mut remote_handle = find_remote(&repo, remote)?;
        let refspecs: [&str; 0] = [];
        remote_handle
            .fetch(
                &refspecs,
                Some(&mut remote_fetch_options(self.credential_helpers)),
                Some("gwz fetch"),
            )
            .map_err(git_error)?;
        Ok(GitFetchResult {
            remote: remote.to_owned(),
        })
    }

    fn fast_forward(
        &self,
        path: &Path,
        branch: &str,
        upstream_ref: &str,
    ) -> ModelResult<GitUpdateResult> {
        let repo = open_repo(path)?;
        let target = repo.revparse_single(upstream_ref).map_err(git_error)?.id();
        let annotated = repo.find_annotated_commit(target).map_err(git_error)?;
        let (analysis, _) = repo.merge_analysis(&[&annotated]).map_err(git_error)?;

        if analysis.is_up_to_date() {
            return Ok(GitUpdateResult {
                updated: false,
                commit: Some(target.to_string()),
            });
        }
        if !analysis.is_fast_forward() {
            return Err(ModelError::new(
                ErrorCode::DivergedMember,
                "branch cannot be fast-forwarded",
            ));
        }

        let local_ref_name = format!("refs/heads/{branch}");
        let mut local_ref = repo.find_reference(&local_ref_name).map_err(git_error)?;
        let target_object = repo.find_object(target, None).map_err(git_error)?;
        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.safe();
        repo.checkout_tree(&target_object, Some(&mut checkout))
            .map_err(git_error)?;
        local_ref
            .set_target(target, "gwz fast-forward")
            .map_err(git_error)?;
        repo.set_head(&local_ref_name).map_err(git_error)?;
        verify_checkout_state(path, target)?;
        Ok(GitUpdateResult {
            updated: true,
            commit: Some(target.to_string()),
        })
    }

    fn checkout_commit(&self, path: &Path, commit: &str) -> ModelResult<GitUpdateResult> {
        let repo = open_repo(path)?;
        let oid = git2::Oid::from_str(commit).map_err(git_error)?;
        let object = repo.find_object(oid, None).map_err(git_error)?;
        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.safe();
        repo.checkout_tree(&object, Some(&mut checkout))
            .map_err(git_error)?;
        repo.set_head_detached(oid).map_err(git_error)?;
        verify_checkout_state(path, oid)?;
        Ok(GitUpdateResult {
            updated: true,
            commit: Some(oid.to_string()),
        })
    }

    fn status(&self, path: &Path) -> ModelResult<GitStatus> {
        let repo = open_repo(path)?;
        let mut opts = git2::StatusOptions::new();
        opts.include_untracked(true).recurse_untracked_dirs(true);
        let statuses = repo.statuses(Some(&mut opts)).map_err(git_error)?;
        let mut out = GitStatus::default();
        for entry in statuses.iter() {
            let status = entry.status();
            if status.intersects(staged_statuses()) {
                out.staged += 1;
            }
            if status.intersects(unstaged_statuses()) {
                out.unstaged += 1;
            }
            if status.contains(git2::Status::WT_NEW) {
                out.untracked += 1;
            }
            if let Some(file) = git_file_status(&entry) {
                out.files.push(file);
            }
        }
        out.is_dirty = out.staged > 0 || out.unstaged > 0 || out.untracked > 0;
        Ok(out)
    }

    fn head(&self, path: &Path) -> ModelResult<GitHeadState> {
        let repo = open_repo(path)?;
        repo_head(&repo)
    }

    fn remotes(&self, path: &Path) -> ModelResult<Vec<GitRemote>> {
        let repo = open_repo(path)?;
        let names = repo.remotes().map_err(git_error)?;
        let mut remotes = Vec::new();
        for name in names.iter() {
            let Some(name) = name.map_err(git_error)? else {
                continue;
            };
            let remote = find_remote(&repo, name)?;
            remotes.push(GitRemote {
                name: name.to_owned(),
                url: Some(remote.url().map_err(git_error)?.to_owned()),
                push_url: remote.pushurl().map_err(git_error)?.map(ToOwned::to_owned),
            });
        }
        Ok(remotes)
    }

    fn add_remote(&self, path: &Path, name: &str, url: &str) -> ModelResult<GitRemoteResult> {
        let repo = open_repo(path)?;
        let remote = repo.remote(name, url).map_err(git_error)?;
        Ok(GitRemoteResult {
            remote: GitRemote {
                name: name.to_owned(),
                url: Some(remote.url().map_err(git_error)?.to_owned()),
                push_url: remote.pushurl().map_err(git_error)?.map(ToOwned::to_owned),
            },
        })
    }

    fn push(&self, path: &Path, remote: &str, refspec: &str) -> ModelResult<GitPushResult> {
        let repo = open_repo(path)?;
        let mut remote_handle = find_remote(&repo, remote)?;
        remote_handle
            .push(
                &[refspec],
                Some(&mut remote_push_options(self.credential_helpers)),
            )
            .map_err(git_error)?;
        Ok(GitPushResult {
            remote: remote.to_owned(),
            refspec: refspec.to_owned(),
        })
    }

    fn stage_paths(&self, path: &Path, pathspecs: &[&str]) -> ModelResult<GitStageResult> {
        let repo = open_repo(path)?;
        let mut index = repo.index().map_err(git_error)?;
        index
            .add_all(
                pathspecs.iter().copied(),
                git2::IndexAddOption::DEFAULT,
                None,
            )
            .map_err(git_error)?;
        index.write().map_err(git_error)?;

        // AD1 self-verify: re-open the repo so the index is read fresh from disk,
        // and confirm every requested *file* persisted into the index. Directory
        // pathspecs are covered by the fresh read; full content parity with
        // porcelain `git add` is proven by the contract test, not asserted here.
        let verify = open_repo(path)?.index().map_err(git_error)?;
        if verify.has_conflicts() {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                "index has conflicts after staging",
            ));
        }
        let mut staged = 0usize;
        for spec in pathspecs {
            if path.join(spec).is_file() {
                if verify.get_path(Path::new(spec), 0).is_none() {
                    return Err(ModelError::new(
                        ErrorCode::GitCommandFailed,
                        format!("staged path missing from index after write: {spec}"),
                    ));
                }
                staged += 1;
            }
        }
        Ok(GitStageResult { staged })
    }

    fn read_ref(&self, path: &Path, ref_spec: &str) -> ModelResult<Option<String>> {
        let repo = open_repo(path)?;
        match repo.revparse_single(ref_spec) {
            Ok(object) => Ok(Some(object.id().to_string())),
            Err(err)
                if matches!(
                    err.code(),
                    git2::ErrorCode::NotFound | git2::ErrorCode::UnbornBranch
                ) =>
            {
                Ok(None)
            }
            Err(err) => Err(git_error(err)),
        }
    }

    fn is_ancestor(&self, path: &Path, ancestor: &str, descendant: &str) -> ModelResult<bool> {
        let repo = open_repo(path)?;
        let ancestor = git2::Oid::from_str(ancestor).map_err(git_error)?;
        let descendant = git2::Oid::from_str(descendant).map_err(git_error)?;
        repo.graph_descendant_of(descendant, ancestor)
            .map_err(git_error)
    }
}

fn open_repo(path: &Path) -> ModelResult<git2::Repository> {
    git2::Repository::open(path).map_err(git_error)
}

fn find_remote<'repo>(
    repo: &'repo git2::Repository,
    name: &str,
) -> ModelResult<git2::Remote<'repo>> {
    repo.find_remote(name).map_err(|err| {
        if err.code() == git2::ErrorCode::NotFound {
            ModelError::new(ErrorCode::MissingRemote, format!("missing remote '{name}'"))
        } else {
            git_error(err)
        }
    })
}

fn remote_fetch_options(credential_helpers: CredentialHelperPolicy) -> git2::FetchOptions<'static> {
    fetch_options_with_progress(credential_helpers, None)
}

fn fetch_options_with_progress<'a>(
    credential_helpers: CredentialHelperPolicy,
    progress: Option<&'a dyn Fn(crate::GitTransferProgress)>,
) -> git2::FetchOptions<'a> {
    let mut callbacks = remote_callbacks(credential_helpers);
    if let Some(progress) = progress {
        callbacks.transfer_progress(move |stats| {
            progress(git_transfer_progress(&stats));
            true
        });
    }
    let mut options = git2::FetchOptions::new();
    options.remote_callbacks(callbacks);
    options
}

fn git_transfer_progress(stats: &git2::Progress) -> crate::GitTransferProgress {
    let received_objects = stats.received_objects();
    let total_objects = stats.total_objects();
    // libgit2's transfer callback hands the same counters for both phases; once
    // every object is received, remaining work is delta resolution.
    let phase = if total_objects > 0 && received_objects >= total_objects {
        crate::GitProgressPhase::Resolving
    } else {
        crate::GitProgressPhase::Receiving
    };
    crate::GitTransferProgress {
        phase,
        received_objects: Some(received_objects as i64),
        total_objects: Some(total_objects as i64),
        received_bytes: Some(stats.received_bytes() as i64),
        indexed_deltas: Some(stats.indexed_deltas() as i64),
        total_deltas: Some(stats.total_deltas() as i64),
    }
}

fn remote_push_options(credential_helpers: CredentialHelperPolicy) -> git2::PushOptions<'static> {
    let mut options = git2::PushOptions::new();
    options.remote_callbacks(remote_callbacks(credential_helpers));
    options
}

fn remote_callbacks<'a>(credential_helpers: CredentialHelperPolicy) -> git2::RemoteCallbacks<'a> {
    let mut callbacks = git2::RemoteCallbacks::new();
    callbacks.credentials(move |url, username_from_url, allowed_types| {
        remote_credential(url, username_from_url, allowed_types, credential_helpers)
    });
    callbacks
}

fn remote_credential(
    url: &str,
    username_from_url: Option<&str>,
    allowed_types: git2::CredentialType,
    credential_helpers: CredentialHelperPolicy,
) -> Result<git2::Cred, git2::Error> {
    let username = username_from_url.unwrap_or("git");
    if allowed_types.is_ssh_key() {
        return git2::Cred::ssh_key_from_agent(username);
    }
    if allowed_types.is_username() {
        return git2::Cred::username(username);
    }
    if allowed_types.is_user_pass_plaintext()
        && credential_helpers == CredentialHelperPolicy::AllowConfigured
        && let Ok(config) = git2::Config::open_default()
        && let Ok(credential) = git2::Cred::credential_helper(&config, url, username_from_url)
    {
        return Ok(credential);
    }
    if allowed_types.is_default() {
        return git2::Cred::default();
    }
    Err(git2::Error::from_str(
        "GWZ could not acquire credentials for the requested remote",
    ))
}

fn repo_head(repo: &git2::Repository) -> ModelResult<GitHeadState> {
    let head = match repo.head() {
        Ok(head) => head,
        Err(err) if err.code() == git2::ErrorCode::UnbornBranch => {
            return unborn_head(repo);
        }
        Err(err) => return Err(git_error(err)),
    };
    let branch = if head.is_branch() {
        Some(head.shorthand().map_err(git_error)?.to_owned())
    } else {
        None
    };
    Ok(GitHeadState {
        branch,
        commit: head.target().map(|target| target.to_string()),
        is_detached: !head.is_branch(),
    })
}

fn unborn_head(repo: &git2::Repository) -> ModelResult<GitHeadState> {
    let head = fs::read_to_string(repo.path().join("HEAD")).map_err(io_error)?;
    let branch = head
        .trim()
        .strip_prefix("ref: refs/heads/")
        .map(ToOwned::to_owned);
    Ok(GitHeadState {
        branch,
        commit: None,
        is_detached: false,
    })
}

fn ensure_clone_target_is_empty(path: &Path) -> ModelResult<()> {
    if !path.exists() {
        return Ok(());
    }
    if !path.is_dir() {
        return Err(ModelError::new(
            ErrorCode::PathCollision,
            "clone target exists and is not a directory",
        ));
    }
    if fs::read_dir(path)
        .map_err(io_error)?
        .next()
        .transpose()
        .map_err(io_error)?
        .is_some()
    {
        return Err(ModelError::new(
            ErrorCode::PathCollision,
            "clone target is not empty",
        ));
    }
    Ok(())
}

fn staged_statuses() -> git2::Status {
    git2::Status::INDEX_NEW
        | git2::Status::INDEX_MODIFIED
        | git2::Status::INDEX_DELETED
        | git2::Status::INDEX_RENAMED
        | git2::Status::INDEX_TYPECHANGE
}

fn unstaged_statuses() -> git2::Status {
    git2::Status::WT_MODIFIED
        | git2::Status::WT_DELETED
        | git2::Status::WT_RENAMED
        | git2::Status::WT_TYPECHANGE
}

fn git_file_status(entry: &git2::StatusEntry<'_>) -> Option<GitFileStatus> {
    let status = entry.status();
    let path = entry.path().ok()?.to_owned();
    Some(GitFileStatus {
        path,
        index_status: index_status_char(status).to_owned(),
        worktree_status: worktree_status_char(status).to_owned(),
        original_path: original_path(entry),
    })
}

fn index_status_char(status: git2::Status) -> &'static str {
    if status.contains(git2::Status::INDEX_NEW) {
        "A"
    } else if status.contains(git2::Status::INDEX_MODIFIED) {
        "M"
    } else if status.contains(git2::Status::INDEX_DELETED) {
        "D"
    } else if status.contains(git2::Status::INDEX_RENAMED) {
        "R"
    } else if status.contains(git2::Status::INDEX_TYPECHANGE) {
        "T"
    } else {
        " "
    }
}

fn worktree_status_char(status: git2::Status) -> &'static str {
    if status.contains(git2::Status::WT_NEW) {
        "?"
    } else if status.contains(git2::Status::WT_MODIFIED) {
        "M"
    } else if status.contains(git2::Status::WT_DELETED) {
        "D"
    } else if status.contains(git2::Status::WT_RENAMED) {
        "R"
    } else if status.contains(git2::Status::WT_TYPECHANGE) {
        "T"
    } else {
        " "
    }
}

fn original_path(entry: &git2::StatusEntry<'_>) -> Option<String> {
    if !entry
        .status()
        .intersects(git2::Status::INDEX_RENAMED | git2::Status::WT_RENAMED)
    {
        return None;
    }
    entry
        .head_to_index()
        .or_else(|| entry.index_to_workdir())
        .and_then(|delta| delta.old_file().path())
        .and_then(|path| path.to_str())
        .map(ToOwned::to_owned)
}

/// AD1 self-verify for ref/worktree-advancing primitives: re-open the repo and
/// confirm HEAD now resolves to `expected` and the tracked worktree matches it
/// (no staged or unstaged drift). Catches the F0 class of bug — a ref advanced
/// without the worktree following, or vice versa — before the primitive reports
/// success. Untracked files are ignored.
fn verify_checkout_state(path: &Path, expected: git2::Oid) -> ModelResult<()> {
    let repo = open_repo(path)?;
    let head_oid = repo
        .head()
        .and_then(|head| head.peel_to_commit())
        .map_err(git_error)?
        .id();
    if head_oid != expected {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            format!("post-update HEAD {head_oid} does not match target {expected}"),
        ));
    }
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(false);
    let drifted = repo
        .statuses(Some(&mut opts))
        .map_err(git_error)?
        .iter()
        .any(|entry| {
            entry
                .status()
                .intersects(staged_statuses() | unstaged_statuses())
        });
    if drifted {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "post-update worktree does not match the target tree",
        ));
    }
    Ok(())
}

fn git_error(error: git2::Error) -> ModelError {
    ModelError::new(ErrorCode::GitCommandFailed, error.message())
}

fn io_error(error: std::io::Error) -> ModelError {
    ModelError::new(ErrorCode::IoError, error.to_string())
}

/// Extracts the remote host from a git URL, for per-host connection limiting.
/// Handles scp-like `git@host:path`, scheme URLs (`https://`, `ssh://`, …), and
/// returns `None` for local paths or any URL with no parseable host (which the
/// caller bounds only by the global concurrency ceiling).
pub fn git_host(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    if url.contains("://") {
        return url::Url::parse(url)
            .ok()
            .and_then(|parsed| parsed.host_str().map(str::to_ascii_lowercase))
            .filter(|host| !host.is_empty());
    }
    // scp-like: [user@]host:path — a colon before any slash.
    let colon = url.find(':')?;
    let authority = &url[..colon];
    if authority.contains('/') {
        return None; // a local path that happens to contain a colon
    }
    let host = authority.rsplit('@').next().unwrap_or(authority).trim();
    // A lone alphabetic char before ':' is a Windows drive letter, not a host.
    if host.is_empty() || (host.len() == 1 && host.chars().all(|c| c.is_ascii_alphabetic())) {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::model::ErrorCode;

    use super::*;

    #[test]
    fn git_host_parses_scheme_scp_and_local_forms() {
        assert_eq!(
            git_host("https://github.com/o/r.git").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            git_host("https://github.com:443/o/r.git").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            git_host("ssh://git@example.org/o/r.git").as_deref(),
            Some("example.org")
        );
        assert_eq!(
            git_host("git@github.com:o/r.git").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            git_host("github.com:o/r.git").as_deref(),
            Some("github.com")
        );
        // Host is case-insensitive.
        assert_eq!(
            git_host("GitHub.COM:o/r.git").as_deref(),
            Some("github.com")
        );
        // Local / hostless forms.
        assert_eq!(git_host("/tmp/repo.git"), None);
        assert_eq!(git_host("file:///tmp/repo.git"), None);
        assert_eq!(git_host("./relative.git"), None);
        assert_eq!(git_host("C:/work/repo.git"), None);
        assert_eq!(git_host(""), None);
    }

    #[test]
    fn creates_and_detects_ordinary_non_bare_repositories() {
        let temp = TempDir::new("create");
        let backend = Git2Backend::new();
        let repo_path = temp.path().join("repo");

        let created = backend.create_repo(&repo_path).unwrap();

        assert_eq!(created.path, repo_path);
        assert!(backend.is_repository(&repo_path).unwrap());
        assert!(!backend.is_repository(&temp.path().join("missing")).unwrap());
        assert!(!git2::Repository::open(&repo_path).unwrap().is_bare());
    }

    #[test]
    fn stage_paths_matches_porcelain_git_add() {
        // Seed two identical repos; stage one via the primitive and one via
        // porcelain `git add`. The resulting index must be byte-identical —
        // pathspec scoping, recursive add, and `.gitignore` honoring all agree.
        let temp = TempDir::new("stage-parity");
        let prim = temp.path().join("prim");
        let porc = temp.path().join("porc");
        seed_stage_repo(&prim);
        seed_stage_repo(&porc);

        let result = Git2Backend::new()
            .stage_paths(&prim, &["tracked", ".gitignore"])
            .expect("primitive stage");
        // Only ".gitignore" is a top-level file; "tracked" is a directory.
        assert_eq!(result.staged, 1);

        run_git(&porc, &["add", "tracked", ".gitignore"]);

        assert_eq!(
            ls_files_stage(&prim),
            ls_files_stage(&porc),
            "primitive index must match `git add` porcelain (mode+oid+path)"
        );
        // Sanity: the gitignored, out-of-pathspec files are staged by neither.
        assert!(!ls_files_stage(&prim).contains("ignored/"));
        assert!(!ls_files_stage(&prim).contains("loose.txt"));
    }

    #[test]
    fn stage_paths_errors_on_non_repository() {
        let temp = TempDir::new("stage-nonrepo");
        let err = Git2Backend::new()
            .stage_paths(temp.path(), &[".gitignore"])
            .expect_err("staging a non-repository must fail");
        assert_eq!(err.code, ErrorCode::GitCommandFailed);
    }

    fn seed_stage_repo(root: &Path) {
        fs::create_dir_all(root.join("tracked")).unwrap();
        fs::write(root.join("tracked").join("a.txt"), "a\n").unwrap();
        fs::create_dir_all(root.join("ignored")).unwrap();
        fs::write(root.join("ignored").join("b.txt"), "b\n").unwrap();
        fs::write(root.join(".gitignore"), "/ignored/\n").unwrap();
        fs::write(root.join("loose.txt"), "loose\n").unwrap();
        Git2Backend::new().create_repo(root).unwrap();
    }

    fn run_git(root: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=GWZ",
                "-c",
                "user.email=gwz@example.invalid",
            ])
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .expect("spawn git");
        assert!(status.success(), "git {args:?} failed");
    }

    fn ls_files_stage(root: &Path) -> String {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["ls-files", "--stage"])
            .output()
            .expect("spawn git ls-files");
        assert!(output.status.success(), "git ls-files failed");
        String::from_utf8(output.stdout).expect("ls-files utf8")
    }

    #[test]
    fn empty_repository_head_reports_unborn_branch_without_commit() {
        let temp = TempDir::new("empty-head");
        let backend = Git2Backend::new();
        let repo_path = temp.path().join("repo");
        backend.create_repo(&repo_path).unwrap();

        let head = backend.head(&repo_path).unwrap();

        assert_eq!(head.branch, Some("main".to_owned()));
        assert_eq!(head.commit, None);
        assert!(!head.is_detached);
        assert_eq!(backend.read_ref(&repo_path, "HEAD").unwrap(), None);
    }

    #[test]
    fn reads_and_adds_remotes() {
        let temp = TempDir::new("remotes");
        let backend = Git2Backend::new();
        let repo_path = temp.path().join("repo");
        backend.create_repo(&repo_path).unwrap();

        backend
            .add_remote(&repo_path, "origin", "file:///tmp/origin.git")
            .unwrap();

        let remotes = backend.remotes(&repo_path).unwrap();
        assert_eq!(
            remotes,
            vec![GitRemote {
                name: "origin".to_owned(),
                url: Some("file:///tmp/origin.git".to_owned()),
                push_url: None,
            }]
        );
    }

    #[test]
    fn reports_clean_untracked_unstaged_and_staged_status() {
        let temp = TempDir::new("status");
        let backend = Git2Backend::new();
        let repo_path = temp.path().join("repo");
        backend.create_repo(&repo_path).unwrap();
        commit_file(&repo_path, "tracked.txt", "one", "initial", &[]).unwrap();

        assert_eq!(backend.status(&repo_path).unwrap(), GitStatus::clean());

        fs::write(repo_path.join("untracked.txt"), "new").unwrap();
        let status = backend.status(&repo_path).unwrap();
        assert!(status.is_dirty);
        assert_eq!(status.untracked, 1);
        fs::remove_file(repo_path.join("untracked.txt")).unwrap();

        fs::write(repo_path.join("tracked.txt"), "two").unwrap();
        let status = backend.status(&repo_path).unwrap();
        assert!(status.is_dirty);
        assert_eq!(status.unstaged, 1);
        assert_eq!(status.staged, 0);

        stage_path(&repo_path, "tracked.txt").unwrap();
        let status = backend.status(&repo_path).unwrap();
        assert!(status.is_dirty);
        assert_eq!(status.staged, 1);
        assert_eq!(status.unstaged, 0);
    }

    #[test]
    fn clones_local_repo_and_rejects_non_empty_targets_before_mutation() {
        let temp = TempDir::new("clone");
        let backend = Git2Backend::new();
        let source_path = temp.path().join("source");
        backend.create_repo(&source_path).unwrap();
        commit_file(&source_path, "README.md", "hello", "initial", &[]).unwrap();

        let clone_path = temp.path().join("clone");
        backend
            .clone_repo(source_path.to_str().unwrap(), &clone_path)
            .unwrap();
        assert!(backend.is_repository(&clone_path).unwrap());
        assert!(clone_path.join("README.md").is_file());

        let blocked_path = temp.path().join("blocked");
        fs::create_dir_all(&blocked_path).unwrap();
        fs::write(blocked_path.join("keep.txt"), "keep").unwrap();
        let err = backend
            .clone_repo(source_path.to_str().unwrap(), &blocked_path)
            .unwrap_err();

        assert_eq!(err.code, ErrorCode::PathCollision);
        assert!(blocked_path.join("keep.txt").is_file());
        assert!(!blocked_path.join(".git").exists());
    }

    #[test]
    fn new_backend_allows_configured_credential_helpers() {
        assert_eq!(
            Git2Backend::new().credential_helpers,
            CredentialHelperPolicy::AllowConfigured
        );
        assert_eq!(
            Git2Backend::without_credential_helpers().credential_helpers,
            CredentialHelperPolicy::Disabled
        );
    }

    #[test]
    fn remote_credentials_support_ssh_agent_username_and_default_auth() {
        let ssh = remote_credential(
            "ssh://github.com/example/repo.git",
            Some("git"),
            git2::CredentialType::SSH_KEY,
            CredentialHelperPolicy::Disabled,
        )
        .unwrap();
        assert!(ssh.has_username());

        let username = remote_credential(
            "ssh://github.com/example/repo.git",
            None,
            git2::CredentialType::USERNAME,
            CredentialHelperPolicy::Disabled,
        )
        .unwrap();
        assert!(username.has_username());

        remote_credential(
            "https://github.com/example/repo.git",
            None,
            git2::CredentialType::DEFAULT,
            CredentialHelperPolicy::Disabled,
        )
        .unwrap();
    }

    #[test]
    fn remote_credentials_reject_plaintext_auth_when_helpers_are_disabled() {
        let result = remote_credential(
            "https://github.com/example/repo.git",
            None,
            git2::CredentialType::USER_PASS_PLAINTEXT,
            CredentialHelperPolicy::Disabled,
        );
        let err = match result {
            Ok(_) => panic!("expected disabled credential helpers to reject plaintext auth"),
            Err(err) => err,
        };

        assert!(err.message().contains("could not acquire credentials"));
    }

    #[test]
    fn pushes_fetches_fast_forwards_and_checks_out_commits() {
        let temp = TempDir::new("networkless");
        let backend = Git2Backend::new();
        let source_path = temp.path().join("source");
        let bare_path = temp.path().join("remote.git");
        let clone_path = temp.path().join("clone");
        backend.create_repo(&source_path).unwrap();
        init_bare_main(&bare_path);
        backend
            .add_remote(&source_path, "origin", bare_path.to_str().unwrap())
            .unwrap();

        let first = commit_file(&source_path, "README.md", "one", "initial", &[]).unwrap();
        backend
            .push(&source_path, "origin", "refs/heads/main:refs/heads/main")
            .unwrap();
        backend
            .clone_repo(bare_path.to_str().unwrap(), &clone_path)
            .unwrap();
        let cloned_head = backend.head(&clone_path).unwrap();
        assert_eq!(cloned_head.branch, Some("main".to_owned()));
        assert!(!cloned_head.is_detached);
        assert_eq!(cloned_head.commit, Some(first.clone()));
        assert_eq!(
            backend.read_ref(&clone_path, "HEAD").unwrap(),
            Some(first.clone())
        );

        let parent = git2::Repository::open(&source_path)
            .unwrap()
            .find_commit(git2::Oid::from_str(&first).unwrap())
            .unwrap()
            .id();
        let second =
            commit_file(&source_path, "dev-docs/new.md", "two", "second", &[parent]).unwrap();
        backend
            .push(&source_path, "origin", "refs/heads/main:refs/heads/main")
            .unwrap();

        backend.fetch(&clone_path, "origin").unwrap();
        backend
            .fast_forward(&clone_path, "main", "refs/remotes/origin/main")
            .unwrap();
        assert_eq!(backend.head(&clone_path).unwrap().commit, Some(second));
        assert_eq!(
            fs::read_to_string(clone_path.join("dev-docs/new.md")).unwrap(),
            "two"
        );
        assert!(!backend.status(&clone_path).unwrap().is_dirty);

        backend.checkout_commit(&clone_path, &first).unwrap();
        let head = backend.head(&clone_path).unwrap();
        assert!(head.is_detached);
        assert_eq!(head.commit, Some(first));
    }

    #[test]
    fn fast_forward_matches_porcelain_merge_ff_only_and_self_verifies() {
        // main@A behind feature@B (A is an ancestor of B): a clean fast-forward.
        let temp = TempDir::new("ff-parity");
        let backend = Git2Backend::new();
        let base = temp.path().join("base");
        backend.create_repo(&base).unwrap();
        let a = commit_file(&base, "f.txt", "a\n", "A", &[]).unwrap();
        let a_oid = git2::Oid::from_str(&a).unwrap();
        run_git(&base, &["branch", "feature"]);
        run_git(&base, &["checkout", "feature"]);
        let b = commit_file(&base, "f.txt", "b\n", "B", &[a_oid]).unwrap();
        run_git(&base, &["checkout", "main"]);

        let prim = temp.path().join("prim");
        let porc = temp.path().join("porc");
        copy_repo(&base, &prim);
        copy_repo(&base, &porc);

        let result = backend
            .fast_forward(&prim, "main", "refs/heads/feature")
            .unwrap();
        assert!(result.updated);
        assert_eq!(result.commit.as_deref(), Some(b.as_str()));

        run_git(&porc, &["merge", "--ff-only", "feature"]);

        // Byte-identical end state vs porcelain: same HEAD, same tree, clean worktree.
        assert_eq!(rev_parse(&prim, "HEAD"), rev_parse(&porc, "HEAD"));
        assert_eq!(rev_parse(&prim, "HEAD"), b);
        assert_eq!(
            rev_parse(&prim, "HEAD^{tree}"),
            rev_parse(&porc, "HEAD^{tree}")
        );
        assert!(status_porcelain(&prim).trim().is_empty());
        assert_eq!(fs::read_to_string(prim.join("f.txt")).unwrap(), "b\n");
    }

    #[test]
    fn fast_forward_rejects_divergent_history_without_moving_branch() {
        // main@D and feature@C both descend from A — not fast-forwardable.
        let temp = TempDir::new("ff-diverge");
        let backend = Git2Backend::new();
        let base = temp.path().join("base");
        backend.create_repo(&base).unwrap();
        let a = commit_file(&base, "f.txt", "a\n", "A", &[]).unwrap();
        let a_oid = git2::Oid::from_str(&a).unwrap();
        run_git(&base, &["branch", "feature"]);
        run_git(&base, &["checkout", "feature"]);
        commit_file(&base, "f.txt", "c\n", "C", &[a_oid]).unwrap();
        run_git(&base, &["checkout", "main"]);
        let d = commit_file(&base, "f.txt", "d\n", "D", &[a_oid]).unwrap();

        let err = backend
            .fast_forward(&base, "main", "refs/heads/feature")
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::DivergedMember);
        // Porcelain agrees it is not fast-forwardable.
        assert!(!run_git_ok(&base, &["merge", "--ff-only", "feature"]));
        // Failed = nothing changed: main is still at D.
        assert_eq!(rev_parse(&base, "HEAD"), d);
    }

    #[test]
    fn checkout_commit_matches_porcelain_and_self_verifies() {
        // Detach onto an older commit A while B is current.
        let temp = TempDir::new("checkout-parity");
        let backend = Git2Backend::new();
        let base = temp.path().join("base");
        backend.create_repo(&base).unwrap();
        let a = commit_file(&base, "f.txt", "a\n", "A", &[]).unwrap();
        let a_oid = git2::Oid::from_str(&a).unwrap();
        commit_file(&base, "f.txt", "b\n", "B", &[a_oid]).unwrap();

        let prim = temp.path().join("prim");
        let porc = temp.path().join("porc");
        copy_repo(&base, &prim);
        copy_repo(&base, &porc);

        let result = backend.checkout_commit(&prim, &a).unwrap();
        assert_eq!(result.commit.as_deref(), Some(a.as_str()));

        run_git(&porc, &["checkout", &a]);

        assert_eq!(rev_parse(&prim, "HEAD"), rev_parse(&porc, "HEAD"));
        assert_eq!(rev_parse(&prim, "HEAD"), a);
        assert_eq!(
            rev_parse(&prim, "HEAD^{tree}"),
            rev_parse(&porc, "HEAD^{tree}")
        );
        assert!(status_porcelain(&prim).trim().is_empty());
        assert_eq!(fs::read_to_string(prim.join("f.txt")).unwrap(), "a\n");
        assert!(backend.head(&prim).unwrap().is_detached);
    }

    #[test]
    fn checkout_commit_rejects_unknown_commit_without_moving_head() {
        let temp = TempDir::new("checkout-missing");
        let backend = Git2Backend::new();
        let base = temp.path().join("base");
        backend.create_repo(&base).unwrap();
        commit_file(&base, "f.txt", "a\n", "A", &[]).unwrap();
        let before = rev_parse(&base, "HEAD");

        let bogus = "0".repeat(40);
        let err = backend.checkout_commit(&base, &bogus).unwrap_err();
        assert_eq!(err.code, ErrorCode::GitCommandFailed);
        assert_eq!(rev_parse(&base, "HEAD"), before);
    }

    #[test]
    fn verify_checkout_state_accepts_match_and_rejects_mismatch() {
        // Direct test of the AD1 self-verify: HEAD is at B.
        let temp = TempDir::new("verify-state");
        let backend = Git2Backend::new();
        let repo = temp.path().join("repo");
        backend.create_repo(&repo).unwrap();
        let a = commit_file(&repo, "f.txt", "a\n", "A", &[]).unwrap();
        let a_oid = git2::Oid::from_str(&a).unwrap();
        let b = commit_file(&repo, "f.txt", "b\n", "B", &[a_oid]).unwrap();
        let b_oid = git2::Oid::from_str(&b).unwrap();

        assert!(verify_checkout_state(&repo, b_oid).is_ok());
        let err = verify_checkout_state(&repo, a_oid).unwrap_err();
        assert_eq!(err.code, ErrorCode::GitCommandFailed);
    }

    #[test]
    fn reports_commit_ancestry_without_moving_head() {
        let temp = TempDir::new("ancestry");
        let backend = Git2Backend::new();
        let repo_path = temp.path().join("repo");
        backend.create_repo(&repo_path).unwrap();
        let first = commit_file(&repo_path, "README.md", "one", "initial", &[]).unwrap();
        let first_oid = git2::Oid::from_str(&first).unwrap();
        let second = commit_file(&repo_path, "README.md", "two", "second", &[first_oid]).unwrap();

        assert!(backend.is_ancestor(&repo_path, &first, &second).unwrap());
        assert!(!backend.is_ancestor(&repo_path, &second, &first).unwrap());
        assert_eq!(backend.head(&repo_path).unwrap().commit, Some(second));
    }

    fn commit_file(
        repo_path: &Path,
        relative_path: &str,
        content: &str,
        message: &str,
        parents: &[git2::Oid],
    ) -> Result<String, git2::Error> {
        if let Some(parent) = Path::new(relative_path).parent() {
            fs::create_dir_all(repo_path.join(parent)).unwrap();
        }
        fs::write(repo_path.join(relative_path), content).unwrap();
        stage_path(repo_path, relative_path)?;

        let repo = git2::Repository::open(repo_path)?;
        let tree_id = repo.index()?.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        let signature = git2::Signature::now("GWZ Test", "gwz@example.invalid")?;
        let parent_commits = parents
            .iter()
            .map(|id| repo.find_commit(*id))
            .collect::<Result<Vec<_>, _>>()?;
        let parent_refs = parent_commits.iter().collect::<Vec<_>>();
        let oid = repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parent_refs,
        )?;
        Ok(oid.to_string())
    }

    fn stage_path(repo_path: &Path, relative_path: &str) -> Result<(), git2::Error> {
        let repo = git2::Repository::open(repo_path)?;
        let mut index = repo.index()?;
        index.add_path(Path::new(relative_path))?;
        index.write()
    }

    fn init_bare_main(path: &Path) {
        let repo = git2::Repository::init_bare(path).unwrap();
        repo.set_head("refs/heads/main").unwrap();
    }

    fn copy_repo(src: &Path, dst: &Path) {
        let status = std::process::Command::new("cp")
            .arg("-R")
            .arg(src)
            .arg(dst)
            .status()
            .expect("spawn cp");
        assert!(status.success(), "cp -R failed");
    }

    fn rev_parse(repo: &Path, rev: &str) -> String {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["rev-parse", rev])
            .output()
            .expect("spawn git rev-parse");
        assert!(output.status.success(), "git rev-parse {rev} failed");
        String::from_utf8(output.stdout)
            .expect("rev-parse utf8")
            .trim()
            .to_owned()
    }

    fn status_porcelain(repo: &Path) -> String {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["status", "--porcelain"])
            .output()
            .expect("spawn git status");
        assert!(output.status.success(), "git status failed");
        String::from_utf8(output.stdout).expect("status utf8")
    }

    fn run_git_ok(root: &Path, args: &[&str]) -> bool {
        std::process::Command::new("git")
            .args(["-c", "user.name=GWZ", "-c", "user.email=gwz@example.invalid"])
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .expect("spawn git")
            .success()
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(prefix: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "gwz-core-git-{prefix}-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

use super::repository_support::{open_repo, pin_creation_time_filter_neutralization};
use super::transport_support::{
    fetch_options_with_progress, remote_callbacks, remote_fetch_options, remote_push_options, identity,
};
use super::*;

pub(super) fn clone_repo(
    backend: &Git2Backend,
    url: &str,
    path: &Path,
) -> ModelResult<GitCloneResult> {
    backend.clone_repo_with_progress(url, path, &|_progress| {})
}

pub(super) fn clone_repo_with_progress(
    backend: &Git2Backend,
    url: &str,
    path: &Path,
    progress: &dyn Fn(crate::GitTransferProgress),
) -> ModelResult<GitCloneResult> {
    ensure_clone_target_is_empty(path)?;
    let mut builder = git2::build::RepoBuilder::new();
    // Creation-time filter neutralization (Decision 1 Option B), clone edge:
    // this is the single production clone funnel, and `RepoBuilder::clone`
    // materializes the initial worktree itself — so the initial checkout runs
    // with content filters DISABLED (blob bytes verbatim; the strategy stays
    // the clone default, SAFE) and the repo-local pins land immediately
    // after, before anything else can materialize files. Invariant: no file
    // is ever written through a smudge filter into a gwz-created repository.
    let mut checkout = git2::build::CheckoutBuilder::new();
    checkout.disable_filters(true);
    builder.with_checkout(checkout);
    builder.fetch_options(fetch_options_with_progress(
        backend.credential_helpers,
        identity::for_remote(backend, None, Some("origin"), url)?,
        Some(progress),
    ));
    let repo = builder.clone(url, path).map_err(git_error)?;
    pin_creation_time_filter_neutralization(&repo)?;
    Ok(GitCloneResult {
        path: path.to_path_buf(),
        head: backend.head(path)?,
    })
}

pub(super) fn fetch(
    backend: &Git2Backend,
    path: &Path,
    remote: &str,
) -> ModelResult<GitFetchResult> {
    let repo = open_repo(path)?;
    let mut remote_handle = find_remote(&repo, remote)?;
    let identity = identity::for_remote(backend, Some(&repo), Some(remote), remote_handle.url().map_err(git_error)?)?;
    let refspecs: [&str; 0] = [];
    remote_handle
        .fetch(
            &refspecs,
            Some(&mut remote_fetch_options(backend.credential_helpers, identity)),
            Some("gwz fetch"),
        )
        .map_err(git_error)?;
    Ok(GitFetchResult {
        remote: remote.to_owned(),
    })
}

pub(super) fn tag_fetch(
    backend: &Git2Backend,
    path: &Path,
    remote: &str,
) -> ModelResult<GitFetchResult> {
    let repo = open_repo(path)?;
    let mut remote_handle = find_remote(&repo, remote)?;
    // Fetch every tag, force-updating local copies.
    let identity = identity::for_remote(backend, Some(&repo), Some(remote), remote_handle.url().map_err(git_error)?)?;
    let refspec = "+refs/tags/*:refs/tags/*";
    remote_handle
        .fetch(
            &[refspec],
            Some(&mut remote_fetch_options(backend.credential_helpers, identity)),
            Some("gwz tag fetch"),
        )
        .map_err(git_error)?;
    Ok(GitFetchResult {
        remote: remote.to_owned(),
    })
}

pub(super) fn ls_remote(
    backend: &Git2Backend,
    path: &Path,
    remote: &str,
) -> ModelResult<Vec<GitRemoteRef>> {
    let repo = open_repo(path)?;
    let mut remote_handle = find_remote(&repo, remote)?;
    let identity = identity::for_remote(backend, Some(&repo), Some(remote), remote_handle.url().map_err(git_error)?)?;
    advertised_refs(backend, &mut remote_handle, identity)
}

pub(super) fn ls_remote_url(
    backend: &Git2Backend,
    path: &Path,
    url: &str,
    remote_name: &str,
    identity_repo: Option<&Path>,
) -> ModelResult<Vec<GitRemoteRef>> {
    let repo = open_repo(path)?;
    let mut remote = repo.remote_anonymous(url).map_err(git_error)?;
    let identity_repo = identity_repo.map(open_repo).transpose()?;
    let identity = identity::for_remote(backend, identity_repo.as_ref(), Some(remote_name), url)?;
    advertised_refs(backend, &mut remote, identity)
}

fn advertised_refs(
    backend: &Git2Backend,
    remote_handle: &mut git2::Remote<'_>,
    identity: Option<identity::SelectedIdentity>,
) -> ModelResult<Vec<GitRemoteRef>> {
    let connection = remote_handle
        .connect_auth(
            git2::Direction::Fetch,
            Some(remote_callbacks(backend.credential_helpers, identity)),
            None,
        )
        .map_err(git_error)?;
    let refs = connection
        .list()
        .map_err(git_error)?
        .iter()
        .map(|head| GitRemoteRef {
            name: head.name().to_owned(),
            target: head.oid().to_string(),
        })
        .collect::<Vec<_>>();
    // `connection` disconnects on drop.
    Ok(refs)
}

pub(super) fn remotes(_backend: &Git2Backend, path: &Path) -> ModelResult<Vec<GitRemote>> {
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

pub(super) fn add_remote(
    _backend: &Git2Backend,
    path: &Path,
    name: &str,
    url: &str,
) -> ModelResult<GitRemoteResult> {
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

/// Admit `url` as an anonymous local peer -- an existing directory reachable
/// as a path, never a transport URL -- and return the string libgit2 is
/// handed for it: the canonical `file://` URL of that directory.
///
/// LCM1.0c-rem1 (Code P2-1). libgit2 selects a transport by URL prefix first
/// (`file://` is its local transport) and falls back to heuristics for a bare
/// string; on non-Windows hosts the heuristic treats ANY `:` in the string
/// as scp-style SSH syntax before it tests for a directory
/// (`libgit2/src/libgit2/transport.c`, `transport_find_fn`), and this crate
/// builds libgit2 with the SSH transport. A bare path such as
/// `/Users/x/gwz:lane/A` -- ordinary on macOS -- therefore left the local
/// transport and failed as a host lookup. The `file://` form makes the local
/// transport the only one that can run for every admitted directory;
/// libgit2 percent-decodes it back to the path (`util/fs_path.c`,
/// `git_fs_path_fromurl`). Callers keep passing paths: a `file://` INPUT is
/// refused with every other URL because the port's contract is a path, and
/// results and error messages name the path the caller gave.
fn admitted_local_peer(url: &str) -> ModelResult<String> {
    let refuse = |detail: &str| {
        ModelError::new(
            ErrorCode::InvalidRequest,
            format!(
                "anonymous local transport accepts an existing local repository path only: {detail} ({url})"
            ),
        )
    };
    if url.contains("://") {
        return Err(refuse("URL schemes are not local paths"));
    }
    if url.starts_with("git@") || url.starts_with("ssh@") {
        return Err(refuse("scp-like remote syntax is not a local path"));
    }
    let path = Path::new(url);
    if !path.is_dir() {
        return Err(refuse("not an existing directory"));
    }
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| refuse(&format!("cannot canonicalise the directory: {error}")))?;
    let file_url = url::Url::from_file_path(&canonical)
        .map_err(|()| refuse("the canonical directory path is not absolute"))?;
    Ok(file_url.as_str().to_owned())
}

pub(super) fn fetch_anonymous(
    _backend: &Git2Backend,
    path: &Path,
    url: &str,
    refspecs: &[&str],
) -> ModelResult<GitFetchResult> {
    let peer = admitted_local_peer(url)?;
    if refspecs.is_empty() {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            "anonymous local fetch requires explicit refspecs",
        ));
    }
    let repo = open_repo(path)?;
    let mut remote_handle = repo.remote_anonymous(&peer).map_err(git_error)?;
    // No `RemoteCallbacks` at all: no credentials, no progress, no network.
    let mut options = git2::FetchOptions::new();
    // No fetch record is written. libgit2 still TRUNCATES the receiver's
    // `FETCH_HEAD` to empty on every fetch (`remote.c`,
    // `git_remote_update_tips` -> `truncate_fetch_head`, creating the file
    // when absent); measured by `local_clone::tests::transport`, stated in
    // the port contract, outside its promise.
    options.update_fetchhead(false);
    options.download_tags(git2::AutotagOption::None);
    remote_handle
        .fetch(refspecs, Some(&mut options), Some("gwz local import"))
        .map_err(git_error)?;
    Ok(GitFetchResult {
        remote: url.to_owned(),
    })
}

pub(super) fn push_anonymous(
    _backend: &Git2Backend,
    path: &Path,
    url: &str,
    refspec: &str,
) -> ModelResult<GitPushResult> {
    let peer = admitted_local_peer(url)?;
    if refspec.trim().is_empty() {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            "anonymous local push requires an explicit refspec",
        ));
    }
    let repo = open_repo(path)?;
    let mut remote_handle = repo.remote_anonymous(&peer).map_err(git_error)?;
    // libgit2 reports a per-ref rejection through this callback and still
    // returns success from `push`; collect it so a rejected update is an
    // error, not a silent no-op.
    let rejected = std::cell::RefCell::new(Vec::<(String, String)>::new());
    let mut callbacks = git2::RemoteCallbacks::new();
    callbacks.push_update_reference(|refname, status| {
        if let Some(message) = status {
            rejected
                .borrow_mut()
                .push((refname.to_owned(), message.to_owned()));
        }
        Ok(())
    });
    let mut options = git2::PushOptions::new();
    options.remote_callbacks(callbacks);
    remote_handle
        .push(&[refspec], Some(&mut options))
        .map_err(|error| {
            // libgit2 checks a non-fast-forward update itself before the
            // transfer ("cannot push because a reference that you are trying
            // to update on the remote contains commits that are not present
            // locally"); that is a rejected ref update, not a Git failure.
            if error.code() == git2::ErrorCode::NotFastForward {
                ModelError::new(
                    ErrorCode::RemoteRejected,
                    format!("{url} rejected {refspec}: {}", error.message()),
                )
            } else {
                git_error(error)
            }
        })?;
    let first_rejection = rejected.borrow().first().cloned();
    if let Some((refname, message)) = first_rejection {
        return Err(ModelError::new(
            ErrorCode::RemoteRejected,
            format!("{url} rejected {refname}: {message}"),
        ));
    }
    Ok(GitPushResult {
        remote: url.to_owned(),
        refspec: refspec.to_owned(),
    })
}

pub(super) fn push(
    backend: &Git2Backend,
    path: &Path,
    remote: &str,
    refspec: &str,
) -> ModelResult<GitPushResult> {
    let repo = open_repo(path)?;
    let mut remote_handle = find_remote(&repo, remote)?;
    let url = remote_handle.pushurl().map_err(git_error)?.unwrap_or(remote_handle.url().map_err(git_error)?);
    let identity = identity::for_remote(backend, Some(&repo), Some(remote), url)?;
    let rejected = std::cell::RefCell::new(Vec::new());
    remote_handle
        .push(
            &[refspec],
            Some(&mut remote_push_options(
                backend.credential_helpers,
                identity,
                &rejected,
            )),
        )
        .map_err(|error| {
            if error.code() == git2::ErrorCode::NotFastForward {
                ModelError::new(ErrorCode::RemoteRejected, error.message())
            } else {
                git_error(error)
            }
        })?;
    if let Some((refname, message)) = rejected.borrow().first() {
        return Err(ModelError::new(
            ErrorCode::RemoteRejected,
            format!("{remote} rejected {refname}: {message}"),
        ));
    }
    Ok(GitPushResult {
        remote: remote.to_owned(),
        refspec: refspec.to_owned(),
    })
}

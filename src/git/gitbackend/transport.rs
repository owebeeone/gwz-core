use super::repository_support::open_repo;
use super::transport_support::{
    fetch_options_with_progress, remote_callbacks, remote_fetch_options, remote_push_options,
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
    builder.fetch_options(fetch_options_with_progress(
        backend.credential_helpers,
        Some(progress),
    ));
    builder.clone(url, path).map_err(git_error)?;
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
    let refspecs: [&str; 0] = [];
    remote_handle
        .fetch(
            &refspecs,
            Some(&mut remote_fetch_options(backend.credential_helpers)),
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
    let refspec = "+refs/tags/*:refs/tags/*";
    remote_handle
        .fetch(
            &[refspec],
            Some(&mut remote_fetch_options(backend.credential_helpers)),
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
    let connection = remote_handle
        .connect_auth(
            git2::Direction::Fetch,
            Some(remote_callbacks(backend.credential_helpers)),
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

pub(super) fn push(
    backend: &Git2Backend,
    path: &Path,
    remote: &str,
    refspec: &str,
) -> ModelResult<GitPushResult> {
    let repo = open_repo(path)?;
    let mut remote_handle = find_remote(&repo, remote)?;
    remote_handle
        .push(
            &[refspec],
            Some(&mut remote_push_options(backend.credential_helpers)),
        )
        .map_err(git_error)?;
    Ok(GitPushResult {
        remote: remote.to_owned(),
        refspec: refspec.to_owned(),
    })
}

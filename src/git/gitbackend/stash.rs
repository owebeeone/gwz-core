use super::merge_support::merge_signature;
use super::repository_support::open_repo;
use super::stash_support::{
    resolve_stash_index, stash_entries, stash_push_flags, stash_restore_error,
    stash_restore_options,
};
use super::*;

pub(super) fn stash_push(
    _backend: &Git2Backend,
    path: &Path,
    message: &str,
    options: GitStashPushOptions,
) -> ModelResult<GitStashPushResult> {
    let mut repo = open_repo(path)?;
    let signature = merge_signature(&repo)?;
    let object_id = repo
        .stash_save(&signature, message, Some(stash_push_flags(options)))
        .map_err(git_error)?;
    Ok(GitStashPushResult {
        object_id: object_id.to_string(),
        message: message.to_owned(),
    })
}

pub(super) fn stash_list(_backend: &Git2Backend, path: &Path) -> ModelResult<Vec<GitStashEntry>> {
    let mut repo = open_repo(path)?;
    stash_entries(&mut repo)
}

pub(super) fn stash_apply(
    _backend: &Git2Backend,
    path: &Path,
    target: &GitStashTarget,
    options: GitStashRestoreOptions,
) -> ModelResult<()> {
    let mut repo = open_repo(path)?;
    let index = resolve_stash_index(&mut repo, target)?;
    let mut apply_options = stash_restore_options(options);
    // libgit2 applies through its merge/checkout machinery and can return
    // Conflict without writing porcelain-style conflict markers. Until GWZ
    // has stash-specific protocol errors, callers should treat GitCommandFailed
    // from this path as "native stash remains pending; inspect before retry".
    repo.stash_apply(index, Some(&mut apply_options))
        .map_err(stash_restore_error)
}

pub(super) fn stash_pop(
    _backend: &Git2Backend,
    path: &Path,
    target: &GitStashTarget,
    options: GitStashRestoreOptions,
) -> ModelResult<()> {
    let mut repo = open_repo(path)?;
    let index = resolve_stash_index(&mut repo, target)?;
    let mut apply_options = stash_restore_options(options);
    // Same conflict caveat as stash_apply: git2 does not guarantee porcelain
    // conflict-marker behavior. git_stash_pop drops only after a successful apply.
    repo.stash_pop(index, Some(&mut apply_options))
        .map_err(stash_restore_error)
}

pub(super) fn stash_drop(
    _backend: &Git2Backend,
    path: &Path,
    target: &GitStashTarget,
) -> ModelResult<()> {
    let mut repo = open_repo(path)?;
    let index = resolve_stash_index(&mut repo, target)?;
    repo.stash_drop(index).map_err(git_error)
}

//! The refs and reads of `Git2Backend`: head, status, tag names, ref and
//! revision reads, commit existence, committed file content, the commit-graph
//! reads and the configured remotes. Step 5.3 (row G2) moved their
//! implementations to gwz-git; this file is core's thin delegation. It opens the
//! repository exactly as before (no parent search, no environment), parses the
//! caller's full object ids in the repository's format, projects gwz-git's
//! owned records onto core's model types and translates a gwz-git error with
//! `model_error`.

use gwz_git::{ObjectId, Repository, StatusOptions};

use super::*;
use crate::git::model_error;

fn open(path: &Path) -> ModelResult<Repository> {
    Repository::open_exact(path).map_err(model_error)
}

/// A full object id as the caller wrote it, in the repository's object format.
fn object_id(repo: &Repository, text: &str) -> ModelResult<ObjectId> {
    ObjectId::parse_hex(repo.object_format(), text).map_err(model_error)
}

/// `HEAD` of the repository at `path`. Also what [`repo_head`] answers for a
/// native handle that rows still in core (branch, switch, merge) hold.
pub(crate) fn head_at(path: &Path) -> ModelResult<GitHeadState> {
    let head = open(path)?.head().map_err(model_error)?;
    Ok(GitHeadState {
        branch: head.branch,
        commit: head.commit.map(|id| id.to_string()),
        is_detached: head.is_detached,
    })
}

pub(super) fn head(_backend: &Git2Backend, path: &Path) -> ModelResult<GitHeadState> {
    head_at(path)
}

pub(super) fn status(backend: &Git2Backend, path: &Path) -> ModelResult<GitStatus> {
    backend.status_with_options(path, GitStatusOptions::default())
}

pub(super) fn status_with_options(
    _backend: &Git2Backend,
    path: &Path,
    options: GitStatusOptions,
) -> ModelResult<GitStatus> {
    let status = open(path)?
        .status(&StatusOptions::new().with_ignored(options.include_ignored))
        .map_err(model_error)?;
    Ok(GitStatus {
        is_dirty: status.is_dirty,
        staged: status.staged,
        unstaged: status.unstaged,
        untracked: status.untracked,
        ignored: status.ignored,
        unresolved: status.unresolved,
        files: status
            .files
            .into_iter()
            .map(|file| GitFileStatus {
                path: file.path,
                index_status: file.index_status.to_string(),
                worktree_status: file.worktree_status.to_string(),
                original_path: file.original_path,
            })
            .collect(),
    })
}

pub(super) fn tag_list(_backend: &Git2Backend, path: &Path) -> ModelResult<Vec<String>> {
    open(path)?.tag_names().map_err(model_error)
}

pub(super) fn read_ref(
    _backend: &Git2Backend,
    path: &Path,
    ref_spec: &str,
) -> ModelResult<Option<String>> {
    open(path)?
        .resolve(ref_spec)
        .map(|id| id.map(|id| id.to_string()))
        .map_err(model_error)
}

pub(super) fn commit_exists(_backend: &Git2Backend, path: &Path, oid: &str) -> ModelResult<bool> {
    let repo = open(path)?;
    let Ok(id) = ObjectId::parse_hex(repo.object_format(), oid) else {
        return Ok(false);
    };
    repo.commit_exists(id).map_err(model_error)
}

pub(super) fn read_file_at_commit(
    _backend: &Git2Backend,
    path: &Path,
    commit: &str,
    relative_path: &str,
) -> ModelResult<Option<Vec<u8>>> {
    let repo = open(path)?;
    let commit = object_id(&repo, commit)?;
    repo.blob_at(commit, relative_path).map_err(model_error)
}

pub(super) fn is_ancestor(
    _backend: &Git2Backend,
    path: &Path,
    ancestor: &str,
    descendant: &str,
) -> ModelResult<bool> {
    let repo = open(path)?;
    let ancestor = object_id(&repo, ancestor)?;
    let descendant = object_id(&repo, descendant)?;
    repo.is_ancestor(ancestor, descendant).map_err(model_error)
}

pub(super) fn ahead_behind(
    _backend: &Git2Backend,
    path: &Path,
    local: &str,
    upstream: &str,
) -> ModelResult<crate::git::GitAheadBehind> {
    let repo = open(path)?;
    let local = object_id(&repo, local)?;
    let upstream = object_id(&repo, upstream)?;
    let counts = repo.ahead_behind(local, upstream).map_err(model_error)?;
    Ok(crate::git::GitAheadBehind {
        ahead: counts.ahead,
        behind: counts.behind,
    })
}

pub(super) fn merge_base(
    _backend: &Git2Backend,
    path: &Path,
    left: &str,
    right: &str,
) -> ModelResult<Option<String>> {
    let repo = open(path)?;
    let left = object_id(&repo, left)?;
    let right = object_id(&repo, right)?;
    repo.merge_base(left, right)
        .map(|base| base.map(|base| base.to_string()))
        .map_err(model_error)
}

pub(super) fn changed_paths_between(
    _backend: &Git2Backend,
    path: &Path,
    old_commit: &str,
    new_commit: &str,
) -> ModelResult<Vec<String>> {
    let repo = open(path)?;
    let old = object_id(&repo, old_commit)?;
    let new = object_id(&repo, new_commit)?;
    repo.changed_paths(old, new).map_err(model_error)
}

pub(super) fn remotes(_backend: &Git2Backend, path: &Path) -> ModelResult<Vec<GitRemote>> {
    Ok(open(path)?
        .remotes()
        .map_err(model_error)?
        .into_iter()
        .map(|remote| GitRemote {
            name: remote.name,
            url: Some(remote.url),
            push_url: remote.push_url,
        })
        .collect())
}

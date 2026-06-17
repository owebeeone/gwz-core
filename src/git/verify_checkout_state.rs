use std::fs;
use std::path::Path;

use crate::model::{ErrorCode, ModelError, ModelResult};


use super::*;

pub(crate) fn find_remote<'repo>(
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

pub(crate) fn repo_head(repo: &git2::Repository) -> ModelResult<GitHeadState> {
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

pub(crate) fn unborn_head(repo: &git2::Repository) -> ModelResult<GitHeadState> {
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

pub(crate) fn ensure_clone_target_is_empty(path: &Path) -> ModelResult<()> {
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

pub(crate) fn staged_statuses() -> git2::Status {
    git2::Status::INDEX_NEW
        | git2::Status::INDEX_MODIFIED
        | git2::Status::INDEX_DELETED
        | git2::Status::INDEX_RENAMED
        | git2::Status::INDEX_TYPECHANGE
}

pub(crate) fn unstaged_statuses() -> git2::Status {
    git2::Status::WT_MODIFIED
        | git2::Status::WT_DELETED
        | git2::Status::WT_RENAMED
        | git2::Status::WT_TYPECHANGE
}

/// AD1 self-verify for ref/worktree-advancing primitives: re-open the repo and
/// confirm HEAD now resolves to `expected` and the tracked worktree matches it
/// (no staged or unstaged drift). Catches the F0 class of bug — a ref advanced
/// without the worktree following, or vice versa — before the primitive reports
/// success. Untracked files are ignored.
pub(crate) fn verify_checkout_state(path: &Path, expected: git2::Oid) -> ModelResult<()> {
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

pub(crate) fn git_error(error: git2::Error) -> ModelError {
    ModelError::new(ErrorCode::GitCommandFailed, error.message())
}

pub(crate) fn io_error(error: std::io::Error) -> ModelError {
    ModelError::new(ErrorCode::IoError, error.to_string())
}


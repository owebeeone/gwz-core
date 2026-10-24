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

/// `HEAD` as a handle held by a row that is still in core sees it: gwz-git reads
/// it (step 5.3, row G2), from the handle's Git directory, so core keeps no
/// reading of its own.
pub(crate) fn repo_head(repo: &git2::Repository) -> ModelResult<GitHeadState> {
    head_at(repo.path())
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
    let code = if credential_helper_timeout(&error) {
        ErrorCode::CredentialHelperTimeout
    } else if super::gitbackend::credential_helper_unavailable(&error) {
        ErrorCode::ExternalToolMissing
    } else {
        ErrorCode::GitCommandFailed
    };
    ModelError::new(code, error.message())
}

/// The translation of an error gwz-git returns, beside [`git_error`] for the
/// native ones core still holds. gwz-git keeps the diagnostic (kind, native
/// code and class, message); the code here is core's choice: `InvalidRequest`
/// for a path argument gwz-git refused (`InvalidPath`, the committed-file
/// read), and the one every other local Git failure has. The credential-helper
/// codes `git_error` recognises come from HTTPS authentication, which no
/// gwz-git call reaches.
pub(crate) fn model_error(error: gwz_git::Error) -> ModelError {
    let code = match error.kind() {
        gwz_git::ErrorKind::InvalidPath => ErrorCode::InvalidRequest,
        _ => ErrorCode::GitCommandFailed,
    };
    ModelError::new(code, error.message())
}

pub(crate) fn io_error(error: std::io::Error) -> ModelError {
    ModelError::new(ErrorCode::IoError, error.to_string())
}

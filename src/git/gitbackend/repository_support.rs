use super::*;

pub(crate) fn open_repo(path: &Path) -> ModelResult<git2::Repository> {
    git2::Repository::open(path).map_err(git_error)
}

pub(crate) fn branch_ref_name(branch: &str) -> String {
    format!("refs/heads/{branch}")
}

pub(crate) fn resolve_commit_oid(
    repo: &git2::Repository,
    ref_spec: &str,
) -> ModelResult<git2::Oid> {
    repo.revparse_single(ref_spec)
        .and_then(|object| object.peel_to_commit())
        .map(|commit| commit.id())
        .map_err(git_error)
}

pub(crate) fn ensure_no_integration_in_progress(repo: &git2::Repository) -> ModelResult<()> {
    let state = repo.state();
    if state != git2::RepositoryState::Clean {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            format!("repository has an integration operation in progress: {state:?}"),
        ));
    }
    Ok(())
}

pub(super) fn parse_existing_commit(
    repo: &git2::Repository,
    value: &str,
) -> ModelResult<git2::Oid> {
    let width = match repo.object_format() {
        git2::ObjectFormat::Sha1 => 40,
        git2::ObjectFormat::Sha256 => 64,
    };
    if value.len() != width
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "commit id is not a complete lowercase id in the repository object format",
        ));
    }
    let oid = git2::Oid::from_str_ext(value, repo.object_format()).map_err(git_error)?;
    repo.find_commit(oid).map_err(git_error)?;
    Ok(oid)
}

pub(crate) fn verify_merge_result(
    backend: &impl GitBackend,
    path: &Path,
    branch: &str,
    expected_commit: &str,
) -> ModelResult<()> {
    let observed = backend.head(path)?;
    let status = backend.status(path)?;
    if observed.branch.as_deref() != Some(branch)
        || observed.commit.as_deref() != Some(expected_commit)
        || status.is_dirty
        || open_repo(path)?.state() != git2::RepositoryState::Clean
    {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "post-merge branch, HEAD, worktree, or native state did not match",
        ));
    }
    Ok(())
}

/// Create `branch` at `oid` when missing. If it exists, require it already points
/// at `oid`; refusing to move an existing branch preserves the checkout_branch
/// orphan-safety behavior used by materialize branch restore.
pub(crate) fn ensure_branch_at_commit(
    repo: &git2::Repository,
    branch: &str,
    oid: git2::Oid,
) -> ModelResult<bool> {
    let ref_name = branch_ref_name(branch);
    match repo.find_reference(&ref_name) {
        Ok(existing) => {
            let existing_oid = existing.peel_to_commit().map_err(git_error)?.id();
            if existing_oid != oid {
                return Err(ModelError::new(
                    ErrorCode::DivergedMember,
                    format!(
                        "branch '{branch}' is at {existing_oid}, not the target {oid}; refusing to move it"
                    ),
                ));
            }
            Ok(false)
        }
        Err(err) if err.code() == git2::ErrorCode::NotFound => {
            let target = repo.find_commit(oid).map_err(git_error)?;
            repo.branch(branch, &target, false).map_err(git_error)?;
            Ok(true)
        }
        Err(err) => Err(git_error(err)),
    }
}

pub(crate) fn git_branch_record(
    branch: &git2::Branch<'_>,
    current: Option<&str>,
) -> ModelResult<GitBranch> {
    let name = branch
        .name()
        .map_err(git_error)?
        .ok_or_else(|| ModelError::new(ErrorCode::GitCommandFailed, "branch name is not UTF-8"))?
        .to_owned();
    let commit = branch
        .get()
        .peel_to_commit()
        .map_err(git_error)?
        .id()
        .to_string();
    Ok(GitBranch {
        is_current: current == Some(name.as_str()),
        name,
        commit,
    })
}

pub(crate) fn git_file_status(entry: &git2::StatusEntry<'_>) -> Option<GitFileStatus> {
    let status = entry.status();
    // git2 reports a rename entry under the OLD path; model it the way `git status` does —
    // current path = the new path, `original_path` = where it came from.
    let (path, original_path) = match rename_delta(entry, status) {
        Some((old, new)) => (new, Some(old)),
        None => (entry.path().ok()?.to_owned(), None),
    };
    Some(GitFileStatus {
        path,
        index_status: index_status_char(status).to_owned(),
        worktree_status: worktree_status_char(status).to_owned(),
        original_path,
    })
}

/// `(old_path, new_path)` when `entry` is a rename (staged or unstaged), else `None`.
pub(crate) fn rename_delta(
    entry: &git2::StatusEntry<'_>,
    status: git2::Status,
) -> Option<(String, String)> {
    if !status.intersects(git2::Status::INDEX_RENAMED | git2::Status::WT_RENAMED) {
        return None;
    }
    let delta = entry.head_to_index().or_else(|| entry.index_to_workdir())?;
    let old = delta.old_file().path()?.to_str()?.to_owned();
    let new = delta.new_file().path()?.to_str()?.to_owned();
    Some((old, new))
}

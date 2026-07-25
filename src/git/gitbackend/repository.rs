use super::repository_support::{
    branch_ref_name, ensure_branch_at_commit, git_file_status, open_repo,
};
use super::*;

pub(super) fn is_repository(_backend: &Git2Backend, path: &Path) -> ModelResult<bool> {
    match git2::Repository::open(path) {
        Ok(_) => Ok(true),
        Err(err) if err.code() == git2::ErrorCode::NotFound => Ok(false),
        Err(err) => Err(git_error(err)),
    }
}

pub(super) fn commit_exists(_backend: &Git2Backend, path: &Path, oid: &str) -> ModelResult<bool> {
    let Ok(oid) = git2::Oid::from_str(oid) else {
        return Ok(false);
    };
    let repo = open_repo(path)?;
    let object = match repo.find_object(oid, None) {
        Ok(object) => object,
        Err(error) if error.code() == git2::ErrorCode::NotFound => return Ok(false),
        Err(error) => return Err(git_error(error)),
    };
    Ok(object.peel_to_commit().is_ok())
}

pub(super) fn read_file_at_commit(
    _backend: &Git2Backend,
    path: &Path,
    commit: &str,
    relative_path: &str,
) -> ModelResult<Option<Vec<u8>>> {
    let relative = Path::new(relative_path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            format!("committed file path must be normalized and relative: '{relative_path}'"),
        ));
    }
    let repo = open_repo(path)?;
    let oid = git2::Oid::from_str(commit).map_err(git_error)?;
    let commit = repo.find_commit(oid).map_err(git_error)?;
    let tree = commit.tree().map_err(git_error)?;
    let entry = match tree.get_path(relative) {
        Ok(entry) => entry,
        Err(error) if error.code() == git2::ErrorCode::NotFound => return Ok(None),
        Err(error) => return Err(git_error(error)),
    };
    let object = entry.to_object(&repo).map_err(git_error)?;
    let blob = object.as_blob().ok_or_else(|| {
        ModelError::new(
            ErrorCode::GitCommandFailed,
            format!("committed path '{relative_path}' is not a file"),
        )
    })?;
    Ok(Some(blob.content().to_vec()))
}

pub(super) fn create_repo(_backend: &Git2Backend, path: &Path) -> ModelResult<GitCreateResult> {
    let mut opts = git2::RepositoryInitOptions::new();
    opts.bare(false).no_reinit(true).initial_head("main");
    git2::Repository::init_opts(path, &opts).map_err(git_error)?;
    Ok(GitCreateResult {
        path: path.to_path_buf(),
    })
}

pub(super) fn reset_hard(
    backend: &Git2Backend,
    path: &Path,
    branch: &str,
    upstream_ref: &str,
) -> ModelResult<GitUpdateResult> {
    let repo = open_repo(path)?;
    let target = repo.revparse_single(upstream_ref).map_err(git_error)?.id();
    let target_object = repo.find_object(target, None).map_err(git_error)?;
    repo.reset(&target_object, git2::ResetType::Hard, None)
        .map_err(git_error)?;
    verify_checkout_state(path, target)?;
    // AD1 self-verify: the branch (not a detached HEAD) now points at upstream.
    let observed = backend.head(path)?;
    if observed.is_detached || observed.branch.as_deref() != Some(branch) {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            format!("post-reset HEAD is not on branch '{branch}'"),
        ));
    }
    Ok(GitUpdateResult {
        updated: true,
        commit: Some(target.to_string()),
    })
}

pub(super) fn checkout_commit(
    _backend: &Git2Backend,
    path: &Path,
    commit: &str,
) -> ModelResult<GitUpdateResult> {
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

pub(super) fn checkout_branch(
    backend: &Git2Backend,
    path: &Path,
    branch: &str,
    commit: &str,
) -> ModelResult<GitUpdateResult> {
    let repo = open_repo(path)?;
    let oid = git2::Oid::from_str(commit).map_err(git_error)?;
    let ref_name = branch_ref_name(branch);
    // AD3(c) orphan-safety: shared with branch_create. Create if missing; refuse
    // if it already exists at a different commit (that would orphan work).
    ensure_branch_at_commit(&repo, branch, oid)?;
    let object = repo.find_object(oid, None).map_err(git_error)?;
    let mut checkout = git2::build::CheckoutBuilder::new();
    checkout.safe();
    repo.checkout_tree(&object, Some(&mut checkout))
        .map_err(git_error)?;
    repo.set_head(&ref_name).map_err(git_error)?;
    verify_checkout_state(path, oid)?;
    // AD1 self-verify: HEAD is attached to the branch, not detached.
    let observed = backend.head(path)?;
    if observed.is_detached || observed.branch.as_deref() != Some(branch) {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            format!("post-checkout HEAD is not on branch '{branch}'"),
        ));
    }
    Ok(GitUpdateResult {
        updated: true,
        commit: Some(oid.to_string()),
    })
}

pub(super) fn status(backend: &Git2Backend, path: &Path) -> ModelResult<GitStatus> {
    backend.status_with_options(path, GitStatusOptions::default())
}

pub(super) fn status_with_options(
    _backend: &Git2Backend,
    path: &Path,
    options: GitStatusOptions,
) -> ModelResult<GitStatus> {
    let repo = open_repo(path)?;
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .include_ignored(options.include_ignored)
        .recurse_untracked_dirs(true)
        // F17: detect renames so a `git mv` reports `R` + original_path (the status
        // model already carries `original_path`) instead of an unrelated delete+add.
        .renames_head_to_index(true)
        .renames_index_to_workdir(true);
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
        if status.contains(git2::Status::IGNORED) {
            out.ignored += 1;
        }
        if status.contains(git2::Status::CONFLICTED) {
            out.unresolved += 1;
        }
        if let Some(file) = git_file_status(&entry) {
            out.files.push(file);
        }
    }
    out.is_dirty = out.staged > 0 || out.unstaged > 0 || out.untracked > 0 || out.unresolved > 0;
    Ok(out)
}

pub(super) fn head(_backend: &Git2Backend, path: &Path) -> ModelResult<GitHeadState> {
    let repo = open_repo(path)?;
    repo_head(&repo)
}

pub(super) fn stage_paths(
    _backend: &Git2Backend,
    path: &Path,
    pathspecs: &[&str],
) -> ModelResult<GitStageResult> {
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

pub(super) fn stage_paths_allowing_other_conflicts(
    _backend: &Git2Backend,
    path: &Path,
    pathspecs: &[&str],
) -> ModelResult<GitStageResult> {
    let repo = open_repo(path)?;
    let mut index = repo.index().map_err(git_error)?;
    let mut staged = 0usize;
    for spec in pathspecs {
        let relative = Path::new(spec);
        match index.conflict_remove(relative) {
            Ok(()) => {}
            Err(err) if err.code() == git2::ErrorCode::NotFound => {}
            Err(err) => return Err(git_error(err)),
        }
        if path.join(relative).exists() {
            index.add_path(relative).map_err(git_error)?;
            staged += 1;
        } else {
            match index.remove_path(relative) {
                Ok(()) => staged += 1,
                Err(err) if err.code() == git2::ErrorCode::NotFound => {}
                Err(err) => return Err(git_error(err)),
            }
        }
    }
    index.write().map_err(git_error)?;

    let verify = open_repo(path)?.index().map_err(git_error)?;
    for spec in pathspecs {
        match verify.conflict_get(Path::new(spec)) {
            Ok(_) => {
                return Err(ModelError::new(
                    ErrorCode::GitCommandFailed,
                    format!("staged path still has conflicts after write: {spec}"),
                ));
            }
            Err(err) if err.code() == git2::ErrorCode::NotFound => {}
            Err(err) => return Err(git_error(err)),
        }
    }
    Ok(GitStageResult { staged })
}

pub(super) fn commit(
    backend: &Git2Backend,
    path: &Path,
    message: &str,
    all: bool,
) -> ModelResult<GitCommitResult> {
    // AD1 CLI fallback: run porcelain `git commit` so hooks / signing / committer
    // config are honored (libgit2's commit honors none of them).
    let before = backend.head(path)?.commit;
    let mut command = std::process::Command::new("git");
    command.arg("-C").arg(path).arg("commit");
    if all {
        command.arg("-a");
    }
    command.arg("-m").arg(message);
    let output = command.output().map_err(|err| {
        ModelError::new(
            ErrorCode::GitCommandFailed,
            format!("failed to run git commit: {err}"),
        )
    })?;
    if !output.status.success() {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            format!(
                "git commit failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    // AD1 self-verify: HEAD advanced to a new commit (read fresh).
    let after = backend.head(path)?.commit.ok_or_else(|| {
        ModelError::new(ErrorCode::GitCommandFailed, "HEAD is unborn after commit")
    })?;
    if Some(&after) == before.as_ref() {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "git commit did not advance HEAD",
        ));
    }
    Ok(GitCommitResult { commit: after })
}

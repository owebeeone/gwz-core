use super::repository_support::{
    branch_ref_name, ensure_branch_at_commit, git_branch_record, open_repo, resolve_commit_oid,
};
use super::*;

pub(super) fn fast_forward(
    _backend: &Git2Backend,
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

pub(super) fn branch_list(_backend: &Git2Backend, path: &Path) -> ModelResult<Vec<GitBranch>> {
    let repo = open_repo(path)?;
    let current = repo_head(&repo)?.branch;
    let mut branches = Vec::new();
    for entry in repo
        .branches(Some(git2::BranchType::Local))
        .map_err(git_error)?
    {
        let (branch, _) = entry.map_err(git_error)?;
        branches.push(git_branch_record(&branch, current.as_deref())?);
    }
    branches.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(branches)
}

pub(super) fn branch_create(
    _backend: &Git2Backend,
    path: &Path,
    branch: &str,
    start_ref: &str,
) -> ModelResult<GitBranchCreateResult> {
    let repo = open_repo(path)?;
    let oid = resolve_commit_oid(&repo, start_ref)?;
    let created = ensure_branch_at_commit(&repo, branch, oid)?;
    let current = repo_head(&repo)?.branch;
    Ok(GitBranchCreateResult {
        branch: GitBranch {
            name: branch.to_owned(),
            commit: oid.to_string(),
            is_current: current.as_deref() == Some(branch),
        },
        created,
    })
}

pub(super) fn branch_delete(_backend: &Git2Backend, path: &Path, branch: &str) -> ModelResult<()> {
    let repo = open_repo(path)?;
    let current = repo_head(&repo)?.branch;
    if current.as_deref() == Some(branch) {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            format!("cannot delete current branch '{branch}'"),
        ));
    }
    repo.find_branch(branch, git2::BranchType::Local)
        .map_err(git_error)?
        .delete()
        .map_err(git_error)?;
    match repo.find_branch(branch, git2::BranchType::Local) {
        Ok(_) => Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            format!("branch '{branch}' still present after delete"),
        )),
        Err(err) if err.code() == git2::ErrorCode::NotFound => Ok(()),
        Err(err) => Err(git_error(err)),
    }
}

pub(super) fn switch_branch(
    backend: &Git2Backend,
    path: &Path,
    branch: &str,
) -> ModelResult<GitUpdateResult> {
    let repo = open_repo(path)?;
    let local_branch = repo
        .find_branch(branch, git2::BranchType::Local)
        .map_err(git_error)?;
    let oid = local_branch.get().peel_to_commit().map_err(git_error)?.id();
    let object = repo.find_object(oid, None).map_err(git_error)?;
    let mut checkout = git2::build::CheckoutBuilder::new();
    checkout.safe();
    repo.checkout_tree(&object, Some(&mut checkout))
        .map_err(git_error)?;
    let ref_name = branch_ref_name(branch);
    repo.set_head(&ref_name).map_err(git_error)?;
    verify_checkout_state(path, oid)?;
    let observed = backend.head(path)?;
    if observed.is_detached || observed.branch.as_deref() != Some(branch) {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            format!("post-switch HEAD is not on branch '{branch}'"),
        ));
    }
    Ok(GitUpdateResult {
        updated: true,
        commit: Some(oid.to_string()),
    })
}

pub(super) fn tag_create(
    backend: &Git2Backend,
    path: &Path,
    name: &str,
    message: Option<&str>,
    signed: bool,
) -> ModelResult<GitTagResult> {
    // AD1 CLI fallback: `git tag` so hooks / signing / tagger config are honored.
    let mut command = std::process::Command::new("git");
    command.arg("-C").arg(path).arg("tag");
    if signed {
        command.arg("-s");
    } else if message.is_some() {
        command.arg("-a");
    }
    if let Some(message) = message {
        command.arg("-m").arg(message);
    }
    command.arg(name);
    let output = command.output().map_err(|err| {
        ModelError::new(
            ErrorCode::GitCommandFailed,
            format!("failed to run git tag: {err}"),
        )
    })?;
    if !output.status.success() {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            format!(
                "git tag failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    // AD1 self-verify: the tag exists (read fresh) and resolves to a commit.
    if !backend.tag_list(path)?.iter().any(|tag| tag == name) {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            format!("tag '{name}' missing after creation"),
        ));
    }
    let commit = backend
        .read_ref(path, &format!("refs/tags/{name}^{{commit}}"))?
        .ok_or_else(|| {
            ModelError::new(
                ErrorCode::GitCommandFailed,
                format!("tag '{name}' did not resolve"),
            )
        })?;
    Ok(GitTagResult {
        name: name.to_owned(),
        commit,
    })
}

pub(super) fn tag_list(_backend: &Git2Backend, path: &Path) -> ModelResult<Vec<String>> {
    let repo = open_repo(path)?;
    let names = repo.tag_names(None).map_err(git_error)?;
    let mut tags = Vec::new();
    for entry in names.iter() {
        if let Some(name) = entry.map_err(git_error)? {
            tags.push(name.to_owned());
        }
    }
    tags.sort();
    Ok(tags)
}

pub(super) fn tag_delete(backend: &Git2Backend, path: &Path, name: &str) -> ModelResult<()> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .arg("tag")
        .arg("-d")
        .arg(name)
        .output()
        .map_err(|err| {
            ModelError::new(
                ErrorCode::GitCommandFailed,
                format!("failed to run git tag -d: {err}"),
            )
        })?;
    if !output.status.success() {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            format!(
                "git tag -d failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    // AD1 self-verify: the tag is gone.
    if backend.tag_list(path)?.iter().any(|tag| tag == name) {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            format!("tag '{name}' still present after delete"),
        ));
    }
    Ok(())
}

pub(super) fn read_ref(
    _backend: &Git2Backend,
    path: &Path,
    ref_spec: &str,
) -> ModelResult<Option<String>> {
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

pub(super) fn is_ancestor(
    _backend: &Git2Backend,
    path: &Path,
    ancestor: &str,
    descendant: &str,
) -> ModelResult<bool> {
    let repo = open_repo(path)?;
    let ancestor = git2::Oid::from_str(ancestor).map_err(git_error)?;
    let descendant = git2::Oid::from_str(descendant).map_err(git_error)?;
    repo.graph_descendant_of(descendant, ancestor)
        .map_err(git_error)
}

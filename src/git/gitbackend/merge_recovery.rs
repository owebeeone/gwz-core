use super::merge_support::{
    conflict_paths, merge_signature, merge_signatures, prepared_signature, same_signature,
};
use super::recovery_support::{
    attached_head_ref, ensure_clean_recovery_state, recovery_drift,
    validate_abort_index_and_worktree, validate_expected_native_merge,
    validate_expected_resolution_repository_state, validate_prepared_merge_resolution_in_repo,
    validate_resolution_index_and_worktree, verify_restored_merge_state,
};
use super::repository_support::{
    branch_ref_name, open_repo, parse_existing_commit, resolve_commit_oid, verify_merge_result,
};
use super::scoped_support::{scoped_attached_head_ref, validate_scoped_expected_head};
use super::*;

pub(super) fn merge_state(
    _backend: &Git2Backend,
    path: &Path,
) -> ModelResult<Option<GitNativeMergeState>> {
    let repo = open_repo(path)?;
    if repo.state() != git2::RepositoryState::Merge {
        return Ok(None);
    }
    let merge_head = std::fs::read_to_string(repo.path().join("MERGE_HEAD")).map_err(|err| {
        ModelError::new(
            ErrorCode::GitCommandFailed,
            format!("failed to read MERGE_HEAD: {err}"),
        )
    })?;
    let merge_oid = resolve_commit_oid(&repo, merge_head.trim())?;
    let index = repo.index().map_err(git_error)?;
    let conflict_paths = conflict_paths(&index)?;
    Ok(Some(GitNativeMergeState {
        merge_head: merge_oid.to_string(),
        unresolved_entries: conflict_paths.len(),
        conflict_paths,
    }))
}

pub(super) fn repository_state(
    _backend: &Git2Backend,
    path: &Path,
) -> ModelResult<GitRepositoryState> {
    let repo = open_repo(path)?;
    Ok(map_repository_state(repo.state()))
}

pub(super) fn validate_merge_recovery_state(
    backend: &Git2Backend,
    path: &Path,
    expected_before: &str,
    expected_merge_head: &str,
    require_resolved: bool,
) -> ModelResult<()> {
    let repo = open_repo(path)?;
    let before = parse_existing_commit(&repo, expected_before)?;
    let merge_head = parse_existing_commit(&repo, expected_merge_head)?;
    validate_expected_native_merge(&repo, before, merge_head)?;
    if require_resolved {
        validate_resolution_index_and_worktree(backend, path, &repo, before, merge_head)
    } else {
        validate_abort_index_and_worktree(backend, path, &repo, before, merge_head)
    }
}

pub(super) fn abort_merge(
    backend: &Git2Backend,
    path: &Path,
    expected_before: &str,
    expected_merge_head: &str,
) -> ModelResult<()> {
    let repo = open_repo(path)?;
    let before = parse_existing_commit(&repo, expected_before)?;
    let merge_head = parse_existing_commit(&repo, expected_merge_head)?;

    if repo.state() == git2::RepositoryState::Clean {
        verify_restored_merge_state(backend, path, before)?;
        return Ok(());
    }
    let ref_name = attached_head_ref(&repo)?;
    let mut transaction = repo.transaction().map_err(git_error)?;
    transaction.lock_ref(&ref_name).map_err(git_error)?;
    validate_expected_native_merge(&repo, before, merge_head)?;
    validate_abort_index_and_worktree(backend, path, &repo, before, merge_head)?;

    let target = repo.find_commit(before).map_err(git_error)?;
    let mut checkout = git2::build::CheckoutBuilder::new();
    checkout
        .force()
        .remove_untracked(false)
        .remove_ignored(false);
    repo.checkout_tree(target.as_object(), Some(&mut checkout))
        .map_err(git_error)?;
    let target_tree = target.tree().map_err(git_error)?;
    let mut index = repo.index().map_err(git_error)?;
    index.read_tree(&target_tree).map_err(git_error)?;
    index.write().map_err(git_error)?;
    repo.cleanup_state().map_err(git_error)?;
    drop(transaction);
    verify_restored_merge_state(backend, path, before)
}

pub(super) fn set_branch_target_checked(
    backend: &Git2Backend,
    path: &Path,
    branch: &str,
    expected_current: &str,
    target: &str,
) -> ModelResult<GitUpdateResult> {
    let repo = open_repo(path)?;
    let expected = parse_existing_commit(&repo, expected_current)?;
    let target = parse_existing_commit(&repo, target)?;
    let ref_name = branch_ref_name(branch);
    let mut transaction = repo.transaction().map_err(git_error)?;
    transaction.lock_ref(&ref_name).map_err(git_error)?;

    ensure_clean_recovery_state(backend, path, &repo, branch)?;
    let current = repo
        .find_reference(&ref_name)
        .and_then(|reference| reference.peel_to_commit())
        .map_err(git_error)?
        .id();
    if current == target {
        drop(transaction);
        verify_merge_result(backend, path, branch, &target.to_string())?;
        return Ok(GitUpdateResult {
            updated: false,
            commit: Some(target.to_string()),
        });
    }
    if current != expected {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            format!(
                "branch '{branch}' changed before rollback; expected {expected}, observed {current}"
            ),
        ));
    }

    let target_object = repo.find_object(target, None).map_err(git_error)?;
    let mut checkout = git2::build::CheckoutBuilder::new();
    checkout.safe();
    repo.checkout_tree(&target_object, Some(&mut checkout))
        .map_err(git_error)?;
    transaction
        .set_target(&ref_name, target, None, "gwz checked merge rollback")
        .map_err(git_error)?;
    transaction.commit().map_err(git_error)?;
    verify_merge_result(backend, path, branch, &target.to_string())?;
    Ok(GitUpdateResult {
        updated: true,
        commit: Some(target.to_string()),
    })
}

pub(super) fn delete_branch_target_checked(
    backend: &Git2Backend,
    path: &Path,
    branch: &str,
    expected_current: &str,
) -> ModelResult<()> {
    let repo = open_repo(path)?;
    let expected = parse_existing_commit(&repo, expected_current)?;
    let ref_name = branch_ref_name(branch);
    if scoped_attached_head_ref(&repo)? != ref_name {
        return Err(recovery_drift(
            "checked branch deletion requires the attached target branch",
        ));
    }
    let mut transaction = repo.transaction().map_err(git_error)?;
    transaction.lock_ref("HEAD").map_err(git_error)?;
    transaction.lock_ref(&ref_name).map_err(git_error)?;
    validate_scoped_expected_head(&repo, &ref_name, Some(expected))?;
    transaction.remove(&ref_name).map_err(git_error)?;
    transaction.commit().map_err(git_error)?;
    let head = backend.head(path)?;
    if head.branch.as_deref() != Some(branch) || head.commit.is_some() || head.is_detached {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "checked branch deletion did not leave the expected unborn branch",
        ));
    }
    Ok(())
}

pub(super) fn commit_merge_resolution(
    backend: &Git2Backend,
    path: &Path,
    message: &str,
) -> ModelResult<GitCommitResult> {
    let repo = open_repo(path)?;
    let mut index = repo.index().map_err(git_error)?;
    if index.has_conflicts() {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "cannot commit merge resolution while index has conflicts",
        ));
    }
    let merge_head = std::fs::read_to_string(repo.path().join("MERGE_HEAD")).map_err(|err| {
        ModelError::new(
            ErrorCode::GitCommandFailed,
            format!("failed to read MERGE_HEAD: {err}"),
        )
    })?;
    let merge_oids = merge_head
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| git2::Oid::from_str(line.trim()).map_err(git_error))
        .collect::<ModelResult<Vec<_>>>()?;
    if merge_oids.is_empty() {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "MERGE_HEAD is empty",
        ));
    }

    let head_commit = repo
        .head()
        .map_err(git_error)?
        .peel_to_commit()
        .map_err(git_error)?;
    let merge_commits = merge_oids
        .iter()
        .map(|oid| repo.find_commit(*oid).map_err(git_error))
        .collect::<ModelResult<Vec<_>>>()?;
    let tree_oid = index.write_tree().map_err(git_error)?;
    let tree = repo.find_tree(tree_oid).map_err(git_error)?;
    let signature = merge_signature(&repo)?;
    let mut parents = Vec::with_capacity(1 + merge_commits.len());
    parents.push(&head_commit);
    parents.extend(merge_commits.iter());
    let oid = repo
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &parents,
        )
        .map_err(git_error)?;
    repo.cleanup_state().map_err(git_error)?;
    let observed = backend.head(path)?;
    if observed.commit.as_deref() != Some(oid.to_string().as_str()) {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "post-merge-resolution HEAD is not the merge commit",
        ));
    }
    Ok(GitCommitResult {
        commit: oid.to_string(),
    })
}

pub(super) fn commit_merge_resolution_checked(
    backend: &Git2Backend,
    path: &Path,
    target_branch: &str,
    expected_before: &str,
    expected_merge_head: &str,
    message: &str,
    attribution: Option<&crate::model::OperationAttribution>,
) -> ModelResult<GitCommitResult> {
    let prepared = backend.prepare_merge_resolution_checked(
        path,
        target_branch,
        expected_before,
        expected_merge_head,
        attribution,
    )?;
    backend.commit_prepared_merge_resolution_checked(
        path,
        target_branch,
        expected_before,
        expected_merge_head,
        message,
        &prepared,
    )
}

pub(super) fn prepare_merge_resolution_checked(
    backend: &Git2Backend,
    path: &Path,
    target_branch: &str,
    expected_before: &str,
    expected_merge_head: &str,
    attribution: Option<&crate::model::OperationAttribution>,
) -> ModelResult<GitPreparedCommit> {
    record_preparation_call();
    let repo = open_repo(path)?;
    validate_expected_resolution_repository_state(
        backend,
        path,
        &repo,
        target_branch,
        expected_before,
        expected_merge_head,
    )?;
    let mut index = repo.index().map_err(git_error)?;
    let tree_oid = index.write_tree().map_err(git_error)?;
    let (author, committer) = merge_signatures(&repo, attribution)?;
    Ok(GitPreparedCommit {
        tree_oid: tree_oid.to_string(),
        author: prepared_signature(&author)?,
        committer: prepared_signature(&committer)?,
    })
}

pub(super) fn validate_prepared_merge_resolution_state(
    backend: &Git2Backend,
    path: &Path,
    target_branch: &str,
    expected_before: &str,
    expected_merge_head: &str,
    prepared: &GitPreparedCommit,
) -> ModelResult<()> {
    let repo = open_repo(path)?;
    validate_prepared_merge_resolution_in_repo(
        backend,
        path,
        &repo,
        target_branch,
        expected_before,
        expected_merge_head,
        prepared,
    )
    .map(|_| ())
}

pub(super) fn commit_prepared_merge_resolution_checked(
    backend: &Git2Backend,
    path: &Path,
    target_branch: &str,
    expected_before: &str,
    expected_merge_head: &str,
    message: &str,
    prepared: &GitPreparedCommit,
) -> ModelResult<GitCommitResult> {
    run_before_prepared_execution();
    let repo = open_repo(path)?;
    let ref_name = branch_ref_name(target_branch);
    let mut transaction = repo.transaction().map_err(git_error)?;
    transaction.lock_ref(&ref_name).map_err(git_error)?;
    let validated = validate_prepared_merge_resolution_in_repo(
        backend,
        path,
        &repo,
        target_branch,
        expected_before,
        expected_merge_head,
        prepared,
    )?;
    let before_commit = repo.find_commit(validated.before).map_err(git_error)?;
    let merge_commit = repo.find_commit(validated.merge_head).map_err(git_error)?;
    let oid = repo
        .commit(
            None,
            &validated.author,
            &validated.committer,
            message,
            &validated.tree,
            &[&before_commit, &merge_commit],
        )
        .map_err(git_error)?;
    let committed = repo.find_commit(oid).map_err(git_error)?;
    if committed.parent_count() != 2
        || committed.parent_id(0).map_err(git_error)? != validated.before
        || committed.parent_id(1).map_err(git_error)? != validated.merge_head
        || committed.message_bytes() != message.as_bytes()
        || committed.tree_id() != validated.tree.id()
        || !same_signature(&committed.author(), &validated.author)
        || !same_signature(&committed.committer(), &validated.committer)
    {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "merge resolution commit does not match its checked specification",
        ));
    }
    transaction
        .set_target(
            &ref_name,
            oid,
            Some(&validated.committer),
            "gwz merge resolution",
        )
        .map_err(git_error)?;
    transaction.commit().map_err(git_error)?;
    repo.cleanup_state().map_err(git_error)?;
    let branch = ref_name.trim_start_matches("refs/heads/");
    verify_merge_result(backend, path, branch, &oid.to_string())?;
    Ok(GitCommitResult {
        commit: oid.to_string(),
    })
}

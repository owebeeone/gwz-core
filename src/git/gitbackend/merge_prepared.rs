use super::merge_support::{
    classify_merge, conflict_paths, in_memory_merge_index, merge_signature, merge_signatures,
    prepared_signature, same_signature, signature_from_prepared, signature_matches_prepared,
    validate_checked_merge_head, validate_prepared_merge_upstream_in_repo,
};
use super::repository_support::{
    branch_ref_name, ensure_no_integration_in_progress, open_repo, resolve_commit_oid,
    verify_merge_result,
};
use super::*;

pub(super) fn commit_matches_merge(
    _backend: &Git2Backend,
    path: &Path,
    commit: &str,
    first_parent: &str,
    second_parent: &str,
    message: &str,
) -> ModelResult<bool> {
    let (Ok(commit), Ok(first_parent), Ok(second_parent)) = (
        git2::Oid::from_str(commit),
        git2::Oid::from_str(first_parent),
        git2::Oid::from_str(second_parent),
    ) else {
        return Ok(false);
    };
    let repo = open_repo(path)?;
    let commit = match repo.find_commit(commit) {
        Ok(commit) => commit,
        Err(error) if error.code() == git2::ErrorCode::NotFound => return Ok(false),
        Err(error) => return Err(git_error(error)),
    };
    Ok(commit.parent_count() == 2
        && commit.parent_id(0).map_err(git_error)? == first_parent
        && commit.parent_id(1).map_err(git_error)? == second_parent
        && commit.message_bytes() == message.as_bytes())
}

pub(super) fn commit_matches_prepared_merge(
    _backend: &Git2Backend,
    path: &Path,
    commit: &str,
    first_parent: &str,
    second_parent: &str,
    message: &str,
    prepared: &GitPreparedCommit,
) -> ModelResult<bool> {
    let (Ok(commit), Ok(first_parent), Ok(second_parent), Ok(tree_oid)) = (
        git2::Oid::from_str(commit),
        git2::Oid::from_str(first_parent),
        git2::Oid::from_str(second_parent),
        git2::Oid::from_str(&prepared.tree_oid),
    ) else {
        return Ok(false);
    };
    let repo = open_repo(path)?;
    let commit = match repo.find_commit(commit) {
        Ok(commit) => commit,
        Err(error) if error.code() == git2::ErrorCode::NotFound => return Ok(false),
        Err(error) => return Err(git_error(error)),
    };
    Ok(commit.parent_count() == 2
        && commit.parent_id(0).map_err(git_error)? == first_parent
        && commit.parent_id(1).map_err(git_error)? == second_parent
        && commit.message_bytes() == message.as_bytes()
        && commit.tree_id() == tree_oid
        && signature_matches_prepared(&commit.author(), &prepared.author)
        && signature_matches_prepared(&commit.committer(), &prepared.committer))
}

pub(super) fn merge_upstream(
    backend: &Git2Backend,
    path: &Path,
    branch: &str,
    upstream_ref: &str,
) -> ModelResult<GitIntegrateResult> {
    let status = backend.status(path)?;
    if status.is_dirty {
        return Err(ModelError::new(
            ErrorCode::DirtyMember,
            "merge requires a clean index and worktree",
        ));
    }
    let plan = backend.merge_analysis(path, branch, upstream_ref)?;
    backend.merge_upstream_checked(
        path,
        branch,
        &plan.target_commit,
        &plan.source_commit,
        &format!("Merge {upstream_ref} into {branch}"),
        None,
    )
}

pub(super) fn merge_upstream_checked(
    backend: &Git2Backend,
    path: &Path,
    branch: &str,
    expected_before: &str,
    source_commit: &str,
    message: &str,
    attribution: Option<&crate::model::OperationAttribution>,
) -> ModelResult<GitIntegrateResult> {
    let prepared = backend.prepare_merge_upstream_checked(
        path,
        branch,
        expected_before,
        source_commit,
        attribution,
    )?;
    backend.execute_prepared_merge_upstream_checked(
        path,
        branch,
        expected_before,
        source_commit,
        message,
        &prepared,
    )
}

pub(super) fn prepare_merge_upstream_checked(
    backend: &Git2Backend,
    path: &Path,
    branch: &str,
    expected_before: &str,
    source_commit: &str,
    attribution: Option<&crate::model::OperationAttribution>,
) -> ModelResult<GitPreparedMerge> {
    record_preparation_call();
    let expected = git2::Oid::from_str(expected_before).map_err(git_error)?;
    let source = git2::Oid::from_str(source_commit).map_err(git_error)?;
    let repo = open_repo(path)?;
    ensure_no_integration_in_progress(&repo)?;
    let status = backend.status(path)?;
    if status.is_dirty {
        return Err(ModelError::new(
            ErrorCode::DirtyMember,
            "merge requires a clean index and worktree",
        ));
    }
    validate_checked_merge_head(&repo, branch, expected)?;
    repo.find_commit(source).map_err(git_error)?;
    match classify_merge(&repo, expected, source)? {
        GitMergeAnalysisKind::UpToDate => Ok(GitPreparedMerge::Unchanged),
        GitMergeAnalysisKind::FastForward => Ok(GitPreparedMerge::FastForward),
        GitMergeAnalysisKind::TrueMerge => {
            let mut index = in_memory_merge_index(&repo, expected, source)?;
            if index.has_conflicts() {
                return Ok(GitPreparedMerge::ExpectedConflict);
            }
            let tree_oid = index.write_tree_to(&repo).map_err(git_error)?;
            let (author, committer) = merge_signatures(&repo, attribution)?;
            Ok(GitPreparedMerge::Commit(GitPreparedCommit {
                tree_oid: tree_oid.to_string(),
                author: prepared_signature(&author)?,
                committer: prepared_signature(&committer)?,
            }))
        }
    }
}

pub(super) fn validate_prepared_merge_upstream_state(
    backend: &Git2Backend,
    path: &Path,
    branch: &str,
    expected_before: &str,
    source_commit: &str,
    prepared: &GitPreparedMerge,
) -> ModelResult<()> {
    let expected = git2::Oid::from_str(expected_before).map_err(git_error)?;
    let source = git2::Oid::from_str(source_commit).map_err(git_error)?;
    let repo = open_repo(path)?;
    validate_prepared_merge_upstream_in_repo(
        backend, path, &repo, branch, expected, source, prepared,
    )
    .map(|_| ())
}

pub(super) fn execute_prepared_merge_upstream_checked(
    backend: &Git2Backend,
    path: &Path,
    branch: &str,
    expected_before: &str,
    source_commit: &str,
    message: &str,
    prepared: &GitPreparedMerge,
) -> ModelResult<GitIntegrateResult> {
    run_before_prepared_execution();
    let expected = git2::Oid::from_str(expected_before).map_err(git_error)?;
    let source = git2::Oid::from_str(source_commit).map_err(git_error)?;
    let expected_text = expected.to_string();
    let source_text = source.to_string();
    if message.contains('\0') {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            "merge commit message contains a NUL byte",
        ));
    }

    let repo = open_repo(path)?;
    let local_ref_name = branch_ref_name(branch);
    let mut transaction = repo.transaction().map_err(git_error)?;
    transaction.lock_ref(&local_ref_name).map_err(git_error)?;

    let kind = validate_prepared_merge_upstream_in_repo(
        backend, path, &repo, branch, expected, source, prepared,
    )?;
    let source_object = repo.find_commit(source).map_err(git_error)?;

    if kind == GitMergeAnalysisKind::UpToDate {
        drop(source_object);
        drop(transaction);
        verify_merge_result(backend, path, branch, &expected_text)?;
        return Ok(GitIntegrateResult::clean(expected_text));
    }
    if kind == GitMergeAnalysisKind::FastForward {
        let target_object = repo.find_object(source, None).map_err(git_error)?;
        let mut checkout = git2::build::CheckoutBuilder::new();
        checkout.safe();
        repo.checkout_tree(&target_object, Some(&mut checkout))
            .map_err(git_error)?;
        transaction
            .set_target(&local_ref_name, source, None, "gwz fast-forward")
            .map_err(git_error)?;
        transaction.commit().map_err(git_error)?;
        verify_merge_result(backend, path, branch, &source_text)?;
        return Ok(GitIntegrateResult::clean(source_text));
    }

    if prepared == &GitPreparedMerge::ExpectedConflict {
        let annotated = repo.find_annotated_commit(source).map_err(git_error)?;
        // Enter native merge state only after the in-memory result has been
        // proven to match the durable expected-conflict intent.
        repo.merge(&[&annotated], None, None).map_err(git_error)?;
        let index = repo.index().map_err(git_error)?;
        // Faithful to porcelain: leave the conflict in the worktree and record
        // MERGE_HEAD so the developer can resolve and `git merge --continue`.
        std::fs::write(repo.path().join("MERGE_HEAD"), format!("{source}\n"))
            .map_err(|err| ModelError::new(ErrorCode::GitCommandFailed, err.to_string()))?;
        let conflicts = conflict_paths(&index)?;
        // AD1 self-verify: the conflict state actually persisted on disk.
        let state = backend.merge_state(path)?;
        let conflict_head = backend.head(path)?;
        if conflicts.is_empty()
            || conflict_head.commit.as_deref() != Some(expected_text.as_str())
            || state.as_ref().is_none_or(|state| {
                state.merge_head != source.to_string() || state.conflict_paths != conflicts
            })
        {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                "merge conflict state did not persist with the expected MERGE_HEAD",
            ));
        }
        return Ok(GitIntegrateResult {
            commit: None,
            conflicts,
        });
    }

    let GitPreparedMerge::Commit(prepared_commit) = prepared else {
        unreachable!("validated clean true merge requires a prepared commit");
    };
    let tree_oid = git2::Oid::from_str(&prepared_commit.tree_oid).map_err(git_error)?;
    let tree = repo.find_tree(tree_oid).map_err(git_error)?;
    let head_commit = repo.find_commit(expected).map_err(git_error)?;
    let author = signature_from_prepared(&prepared_commit.author)?;
    let committer = signature_from_prepared(&prepared_commit.committer)?;
    let merge_oid = repo
        .commit(
            None,
            &author,
            &committer,
            message,
            &tree,
            &[&head_commit, &source_object],
        )
        .map_err(git_error)?;
    let committed = repo.find_commit(merge_oid).map_err(git_error)?;
    if committed.parent_count() != 2
        || committed.parent_id(0).map_err(git_error)? != head_commit.id()
        || committed.parent_id(1).map_err(git_error)? != source
        || committed.message_bytes() != message.as_bytes()
        || committed.tree_id() != tree_oid
        || !same_signature(&committed.author(), &author)
        || !same_signature(&committed.committer(), &committer)
    {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "post-merge commit metadata does not match the checked merge plan",
        ));
    }
    repo.checkout_tree(tree.as_object(), None)
        .map_err(git_error)?;
    transaction
        .set_target(&local_ref_name, merge_oid, Some(&committer), "gwz merge")
        .map_err(git_error)?;
    transaction.commit().map_err(git_error)?;
    repo.cleanup_state().map_err(git_error)?;
    verify_merge_result(backend, path, branch, &merge_oid.to_string())?;
    Ok(GitIntegrateResult::clean(merge_oid.to_string()))
}

pub(super) fn merge_analysis(
    _backend: &Git2Backend,
    path: &Path,
    target_branch: &str,
    source: &str,
) -> ModelResult<GitMergeAnalysis> {
    let repo = open_repo(path)?;
    ensure_no_integration_in_progress(&repo)?;
    let target_commit = repo
        .find_reference(&branch_ref_name(target_branch))
        .and_then(|reference| reference.peel_to_commit())
        .map_err(git_error)?
        .id();
    let source_commit = resolve_commit_oid(&repo, source)?;
    let kind = classify_merge(&repo, target_commit, source_commit)?;
    Ok(GitMergeAnalysis {
        target_branch: target_branch.to_owned(),
        target_commit: target_commit.to_string(),
        source_commit: source_commit.to_string(),
        kind,
        commit_identity_required: kind == GitMergeAnalysisKind::TrueMerge,
        prediction_complete: kind != GitMergeAnalysisKind::TrueMerge,
    })
}

pub(super) fn rebase_onto(
    backend: &Git2Backend,
    path: &Path,
    branch: &str,
    upstream_ref: &str,
) -> ModelResult<GitIntegrateResult> {
    let repo = open_repo(path)?;
    let upstream_oid = repo.revparse_single(upstream_ref).map_err(git_error)?.id();
    let upstream_annotated = repo
        .find_annotated_commit(upstream_oid)
        .map_err(git_error)?;
    let (analysis, _) = repo
        .merge_analysis(&[&upstream_annotated])
        .map_err(git_error)?;
    if analysis.is_up_to_date() {
        return Ok(GitIntegrateResult::clean(upstream_oid.to_string()));
    }
    if analysis.is_fast_forward() {
        // Nothing to replay: `git rebase` of a strictly-behind branch fast-forwards.
        let ff = backend.fast_forward(path, branch, upstream_ref)?;
        return Ok(GitIntegrateResult {
            commit: ff.commit,
            conflicts: Vec::new(),
        });
    }

    let signature = merge_signature(&repo)?;
    let mut rebase = repo
        .rebase(None, Some(&upstream_annotated), None, None)
        .map_err(git_error)?;
    // Replay each commit; git2 patches it into the index + worktree on `next()`. The
    // operation handle is dropped within the loop condition so the body can re-borrow.
    while rebase.next().transpose().map_err(git_error)?.is_some() {
        let index = repo.index().map_err(git_error)?;
        if index.has_conflicts() {
            // Faithful to porcelain: leave the rebase in progress for the developer
            // to resolve and `git rebase --continue`. Dropping `rebase` frees the
            // in-memory handle but leaves `.git/rebase-merge/` on disk.
            return Ok(GitIntegrateResult {
                commit: None,
                conflicts: conflict_paths(&index)?,
            });
        }
        rebase.commit(None, &signature, None).map_err(git_error)?;
    }
    rebase.finish(Some(&signature)).map_err(git_error)?;

    // AD1 self-verify: HEAD reattached to the branch and now descends from upstream.
    let observed = backend.head(path)?;
    let Some(new_head) = observed.commit.clone() else {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "post-rebase HEAD is unborn",
        ));
    };
    if observed.is_detached || observed.branch.as_deref() != Some(branch) {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            format!("post-rebase HEAD is not on branch '{branch}'"),
        ));
    }
    if !backend.is_ancestor(path, &upstream_oid.to_string(), &new_head)? {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "post-rebase HEAD is not based on upstream",
        ));
    }
    Ok(GitIntegrateResult::clean(new_head))
}

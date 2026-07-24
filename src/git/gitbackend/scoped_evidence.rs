use super::recovery_support::recovery_drift;
use super::repository_support::{branch_ref_name, open_repo, parse_existing_commit};
use super::scoped_support::{
    scoped_attached_head_ref, validate_scoped_candidates, validate_scoped_expected_head,
    verify_scoped_candidate_tree, verify_scoped_commit_object, verify_scoped_commit_publication,
};
use super::*;

pub(super) fn commit_gwz_paths_checked(
    _backend: &Git2Backend,
    root: &Path,
    expected_head: Option<&str>,
    candidate_files: &[GitCandidateFile],
    message: &str,
) -> ModelResult<GitScopedCommitResult> {
    let candidates = validate_scoped_candidates(candidate_files, message)?;
    let repo = open_repo(root)?;
    let ref_name = scoped_attached_head_ref(&repo)?;
    let expected = expected_head
        .map(|value| parse_existing_commit(&repo, value))
        .transpose()?;
    validate_scoped_expected_head(&repo, &ref_name, expected)?;

    let parent = expected
        .map(|oid| repo.find_commit(oid).map_err(git_error))
        .transpose()?;
    let parent_tree = parent
        .as_ref()
        .map(|commit| commit.tree().map_err(git_error))
        .transpose()?;
    let mut index = git2::Index::new().map_err(git_error)?;
    if let Some(tree) = parent_tree.as_ref() {
        index.read_tree(tree).map_err(git_error)?;
    }

    let mut candidate_hashes = Vec::with_capacity(candidates.len());
    for candidate in &candidates {
        let blob = repo.blob(&candidate.bytes).map_err(git_error)?;
        let file_size = u32::try_from(candidate.bytes.len()).map_err(|_| {
            ModelError::new(
                ErrorCode::InvalidRequest,
                format!(
                    "candidate '{}' is too large for a Git blob entry",
                    candidate.path
                ),
            )
        })?;
        index
            .add(&git2::IndexEntry {
                ctime: git2::IndexTime::new(0, 0),
                mtime: git2::IndexTime::new(0, 0),
                dev: 0,
                ino: 0,
                mode: 0o100644,
                uid: 0,
                gid: 0,
                file_size,
                id: blob,
                flags: 0,
                flags_extended: 0,
                path: candidate.path.as_bytes().to_vec(),
            })
            .map_err(git_error)?;
        candidate_hashes.push(GitCandidateHash {
            path: candidate.path.clone(),
            sha256: format!("{:x}", Sha256::digest(&candidate.bytes)),
        });
    }
    let tree_oid = index.write_tree_to(&repo).map_err(git_error)?;
    let tree = repo.find_tree(tree_oid).map_err(git_error)?;
    verify_scoped_candidate_tree(&repo, parent_tree.as_ref(), &tree, &candidates)?;

    run_before_scoped_commit_ref_lock();
    let mut transaction = repo.transaction().map_err(git_error)?;
    transaction.lock_ref("HEAD").map_err(git_error)?;
    transaction.lock_ref(&ref_name).map_err(git_error)?;
    if scoped_attached_head_ref(&repo)? != ref_name {
        return Err(recovery_drift(
            "workspace root HEAD changed before scoped commit publication",
        ));
    }
    validate_scoped_expected_head(&repo, &ref_name, expected)?;

    let signature = repo.signature().map_err(git_error)?;
    let parents = parent.iter().collect::<Vec<_>>();
    let commit_oid = repo
        .commit(None, &signature, &signature, message, &tree, &parents)
        .map_err(git_error)?;
    verify_scoped_commit_object(&repo, commit_oid, expected, tree_oid, message, &signature)?;
    transaction
        .set_target(
            &ref_name,
            commit_oid,
            Some(&signature),
            "gwz merge composition",
        )
        .map_err(git_error)?;
    transaction.commit().map_err(git_error)?;
    verify_scoped_commit_publication(&repo, &ref_name, commit_oid, tree_oid, &candidates)?;

    Ok(GitScopedCommitResult {
        commit: commit_oid.to_string(),
        tree: tree_oid.to_string(),
        candidate_hashes,
    })
}

pub(super) fn verify_gwz_paths_commit(
    _backend: &Git2Backend,
    root: &Path,
    commit: &str,
    expected_parent: Option<&str>,
    candidate_files: &[GitCandidateFile],
    message: &str,
) -> ModelResult<GitScopedCommitResult> {
    let candidates = validate_scoped_candidates(candidate_files, message)?;
    let repo = open_repo(root)?;
    let commit_oid = parse_existing_commit(&repo, commit)?;
    let expected = expected_parent
        .map(|value| parse_existing_commit(&repo, value))
        .transpose()?;
    let published = repo.find_commit(commit_oid).map_err(git_error)?;
    let parents_match = match expected {
        Some(parent) => published.parent_count() == 1 && published.parent_id(0) == Ok(parent),
        None => published.parent_count() == 0,
    };
    if !parents_match || published.message_bytes() != message.as_bytes() {
        return Err(recovery_drift(
            "root HEAD is not the recorded merge composition commit",
        ));
    }
    let parent_tree = expected
        .map(|parent| {
            repo.find_commit(parent)
                .and_then(|commit| commit.tree())
                .map_err(git_error)
        })
        .transpose()?;
    let tree = published.tree().map_err(git_error)?;
    verify_scoped_candidate_tree(&repo, parent_tree.as_ref(), &tree, &candidates)
        .map_err(|_| recovery_drift("root composition candidate tree changed"))?;
    let ref_name = scoped_attached_head_ref(&repo)?;
    validate_scoped_expected_head(&repo, &ref_name, Some(commit_oid))?;
    Ok(GitScopedCommitResult {
        commit: commit_oid.to_string(),
        tree: tree.id().to_string(),
        candidate_hashes: candidates
            .iter()
            .map(|candidate| GitCandidateHash {
                path: candidate.path.clone(),
                sha256: format!("{:x}", Sha256::digest(&candidate.bytes)),
            })
            .collect(),
    })
}

pub(super) fn rollback_gwz_paths_commit_checked(
    backend: &Git2Backend,
    root: &Path,
    branch: &str,
    commit: &str,
    expected_parent: Option<&str>,
    candidate_files: &[GitCandidateFile],
    message: &str,
) -> ModelResult<()> {
    backend.verify_gwz_paths_commit(root, commit, expected_parent, candidate_files, message)?;
    let repo = open_repo(root)?;
    let commit_oid = parse_existing_commit(&repo, commit)?;
    let parent_oid = expected_parent
        .map(|value| parse_existing_commit(&repo, value))
        .transpose()?;
    let ref_name = branch_ref_name(branch);
    if scoped_attached_head_ref(&repo)? != ref_name {
        return Err(recovery_drift(
            "scoped evidence rollback requires the attached target branch",
        ));
    }

    let mut transaction = repo.transaction().map_err(git_error)?;
    transaction.lock_ref("HEAD").map_err(git_error)?;
    transaction.lock_ref(&ref_name).map_err(git_error)?;
    if scoped_attached_head_ref(&repo)? != ref_name {
        return Err(recovery_drift(
            "workspace root HEAD changed before scoped evidence rollback",
        ));
    }
    validate_scoped_expected_head(&repo, &ref_name, Some(commit_oid))?;
    backend.verify_gwz_paths_commit(root, commit, expected_parent, candidate_files, message)?;
    if let Some(parent) = parent_oid {
        transaction
            .set_target(
                &ref_name,
                parent,
                None,
                "gwz checked composition evidence rollback",
            )
            .map_err(git_error)?;
    } else {
        transaction.remove(&ref_name).map_err(git_error)?;
    }
    transaction.commit().map_err(git_error)?;

    let head = backend.head(root)?;
    if head.is_detached
        || head.branch.as_deref() != Some(branch)
        || head.commit.as_deref() != expected_parent
    {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "checked composition evidence rollback did not restore the expected root HEAD",
        ));
    }
    Ok(())
}

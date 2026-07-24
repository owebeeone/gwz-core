use super::merge_support::same_signature;
use super::recovery_support::recovery_drift;
use super::*;

pub(super) fn validate_scoped_candidates<'a>(
    candidate_files: &'a [GitCandidateFile],
    message: &str,
) -> ModelResult<Vec<&'a GitCandidateFile>> {
    if candidate_files.is_empty() {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            "scoped root commit requires at least one candidate file",
        ));
    }
    if message.contains('\0') {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            "scoped root commit message contains a NUL byte",
        ));
    }
    let mut seen = BTreeSet::new();
    for candidate in candidate_files {
        let path = Path::new(&candidate.path);
        let components = path
            .components()
            .map(|component| match component {
                std::path::Component::Normal(value) => value.to_str(),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| invalid_scoped_path(&candidate.path))?;
        if path.is_absolute()
            || components.len() < 2
            || components[0] != "gwz.conf"
            || components.contains(&".git")
            || components.join("/") != candidate.path
        {
            return Err(invalid_scoped_path(&candidate.path));
        }
        if !seen.insert(candidate.path.as_str()) {
            return Err(ModelError::new(
                ErrorCode::InvalidRequest,
                format!("duplicate scoped root candidate path '{}'", candidate.path),
            ));
        }
    }
    let mut candidates = candidate_files.iter().collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(candidates)
}

pub(super) fn invalid_scoped_path(path: &str) -> ModelError {
    ModelError::new(
        ErrorCode::InvalidRequest,
        format!(
            "candidate path '{path}' must be a normalized repository-relative file below gwz.conf/"
        ),
    )
}

pub(super) fn scoped_attached_head_ref(repo: &git2::Repository) -> ModelResult<String> {
    let head = repo.find_reference("HEAD").map_err(git_error)?;
    let Some(target) = head.symbolic_target().map_err(git_error)? else {
        return Err(recovery_drift(
            "scoped root commit requires an attached local branch",
        ));
    };
    if target
        .strip_prefix("refs/heads/")
        .is_none_or(|branch| branch.is_empty())
    {
        return Err(recovery_drift(
            "scoped root commit requires an attached local branch",
        ));
    }
    Ok(target.to_owned())
}

pub(super) fn validate_scoped_expected_head(
    repo: &git2::Repository,
    ref_name: &str,
    expected: Option<git2::Oid>,
) -> ModelResult<()> {
    match (repo.find_reference(ref_name), expected) {
        (Ok(reference), Some(expected)) => {
            let observed = reference.peel_to_commit().map_err(git_error)?.id();
            if observed != expected {
                return Err(recovery_drift(format!(
                    "workspace root changed before scoped commit; expected {expected}, observed {observed}"
                )));
            }
            Ok(())
        }
        (Err(error), None) if error.code() == git2::ErrorCode::NotFound => Ok(()),
        (Ok(reference), None) => {
            let observed = reference.peel_to_commit().map_err(git_error)?.id();
            Err(recovery_drift(format!(
                "workspace root was expected to be unborn, observed {observed}"
            )))
        }
        (Err(error), Some(expected)) if error.code() == git2::ErrorCode::NotFound => {
            Err(recovery_drift(format!(
                "workspace root was expected at {expected}, but its attached branch is unborn"
            )))
        }
        (Err(error), _) => Err(git_error(error)),
    }
}

pub(super) fn verify_scoped_candidate_tree(
    repo: &git2::Repository,
    parent: Option<&git2::Tree<'_>>,
    candidate: &git2::Tree<'_>,
    files: &[&GitCandidateFile],
) -> ModelResult<()> {
    verify_scoped_candidate_blobs(repo, candidate, files)?;
    let paths = files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<BTreeSet<_>>();
    let diff = repo
        .diff_tree_to_tree(parent, Some(candidate), None)
        .map_err(git_error)?;
    if diff.deltas().any(|delta| {
        delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .and_then(Path::to_str)
            .is_none_or(|path| !paths.contains(path))
    }) {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "scoped candidate tree changes a path outside the supplied GWZ files",
        ));
    }
    Ok(())
}

pub(super) fn verify_scoped_candidate_blobs(
    repo: &git2::Repository,
    tree: &git2::Tree<'_>,
    files: &[&GitCandidateFile],
) -> ModelResult<()> {
    for file in files {
        let entry = tree.get_path(Path::new(&file.path)).map_err(git_error)?;
        if entry.kind() != Some(git2::ObjectType::Blob)
            || entry.filemode() != 0o100644
            || repo.find_blob(entry.id()).map_err(git_error)?.content() != file.bytes
        {
            return Err(ModelError::new(
                ErrorCode::GitCommandFailed,
                format!("candidate tree does not contain exact file '{}'", file.path),
            ));
        }
    }
    Ok(())
}

pub(super) fn verify_scoped_commit_object(
    repo: &git2::Repository,
    oid: git2::Oid,
    expected_parent: Option<git2::Oid>,
    tree: git2::Oid,
    message: &str,
    signature: &git2::Signature<'_>,
) -> ModelResult<()> {
    let commit = repo.find_commit(oid).map_err(git_error)?;
    let parents_match = match expected_parent {
        Some(parent) => commit.parent_count() == 1 && commit.parent_id(0) == Ok(parent),
        None => commit.parent_count() == 0,
    };
    if !parents_match
        || commit.tree_id() != tree
        || commit.message_bytes() != message.as_bytes()
        || !same_signature(&commit.author(), signature)
        || !same_signature(&commit.committer(), signature)
    {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "scoped root commit does not match its checked specification",
        ));
    }
    Ok(())
}

pub(super) fn verify_scoped_commit_publication(
    repo: &git2::Repository,
    ref_name: &str,
    commit: git2::Oid,
    tree: git2::Oid,
    files: &[&GitCandidateFile],
) -> ModelResult<()> {
    if scoped_attached_head_ref(repo)? != ref_name
        || repo
            .find_reference(ref_name)
            .and_then(|reference| reference.peel_to_commit())
            .map_err(git_error)?
            .id()
            != commit
    {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "scoped root commit ref publication did not persist",
        ));
    }
    let published = repo.find_commit(commit).map_err(git_error)?;
    if published.tree_id() != tree {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "published scoped root commit tree changed",
        ));
    }
    verify_scoped_candidate_blobs(repo, &published.tree().map_err(git_error)?, files)
}

use super::repository_support::open_repo;
use super::stash_support::stash_message_matches_gwz_prefix;
use super::*;

const MERGE_REF_PREFIX: &str = "refs/gwz/merge/";

pub(super) fn create_backup_ref(
    _backend: &Git2Backend,
    path: &Path,
    name: &str,
    target: &str,
) -> ModelResult<GitBackupRefResult> {
    validate_backup_ref_name(name)?;
    let repo = open_repo(path)?;
    let target = parse_commit(&repo, target)?;
    match repo.find_reference(name) {
        Ok(reference) if reference.target() == Some(target) => {
            return Ok(GitBackupRefResult {
                name: name.to_owned(),
                target: target.to_string(),
            });
        }
        Ok(reference) => {
            return Err(ModelError::new(
                ErrorCode::MergeDrift,
                format!(
                    "preservation ref '{name}' points to '{}' instead of '{}'",
                    reference
                        .target()
                        .map_or_else(|| "a symbolic ref".to_owned(), |oid| oid.to_string()),
                    target
                ),
            ));
        }
        Err(error) if error.code() == git2::ErrorCode::NotFound => {}
        Err(error) => return Err(git_error(error)),
    }

    repo.reference(name, target, false, "gwz merge preservation")
        .map_err(git_error)?;
    verify_backup_ref(&repo, name, target)?;
    Ok(GitBackupRefResult {
        name: name.to_owned(),
        target: target.to_string(),
    })
}

pub(super) fn delete_backup_ref_checked(
    _backend: &Git2Backend,
    path: &Path,
    name: &str,
    expected_target: &str,
) -> ModelResult<()> {
    validate_backup_ref_name(name)?;
    let repo = open_repo(path)?;
    let expected = git2::Oid::from_str(expected_target).map_err(|error| {
        ModelError::new(
            ErrorCode::InvalidRequest,
            format!("invalid preservation target '{expected_target}': {error}"),
        )
    })?;
    let mut reference = match repo.find_reference(name) {
        Ok(reference) => reference,
        Err(error) if error.code() == git2::ErrorCode::NotFound => return Ok(()),
        Err(error) => return Err(git_error(error)),
    };
    if reference.target() != Some(expected) {
        return Err(ModelError::new(
            ErrorCode::MergeDrift,
            format!(
                "preservation ref '{name}' no longer points to recorded commit '{expected_target}'"
            ),
        ));
    }
    reference.delete().map_err(git_error)?;
    match repo.find_reference(name) {
        Err(error) if error.code() == git2::ErrorCode::NotFound => Ok(()),
        Ok(_) => Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            format!("preservation ref '{name}' still exists after deletion"),
        )),
        Err(error) => Err(git_error(error)),
    }
}

pub(super) fn stash_for_merge_preservation(
    backend: &Git2Backend,
    path: &Path,
    merge_id: &str,
    include_untracked: bool,
) -> ModelResult<GitStashPushResult> {
    validate_merge_id(merge_id)?;
    let stash_id = format!("stash_{merge_id}");
    let prefix = format!("gwz:{stash_id}:");
    let message = format!("{prefix} merge preservation");
    let status = backend.status(path)?;
    if status.unresolved > 0 || backend.repository_state(path)? != GitRepositoryState::Clean {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            "merge preservation requires a clean integration state and an index without unresolved entries",
        ));
    }

    let matching = backend
        .stash_list(path)?
        .into_iter()
        .filter(|entry| stash_message_matches_gwz_prefix(&entry.message, &prefix))
        .collect::<Vec<_>>();
    match matching.as_slice() {
        [existing] if status.is_dirty => {
            return Err(ModelError::new(
                ErrorCode::MergeDrift,
                format!(
                    "repository contains new work after preservation stash '{}'",
                    existing.object_id
                ),
            ));
        }
        [existing] => {
            return Ok(GitStashPushResult {
                object_id: existing.object_id.clone(),
                message,
            });
        }
        [] => {}
        _ => {
            return Err(ModelError::new(
                ErrorCode::MergeRecoveryRequired,
                format!("multiple native stashes use preservation id '{stash_id}'"),
            ));
        }
    }

    let preservable =
        status.staged > 0 || status.unstaged > 0 || include_untracked && status.untracked > 0;
    if !preservable {
        return Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            "repository has no eligible staged, unstaged, or untracked work to preserve",
        ));
    }
    let result = super::stash::stash_push(
        backend,
        path,
        &message,
        GitStashPushOptions {
            include_untracked,
            include_ignored: false,
            preserve_index: false,
        },
    )?;
    if !backend.stash_list(path)?.iter().any(|entry| {
        entry.object_id == result.object_id
            && stash_message_matches_gwz_prefix(&entry.message, &prefix)
    }) {
        return Err(ModelError::new(
            ErrorCode::StashIncomplete,
            format!(
                "preservation stash '{}' was not verified after creation",
                result.object_id
            ),
        ));
    }
    Ok(result)
}

pub(super) fn index_matches_candidate_files(
    _backend: &Git2Backend,
    path: &Path,
    expected_files: &[GitCandidateFile],
    absent_paths: &[String],
) -> ModelResult<bool> {
    let repo = open_repo(path)?;
    let index = repo.index().map_err(git_error)?;
    for file in expected_files {
        let entries = index
            .iter()
            .filter(|entry| entry.path == file.path.as_bytes())
            .collect::<Vec<_>>();
        let Some(entry) = entries.as_slice().first() else {
            return Ok(false);
        };
        let expected_blob =
            git2::Oid::hash_object(git2::ObjectType::Blob, &file.bytes).map_err(git_error)?;
        if entries.len() != 1
            || (entry.flags >> 12) & 3 != 0
            || entry.mode != 0o100644
            || entry.id != expected_blob
        {
            return Ok(false);
        }
        let worktree_path = path.join(&file.path);
        let metadata = match std::fs::symlink_metadata(&worktree_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(crate::git::io_error(error)),
        };
        if !metadata.file_type().is_file()
            || std::fs::read(&worktree_path).map_err(crate::git::io_error)? != file.bytes
        {
            return Ok(false);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 != 0 {
                return Ok(false);
            }
        }
    }
    for absent in absent_paths {
        if index.iter().any(|entry| entry.path == absent.as_bytes()) {
            return Ok(false);
        }
        match std::fs::symlink_metadata(path.join(absent)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) => return Ok(false),
            Err(error) => return Err(crate::git::io_error(error)),
        }
    }
    Ok(true)
}

pub(super) fn index_entries_match_candidate_files(
    _backend: &Git2Backend,
    path: &Path,
    expected_files: &[GitCandidateFile],
    absent_paths: &[String],
) -> ModelResult<bool> {
    let repo = open_repo(path)?;
    let index = repo.index().map_err(git_error)?;
    for file in expected_files {
        let entries = index
            .iter()
            .filter(|entry| entry.path == file.path.as_bytes())
            .collect::<Vec<_>>();
        let Some(entry) = entries.as_slice().first() else {
            return Ok(false);
        };
        let expected_blob =
            git2::Oid::hash_object(git2::ObjectType::Blob, &file.bytes).map_err(git_error)?;
        if entries.len() != 1
            || (entry.flags >> 12) & 3 != 0
            || entry.flags & 0xc000 != 0
            || entry.flags_extended != 0
            || entry.mode != 0o100644
            || entry.id != expected_blob
        {
            return Ok(false);
        }
    }
    Ok(!absent_paths
        .iter()
        .any(|absent| index.iter().any(|entry| entry.path == absent.as_bytes())))
}

fn parse_commit(repo: &git2::Repository, target: &str) -> ModelResult<git2::Oid> {
    let oid = git2::Oid::from_str(target).map_err(|error| {
        ModelError::new(
            ErrorCode::InvalidRequest,
            format!("invalid preservation target '{target}': {error}"),
        )
    })?;
    repo.find_commit(oid).map_err(git_error)?;
    Ok(oid)
}

fn validate_backup_ref_name(name: &str) -> ModelResult<()> {
    if !name.starts_with(MERGE_REF_PREFIX)
        || !name.ends_with("/head")
        || !git2::Reference::is_valid_name(name)
    {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            format!(
                "preservation ref must be a valid '{MERGE_REF_PREFIX}<merge>/<target>/head' name"
            ),
        ));
    }
    Ok(())
}

fn validate_merge_id(merge_id: &str) -> ModelResult<()> {
    if merge_id.is_empty()
        || !merge_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            format!("invalid merge preservation id '{merge_id}'"),
        ));
    }
    Ok(())
}

fn verify_backup_ref(repo: &git2::Repository, name: &str, target: git2::Oid) -> ModelResult<()> {
    let observed = repo.find_reference(name).map_err(git_error)?;
    if observed.target() != Some(target) {
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            format!("preservation ref '{name}' failed post-creation verification"),
        ));
    }
    Ok(())
}

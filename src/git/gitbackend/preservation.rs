use super::repository_support::open_repo;
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

pub(super) fn create_backup_ref_checked(
    _backend: &Git2Backend,
    path: &Path,
    branch: &str,
    expected_head: &str,
    name: &str,
    target: &str,
) -> ModelResult<GitBackupRefResult> {
    validate_backup_ref_name(name)?;
    if target != expected_head {
        return Err(ModelError::new(
            ErrorCode::PreservationEvidenceMismatch,
            "backup-ref target differs from its persisted attached HEAD",
        ));
    }
    let branch_ref = format!("refs/heads/{branch}");
    if !git2::Reference::is_valid_name(&branch_ref) {
        return Err(ModelError::new(
            ErrorCode::InvalidRequest,
            format!("invalid preservation branch '{branch}'"),
        ));
    }
    let repo = open_repo(path)?;
    let target = parse_commit(&repo, target)?;
    let mut transaction = repo.transaction().map_err(git_error)?;
    transaction.lock_ref("HEAD").map_err(git_error)?;
    transaction.lock_ref(&branch_ref).map_err(git_error)?;
    transaction.lock_ref(name).map_err(git_error)?;
    require_attached_head(&repo, &branch_ref, target)?;
    match repo.find_reference(name) {
        Ok(reference) if reference.target() == Some(target) => {
            drop(transaction);
        }
        Ok(reference) => {
            return Err(ModelError::new(
                ErrorCode::PreservationEvidenceMismatch,
                format!(
                    "preservation ref '{name}' points to '{}' instead of persisted target '{target}'",
                    reference
                        .target()
                        .map_or_else(|| "a symbolic ref".to_owned(), |actual| actual.to_string())
                ),
            ));
        }
        Err(error) if error.code() == git2::ErrorCode::NotFound => {
            transaction
                .set_target(name, target, None, "gwz merge preservation")
                .map_err(git_error)?;
            transaction.commit().map_err(git_error)?;
        }
        Err(error) => return Err(git_error(error)),
    }
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
    let expected =
        preservation_root::index::parse_exact_oid(&repo, expected_target, "preservation target")
            .map_err(|error| {
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

pub(super) fn observe_direct_ref(
    _backend: &Git2Backend,
    path: &Path,
    name: &str,
) -> ModelResult<GitDirectRefObservation> {
    let repo = open_repo(path)?;
    match repo.find_reference(name) {
        Ok(reference) => Ok(match reference.target() {
            Some(target) => GitDirectRefObservation::Direct {
                target: target.to_string(),
            },
            None => GitDirectRefObservation::NonDirect,
        }),
        Err(error) if error.code() == git2::ErrorCode::NotFound => {
            Ok(GitDirectRefObservation::Absent)
        }
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
    let message = format!("gwz:{stash_id}: merge preservation");
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
        .filter(|entry| canonical_preservation_stash_message(&entry.message, &message).is_some())
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
            && canonical_preservation_stash_message(&entry.message, &message).is_some()
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

pub(super) fn stash_for_merge_preservation_checked(
    backend: &Git2Backend,
    path: &Path,
    branch: &str,
    expected_head: &str,
    expected_preimage_sha256: &str,
    merge_id: &str,
    include_untracked: bool,
) -> ModelResult<GitStashPushResult> {
    validate_merge_id(merge_id)?;
    let mut repo = open_repo(path)?;
    let expected = parse_commit(&repo, expected_head)?;
    let branch_ref = format!("refs/heads/{branch}");
    require_attached_head(&repo, &branch_ref, expected)?;
    if backend.repository_state(path)? != GitRepositoryState::Clean {
        return Err(ModelError::new(
            ErrorCode::PreservationEvidenceMismatch,
            "preservation stash no longer has a clean native Git state",
        ));
    }
    let stash_id = format!("stash_{merge_id}");
    let message = format!("gwz:{stash_id}: merge preservation");
    let status = backend.status(path)?;
    if status.unresolved > 0 {
        return Err(ModelError::new(
            ErrorCode::PreservationEvidenceMismatch,
            "preservation stash has unresolved index entries",
        ));
    }
    // Retain the complete native stash set, not only the merge-owned subset.
    // Existing foreign stashes are permitted, but a concurrent addition,
    // removal, or reorder at the final mutation boundary invalidates the
    // prepared operation.
    let native_stashes = backend.stash_list(path)?;
    let stashes = preservation_image::decode_stashes(backend, path, merge_id)?;
    let current = preservation_image::capture(path, include_untracked)?;
    match stashes.as_slice() {
        [stash]
            if stash.message == message
                && stash.head_commit == expected_head
                && stash.image.preimage_sha256 == expected_preimage_sha256
                && current.dirty == GitPreservationDirtySummary::default() =>
        {
            return Ok(GitStashPushResult {
                object_id: stash.object_id.clone(),
                message,
            });
        }
        [] => {}
        _ => {
            return Err(ModelError::new(
                ErrorCode::PreservationEvidenceMismatch,
                "native preservation stash is missing, duplicated, or disagrees with the persisted action",
            ));
        }
    }
    let preservable =
        status.staged > 0 || status.unstaged > 0 || include_untracked && status.untracked > 0;
    if !preservable {
        return Err(ModelError::new(
            ErrorCode::PreservationEvidenceMismatch,
            "repository has no eligible work matching the persisted stash action",
        ));
    }
    if current.preimage_sha256 != expected_preimage_sha256
        || current.dirty == GitPreservationDirtySummary::default()
    {
        return Err(ModelError::new(
            ErrorCode::PreservationEvidenceMismatch,
            "preservation checkout no longer matches the persisted stash preimage",
        ));
    }
    let signature = super::merge_support::merge_signature(&repo)?;
    let flags = super::stash_support::stash_push_flags(GitStashPushOptions {
        include_untracked,
        include_ignored: false,
        preserve_index: false,
    });
    // The deterministic hook is after every preparatory query. A second exact
    // proof follows it and is the final in-contract check before libgit2's one
    // native stash call. Raw filesystem writers after that proof are outside
    // the workspace mutation-lease contract, as documented for native stash.
    run_before_preservation_stash();
    require_attached_head(&repo, &branch_ref, expected)?;
    if backend.repository_state(path)? != GitRepositoryState::Clean
        || backend.stash_list(path)? != native_stashes
        || !preservation_image::decode_stashes(backend, path, merge_id)?.is_empty()
        || preservation_image::capture(path, include_untracked)?.preimage_sha256
            != expected_preimage_sha256
    {
        return Err(ModelError::new(
            ErrorCode::PreservationEvidenceMismatch,
            "preservation checkout or stash set changed at the native mutation boundary",
        ));
    }
    let object_id = repo
        .stash_save(&signature, &message, Some(flags))
        .map_err(git_error)?;
    let result = GitStashPushResult {
        object_id: object_id.to_string(),
        message: message.clone(),
    };
    let verified = preservation_image::decode_stashes(backend, path, merge_id)?;
    let postimage = preservation_image::capture(path, include_untracked)?;
    if !matches!(verified.as_slice(), [stash]
        if stash.object_id == result.object_id
            && stash.message == message
            && stash.head_commit == expected_head
            && stash.image.preimage_sha256 == expected_preimage_sha256)
        || postimage.dirty != GitPreservationDirtySummary::default()
    {
        return Err(ModelError::new(
            ErrorCode::PreservationEvidenceMismatch,
            "preservation stash failed exact post-mutation verification",
        ));
    }
    Ok(result)
}

pub(super) fn preservation_image(
    _backend: &Git2Backend,
    path: &Path,
    include_untracked: bool,
) -> ModelResult<GitPreservationImage> {
    preservation_image::capture(path, include_untracked)
}

pub(super) fn preservation_stashes(
    backend: &Git2Backend,
    path: &Path,
    merge_id: &str,
) -> ModelResult<Vec<GitPreservationStashEvidence>> {
    validate_merge_id(merge_id)?;
    preservation_image::decode_stashes(backend, path, merge_id)
}

fn canonical_preservation_stash_message<'a>(
    native_message: &str,
    expected_message: &'a str,
) -> Option<&'a str> {
    preservation_image::canonical_stash_message(native_message, expected_message)
        .then_some(expected_message)
}

pub(super) fn checkout_matches_commit(
    backend: &Git2Backend,
    path: &Path,
    branch: &str,
    commit: &str,
) -> ModelResult<bool> {
    if backend.repository_state(path)? != GitRepositoryState::Clean {
        return Ok(false);
    }
    let head = backend.head(path)?;
    if head.is_detached
        || head.branch.as_deref() != Some(branch)
        || head.commit.as_deref() != Some(commit)
        || backend
            .read_ref(path, &format!("refs/heads/{branch}"))?
            .as_deref()
            != Some(commit)
    {
        return Ok(false);
    }
    preservation_image::checkout_matches_commit_except(path, commit, &[])
}

pub(super) fn checkout_matches_commit_except(
    _backend: &Git2Backend,
    path: &Path,
    commit: &str,
    allowed_paths: &[String],
) -> ModelResult<bool> {
    preservation_image::checkout_matches_commit_except(path, commit, allowed_paths)
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
            git2::Oid::hash_object_ext(git2::ObjectType::Blob, &file.bytes, repo.object_format())
                .map_err(git_error)?;
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
            git2::Oid::hash_object_ext(git2::ObjectType::Blob, &file.bytes, repo.object_format())
                .map_err(git_error)?;
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
    let oid = preservation_root::index::parse_exact_oid(repo, target, "preservation target")
        .map_err(|error| {
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

fn require_attached_head(
    repo: &git2::Repository,
    branch_ref: &str,
    expected: git2::Oid,
) -> ModelResult<()> {
    let head = repo.head().map_err(git_error)?;
    let branch = repo.find_reference(branch_ref).map_err(git_error)?;
    if !head.is_branch()
        || head.name().ok() != Some(branch_ref)
        || head.target() != Some(expected)
        || branch.target() != Some(expected)
    {
        return Err(ModelError::new(
            ErrorCode::PreservationEvidenceMismatch,
            "preservation action no longer has its persisted attached HEAD",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FaultBoundary {
    Before,
    After,
    BeforeIndexCommit,
    AfterIndexCommit,
    BeforeParentStageCreate,
    AfterParentStageCreate,
    BeforeParentPublish,
    AfterParentPublish,
    #[cfg(unix)]
    BeforeUnixParentSync,
    #[cfg(unix)]
    AfterUnixParentSync,
    #[cfg(windows)]
    BeforeWindowsFirstBarrierRename,
    #[cfg(windows)]
    AfterWindowsFirstBarrierRename,
    #[cfg(windows)]
    BeforeWindowsSecondBarrierRename,
    #[cfg(windows)]
    AfterWindowsSecondBarrierRename,
}

#[cfg(test)]
type FaultHook = (FaultBoundary, Box<dyn FnOnce()>);

#[cfg(test)]
thread_local! {
    static NEXT_FAULT: std::cell::Cell<Option<FaultBoundary>> = const { std::cell::Cell::new(None) };
    static NEXT_HOOK: std::cell::RefCell<Option<FaultHook>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn fail_next_at(boundary: FaultBoundary) {
    NEXT_FAULT.set(Some(boundary));
}

#[cfg(test)]
pub(crate) fn run_next_at(boundary: FaultBoundary, hook: impl FnOnce() + 'static) {
    NEXT_HOOK.with_borrow_mut(|next| *next = Some((boundary, Box::new(hook))));
}

#[cfg(test)]
pub(super) fn fault(boundary: FaultBoundary) -> ModelResult<()> {
    let hook = NEXT_HOOK.with_borrow_mut(|next| {
        (next.as_ref().map(|(at, _)| *at) == Some(boundary))
            .then(|| next.take().expect("matching hook exists").1)
    });
    if let Some(hook) = hook {
        hook();
    }
    if NEXT_FAULT.get() == Some(boundary) {
        NEXT_FAULT.set(None);
        return Err(ModelError::new(
            ErrorCode::GitCommandFailed,
            "injected root-preservation fault",
        ));
    }
    Ok(())
}

#[cfg(not(test))]
pub(super) fn fault(_boundary: FaultBoundary) -> ModelResult<()> {
    Ok(())
}

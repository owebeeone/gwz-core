//! `observe_work`: what is on disk that is not in a commit.
//!
//! Design §5.1 and architecture §4 set the rules this module follows:
//!
//! - "Do not substitute 'Git status is empty' for this contract." Status is
//!   one input. Every index entry carrying a status-suppression flag is
//!   compared against the **bytes on disk**, and a path whose bytes cannot be
//!   read is [`PhysicalState::Unobservable`] — never clean.
//! - "Never clear live index flags to implement a read-only check." Nothing
//!   here writes: the status walk runs with the index refresh and the index
//!   update disabled, and the physical comparison hashes the worktree file
//!   without storing the result.
//! - "Ignored does not mean disposable." Ignored data is reported.
//! - "Treat valid sparse absence separately." A skip-worktree path that is
//!   absent while `core.sparseCheckout` is on is a valid sparse absence and
//!   goes in `sparse_absent`; the same path absent *without* sparse checkout
//!   is a suppressed entry whose physical state is `Absent`.
//!
//! Partial failures use `WorkObservation::unknown` (lane W proposal W1): one
//! unreadable entry makes that path unknown, not the whole inventory.
//!
//! ## Two documented approximations
//!
//! - **Filters.** The physical comparison hashes worktree bytes as they are.
//!   In a repository with `core.autocrlf` or a `text` attribute, Git's index
//!   blob is the *cleaned* form, so an unchanged file can compare as
//!   `Differs`. That is the conservative direction — it can call a clean file
//!   dirty, never a dirty file clean.
//! - **Ignored directories** are reported as the directory, not recursively
//!   as every file beneath it, so a build tree costs one entry rather than
//!   thousands. Presence of ignored user data is what the safety decision
//!   needs.

use std::path::Path;

use git2::{ErrorCode, Repository, RepositoryOpenFlags, Status, StatusOptions, StatusShow};
use gwz_repo_contract::{
    BytePath, NativeOperation, Observation, PhysicalState, RepositoryInfo, SuppressedEntry,
    SuppressionFlag, UnknownKind, UnknownReason, WorkEntry, WorkKind, WorkObservation,
};

use crate::oid::git_format;
use crate::paths::byte_path_to_path;

/// Git's own index flag bits (`git2/index.h`). git2 exposes the raw `flags`
/// and `flags_extended` words but not their meanings.
const FLAG_ASSUME_VALID: u16 = 0x8000;
const FLAG_STAGE_MASK: u16 = 0x3000;
const FLAG_EXT_INTENT_TO_ADD: u16 = 1 << 13;
const FLAG_EXT_SKIP_WORKTREE: u16 = 1 << 14;

/// Bytes read from a worktree file when deciding binary-ness, matching Git's
/// own buffer for the same question.
const BINARY_PROBE_BYTES: usize = 8000;

pub(crate) fn open(repository: &RepositoryInfo) -> Result<Repository, UnknownReason> {
    Repository::open_ext(
        &repository.git_dir,
        RepositoryOpenFlags::NO_SEARCH | RepositoryOpenFlags::NO_DOTGIT,
        std::iter::empty::<&std::ffi::OsStr>(),
    )
    .map_err(|error| {
        UnknownReason::new(
            UnknownKind::Unreadable,
            format!("{}: {}", repository.git_dir.display(), error.message()),
        )
    })
}

pub(crate) fn observe_work(repository: &RepositoryInfo) -> Observation<WorkObservation> {
    let opened = match open(repository) {
        Ok(opened) => opened,
        Err(reason) => return Observation::Unknown(vec![reason]),
    };
    let mut unknown = Vec::new();
    let mut observation = WorkObservation {
        native_operation: native_operation(&opened),
        stash_entries: stash_entries(&opened, &mut unknown),
        ..WorkObservation::default()
    };

    let work_dir = opened.workdir().map(Path::to_path_buf);
    if let Some(work_dir) = work_dir {
        // A bare repository has no worktree, no index dirt and no untracked
        // data: there is nothing to observe beyond operation state.
        collect_status(&opened, &mut observation, &work_dir, &mut unknown);
        collect_suppressed(
            repository,
            &opened,
            &mut observation,
            &work_dir,
            &mut unknown,
        );
    }
    observation.unknown = unknown;
    Observation::Known(observation)
}

fn native_operation(repository: &Repository) -> Option<NativeOperation> {
    use git2::RepositoryState as State;
    match repository.state() {
        State::Clean => None,
        State::Merge => Some(NativeOperation::Merge),
        State::Revert | State::RevertSequence => Some(NativeOperation::Revert),
        State::CherryPick | State::CherryPickSequence => Some(NativeOperation::CherryPick),
        State::Bisect => Some(NativeOperation::Bisect),
        State::Rebase | State::RebaseInteractive | State::RebaseMerge => {
            Some(NativeOperation::Rebase)
        }
        State::ApplyMailbox | State::ApplyMailboxOrRebase => Some(NativeOperation::ApplyMailbox),
    }
}

/// `refs/stash`'s reflog is the stash stack; its length is the number of
/// entries, including the older ones design §5.1 names.
fn stash_entries(repository: &Repository, unknown: &mut Vec<UnknownReason>) -> u64 {
    if !crate::history::has_reflog(repository, "refs/stash") {
        return 0;
    }
    match repository.reflog("refs/stash") {
        Ok(reflog) => reflog.len() as u64,
        Err(error) if error.code() == ErrorCode::NotFound => 0,
        Err(error) => {
            unknown.push(UnknownReason::new(
                UnknownKind::Unreadable,
                format!("the stash reflog could not be read: {}", error.message()),
            ));
            0
        }
    }
}

fn collect_status(
    repository: &Repository,
    observation: &mut WorkObservation,
    work_dir: &Path,
    unknown: &mut Vec<UnknownReason>,
) {
    let mut options = StatusOptions::new();
    options
        .show(StatusShow::IndexAndWorkdir)
        .include_untracked(true)
        .include_ignored(true)
        .include_unmodified(false)
        .recurse_untracked_dirs(true)
        .recurse_ignored_dirs(false)
        .renames_head_to_index(true)
        .renames_index_to_workdir(true)
        .include_unreadable(true)
        .no_refresh(true)
        .update_index(false);
    let statuses = match repository.statuses(Some(&mut options)) {
        Ok(statuses) => statuses,
        Err(error) => {
            unknown.push(UnknownReason::new(
                UnknownKind::Unreadable,
                format!("the status walk failed: {}", error.message()),
            ));
            return;
        }
    };
    for entry in statuses.iter() {
        let path: BytePath = entry.path_bytes().to_vec();
        let status = entry.status();
        if status.contains(Status::WT_UNREADABLE) {
            unknown.push(UnknownReason {
                kind: UnknownKind::Unreadable,
                path: Some(path.clone()),
                detail: "the worktree entry could not be read".to_owned(),
            });
            continue;
        }
        let mut kinds = kinds_of(status);
        kinds.extend(type_changes(&entry));
        kinds.sort_by_key(|kind| format!("{kind:?}"));
        kinds.dedup();
        let binary = binary_of(work_dir, &path);
        for kind in kinds {
            observation.entries.push(WorkEntry {
                path: path.clone(),
                kind,
                binary,
            });
        }
    }
    observation.entries.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| format!("{:?}", left.kind).cmp(&format!("{:?}", right.kind)))
    });
    observation.entries.dedup();
}

fn kinds_of(status: Status) -> Vec<WorkKind> {
    let mut kinds = Vec::new();
    if status.contains(Status::CONFLICTED) {
        kinds.push(WorkKind::Conflict);
    }
    if status.intersects(Status::INDEX_NEW | Status::INDEX_MODIFIED) {
        kinds.push(WorkKind::Staged);
    }
    if status.contains(Status::INDEX_RENAMED) {
        kinds.push(WorkKind::Renamed);
    }
    if status.contains(Status::WT_RENAMED) {
        kinds.push(WorkKind::Renamed);
    }
    if status.intersects(Status::INDEX_DELETED | Status::WT_DELETED) {
        kinds.push(WorkKind::Deleted);
    }
    if status.contains(Status::WT_MODIFIED) {
        kinds.push(WorkKind::Unstaged);
    }
    if status.contains(Status::WT_NEW) {
        kinds.push(WorkKind::Untracked);
    }
    if status.contains(Status::IGNORED) {
        kinds.push(WorkKind::Ignored);
    }
    if status.intersects(Status::INDEX_TYPECHANGE | Status::WT_TYPECHANGE) {
        kinds.push(WorkKind::LinkChange);
    }
    kinds
}

/// A permission change and a file/symlink change both arrive as "modified";
/// only the deltas say which. Distinguishing them matters because a mode or
/// link change is unsaved work that a content diff would not show.
fn type_changes(entry: &git2::StatusEntry<'_>) -> Vec<WorkKind> {
    let mut kinds = Vec::new();
    for delta in [entry.head_to_index(), entry.index_to_workdir()]
        .into_iter()
        .flatten()
    {
        let old = delta.old_file().mode();
        let new = delta.new_file().mode();
        if old == git2::FileMode::Unreadable || new == git2::FileMode::Unreadable || old == new {
            continue;
        }
        let is_link = |mode| mode == git2::FileMode::Link;
        if is_link(old) || is_link(new) {
            kinds.push(WorkKind::LinkChange);
        } else if matches!(old, git2::FileMode::Blob | git2::FileMode::BlobExecutable)
            && matches!(new, git2::FileMode::Blob | git2::FileMode::BlobExecutable)
        {
            kinds.push(WorkKind::ModeChange);
        }
    }
    kinds
}

/// Binary-ness "where cheap" (architecture §4): the first
/// [`BINARY_PROBE_BYTES`] of the worktree file, the same test Git applies.
/// `None` when there is no worktree file to look at.
fn binary_of(work_dir: &Path, path: &BytePath) -> Option<bool> {
    let full = work_dir.join(byte_path_to_path(path)?);
    let metadata = std::fs::symlink_metadata(&full).ok()?;
    if !metadata.file_type().is_file() {
        return None;
    }
    let bytes = read_prefix(&full, BINARY_PROBE_BYTES).ok()?;
    Some(bytes.contains(&0))
}

fn read_prefix(path: &Path, limit: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut buffer = vec![0u8; limit];
    let mut filled = 0;
    while filled < limit {
        match file.read(&mut buffer[filled..])? {
            0 => break,
            read => filled += read,
        }
    }
    buffer.truncate(filled);
    Ok(buffer)
}

fn collect_suppressed(
    info: &RepositoryInfo,
    repository: &Repository,
    observation: &mut WorkObservation,
    work_dir: &Path,
    unknown: &mut Vec<UnknownReason>,
) {
    let index = match repository.index() {
        Ok(index) => index,
        Err(error) => {
            unknown.push(UnknownReason::new(
                UnknownKind::Unreadable,
                format!("the index could not be read: {}", error.message()),
            ));
            return;
        }
    };
    let sparse = repository
        .config()
        .and_then(|config| config.get_bool("core.sparseCheckout"))
        .unwrap_or(false);
    for entry in index.iter() {
        if entry.flags & FLAG_STAGE_MASK != 0 {
            // Conflict stages are reported by the status walk, not here.
            continue;
        }
        let mut flags = Vec::new();
        if entry.flags & FLAG_ASSUME_VALID != 0 {
            flags.push(SuppressionFlag::AssumeUnchanged);
        }
        if entry.flags_extended & FLAG_EXT_SKIP_WORKTREE != 0 {
            flags.push(SuppressionFlag::SkipWorktree);
        }
        if entry.flags_extended & FLAG_EXT_INTENT_TO_ADD != 0 {
            flags.push(SuppressionFlag::Other);
        }
        if flags.is_empty() {
            continue;
        }
        let physical = physical_state(info, work_dir, &entry, unknown);
        if physical == PhysicalState::Absent
            && sparse
            && flags.contains(&SuppressionFlag::SkipWorktree)
        {
            // A valid sparse absence, accounted for explicitly rather than
            // reported as a suppressed path whose bytes are missing.
            observation.sparse_absent.push(entry.path.clone());
            continue;
        }
        for flag in flags {
            observation.suppressed.push(SuppressedEntry {
                path: entry.path.clone(),
                flag,
                physical,
            });
        }
    }
    observation.sparse_absent.sort();
    observation.sparse_absent.dedup();
    observation.suppressed.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| format!("{:?}", left.flag).cmp(&format!("{:?}", right.flag)))
    });
    observation.suppressed.dedup();
}

/// The bytes on disk, compared with the index blob. This is the physical
/// observation design §5.1 requires instead of trusting a suppressed status.
fn physical_state(
    info: &RepositoryInfo,
    work_dir: &Path,
    entry: &git2::IndexEntry,
    unknown: &mut Vec<UnknownReason>,
) -> PhysicalState {
    let Some(relative) = byte_path_to_path(&entry.path) else {
        unknown.push(UnknownReason {
            kind: UnknownKind::Unreadable,
            path: Some(entry.path.clone()),
            detail: "the index path is not representable on this platform".to_owned(),
        });
        return PhysicalState::Unobservable;
    };
    let full = work_dir.join(relative);
    let metadata = match std::fs::symlink_metadata(&full) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return PhysicalState::Absent,
        Err(error) => {
            unknown.push(UnknownReason {
                kind: UnknownKind::Unreadable,
                path: Some(entry.path.clone()),
                detail: format!("{}: {error}", full.display()),
            });
            return PhysicalState::Unobservable;
        }
    };
    let format = git_format(info.object_format);
    let file_type = metadata.file_type();
    let hashed = if file_type.is_symlink() {
        match std::fs::read_link(&full) {
            Ok(target) => {
                git2::Oid::hash_object_ext(git2::ObjectType::Blob, path_bytes(&target), format)
            }
            Err(error) => {
                unknown.push(UnknownReason {
                    kind: UnknownKind::Unreadable,
                    path: Some(entry.path.clone()),
                    detail: format!("{}: {error}", full.display()),
                });
                return PhysicalState::Unobservable;
            }
        }
    } else if file_type.is_file() {
        git2::Oid::hash_file_ext(git2::ObjectType::Blob, &full, format)
    } else {
        // A tracked path that is now a directory is not the indexed blob.
        return PhysicalState::Differs;
    };
    let hashed = match hashed {
        Ok(hashed) => hashed,
        Err(error) => {
            unknown.push(UnknownReason {
                kind: UnknownKind::Unreadable,
                path: Some(entry.path.clone()),
                detail: format!("{}: {}", full.display(), error.message()),
            });
            return PhysicalState::Unobservable;
        }
    };
    if hashed != entry.id {
        return PhysicalState::Differs;
    }
    if mode_of(&metadata) != entry.mode {
        return PhysicalState::Differs;
    }
    PhysicalState::MatchesIndex
}

/// The index mode Git would record for this worktree entry.
fn mode_of(metadata: &std::fs::Metadata) -> u32 {
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return 0o120000;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 != 0 {
            return 0o100755;
        }
    }
    0o100644
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> &[u8] {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes()
}

#[cfg(not(unix))]
fn path_bytes(path: &Path) -> &[u8] {
    path.as_os_str().as_encoded_bytes()
}

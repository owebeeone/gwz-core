use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::durable_fs::{rename_durable, sync_dir};
use crate::model::{ErrorCode, ModelError, ModelResult};

use super::super::checked::{StoredV1Record, V1MutationLease};
use super::super::transition::PreparedV1Rewrite;
use super::{CommitFault, unknown};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) fn load_open(root: &Path, merge_id: &str) -> ModelResult<StoredV1Record> {
    validate_merge_id(merge_id)?;
    let root = root.canonicalize().map_err(io_error)?;
    let path = root.join(".gwz/merge").join(format!("{merge_id}.yaml"));
    let bytes = read_regular(&path)?;
    StoredV1Record::from_open_bytes(&root, &path, &bytes)
}

/// Create the durable open record for one accepted start, at the version the
/// contract-§2 writer floor selected.
///
/// A1's writer floor needs a creation owner: `commit` rewrites a record that
/// already exists (it requires a lease-covered base digest), and before the
/// activation nothing in this tree could bring a v1 record into being. This
/// is that owner.
///
/// **R2-E Step E4.2 — the converted creation path (O13's substantive half,
/// ConsumerCheckpoint §10 row `:280`).** The publication is a CHECKED ARTIFACT
/// ACTION now, not this module's own raw `durable_fs` staged/rename/fsync:
/// `entry::create_merge_store_record` replaces an expected-`Missing` leaf,
/// staging, publishing and flushing the managed parent inside the boundary. So
/// this path no longer creates its parent either — `create_temporary`'s
/// `create_dir_all` was the raw when-missing bootstrap of `.gwz/merge`, and row
/// `:273` gives that to the provider, reached through `acquire_for_merge_start`
/// BEFORE this call; an unbootstrapped parent is refused, not papered over. The
/// single-open-record invariant survives: the pre-flight existence check stays
/// and the checked replacement publishes no-replace onto an absent leaf.
/// `commit`'s raw writers are untouched — E4.3's half of O13.
pub(super) fn create_open(
    lease: &V1MutationLease,
    root: &Path,
    record: &crate::workspace_ops::merge::model::v1::MergeOperationRecordV1,
) -> ModelResult<StoredV1Record> {
    validate_merge_id(&record.merge_id)?;
    let root = root.canonicalize().map_err(io_error)?;
    let relative = PathBuf::from(".gwz/merge").join(format!("{}.yaml", record.merge_id));
    let path = root.join(&relative);
    if path_exists(&path)? {
        return Err(recovery(format!(
            "merge record '{}' already exists",
            record.merge_id
        )));
    }

    let raw = serde_yaml::to_value(record).map_err(encode_error)?;
    let encoded = serde_yaml::to_string(&raw)
        .map(String::into_bytes)
        .map_err(encode_error)?;
    crate::checked_artifact::entry::create_merge_store_record(&root, &relative, &encoded)?;

    let published = StoredV1Record::from_open_bytes(&root, &path, &read_regular(&path)?)?;
    if published.record() != record {
        return Err(recovery(
            "checked v1 published record differs from the created record",
        ));
    }
    if !lease.covers(published.location()) {
        return Err(recovery("checked v1 creation is outside its lease"));
    }
    Ok(published)
}

pub(super) fn commit(
    lease: &V1MutationLease,
    current: &StoredV1Record,
    rewrite: PreparedV1Rewrite,
    fault: Option<CommitFault>,
) -> ModelResult<StoredV1Record> {
    if !lease.covers(current.location()) || rewrite.base_digest() != current.source_digest() {
        return Err(recovery(
            "checked v1 rewrite does not match its lease or source digest",
        ));
    }
    let path = current.location().path();
    let source_bytes = read_regular(path)?;
    let reopened = StoredV1Record::from_open_bytes(current.location().root(), path, &source_bytes)?;
    if !current.same_source_as(&reopened) {
        return Err(recovery("checked v1 source bytes changed before commit"));
    }

    rewrite
        .effect()
        .verify_known_diff(current.record(), rewrite.next())?;
    let mut raw = serde_yaml::to_value(rewrite.next()).map_err(encode_error)?;
    let expected_unknown = unknown::overlay(current, rewrite.effect(), &mut raw)?;
    let encoded = serde_yaml::to_string(&raw)
        .map(String::into_bytes)
        .map_err(encode_error)?;
    let (temporary, mut file) = create_temporary(path)?;
    let staged_write = file.write_all(&encoded).and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = staged_write {
        let _ = fs::remove_file(&temporary);
        return Err(io_error(error));
    }
    let staged = match read_regular(&temporary).and_then(|bytes| {
        let staged = StoredV1Record::from_open_bytes(current.location().root(), path, &bytes)?;
        require_expected(&staged, rewrite.next(), &expected_unknown)?;
        if bytes != encoded {
            return Err(recovery(
                "checked v1 temporary bytes changed after serialization",
            ));
        }
        Ok(staged)
    }) {
        Ok(staged) => staged,
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
    };
    if fault == Some(CommitFault::AfterTemporarySync) {
        let _ = fs::remove_file(&temporary);
        return Err(recovery(
            "injected checked-store fault after temporary sync",
        ));
    }
    if let Err(error) = rename_durable(&temporary, path, true) {
        let _ = fs::remove_file(&temporary);
        return Err(io_error(error));
    }
    if fault == Some(CommitFault::AfterPublish) {
        return Err(recovery("injected checked-store fault after publication"));
    }
    sync_dir(path.parent().expect("open record has a parent")).map_err(io_error)?;

    let published_bytes = read_regular(path)?;
    let published =
        StoredV1Record::from_open_bytes(current.location().root(), path, &published_bytes)?;
    require_expected(&published, rewrite.next(), &expected_unknown)?;
    if !staged.same_source_as(&published) {
        return Err(recovery(
            "checked v1 published bytes differ from the synced temporary",
        ));
    }
    Ok(published)
}

fn require_expected(
    actual: &StoredV1Record,
    expected_record: &crate::workspace_ops::merge::model::v1::MergeOperationRecordV1,
    expected_unknown: &crate::workspace_ops::merge::record_wire::UnknownFieldManifest,
) -> ModelResult<()> {
    if actual.record() == expected_record && actual.unknown_fields() == expected_unknown {
        Ok(())
    } else {
        Err(recovery(
            "checked v1 canonical model or unknown manifest changed during commit",
        ))
    }
}

fn create_temporary(path: &Path) -> ModelResult<(PathBuf, File)> {
    let parent = path
        .parent()
        .ok_or_else(|| recovery("open record path has no parent"))?;
    fs::create_dir_all(parent).map_err(io_error)?;
    loop {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let candidate =
            path.with_extension(format!("yaml.{}.{}.v1.tmp", std::process::id(), sequence));
        match File::options()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(io_error(error)),
        }
    }
}

pub(super) fn read_regular(path: &Path) -> ModelResult<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if !metadata.file_type().is_file() {
        return Err(unreadable(format!(
            "record path '{}' is not a regular file",
            path.display()
        )));
    }
    if path.canonicalize().map_err(io_error)? != path {
        return Err(unreadable(format!(
            "record path '{}' traverses a symbolic link",
            path.display()
        )));
    }
    fs::read(path).map_err(io_error)
}

pub(super) fn validate_merge_id(merge_id: &str) -> ModelResult<()> {
    if merge_id.is_empty()
        || matches!(merge_id, "." | "..")
        || !merge_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(recovery(format!("invalid merge record id '{merge_id}'")));
    }
    Ok(())
}

pub(super) fn path_exists(path: &Path) -> ModelResult<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error(error)),
    }
}

pub(super) fn io_error(error: io::Error) -> ModelError {
    ModelError::new(ErrorCode::IoError, error.to_string())
}

fn encode_error(error: serde_yaml::Error) -> ModelError {
    ModelError::new(ErrorCode::InternalError, error.to_string())
}

pub(super) fn recovery(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecoveryRequired, detail)
}

fn unreadable(detail: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecordUnreadable, detail)
}

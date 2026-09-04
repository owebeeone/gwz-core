//! The archived merge record's reader, retention sweep and collector.
//!
//! **M5d (`GwzM5-8M5d-Charter.md` §1, §5).** Until this milestone this module
//! was the v0 merge record *store*: it created, rewrote, transitioned and
//! archived the open v0 record, and every gate in the tree discovered an open
//! merge by decoding a v0 body through it. That engine is deleted. What
//! remains is what charter §5 retains as production — "The I2 §7 archive
//! decoder and GC projection **remain production**": reading a `done/` record
//! of **either** envelope, sweeping retention, and collecting one archive.
//!
//! Open-record occupancy moved to `merge::open_record`, which classifies by
//! header and never decodes a v0 body (charter §2). The v1 lifecycle owns its
//! own writer (`v1_lifecycle/store/`), published through the checked door.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_yaml::Value;

use super::record_wire::MergeOperationRecordV0;
use crate::durable_fs::sync_dir;
use crate::model::{ErrorCode, ModelError, ModelResult};

mod compatibility_errors;
mod gc;
mod retention;

pub(super) use compatibility_errors::location_unreadable;
use compatibility_errors::{archived_contradiction, decode_error, record_context};

pub(super) const MERGE_DIR: &str = ".gwz/merge";
const DONE_DIR: &str = ".gwz/merge/done";
const ORDINARY_RETENTION: usize = 20;

/// The archive-collection seam.
///
/// I0 froze a six-method persistence trait here. Five of those methods —
/// `discover_open`, `load`, `load_archived`, `write_open`, `archive` — served
/// the v0 lifecycle and left with it (charter §1). `gc` is the one that is
/// still a product: `GwzM5-8I2CompatibilityContract.md` §7, which charter §7
/// retains and does not amend.
pub(crate) trait MergeStore {
    fn gc(&self, _root: &Path, _merge_id: Option<&str>) -> ModelResult<()> {
        Err(ModelError::new(
            ErrorCode::UnsupportedOperation,
            "merge store method 'gc' is not implemented",
        ))
    }
}

/// Filesystem implementation of the retained collection seam.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FileMergeStore;

impl MergeStore for FileMergeStore {
    fn gc(&self, root: &Path, merge_id: Option<&str>) -> ModelResult<()> {
        gc::collect(root, merge_id)
    }
}

pub(super) fn open_path(root: &Path, merge_id: &str) -> PathBuf {
    root.join(MERGE_DIR).join(format!("{merge_id}.yaml"))
}

fn done_path(root: &Path, merge_id: &str) -> PathBuf {
    root.join(DONE_DIR).join(format!("{merge_id}.yaml"))
}

pub(super) fn validate_merge_id(merge_id: &str) -> ModelResult<()> {
    if merge_id.is_empty()
        || matches!(merge_id, "." | "..")
        || !merge_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(recovery_error(format!(
            "invalid merge record id '{merge_id}'"
        )));
    }
    Ok(())
}

/// The typed refusal for an open record whose envelope no installed lifecycle
/// owns (v2-v4, or an unknown schema). Charter §2's own refusal for an open
/// **v0** envelope is `open_record::pre_014_merge_error` and is not this.
pub(super) fn unsupported_open_envelope(
    merge_id: &str,
    header: &super::record_wire::MergeRecordHeader,
) -> ModelError {
    compatibility_errors::unsupported_open_record(merge_id, header)
}

/// Map a v1 open-record decode failure onto the store's own error vocabulary,
/// so an unreadable open record answers the same way it always has.
pub(super) fn open_decode_error(
    path: &Path,
    merge_id: &str,
    error: super::record_wire::RecordDecodeError,
) -> ModelError {
    decode_error(path, merge_id, RecordLocation::Open, error)
}

#[derive(Clone, Copy)]
pub(super) enum RecordLocation {
    Open,
    Archived,
}

/// Read one **archived** record over both installed envelopes, projected onto
/// the half a v0 and a v1 archive hold identically.
///
/// This is I2 §7's decoder in its retention/GC role: the v0 arm is the whole
/// reason `MergeOperationRecordV0` still exists (charter §5), and the v1 arm
/// keeps a v1 archive classifiable so it can be swept and collected.
fn read_archived_record(path: &Path) -> ModelResult<(Value, MergeOperationRecordV0)> {
    let location = RecordLocation::Archived;
    let merge_id = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| unreadable(Some(path), "record file name is not valid UTF-8"))?;
    if !fs::symlink_metadata(path)
        .map_err(|error| location_unreadable(path, merge_id, location, error))?
        .file_type()
        .is_file()
    {
        return Err(location_unreadable(
            path,
            merge_id,
            location,
            "record path is not a regular file",
        ));
    }
    let bytes =
        fs::read(path).map_err(|error| location_unreadable(path, merge_id, location, error))?;
    let (raw, header, record) = super::record_wire::decode_archived_common(&bytes)
        .map_err(|error| decode_error(path, merge_id, location, error))?;
    validate_merge_id(&record.merge_id)?;
    if path.file_stem().and_then(|value| value.to_str()) != Some(record.merge_id.as_str()) {
        return Err(archived_contradiction(merge_id, &header));
    }
    if record.state.is_open() {
        return Err(archived_contradiction(merge_id, &header));
    }
    let _ = record_context(merge_id, &header, None);
    Ok((raw, record))
}

fn path_exists(path: &Path) -> ModelResult<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(unreadable(Some(path), error)),
    }
}

pub(super) fn record_files(directory: &Path) -> ModelResult<Vec<PathBuf>> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(unreadable(Some(directory), error)),
    };
    let mut records = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| unreadable(Some(directory), error))?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) == Some("yaml") {
            records.push(path);
        }
    }
    records.sort();
    Ok(records)
}

fn unreadable(path: Option<&Path>, reason: impl std::fmt::Display) -> ModelError {
    let location = path.map_or_else(
        || "merge record".to_owned(),
        |path| path.display().to_string(),
    );
    ModelError::new(
        ErrorCode::MergeRecordUnreadable,
        format!("merge record at '{location}' is unreadable: {reason}"),
    )
}

fn recovery_error(message: impl Into<String>) -> ModelError {
    ModelError::new(ErrorCode::MergeRecoveryRequired, message)
}

fn io_error(error: io::Error) -> ModelError {
    ModelError::new(ErrorCode::IoError, error.to_string())
}

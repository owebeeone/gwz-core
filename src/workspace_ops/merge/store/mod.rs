use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_yaml::Value;

use super::{MERGE_RECORD_SCHEMA, MERGE_RECORD_SCHEMA_VERSION, MergeOperationRecord};
use crate::durable_fs::{rename_durable, sync_dir};
use crate::model::{ErrorCode, ModelError, ModelResult};

mod archived;
mod gc;
mod persistence;
mod retention;

pub(crate) use persistence::{
    archive_merge_record, enter_finalizing, persist_merge_record, persist_operation_transition,
};

const MERGE_DIR: &str = ".gwz/merge";
const DONE_DIR: &str = ".gwz/merge/done";
const ORDINARY_RETENTION: usize = 20;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Persistence seam frozen at I0 and extended in M3 with exact archived loads
/// for id-qualified status and checked cleanup.
pub(crate) trait MergeStore {
    fn discover_open(&self, _root: &Path) -> ModelResult<Option<MergeOperationRecord>> {
        unsupported_store("discover_open")
    }
    fn load(&self, _root: &Path, _merge_id: &str) -> ModelResult<MergeOperationRecord> {
        unsupported_store("load")
    }
    fn load_archived(&self, _root: &Path, _merge_id: &str) -> ModelResult<MergeOperationRecord> {
        unsupported_store("load_archived")
    }
    fn write_open(&self, _root: &Path, _record: &MergeOperationRecord) -> ModelResult<()> {
        unsupported_store("write_open")
    }
    fn archive(&self, _root: &Path, _merge_id: &str) -> ModelResult<()> {
        unsupported_store("archive")
    }
    fn gc(&self, _root: &Path, _merge_id: Option<&str>) -> ModelResult<()> {
        unsupported_store("gc")
    }
}

fn unsupported_store<T>(method: &str) -> ModelResult<T> {
    Err(ModelError::new(
        ErrorCode::UnsupportedOperation,
        format!("merge store method '{method}' is not implemented"),
    ))
}

/// Filesystem implementation of the frozen merge persistence seam.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FileMergeStore;

impl MergeStore for FileMergeStore {
    fn discover_open(&self, root: &Path) -> ModelResult<Option<MergeOperationRecord>> {
        let records = record_files(&root.join(MERGE_DIR))?;
        match records.as_slice() {
            [] => Ok(None),
            [path] => read_record(path).map(|(_, record)| Some(record)),
            _ => Err(ModelError::new(
                ErrorCode::MergeRecoveryRequired,
                format!(
                    "multiple merge records exist under '{}'",
                    root.join(MERGE_DIR).display()
                ),
            )),
        }
    }

    fn load(&self, root: &Path, merge_id: &str) -> ModelResult<MergeOperationRecord> {
        validate_merge_id(merge_id)?;
        for path in [open_path(root, merge_id), done_path(root, merge_id)] {
            if path_exists(&path)? {
                return read_record(&path).map(|(_, record)| record);
            }
        }
        Err(ModelError::new(
            ErrorCode::OperationNotFound,
            format!("merge record '{merge_id}' was not found"),
        ))
    }

    fn load_archived(&self, root: &Path, merge_id: &str) -> ModelResult<MergeOperationRecord> {
        archived::load(root, merge_id)
    }

    fn write_open(&self, root: &Path, record: &MergeOperationRecord) -> ModelResult<()> {
        validate_record(record, None)?;
        let path = open_path(root, &record.merge_id);
        for existing in record_files(&root.join(MERGE_DIR))? {
            if existing != path {
                return Err(ModelError::new(
                    ErrorCode::OpenOperation,
                    format!(
                        "another merge record already exists at '{}'",
                        existing.display()
                    ),
                ));
            }
        }

        let mut next = serde_yaml::to_value(record).map_err(encode_error)?;
        if path_exists(&path)? {
            let (old_raw, old_record) = read_record(&path)?;
            let old_known = serde_yaml::to_value(old_record).map_err(encode_error)?;
            carry_unknown(&old_raw, &old_known, &mut next);
        }
        let encoded = serde_yaml::to_string(&next).map_err(encode_error)?;
        write_atomic_verified(&path, encoded.as_bytes())?;
        let (_, verified) = read_record(&path)?;
        if verified != *record {
            return Err(recovery_error(format!(
                "merge record verification failed at '{}'",
                path.display()
            )));
        }
        Ok(())
    }

    fn archive(&self, root: &Path, merge_id: &str) -> ModelResult<()> {
        archived::archive(root, merge_id)
    }

    fn gc(&self, root: &Path, merge_id: Option<&str>) -> ModelResult<()> {
        gc::collect(root, merge_id)
    }
}

fn ensure_terminal_for_archive(record: &MergeOperationRecord) -> ModelResult<()> {
    if record.state.is_open() {
        return Err(recovery_error(format!(
            "cannot archive open merge record '{}' in state {:?}",
            record.merge_id, record.state
        )));
    }
    Ok(())
}

fn open_path(root: &Path, merge_id: &str) -> PathBuf {
    root.join(MERGE_DIR).join(format!("{merge_id}.yaml"))
}

fn done_path(root: &Path, merge_id: &str) -> PathBuf {
    root.join(DONE_DIR).join(format!("{merge_id}.yaml"))
}

fn validate_merge_id(merge_id: &str) -> ModelResult<()> {
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

fn validate_record(record: &MergeOperationRecord, path: Option<&Path>) -> ModelResult<()> {
    validate_merge_id(&record.merge_id)?;
    if record.schema != MERGE_RECORD_SCHEMA
        || record.record_schema_version != MERGE_RECORD_SCHEMA_VERSION
    {
        return Err(unreadable(path, "unsupported merge record schema"));
    }
    if let Some(path) = path {
        let expected = path.file_stem().and_then(|value| value.to_str());
        if expected != Some(record.merge_id.as_str()) {
            return Err(unreadable(
                path.into(),
                "record id does not match its file name",
            ));
        }
    }
    Ok(())
}

fn read_record(path: &Path) -> ModelResult<(Value, MergeOperationRecord)> {
    if !fs::symlink_metadata(path)
        .map_err(|error| unreadable(Some(path), error))?
        .file_type()
        .is_file()
    {
        return Err(unreadable(Some(path), "record path is not a regular file"));
    }
    let bytes = fs::read(path).map_err(|error| unreadable(Some(path), error))?;
    let raw: Value = serde_yaml::from_slice(&bytes)
        .map_err(|error| unreadable(Some(path), format!("invalid YAML: {error}")))?;
    let record: MergeOperationRecord = serde_yaml::from_value(raw.clone())
        .map_err(|error| unreadable(Some(path), format!("invalid record: {error}")))?;
    validate_record(&record, Some(path))?;
    Ok((raw, record))
}

fn path_exists(path: &Path) -> ModelResult<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(unreadable(Some(path), error)),
    }
}

fn record_files(directory: &Path) -> ModelResult<Vec<PathBuf>> {
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
            if !entry
                .file_type()
                .map_err(|error| unreadable(Some(&path), error))?
                .is_file()
            {
                return Err(unreadable(Some(&path), "record path is not a regular file"));
            }
            records.push(path);
        }
    }
    records.sort();
    Ok(records)
}

fn write_atomic_verified(path: &Path, bytes: &[u8]) -> ModelResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| recovery_error("record path has no parent"))?;
    fs::create_dir_all(parent).map_err(io_error)?;
    let (temporary, mut file) = loop {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let candidate =
            path.with_extension(format!("yaml.{}.{}.tmp", std::process::id(), sequence));
        match File::options()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => break (candidate, file),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(io_error(error)),
        }
    };
    let staged = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = staged {
        let _ = fs::remove_file(&temporary);
        return Err(io_error(error));
    }
    if let Err(error) = rename_durable(&temporary, path, true) {
        let _ = fs::remove_file(&temporary);
        return Err(io_error(error));
    }
    sync_dir(parent).map_err(io_error)?;
    if fs::read(path).map_err(io_error)? != bytes {
        return Err(recovery_error(
            "merge record bytes failed write verification",
        ));
    }
    Ok(())
}

/// Overlay new known state while retaining fields the old serde model did not know.
fn carry_unknown(old_raw: &Value, old_known: &Value, new: &mut Value) {
    match (old_raw, old_known, new) {
        (Value::Mapping(raw), Value::Mapping(known), Value::Mapping(next)) => {
            for (key, raw_value) in raw {
                match (known.get(key), next.get_mut(key)) {
                    (None, None) => {
                        next.insert(key.clone(), raw_value.clone());
                    }
                    (Some(known_value), Some(next_value)) => {
                        carry_unknown(raw_value, known_value, next_value);
                    }
                    _ => {}
                }
            }
        }
        (Value::Sequence(raw), Value::Sequence(known), Value::Sequence(next)) => {
            for (index, next_value) in next.iter_mut().enumerate() {
                if let (Some(raw_value), Some(known_value)) = (raw.get(index), known.get(index)) {
                    carry_unknown(raw_value, known_value, next_value);
                }
            }
        }
        _ => {}
    }
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

fn encode_error(error: serde_yaml::Error) -> ModelError {
    ModelError::new(
        ErrorCode::InternalError,
        format!("failed to encode merge record: {error}"),
    )
}

fn io_error(error: io::Error) -> ModelError {
    ModelError::new(ErrorCode::IoError, error.to_string())
}

#[cfg(test)]
mod tests;

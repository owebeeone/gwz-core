use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

use serde_yaml::Value;

use super::{MERGE_RECORD_SCHEMA, MERGE_RECORD_SCHEMA_VERSION, MergeOperationRecord, MergeStore};
use crate::model::{ErrorCode, ModelError, ModelResult};

const MERGE_DIR: &str = ".gwz/merge";
const DONE_DIR: &str = ".gwz/merge/done";
const ORDINARY_RETENTION: usize = 20;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
        validate_merge_id(merge_id)?;
        let source = open_path(root, merge_id);
        let destination = done_path(root, merge_id);
        if !path_exists(&source)? {
            if path_exists(&destination)? {
                let _ = read_record(&destination)?;
                return enforce_retention(root);
            }
            return Err(ModelError::new(
                ErrorCode::OperationNotFound,
                format!("merge record '{merge_id}' was not found"),
            ));
        }
        let (source_raw, record) = read_record(&source)?;
        if record.state.is_open() {
            return Err(recovery_error(format!(
                "cannot archive open merge record '{merge_id}' in state {:?}",
                record.state
            )));
        }
        fs::create_dir_all(root.join(DONE_DIR)).map_err(io_error)?;
        if path_exists(&destination)? {
            let (archived_raw, archived) = read_record(&destination)?;
            if archived != record || archived_raw != source_raw {
                return Err(recovery_error(format!(
                    "archived merge record '{merge_id}' does not match the open record"
                )));
            }
            fs::remove_file(&source).map_err(io_error)?;
        } else {
            fs::rename(&source, &destination).map_err(io_error)?;
        }
        sync_dir(&root.join(MERGE_DIR))?;
        sync_dir(&root.join(DONE_DIR))?;
        let (_, verified) = read_record(&destination)?;
        if verified != record {
            return Err(recovery_error(format!(
                "archived merge record '{merge_id}' failed verification"
            )));
        }
        enforce_retention(root)
    }

    fn gc(&self, _root: &Path, _merge_id: Option<&str>) -> ModelResult<()> {
        Err(ModelError::new(
            ErrorCode::MergePhaseUnsupported,
            "merge record GC is not available",
        ))
    }
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
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(io_error(error));
    }
    sync_dir(parent)?;
    if fs::read(path).map_err(io_error)? != bytes {
        return Err(recovery_error(
            "merge record bytes failed write verification",
        ));
    }
    Ok(())
}

fn sync_dir(path: &Path) -> ModelResult<()> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(io_error)
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

fn enforce_retention(root: &Path) -> ModelResult<()> {
    let mut ordinary = Vec::new();
    for path in record_files(&root.join(DONE_DIR))? {
        let Ok((_, record)) = read_record(&path) else {
            continue; // Unknown/corrupt archives may own evidence: fail safe by retaining them.
        };
        if record
            .participants
            .values()
            .any(|participant| !participant.preservation.is_empty())
        {
            continue;
        }
        let modified = fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |duration| duration.as_nanos());
        ordinary.push((modified, path));
    }
    ordinary.sort_by(|left, right| right.cmp(left));
    for (_, path) in ordinary.into_iter().skip(ORDINARY_RETENTION) {
        fs::remove_file(path).map_err(io_error)?;
    }
    sync_dir(&root.join(DONE_DIR))
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
mod tests {
    use super::*;
    use crate::workspace_ops::merge::{
        MergeParticipantRecord, OperationState, PublicationProgress, PublicationStep,
    };
    use crate::workspace_ops::tests::TempDir;

    fn temp(name: &str) -> TempDir {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "gwz-merge-{name}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        TempDir { path }
    }

    fn record(id: &str, state: OperationState) -> MergeOperationRecord {
        let mut record: MergeOperationRecord = serde_yaml::from_str(
            r#"{schema: gwz.merge-operation/v0, record_schema_version: 0, writer_version: test, workspace_id: ws_test, merge_id: merge_test, operation_id: op_test, state: executing, source_ref: feature/x, created_at: now, baseline: {lock_sha256: lock, manifest_sha256: manifest}, selected_targets: [], participants: {}}"#,
        )
        .unwrap();
        record.merge_id = id.to_owned();
        record.state = state;
        record
    }

    fn preservation_participant() -> MergeParticipantRecord {
        serde_yaml::from_str(
            r#"{path: app, target_kind: member, target_branch: main, before_commit: '111', source_commit: '222', commit_message: merge, state: aborted, preservation: [{backup_ref: refs/gwz/merge/kept/app/head, backup_commit: '333'}]}"#,
        )
        .unwrap()
    }

    #[test]
    fn write_load_discover_and_unknown_round_trip() {
        let temp = temp("merge-store-roundtrip");
        let store = FileMergeStore;
        let mut expected = record("merge_1", OperationState::Executing);
        expected.publication = Some(PublicationProgress {
            step: PublicationStep::NotStarted,
            candidate_lock_sha256: Some("candidate".to_owned()),
            candidate_marker_path: None,
            root_merge_commit: None,
            composition_commit: None,
        });
        store.write_open(&temp.path, &expected).unwrap();
        assert_eq!(store.load(&temp.path, "merge_1").unwrap(), expected);

        let path = open_path(&temp.path, "merge_1");
        let mut raw: Value = serde_yaml::from_slice(&fs::read(&path).unwrap()).unwrap();
        raw["publication"].as_mapping_mut().unwrap().insert(
            Value::String("future_publication".to_owned()),
            Value::String("retained".to_owned()),
        );
        fs::write(&path, serde_yaml::to_string(&raw).unwrap()).unwrap();
        expected = store.discover_open(&temp.path).unwrap().unwrap();
        expected.state = OperationState::Halted;
        expected.publication.as_mut().unwrap().candidate_lock_sha256 = None;
        store.write_open(&temp.path, &expected).unwrap();
        let rewritten = fs::read_to_string(path).unwrap();
        assert!(rewritten.contains("future_publication: retained"));
        let rewritten_value: Value = serde_yaml::from_str(&rewritten).unwrap();
        assert!(rewritten_value["publication"]["candidate_lock_sha256"].is_null());
    }

    #[test]
    fn corrupt_open_records_fail_closed() {
        let temp = temp("merge-store-corrupt");
        let directory = temp.path.join(MERGE_DIR);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("merge_bad.yaml"), "not: [valid").unwrap();
        assert_eq!(
            FileMergeStore.discover_open(&temp.path).unwrap_err().code,
            ErrorCode::MergeRecordUnreadable
        );
    }

    #[test]
    fn archive_retention_keeps_preservation_owners() {
        let temp = temp("merge-store-archive");
        let store = FileMergeStore;
        let open = record("merge_open", OperationState::Executing);
        store.write_open(&temp.path, &open).unwrap();
        assert_eq!(
            store.archive(&temp.path, "merge_open").unwrap_err().code,
            ErrorCode::MergeRecoveryRequired
        );
        fs::remove_file(open_path(&temp.path, "merge_open")).unwrap();
        for index in 0..22 {
            let closed = record(&format!("merge_{index:02}"), OperationState::Completed);
            store.write_open(&temp.path, &closed).unwrap();
            store.archive(&temp.path, &closed.merge_id).unwrap();
        }
        let mut kept = record("merge_kept", OperationState::Aborted);
        kept.selected_targets.push("mem_app".to_owned());
        kept.participants
            .insert("mem_app".to_owned(), preservation_participant());
        store.write_open(&temp.path, &kept).unwrap();
        store.archive(&temp.path, &kept.merge_id).unwrap();

        assert_eq!(record_files(&temp.path.join(DONE_DIR)).unwrap().len(), 21);
        assert!(done_path(&temp.path, "merge_kept").is_file());
        assert!(store.discover_open(&temp.path).unwrap().is_none());
        assert_eq!(store.load(&temp.path, "merge_kept").unwrap(), kept);
    }

    #[test]
    fn failed_atomic_publish_removes_its_temporary_file() {
        let temp = temp("merge-store-atomic-fault");
        let target = temp.path.join("record.yaml");
        fs::create_dir(&target).unwrap();
        assert_eq!(
            write_atomic_verified(&target, b"record").unwrap_err().code,
            ErrorCode::IoError
        );
        assert_eq!(fs::read_dir(&temp.path).unwrap().count(), 1);
        assert_eq!(
            FileMergeStore.gc(Path::new("."), None).unwrap_err().code,
            ErrorCode::MergePhaseUnsupported
        );
    }
}

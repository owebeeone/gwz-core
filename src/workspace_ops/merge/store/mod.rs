use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_yaml::Value;

use super::MergeOperationRecord;
use super::record_wire::decode_production_v0;
use crate::durable_fs::{rename_durable, sync_dir};
use crate::model::{ErrorCode, ModelError, ModelResult};

mod archived;
mod atomic_upgrade;
mod compatibility_errors;
mod gc;
mod persistence;
mod retention;

pub(crate) use persistence::{
    archive_merge_record, enter_finalizing, persist_merge_record, persist_operation_transition,
};

pub(crate) use atomic_upgrade::{AtomicUpgradeFault, AtomicUpgradeOutcome, upgrade_open_v0};

use compatibility_errors::{
    archived_contradiction, decode_error, location_unreadable, record_context,
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
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "R1-frozen archived-load compatibility seam remains until the A1 activation review"
        )
    )]
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
        let paths = record_files(&root.join(MERGE_DIR))?;
        let mut records = Vec::with_capacity(paths.len());
        for path in paths {
            records.push(read_record(&path, RecordLocation::Open)?.1);
        }
        match records.len() {
            0 => Ok(None),
            1 => Ok(records.pop()),
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
        for (path, location) in [
            (open_path(root, merge_id), RecordLocation::Open),
            (done_path(root, merge_id), RecordLocation::Archived),
        ] {
            if path_exists(&path)? {
                return read_record(&path, location).map(|(_, record)| record);
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
        if !path_exists(&path)? {
            crate::checked_artifact::entry::prepare_merge_store_parents(root)?;
        }

        let mut next = serde_yaml::to_value(record).map_err(encode_error)?;
        if path_exists(&path)? {
            let (old_raw, old_record) = read_record(&path, RecordLocation::Open)?;
            let old_known = serde_yaml::to_value(old_record).map_err(encode_error)?;
            carry_unknown(&old_raw, &old_known, &mut next);
        }
        let encoded = serde_yaml::to_string(&next).map_err(encode_error)?;
        write_atomic_verified(&path, encoded.as_bytes())?;
        let (_, verified) = read_record(&path, RecordLocation::Open)?;
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

/// One open record found by its envelope alone.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OpenRecordEnvelope {
    pub(crate) root: PathBuf,
    pub(crate) merge_id: String,
    pub(crate) version: super::RecordVersion,
    /// [P2-1]'s cheap state pre-classification for open v0 rows.
    pub(crate) adaptation: AdaptationPrecheck,
}

/// Whether an open v0 record may reach the A1 adaptation preflight at all.
///
/// A1 Safety [P2-1] / §4.2. The adapter's order is envelope -> legacy-mode
/// check ->
/// `validate_v0_structure` -> `classify_open_v0`, so the substantial typed
/// refusal surface of the structural validator runs BEFORE the cheap state
/// pre-classification that would answer `ValidUnlisted` anyway. Two open v0
/// progress shapes a pre-A1 binary's crash can leave on disk
/// (`B-NOT-STARTED`, `B-PREPARING-EMPTY`) carry zero fixtures, so whether
/// they survive that validator is unmeasured.
///
/// This is condition (i) of the finding: the dispatch gates adaptation on the
/// pre-classification, so a non-`Finalizing` or non-normal-mode open v0 row
/// takes the v0 path by construction and never reaches
/// `validate_v0_structure` through the new path. It is contract-compliant
/// because only one-member `Finalizing` normal-mode shapes are whitelisted at
/// all (contract §4), so nothing this skips could have migrated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AdaptationPrecheck {
    /// A `Finalizing`, normal-mode v0 row: the only shape class the
    /// migration whitelist can match. It may enter the preflight.
    MayAdapt,
    /// Every other row, and every v1 record. Skip the preflight entirely.
    Skip,
}

/// Classify the single open record under `root`: the envelope selects the
/// decoder, and that decoder then decodes the body.
///
/// A1 (Safety review §2.2 R3 / §2.4): the dispatch must know which lifecycle
/// owns a record before it hands the record to one, and the v0 store's own
/// decoder installs v0 only — an open v1 record is not readable there.
///
/// [P3-R2-5]: the header decides the dispatch, but the selected arm runs the
/// full body decode — the v0 arm uses the very decoder the store uses, which
/// is what makes the `Finalizing`/normal-mode pre-check below readable
/// straight off the decoded record. What the contract-§1 ordering buys is
/// unchanged: header validation precedes status eligibility, migration,
/// record rewrite, archive deletion, and all Git/filesystem mutation, so
/// classifying here commits to nothing.
pub(crate) fn classify_open_record(root: &Path) -> ModelResult<Option<OpenRecordEnvelope>> {
    let directory = root.join(MERGE_DIR);
    let paths = record_files(&directory)?;
    let mut found: Vec<OpenRecordEnvelope> = Vec::with_capacity(paths.len());
    for path in paths {
        let merge_id = path
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or_else(|| unreadable(Some(&path), "record file name is not valid UTF-8"))?;
        let bytes = fs::read(&path)
            .map_err(|error| location_unreadable(&path, merge_id, RecordLocation::Open, error))?;
        let (version, adaptation) = match super::record_wire::decode_production(&bytes) {
            Ok(super::record_wire::DecodedRecord::V0(decoded)) => {
                (super::RecordVersion::V0, precheck(decoded.record()))
            }
            Ok(super::record_wire::DecodedRecord::V1(_)) => {
                (super::RecordVersion::V1, AdaptationPrecheck::Skip)
            }
            Err(error) => {
                return Err(decode_error(&path, merge_id, RecordLocation::Open, error));
            }
        };
        found.push(OpenRecordEnvelope {
            root: root.to_path_buf(),
            merge_id: merge_id.to_owned(),
            version,
            adaptation,
        });
    }
    match found.len() {
        0 => Ok(None),
        1 => Ok(found.pop()),
        _ => Err(ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            format!(
                "multiple merge records exist under '{}'",
                directory.display()
            ),
        )),
    }
}

/// [P2-1]'s pre-check, read from the already-decoded v0 body. It touches only
/// two scalar fields and calls no validator.
fn precheck(record: &MergeOperationRecord) -> AdaptationPrecheck {
    if record.state == super::OperationState::Finalizing
        && record.mode == super::MergeExecutionMode::Normal
    {
        AdaptationPrecheck::MayAdapt
    } else {
        AdaptationPrecheck::Skip
    }
}

/// Search ancestors for an open record of any installed version, before
/// parsing the manifest or lock — the envelope-aware twin of
/// `recovery::discover_open_before_manifest`.
pub(crate) fn discover_open_envelope_before_manifest(
    start: &Path,
) -> ModelResult<Option<OpenRecordEnvelope>> {
    let mut current = if start.is_file() {
        start.parent().unwrap_or(start).to_path_buf()
    } else {
        start.to_path_buf()
    };
    loop {
        if current.join(MERGE_DIR).try_exists().unwrap_or(true)
            && let Some(found) = classify_open_record(&current)?
        {
            return Ok(Some(found));
        }
        if current
            .join(crate::workspace::WORKSPACE_MANIFEST)
            .try_exists()
            .unwrap_or(true)
        {
            return Ok(None);
        }
        if !current.pop() {
            return Ok(None);
        }
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

/// Envelope validation for the v0 record store.
///
/// A1 replaced the hard-coded v0-only pair test that stood here
/// (`schema != MERGE_RECORD_SCHEMA || record_schema_version !=
/// MERGE_RECORD_SCHEMA_VERSION -> unreadable`) with the compatibility
/// contract §1 envelope registry: `classify_merge_record_header` owns which
/// pairs exist, which are installed, and what an uninstalled or unknown pair
/// projects. This store owns v0 bodies only, so it classifies against the
/// v0-only installed set — a v1 envelope is not "unreadable", it is *not
/// this store's record*, and `runtime::dispatch` routes it to the v1
/// lifecycle before this store is reached (Safety review §2.2 R3).
fn validate_record(record: &MergeOperationRecord, path: Option<&Path>) -> ModelResult<()> {
    validate_merge_id(&record.merge_id)?;
    let header = super::record_wire::MergeRecordHeader {
        schema: record.schema.clone(),
        record_schema_version: record.record_schema_version,
    };
    match super::record_wire::classify_merge_record_header(
        &header,
        super::record_wire::InstalledMergeRecordVersions::V0_ONLY,
    ) {
        Ok(super::record_wire::MergeRecordDispatch::V0) => {}
        Ok(super::record_wire::MergeRecordDispatch::V1) => {
            unreachable!("the v0-only installed set never dispatches v1")
        }
        Err(_) => return Err(unreadable(path, "unsupported merge record schema")),
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

#[derive(Clone, Copy)]
enum RecordLocation {
    Open,
    Archived,
}

fn read_record(
    path: &Path,
    location: RecordLocation,
) -> ModelResult<(Value, MergeOperationRecord)> {
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
    let decoded = decode_production_v0(&bytes)
        .map_err(|error| decode_error(path, merge_id, location, error))?;
    let (raw, header, record) = decoded.into_production_parts();
    if let Err(error) = validate_record(&record, Some(path)) {
        return Err(match location {
            RecordLocation::Open => {
                error.with_record_context(record_context(merge_id, &header, None))
            }
            RecordLocation::Archived => archived_contradiction(merge_id, &header),
        });
    }
    if matches!(location, RecordLocation::Archived) && record.state.is_open() {
        return Err(archived_contradiction(merge_id, &header));
    }
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

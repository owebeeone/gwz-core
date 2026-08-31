use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(test)]
use std::sync::atomic::AtomicBool;

use crate::model::{self, ErrorCode, ModelError};

use super::super::OperationRequest;
use super::coalesce::{CommitLogGroup, CommitLogProvenance};
use super::merge::{CommitLogMergedEvent, stream_request_histories};
use super::open_request_histories;
use super::{CommitLogDegradation, CommitLogDegradationKind, CommitLogIdentity};

const DEFAULT_READ_RECORDS: usize = 128;
const MAX_READ_RECORDS: usize = 1_024;
const RECORD_PREFIX_BYTES: u64 = 8;

/// One bounded read from an operation-scoped commit-log output spool.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CommitLogReadRequest {
    /// Opaque resume cursor returned by the preceding read; absent starts at 0.
    pub cursor: Option<u64>,
    /// Maximum records to return. Absent defaults to 128; 1,024 is the ceiling.
    pub max_records: Option<u32>,
}

/// Terminal state of one commit-log output read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitLogReadState {
    Data,
    Eof,
}

/// Records plus the opaque cursor for the next bounded read.
#[derive(Clone, Debug, PartialEq)]
pub struct CommitLogReadResponse {
    pub records: Vec<crate::LogOutputRecord>,
    pub next_cursor: u64,
    pub state: CommitLogReadState,
}

struct OutputSpool {
    file: File,
    length: u64,
    sealed: bool,
    #[cfg(test)]
    fail_append: bool,
    #[cfg(test)]
    fail_seal: bool,
}

impl OutputSpool {
    fn new(file: File, #[cfg(test)] fail_append: bool, #[cfg(test)] fail_seal: bool) -> Self {
        Self {
            file,
            length: 0,
            sealed: false,
            #[cfg(test)]
            fail_append,
            #[cfg(test)]
            fail_seal,
        }
    }

    fn append(&mut self, record: &crate::LogOutputRecord) -> model::ModelResult<()> {
        #[cfg(test)]
        if std::mem::take(&mut self.fail_append) {
            return Err(ModelError::new(
                ErrorCode::IoError,
                "injected commit-log spool append failure",
            ));
        }
        if self.sealed {
            return Err(ModelError::new(
                ErrorCode::InternalError,
                "commit-log output spool is already sealed",
            ));
        }
        let payload = crate::encode(&record.to_cbor());
        let payload_len = u64::try_from(payload.len()).map_err(|_| {
            ModelError::new(ErrorCode::InternalError, "commit-log record is too large")
        })?;
        self.file
            .seek(SeekFrom::Start(self.length))
            .and_then(|_| self.file.write_all(&payload_len.to_be_bytes()))
            .and_then(|_| self.file.write_all(&payload))
            .map_err(spool_io_error)?;
        self.length = self
            .length
            .checked_add(RECORD_PREFIX_BYTES)
            .and_then(|length| length.checked_add(payload_len))
            .ok_or_else(|| {
                ModelError::new(ErrorCode::InternalError, "commit-log spool length overflow")
            })?;
        Ok(())
    }

    fn seal(&mut self) -> model::ModelResult<()> {
        #[cfg(test)]
        if std::mem::take(&mut self.fail_seal) {
            return Err(ModelError::new(
                ErrorCode::IoError,
                "injected commit-log spool seal failure",
            ));
        }
        self.file.flush().map_err(spool_io_error)?;
        self.sealed = true;
        Ok(())
    }

    fn read(
        &mut self,
        request: &CommitLogReadRequest,
    ) -> model::ModelResult<CommitLogReadResponse> {
        if !self.sealed {
            return Err(ModelError::new(
                ErrorCode::InternalError,
                "commit-log output spool is not sealed",
            ));
        }
        let limit = match request.max_records {
            None => DEFAULT_READ_RECORDS,
            Some(0) => {
                return Err(ModelError::new(
                    ErrorCode::InvalidRequest,
                    "commit-log read max_records must be positive",
                ));
            }
            Some(value) => usize::try_from(value).unwrap_or(usize::MAX),
        };
        if limit > MAX_READ_RECORDS {
            return Err(ModelError::new(
                ErrorCode::InvalidRequest,
                format!("commit-log read max_records cannot exceed {MAX_READ_RECORDS}"),
            ));
        }

        let cursor = request.cursor.unwrap_or(0);
        self.validate_cursor(cursor)?;
        if cursor == self.length {
            return Ok(CommitLogReadResponse {
                records: Vec::new(),
                next_cursor: cursor,
                state: CommitLogReadState::Eof,
            });
        }

        let mut position = cursor;
        let mut records = Vec::with_capacity(limit.min(16));
        while records.len() < limit && position < self.length {
            let payload = self.read_payload(&mut position)?;
            let cbor = crate::cbor::try_decode(&payload).map_err(spool_decode_error)?;
            records.push(crate::LogOutputRecord::from_cbor(&cbor).map_err(spool_decode_error)?);
        }
        Ok(CommitLogReadResponse {
            records,
            next_cursor: position,
            state: CommitLogReadState::Data,
        })
    }

    fn validate_cursor(&mut self, cursor: u64) -> model::ModelResult<()> {
        if cursor > self.length {
            return Err(invalid_cursor(cursor));
        }
        let mut position = 0;
        while position < cursor {
            self.skip_payload(&mut position)?;
            if position > cursor {
                return Err(invalid_cursor(cursor));
            }
        }
        if position == cursor {
            Ok(())
        } else {
            Err(invalid_cursor(cursor))
        }
    }

    fn skip_payload(&mut self, position: &mut u64) -> model::ModelResult<()> {
        let payload_len = self.read_length(*position)?;
        *position = position
            .checked_add(RECORD_PREFIX_BYTES)
            .and_then(|value| value.checked_add(payload_len))
            .filter(|value| *value <= self.length)
            .ok_or_else(corrupt_spool)?;
        Ok(())
    }

    fn read_payload(&mut self, position: &mut u64) -> model::ModelResult<Vec<u8>> {
        let payload_len = self.read_length(*position)?;
        let payload_start = position
            .checked_add(RECORD_PREFIX_BYTES)
            .ok_or_else(corrupt_spool)?;
        let payload_end = payload_start
            .checked_add(payload_len)
            .filter(|value| *value <= self.length)
            .ok_or_else(corrupt_spool)?;
        let allocation = usize::try_from(payload_len).map_err(|_| corrupt_spool())?;
        let mut payload = vec![0; allocation];
        self.file
            .seek(SeekFrom::Start(payload_start))
            .and_then(|_| self.file.read_exact(&mut payload))
            .map_err(spool_io_error)?;
        *position = payload_end;
        Ok(payload)
    }

    fn read_length(&mut self, position: u64) -> model::ModelResult<u64> {
        if position
            .checked_add(RECORD_PREFIX_BYTES)
            .is_none_or(|end| end > self.length)
        {
            return Err(corrupt_spool());
        }
        let mut bytes = [0; RECORD_PREFIX_BYTES as usize];
        self.file
            .seek(SeekFrom::Start(position))
            .and_then(|_| self.file.read_exact(&mut bytes))
            .map_err(spool_io_error)?;
        Ok(u64::from_be_bytes(bytes))
    }
}

/// Caller-owned registry for finite commit-log result spools.
#[derive(Clone)]
pub struct CommitLogOutputRegistry {
    logs: Arc<Mutex<HashMap<String, Arc<Mutex<OutputSpool>>>>>,
    next_id: Arc<AtomicU64>,
    #[cfg(test)]
    fail_next_spool: Arc<AtomicBool>,
    #[cfg(test)]
    fail_next_append: Arc<AtomicBool>,
    #[cfg(test)]
    fail_next_seal: Arc<AtomicBool>,
}

impl Default for CommitLogOutputRegistry {
    fn default() -> Self {
        Self {
            logs: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(0)),
            #[cfg(test)]
            fail_next_spool: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_next_append: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_next_seal: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl CommitLogOutputRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn read(
        &self,
        log_id: &str,
        request: &CommitLogReadRequest,
    ) -> model::ModelResult<CommitLogReadResponse> {
        let spool = self
            .logs
            .lock()
            .unwrap()
            .get(log_id)
            .cloned()
            .ok_or_else(|| {
                ModelError::new(
                    ErrorCode::InvalidRequest,
                    format!("unknown commit-log output log '{log_id}'"),
                )
            })?;
        spool.lock().unwrap().read(request)
    }

    /// Release a log and its automatically removed process-temp spool.
    pub fn release(&self, log_id: &str) {
        self.logs.lock().unwrap().remove(log_id);
    }

    fn create(&self) -> model::ModelResult<(String, Arc<Mutex<OutputSpool>>)> {
        #[cfg(test)]
        if self.fail_next_spool.swap(false, Ordering::SeqCst) {
            return Err(ModelError::new(
                ErrorCode::IoError,
                "injected commit-log spool creation failure",
            ));
        }
        let file = tempfile::tempfile().map_err(spool_io_error)?;
        let sequence = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let log_id = format!("commitlog_{sequence:012}");
        let spool = Arc::new(Mutex::new(OutputSpool::new(
            file,
            #[cfg(test)]
            self.fail_next_append.swap(false, Ordering::SeqCst),
            #[cfg(test)]
            self.fail_next_seal.swap(false, Ordering::SeqCst),
        )));
        self.logs
            .lock()
            .unwrap()
            .insert(log_id.clone(), spool.clone());
        Ok((log_id, spool))
    }

    #[cfg(test)]
    pub(super) fn fail_next_spool_for_test(&self) {
        self.fail_next_spool.store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(super) fn registered_log_count_for_test(&self) -> usize {
        self.logs.lock().unwrap().len()
    }

    #[cfg(test)]
    pub(super) fn fail_next_append_for_test(&self) {
        self.fail_next_append.store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(super) fn fail_next_seal_for_test(&self) {
        self.fail_next_seal.store(true, Ordering::SeqCst);
    }
}

/// Execute the completed commit-history engine and retain its finite stream.
pub(in crate::operation) fn handle_log(
    start: &Path,
    request: crate::LogRequest,
    operation_id: impl Into<String>,
    output_registry: &CommitLogOutputRegistry,
) -> model::ModelResult<crate::LogResponse> {
    let context = OperationRequest::Log(request.clone()).context(operation_id.into())?;
    let histories = open_request_histories(start, &request)?;
    let (log_id, spool) = output_registry.create()?;
    let include_body = request
        .options
        .as_ref()
        .and_then(|options| options.include_body)
        .unwrap_or(false);
    let mut emission_error = None;
    let result = stream_request_histories(histories, &request, |event| {
        if emission_error.is_some() {
            return;
        }
        emission_error = project_merged_event(event, include_body)
            .and_then(|record| spool.lock().unwrap().append(&record))
            .err();
    });
    if let Some(error) = emission_error {
        output_registry.release(&log_id);
        return Err(error);
    }
    if let Err(error) = spool.lock().unwrap().seal() {
        output_registry.release(&log_id);
        return Err(error);
    }

    Ok(crate::LogResponse {
        response: crate::ResponseEnvelope {
            meta: crate::ResponseMeta {
                request_id: context.request_id,
                schema_version: context.schema_version,
                action: crate::ActionKind::Log,
                aggregate_status: result.aggregate().status(),
                operation_id: Some(context.operation_id),
                message: None,
                attribution: context.attribution.as_ref().map(Into::into),
            },
            members: Vec::new(),
            errors: Vec::new(),
        },
        output: crate::LogOutputLogRef { log_id },
    })
}

pub(super) fn project_merged_event(
    event: CommitLogMergedEvent,
    include_body: bool,
) -> model::ModelResult<crate::LogOutputRecord> {
    match event {
        CommitLogMergedEvent::Group(group) => Ok(crate::LogOutputRecord {
            kind: crate::LogOutputRecordKind::Entry,
            entry: Some(project_group(&group, include_body)?),
            degradation: None,
        }),
        CommitLogMergedEvent::Degradation(record) => Ok(crate::LogOutputRecord {
            kind: crate::LogOutputRecordKind::Degradation,
            entry: None,
            degradation: Some(project_degradation(record)),
        }),
    }
}

fn project_group(
    group: &CommitLogGroup,
    include_body: bool,
) -> model::ModelResult<crate::LogEntry> {
    let mut entries = group.entries().iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        (&left.target.member_id, &left.commit_id).cmp(&(&right.target.member_id, &right.commit_id))
    });
    let representative = entries
        .first()
        .copied()
        .ok_or_else(|| ModelError::new(ErrorCode::InternalError, "commit-log group is empty"))?;
    let (subject_bytes, body_bytes) = split_message(&representative.message);
    let author = project_identity(&representative.author);
    let committer = project_identity(&representative.committer);
    let lossy = !is_utf8(subject_bytes)
        || include_body && body_bytes.is_some_and(|body| !is_utf8(body))
        || !is_utf8(&representative.author.name)
        || !is_utf8(&representative.author.email)
        || !is_utf8(&representative.committer.name)
        || !is_utf8(&representative.committer.email);
    let ordering_seconds = group.ordering_timestamp_seconds();

    Ok(crate::LogEntry {
        members: entries
            .into_iter()
            .map(|entry| crate::LogEntryMember {
                member_id: entry.target.member_id.clone(),
                member_path: entry.target.member_path.clone(),
                source_kind: Some(entry.target.source_kind.into()),
                commit: entry.commit_id.clone(),
                parents: entry.parent_ids.clone(),
            })
            .collect(),
        provenance: project_provenance(group.provenance()),
        author,
        committer,
        subject: String::from_utf8_lossy(subject_bytes).into_owned(),
        body: include_body
            .then(|| body_bytes.map(|body| String::from_utf8_lossy(body).into_owned()))
            .flatten()
            .filter(|body| !body.is_empty()),
        ordering_timestamp_ms: milliseconds(ordering_seconds),
        author_timestamp_seconds: representative.author.time.seconds,
        committer_timestamp_seconds: representative.committer.time.seconds,
        ordering_timestamp_seconds: ordering_seconds,
        lossy: Some(lossy),
    })
}

fn project_identity(identity: &CommitLogIdentity) -> crate::GitObjectIdentity {
    crate::GitObjectIdentity {
        name: String::from_utf8_lossy(&identity.name).into_owned(),
        email: String::from_utf8_lossy(&identity.email).into_owned(),
        time_ms: milliseconds(identity.time.seconds),
        timezone_offset_minutes: Some(i64::from(identity.time.offset_minutes)),
    }
}

fn project_provenance(provenance: &CommitLogProvenance) -> crate::LogMergeProvenance {
    match provenance {
        CommitLogProvenance::None => crate::LogMergeProvenance {
            kind: crate::LogMergeKind::None,
            gwz_commit_id: None,
        },
        CommitLogProvenance::Heuristic => crate::LogMergeProvenance {
            kind: crate::LogMergeKind::Heuristic,
            gwz_commit_id: None,
        },
        CommitLogProvenance::Marker(marker) => crate::LogMergeProvenance {
            kind: crate::LogMergeKind::Marker,
            gwz_commit_id: Some(marker.clone()),
        },
        // L-PRO-1 freezes the three-arm S2.0 enum. Preserve the additive
        // L-COA-6 token in its existing optional provenance text slot.
        CommitLogProvenance::MarkerInvalid => crate::LogMergeProvenance {
            kind: crate::LogMergeKind::None,
            gwz_commit_id: Some("marker-invalid".to_owned()),
        },
    }
}

fn project_degradation(record: CommitLogDegradation) -> crate::LogDegradation {
    crate::LogDegradation {
        member_id: record.target.member_id,
        member_path: record.target.member_path,
        source_kind: Some(record.target.source_kind.into()),
        reason: match record.kind {
            CommitLogDegradationKind::UnsupportedSourceKind => {
                crate::LogDegradationReason::UnsupportedSourceKind
            }
            CommitLogDegradationKind::RepositoryUnreadable
            | CommitLogDegradationKind::HistoryUnreadable => {
                crate::LogDegradationReason::RepositoryUnreadable
            }
            CommitLogDegradationKind::UnbornHead => crate::LogDegradationReason::Unborn,
            CommitLogDegradationKind::RevisionUnresolved => {
                crate::LogDegradationReason::RevisionUnresolved
            }
            CommitLogDegradationKind::SnapshotEntryMissing => {
                crate::LogDegradationReason::SnapshotEntryMissing
            }
            CommitLogDegradationKind::LockEntryMissing => {
                crate::LogDegradationReason::LockEntryMissing
            }
        },
        operand: record.operand,
        message: Some(record.detail),
    }
}

fn split_message(message: &[u8]) -> (&[u8], Option<&[u8]>) {
    match message.iter().position(|byte| *byte == b'\n') {
        Some(index) => (&message[..index], Some(&message[index + 1..])),
        None => (message, None),
    }
}

fn is_utf8(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok()
}

fn milliseconds(seconds: i64) -> Option<i64> {
    seconds.checked_mul(1_000)
}

fn spool_io_error(error: std::io::Error) -> ModelError {
    ModelError::new(ErrorCode::IoError, error.to_string())
}

fn spool_decode_error(error: impl std::fmt::Display) -> ModelError {
    ModelError::new(
        ErrorCode::InternalError,
        format!("commit-log output spool is corrupt: {error}"),
    )
}

fn corrupt_spool() -> ModelError {
    ModelError::new(
        ErrorCode::InternalError,
        "commit-log output spool is corrupt",
    )
}

fn invalid_cursor(cursor: u64) -> ModelError {
    ModelError::new(
        ErrorCode::InvalidRequest,
        format!("invalid commit-log output cursor {cursor}"),
    )
}

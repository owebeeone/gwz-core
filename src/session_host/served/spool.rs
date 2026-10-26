//! The session's spools, `diff.output` and `log.output` (gwz-dev
//! `dev-docs/GwzCoreSessionDesign.md` §4.2, §5.4, adopted unchanged by
//! `GwzSplitCommandChannel.md` §3.2), and the `accepted` response of each
//! operation method (§4.2).
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | The logs belong to the session, under ids no other session can name | §5.4 |
//! | At most `open_logs` (64) are open; an open first releases the oldest log sealed at least 30 s ago with no reader stream open, and with none is refused with `transport_session_full` before any effect | §5.4 |
//! | A log is released when its producer has sealed it and its last reader stream has ended; an end stream on `log.output` releases it; every log is released at the session's end | §5.4; §8 step 7 |
//! | A released log reads as `operation_expired`; a log this session never issued, another session's included, as `operation_not_found` | §5.4 |
//! | A read is taut-shape's `LogReadRequest`, answered with `LogReadResponse` of at most the session's read limit (1 MiB by default) and at least one record; a single record over the limit is refused | §4.2 |
//! | It never blocks: a bounded wait reads as a probe, the in-process log service's v0 rule (`diff/log_service.rs`), a deviation recorded with the lane owner's acceptance | §4.2; log service "The timer choice" |

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use gwz_session_host::{Call, MAX_READ_WAIT, Verb};
use taut_shape::generated as shape;

use crate::diff::DiffLogRegistry;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::CommitLogOutputRegistry;

fn undecodable() -> ModelError {
    ModelError::new(
        ErrorCode::InvalidRequest,
        "the log call's request message does not decode",
    )
}

/// Which registry a log lives in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Kind {
    /// A `diff.output` log.
    Diff,
    /// A `log.output` log.
    Log,
}

impl Kind {
    fn method(self) -> &'static str {
        match self {
            Kind::Diff => "diff.output",
            Kind::Log => "log.output",
        }
    }
}

struct Entry {
    kind: Kind,
    inner: String,
    sealed_at: Instant,
    /// Reader streams that have read and not ended.
    readers: HashSet<String>,
    /// Every stream that has read.
    read_by: HashSet<String>,
}

#[derive(Default)]
struct Table {
    open: HashMap<String, Entry>,
    issued: HashSet<String>,
    /// Opens in progress, counted against the limit.
    reserved: usize,
    ended: bool,
}

/// The session's spool layer over the two registries.
pub(crate) struct Spools {
    pub(crate) diffs: DiffLogRegistry,
    pub(crate) logs: CommitLogOutputRegistry,
    /// This session's prefix of every log id it issues.
    tag: String,
    limit: usize,
    read_bytes: usize,
    table: Mutex<Table>,
}

/// An open slot taken before a producer registers its log.
#[must_use = "a reservation is committed with its log, or dropped"]
pub(crate) struct Reservation<'a> {
    spools: &'a Spools,
    done: bool,
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if !self.done {
            let mut table = self.spools.table();
            table.reserved = table.reserved.saturating_sub(1);
        }
    }
}

impl Reservation<'_> {
    /// Commits the log the producer registered, sealed now, and returns the
    /// id the client names it by.
    pub(crate) fn commit(mut self, kind: Kind, inner: String) -> String {
        self.done = true;
        let external = format!("{}.{inner}", self.spools.tag);
        let mut table = self.spools.table();
        table.reserved = table.reserved.saturating_sub(1);
        table.issued.insert(external.clone());
        table.open.insert(
            external.clone(),
            Entry {
                kind,
                inner,
                sealed_at: Instant::now(),
                readers: HashSet::new(),
                read_by: HashSet::new(),
            },
        );
        external
    }
}

impl Spools {
    /// The spools of a session whose ids start with `tag`, with at most
    /// `limit` open logs and reads of at most `read_bytes`.
    pub(crate) fn new(tag: String, limit: usize, read_bytes: usize) -> Self {
        Self {
            diffs: DiffLogRegistry::new(),
            logs: CommitLogOutputRegistry::new(),
            tag,
            limit,
            read_bytes,
            table: Mutex::new(Table::default()),
        }
    }

    fn table(&self) -> MutexGuard<'_, Table> {
        self.table.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes an open slot, first releasing the oldest log sealed at least
    /// the read wait ago with no reader open when the limit is reached
    /// (§5.4).
    ///
    /// # Errors
    ///
    /// `transport_session_full` when no log can be released, and
    /// `operation_expired` once the session's spools are released.
    pub(crate) fn reserve(&self) -> ModelResult<Reservation<'_>> {
        let mut table = self.table();
        if table.ended {
            return Err(ModelError::new(
                ErrorCode::OperationExpired,
                "the session's diff and log output has been released",
            ));
        }
        if table.open.len() + table.reserved >= self.limit {
            let oldest = table
                .open
                .iter()
                .filter(|(_, entry)| {
                    entry.readers.is_empty() && entry.sealed_at.elapsed() >= MAX_READ_WAIT
                })
                .min_by_key(|(_, entry)| entry.sealed_at)
                .map(|(id, _)| id.clone());
            let Some(oldest) = oldest else {
                return Err(ModelError::new(
                    ErrorCode::TransportSessionFull,
                    format!(
                        "the session has {} diff and log outputs open, none of them releasable; end a reader stream to release one",
                        self.limit
                    ),
                ));
            };
            self.release_locked(&mut table, &oldest);
        }
        table.reserved += 1;
        Ok(Reservation {
            spools: self,
            done: false,
        })
    }

    fn release_locked(&self, table: &mut Table, external: &str) {
        if let Some(entry) = table.open.remove(external) {
            match entry.kind {
                Kind::Diff => self.diffs.release(&entry.inner),
                Kind::Log => self.logs.release(&entry.inner),
            }
        }
    }

    /// Releases every log (§8 step 7).
    pub(crate) fn release_all(&self) {
        let mut table = self.table();
        table.ended = true;
        let ids: Vec<String> = table.open.keys().cloned().collect();
        for id in ids {
            self.release_locked(&mut table, &id);
        }
    }

    /// The open entry `external` of `kind`, or the refusal §5.4 gives.
    fn lookup(&self, table: &Table, external: &str, kind: Kind) -> ModelResult<(String, Instant)> {
        match table.open.get(external) {
            Some(entry) if entry.kind == kind => Ok((entry.inner.clone(), entry.sealed_at)),
            _ if table.issued.contains(external) => Err(ModelError::new(
                ErrorCode::OperationExpired,
                format!("{} log {external} has been released", kind.method()),
            )),
            _ => Err(ModelError::new(
                ErrorCode::OperationNotFound,
                format!("this session issued no {} log {external}", kind.method()),
            )),
        }
    }

    /// Answers a `diff.output` or `log.output` call.
    pub(crate) fn answer(&self, call: &Call) -> ModelResult<Vec<u8>> {
        let kind = if call.method == "log.output" {
            Kind::Log
        } else {
            Kind::Diff
        };
        if call.verb == Some(Verb::EndStream) {
            let end = shape::LogEndStream::decode(&call.request).map_err(|_| undecodable())?;
            self.end_stream(kind, &end.log_id, &end.stream_id)?;
            return Ok(taut_shape::cbor::encode(&end.to_cbor()));
        }
        let read = shape::LogReadRequest::decode(&call.request).map_err(|_| undecodable())?;
        let inner = {
            let mut table = self.table();
            let (inner, _) = self.lookup(&table, &read.log_id, kind)?;
            if let Some(entry) = table.open.get_mut(&read.log_id) {
                entry.readers.insert(read.stream_id.clone());
                entry.read_by.insert(read.stream_id.clone());
            }
            inner
        };
        let response = match kind {
            Kind::Diff => self.read_diff(&inner, &read)?,
            Kind::Log => self.read_log(&inner, &read)?,
        };
        Ok(taut_shape::cbor::encode(&response.to_cbor()))
    }

    fn end_stream(&self, kind: Kind, external: &str, stream_id: &str) -> ModelResult<()> {
        let mut table = self.table();
        if table.issued.contains(external) && !table.open.contains_key(external) {
            // Repeating an end stream on a released log is harmless.
            return Ok(());
        }
        let (inner, _) = self.lookup(&table, external, kind)?;
        match kind {
            Kind::Log => {
                self.release_locked(&mut table, external);
            }
            Kind::Diff => {
                let Some(entry) = table.open.get_mut(external) else {
                    return Ok(());
                };
                if !entry.read_by.contains(stream_id) {
                    // A stream that never read the log ends nothing.
                    return Ok(());
                }
                entry.readers.remove(stream_id);
                let last = entry.readers.is_empty();
                if let Ok(log) = self.diffs.get(&inner) {
                    log.end_stream(stream_id);
                }
                if last {
                    self.release_locked(&mut table, external);
                }
            }
        }
        Ok(())
    }

    fn read_diff(
        &self,
        inner: &str,
        read: &shape::LogReadRequest,
    ) -> ModelResult<shape::LogReadResponse> {
        let limit = self.read_bytes as u64;
        let request = crate::diff::LogReadRequest {
            stream_id: read.stream_id.clone(),
            cursor: read.cursor.as_ref().map(|cursor| cursor.seq.max(0) as u64),
            max_records: read
                .max_records
                .map(|count| count.clamp(0, i64::from(u32::MAX)) as u32),
            max_bytes: Some(
                read.max_bytes
                    .map_or(limit, |bytes| (bytes.max(0) as u64).min(limit)),
            ),
            timeout_ms: Some(0),
        };
        let response = self.diffs.read(inner, &request)?;
        use crate::diff::LogReadState as S;
        let state = match response.state {
            S::Data => shape::LogState::Data,
            S::WouldBlock => shape::LogState::WouldBlock,
            S::Eof => shape::LogState::Eof,
            S::Closed => shape::LogState::Closed,
            S::Failed => shape::LogState::Failed,
            S::Expired => shape::LogState::Expired,
        };
        Ok(shape::LogReadResponse {
            log_id: read.log_id.clone(),
            stream_id: read.stream_id.clone(),
            records: response
                .records
                .into_iter()
                .map(|record| shape::LogRecord {
                    seq: record.seq as i64,
                    payload: record.payload,
                })
                .collect(),
            next_cursor: shape::LogCursor {
                seq: response.next_cursor as i64,
            },
            state,
            error: response.error.map(|message| shape::LogError {
                code: shape::LogErrorCode::ProducerError,
                message: Some(message),
            }),
        })
    }

    /// Reads one record at a time, so the reply holds as many as fit in the
    /// read limit, and at least one (§4.2).
    fn read_log(
        &self,
        inner: &str,
        read: &shape::LogReadRequest,
    ) -> ModelResult<shape::LogReadResponse> {
        let limit = read.max_bytes.map_or(self.read_bytes, |bytes| {
            (bytes.max(0) as usize).min(self.read_bytes)
        });
        let max_records = read
            .max_records
            .map_or(128, |count| count.clamp(1, 1024) as usize);
        let mut cursor = read.cursor.as_ref().map(|cursor| cursor.seq.max(0) as u64);
        let mut records = Vec::new();
        let mut used = 0usize;
        let mut state = shape::LogState::Data;
        while records.len() < max_records {
            let request = crate::operation::CommitLogReadRequest {
                cursor,
                max_records: Some(1),
            };
            let response = self.logs.read(inner, &request)?;
            let Some(record) = response.records.first() else {
                state = shape::LogState::Eof;
                break;
            };
            let payload = crate::encode(&record.to_cbor());
            if used + payload.len() > limit {
                if records.is_empty() {
                    return Err(ModelError::new(
                        ErrorCode::InvalidRequest,
                        format!(
                            "a log.output record of {} bytes exceeds the read limit of {limit} bytes",
                            payload.len()
                        ),
                    ));
                }
                break;
            }
            used += payload.len();
            records.push(shape::LogRecord {
                seq: response.next_cursor as i64,
                payload,
            });
            cursor = Some(response.next_cursor);
            if response.state == crate::operation::CommitLogReadState::Eof {
                state = shape::LogState::Eof;
                break;
            }
        }
        Ok(shape::LogReadResponse {
            log_id: read.log_id.clone(),
            stream_id: read.stream_id.clone(),
            records,
            next_cursor: shape::LogCursor {
                seq: cursor.unwrap_or(0) as i64,
            },
            state,
            error: None,
        })
    }
}

/// The `accepted` response of `method`: its own response message carrying
/// `envelope` and nothing else (§4.2).
#[allow(
    clippy::needless_update,
    reason = "some response messages carry their envelope alone"
)]
pub(super) fn accepted(method: &str, envelope: crate::ResponseEnvelope) -> crate::Cbor {
    macro_rules! accepted {
        ($($name:literal => $response:ident),* $(,)?) => {
            match method {
                $($name => crate::$response {
                    response: envelope,
                    ..crate::$response::default()
                }
                .to_cbor(),)*
                _ => crate::Cbor::Map(vec![(1, envelope.to_cbor())]),
            }
        };
    }
    accepted!(
        "fetch" => FetchResponse,
        "push" => PushResponse,
        "pull_head" => PullHeadResponse,
        "pull_snapshot" => PullSnapshotResponse,
        "materialize" => MaterializeResponse,
        "clone_repo_member" => CloneRepoMemberResponse,
        "attach_repo_member" => AttachRepoMemberResponse,
        "detach_repo_member" => DetachRepoMemberResponse,
        "merge" => MergeResponse,
        "local_family" => LocalFamilyResponse,
        "snapshot" => SnapshotResponse,
        "capture" => CaptureResponse,
        "commit" => CommitResponse,
        "stage" => StageResponse,
        "branch" => BranchResponse,
        "stash" => StashResponse,
        "repo_sync" => RepoSyncResponse,
        "create_repo" => CreateRepoResponse,
        "add_existing_repo" => AddExistingRepoResponse,
        "remote_identity" => RemoteIdentityResponse,
        "tag" => TagResponse,
    )
}

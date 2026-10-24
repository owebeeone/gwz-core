//! The session's spools, `diff.output` and `log.output`, answered on the
//! reading thread (gwz-dev `dev-docs/GwzCoreSessionDesign.md` §4.2, §5.4), and
//! the `accepted` response of each operation method (§4.2).
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | A read is taut-shape's `LogReadRequest`, answered with `LogReadResponse`; its byte limit is the session's read limit, 1 MiB by default | §4.2 |
//! | It never blocks: a bounded wait reads as a probe, the in-process log service's v0 rule (`diff/log_service.rs`) | §4.2; log service "The timer choice" |
//! | An end stream on `log.output` releases the log and its spool; repeating it is harmless | §4.2 |
//! | A log of another session, or never created, is not found; one the session released is `operation_expired` | §5.4 |

use gwz_session_host::{Call, Verb};
use taut_shape::generated as shape;

use super::ServedSession;
use crate::model::{ErrorCode, ModelError, ModelResult};

fn undecodable() -> ModelError {
    ModelError::new(
        ErrorCode::InvalidRequest,
        "the log call's request message does not decode",
    )
}

/// Answers a `diff.output` or `log.output` call.
pub(super) fn answer(
    session: &ServedSession,
    call: &Call,
    read_bytes: usize,
) -> ModelResult<Vec<u8>> {
    let spools = session.spools()?;
    if call.verb == Some(Verb::EndStream) {
        let end = shape::LogEndStream::decode(&call.request).map_err(|_| undecodable())?;
        if call.method == "log.output" {
            spools.logs.release(&end.log_id);
        } else {
            spools.diffs.get(&end.log_id)?.end_stream(&end.stream_id);
        }
        return Ok(taut_shape::cbor::encode(&end.to_cbor()));
    }
    let read = shape::LogReadRequest::decode(&call.request).map_err(|_| undecodable())?;
    let cursor = read.cursor.map(|cursor| cursor.seq.max(0) as u64);
    let records;
    let next_cursor;
    let state;
    let mut error = None;
    if call.method == "log.output" {
        let request = crate::operation::CommitLogReadRequest {
            cursor,
            max_records: read
                .max_records
                .map(|count| count.clamp(0, i64::from(u32::MAX)) as u32),
        };
        let response = spools.logs.read(&read.log_id, &request)?;
        let first = cursor.unwrap_or(0);
        records = response
            .records
            .iter()
            .enumerate()
            .map(|(index, record)| shape::LogRecord {
                seq: (first + index as u64 + 1) as i64,
                payload: crate::encode(&record.to_cbor()),
            })
            .collect();
        next_cursor = response.next_cursor;
        state = match response.state {
            crate::operation::CommitLogReadState::Data => shape::LogState::Data,
            crate::operation::CommitLogReadState::Eof => shape::LogState::Eof,
        };
    } else {
        let limit = read_bytes as u64;
        let request = crate::diff::LogReadRequest {
            stream_id: read.stream_id.clone(),
            cursor,
            max_records: read
                .max_records
                .map(|count| count.clamp(0, i64::from(u32::MAX)) as u32),
            max_bytes: Some(
                read.max_bytes
                    .map_or(limit, |bytes| (bytes.max(0) as u64).min(limit)),
            ),
            timeout_ms: Some(0),
        };
        let response = spools.diffs.read(&read.log_id, &request)?;
        records = response
            .records
            .into_iter()
            .map(|record| shape::LogRecord {
                seq: record.seq as i64,
                payload: record.payload,
            })
            .collect();
        next_cursor = response.next_cursor;
        use crate::diff::LogReadState as S;
        state = match response.state {
            S::Data => shape::LogState::Data,
            S::WouldBlock => shape::LogState::WouldBlock,
            S::Eof => shape::LogState::Eof,
            S::Closed => shape::LogState::Closed,
            S::Failed => shape::LogState::Failed,
            S::Expired => shape::LogState::Expired,
        };
        error = response.error.map(|message| shape::LogError {
            code: shape::LogErrorCode::ProducerError,
            message: Some(message),
        });
    }
    let response = shape::LogReadResponse {
        log_id: read.log_id,
        stream_id: read.stream_id,
        records,
        next_cursor: shape::LogCursor {
            seq: next_cursor as i64,
        },
        state,
        error,
    };
    Ok(taut_shape::cbor::encode(&response.to_cbor()))
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

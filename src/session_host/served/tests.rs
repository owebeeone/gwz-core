//! The served core's operation service over the in-process carrier, with the
//! schema's own frame bodies (step 2.2b's red rows; gwz-dev
//! `dev-docs/GwzSplitCommandChannel.md` §3.2, §3.3; `GwzCoreSessionDesign.md`
//! §4.1, §4.2, §5, §8).
//!
//! Handlers are scripted where a row needs to hold one; `ls` runs gwz-core's
//! own handler on a real workspace.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use gwz_session_channel::InProcessEnd;
use gwz_session_contract::{Closed, Frame, FrameSink, FrameSource, Lane, Tag};
use gwz_session_host::{Limits, Worker};

use super::dispatch::CoreDispatch;
use super::{Dispatch, HostContext, ServedSession, SpoolKind, serve_in_process};
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::EventSink;
use crate::test_support::TempDir;

/// What the scripted handlers saw.
#[derive(Default)]
struct Script {
    released: Mutex<bool>,
    changed: Condvar,
    cancelled_seen: AtomicBool,
    /// A `held-` diff waits here, ignoring its token, before `CoreDispatch`.
    diff_waiting: AtomicBool,
    diff_released: AtomicBool,
}

impl Script {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }
}

/// `ls` runs gwz-core's handler; `fetch` holds until released or cancelled,
/// reporting a progress event first.
struct TestDispatch {
    core: CoreDispatch,
    script: Arc<Script>,
}

impl Dispatch for TestDispatch {
    fn run(
        &self,
        method: &str,
        request: &[u8],
        start: &Path,
        worker: &Worker<ServedSession>,
        events: &dyn EventSink,
    ) -> ModelResult<Vec<u8>> {
        if method == "diff" {
            let held = crate::DiffRequest::decode(request)
                .is_ok_and(|diff| diff.meta.request_id.starts_with("held-"));
            if held {
                self.script.diff_waiting.store(true, Ordering::SeqCst);
                // Released by the test once it has sent the cancel; the
                // cancel is read on the host's reader thread, so wait for it
                // to reach the token too, without acting on it here.
                let deadline = Instant::now() + Duration::from_secs(20);
                while !(self.script.diff_released.load(Ordering::SeqCst)
                    && worker.token().is_cancelled())
                    && Instant::now() < deadline
                {
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        }
        if method != "fetch" {
            return self.core.run(method, request, start, worker, events);
        }
        let request = crate::FetchRequest::decode(request).unwrap();
        events.deliver(crate::OperationEvent {
            operation_id: worker.operation_id().unwrap().to_owned(),
            request_id: request.meta.request_id.clone(),
            sequence: 1,
            kind: crate::EventKind::MemberProgress,
            message: Some("fetching".to_owned()),
            ..crate::OperationEvent::default()
        });
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut released = self.script.released.lock().unwrap();
        while !*released && Instant::now() < deadline {
            if worker.token().is_cancelled() {
                self.script.cancelled_seen.store(true, Ordering::SeqCst);
                return Err(ModelError::new(
                    ErrorCode::Cancelled,
                    "the fetch saw its cancel",
                ));
            }
            released = self
                .script
                .changed
                .wait_timeout(released, Duration::from_millis(5))
                .unwrap()
                .0;
        }
        let response = crate::FetchResponse {
            response: crate::ResponseEnvelope {
                meta: crate::ResponseMeta {
                    request_id: request.meta.request_id,
                    action: crate::ActionKind::Fetch,
                    aggregate_status: crate::AggregateStatus::Ok,
                    ..crate::ResponseMeta::default()
                },
                members: Vec::new(),
                errors: Vec::new(),
            },
            repos: None,
        };
        Ok(crate::encode(&response.to_cbor()))
    }
}

struct Session {
    _temp: TempDir,
    root: PathBuf,
    client: InProcessEnd,
    service: gwz_session_host::Service<super::ServedPorts>,
    script: Arc<Script>,
    next: i64,
    /// Frames received but not yet asked for.
    inbox: HashMap<i64, Frame>,
}

fn meta(root: &Path, request_id: &str) -> crate::RequestMeta {
    crate::RequestMeta {
        request_id: request_id.to_owned(),
        schema_version: "gwz.v0".to_owned(),
        invocation: Some(crate::InvocationContext {
            caller_cwd: root.display().to_string(),
        }),
        ..crate::RequestMeta::default()
    }
}

fn workspace() -> (TempDir, PathBuf) {
    let temp = TempDir::new("served");
    let root = temp.path().join("ws");
    std::fs::create_dir_all(&root).unwrap();
    crate::workspace_ops::handle_create_workspace(
        crate::CreateWorkspaceRequest {
            meta: meta(&root, "create"),
            workspace_root: root.display().to_string(),
            workspace_id: Some("ws_served".to_owned()),
        },
        "op-create",
    )
    .unwrap();
    let start = crate::workspace_ops::caller_directory(&meta(&root, "x")).unwrap();
    let root =
        crate::workspace_ops::resolve_request_workspace_root(&start, &meta(&root, "x")).unwrap();
    (temp, root)
}

impl Session {
    fn open(limits: Limits) -> Self {
        let (temp, root) = workspace();
        let script = Arc::new(Script::default());
        let dispatch = Arc::new(TestDispatch {
            core: CoreDispatch::new(crate::git::Git2Backend::new()),
            script: Arc::clone(&script),
        });
        let channel =
            serve_in_process(dispatch, root.clone(), &HostContext::new(), limits).unwrap();
        Self {
            _temp: temp,
            root,
            client: channel.client,
            service: channel.service,
            script,
            next: 0,
            inbox: HashMap::new(),
        }
    }

    fn send(
        &mut self,
        method: &str,
        mode: Option<crate::SessionMode>,
        request: Vec<u8>,
        lane: Lane,
    ) -> i64 {
        self.next += 1;
        self.send_as(self.next, method, mode, request, None, lane);
        self.next
    }

    fn send_as(
        &self,
        call_id: i64,
        method: &str,
        mode: Option<crate::SessionMode>,
        request: Vec<u8>,
        log_verb: Option<crate::LogVerb>,
        lane: Lane,
    ) {
        let call = crate::SessionCall {
            call_id,
            method: method.to_owned(),
            mode,
            request,
            log_verb,
        };
        self.client
            .send(
                Frame::new(Tag::SessionCall, crate::encode(&call.to_cbor())),
                lane,
            )
            .unwrap();
    }

    fn fetch(&mut self, request_id: &str, mode: Option<crate::SessionMode>) -> i64 {
        let request = crate::FetchRequest {
            meta: meta(&self.root, request_id),
        };
        self.send("fetch", mode, crate::encode(&request.to_cbor()), Lane::Call)
    }

    fn read_events(&mut self, operation_id: &str, cursor: i64) -> i64 {
        let read = taut_shape::generated::LogReadRequest {
            log_id: operation_id.to_owned(),
            stream_id: "events".to_owned(),
            cursor: Some(taut_shape::generated::LogCursor { seq: cursor }),
            max_records: None,
            max_bytes: None,
            timeout_ms: Some(10_000),
        };
        self.send(
            "events.subscribe",
            None,
            taut_shape::cbor::encode(&read.to_cbor()),
            Lane::Call,
        )
    }

    /// A `diff.output` or `log.output` read of `log_id` by `stream`.
    fn read_spool(&mut self, method: &str, log_id: &str, stream: &str) -> i64 {
        let read = taut_shape::generated::LogReadRequest {
            log_id: log_id.to_owned(),
            stream_id: stream.to_owned(),
            cursor: None,
            max_records: None,
            max_bytes: None,
            timeout_ms: Some(0),
        };
        self.send(
            method,
            None,
            taut_shape::cbor::encode(&read.to_cbor()),
            Lane::Call,
        )
    }

    /// The end of `stream`'s reading of `log_id`.
    fn end_spool(&mut self, method: &str, log_id: &str, stream: &str) -> i64 {
        let end = taut_shape::generated::LogEndStream {
            log_id: log_id.to_owned(),
            stream_id: stream.to_owned(),
        };
        self.next += 1;
        self.send_as(
            self.next,
            method,
            None,
            taut_shape::cbor::encode(&end.to_cbor()),
            Some(crate::LogVerb::EndStream),
            Lane::Call,
        );
        self.next
    }

    /// A sealed `diff.output` log of `records`, registered as `diff` does.
    fn diff_log(&self, records: usize) -> String {
        let spools = self.service.session().spools();
        let slot = spools.reserve().unwrap();
        let (inner, log) = spools.diffs.create();
        for index in 0..records {
            log.push(format!("record {index}").into_bytes());
        }
        log.seal();
        slot.commit(SpoolKind::Diff, inner)
    }

    fn cancel(&mut self, call_id: i64) -> i64 {
        let cancel = crate::OperationCancelRequest {
            call_id: Some(call_id),
            operation_id: None,
        };
        self.send(
            "operation.cancel",
            None,
            crate::encode(&cancel.to_cbor()),
            Lane::Control,
        )
    }

    /// The reply to `call_id`, keeping others that arrive first; also returns
    /// the order the frames arrived in.
    fn reply(&mut self, call_id: i64) -> Result<Vec<u8>, Box<crate::GwzError>> {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !self.inbox.contains_key(&call_id) {
            assert!(Instant::now() < deadline, "no reply to call {call_id}");
            let frame = self.client.recv().expect("the session is open");
            self.inbox.insert(frame_call_id(&frame), frame);
        }
        decode_reply(&self.inbox.remove(&call_id).unwrap())
    }
}

fn frame_call_id(frame: &Frame) -> i64 {
    if frame.tag == Tag::SessionReply.byte() {
        crate::SessionReply::decode(&frame.body).unwrap().call_id
    } else {
        crate::SessionError::decode(&frame.body).unwrap().call_id
    }
}

fn decode_reply(frame: &Frame) -> Result<Vec<u8>, Box<crate::GwzError>> {
    if frame.tag == Tag::SessionReply.byte() {
        Ok(crate::SessionReply::decode(&frame.body).unwrap().response)
    } else {
        assert_eq!(frame.tag, Tag::SessionError.byte());
        Err(Box::new(
            crate::SessionError::decode(&frame.body).unwrap().error,
        ))
    }
}

#[test]
fn a_real_ls_call_round_trips_and_another_workspace_is_refused() {
    let mut session = Session::open(Limits::default());
    let request = crate::LsRequest {
        meta: meta(&session.root, "ls-1"),
        ..crate::LsRequest::default()
    };
    let ls = session.send("ls", None, crate::encode(&request.to_cbor()), Lane::Call);
    let response = crate::LsResponse::decode(&session.reply(ls).unwrap()).unwrap();
    assert_eq!(response.response.meta.request_id, "ls-1");
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );

    let (_other_temp, other) = workspace();
    let elsewhere = crate::LsRequest {
        meta: meta(&other, "ls-2"),
        ..crate::LsRequest::default()
    };
    let refused = session.send("ls", None, crate::encode(&elsewhere.to_cbor()), Lane::Call);
    let error = session.reply(refused).unwrap_err();
    assert_eq!(error.code, crate::GwzErrorCode::InvalidRequest);
    assert!(
        error.message.contains(&session.root.display().to_string()),
        "{}",
        error.message
    );

    let create = crate::CreateWorkspaceRequest {
        meta: meta(&session.root, "create-2"),
        workspace_root: other.display().to_string(),
        workspace_id: None,
    };
    let created = session.send(
        "create_workspace",
        None,
        crate::encode(&create.to_cbor()),
        Lane::Call,
    );
    assert_eq!(
        session.reply(created).unwrap_err().code,
        crate::GwzErrorCode::InvalidRequest
    );
}

/// Red row (step 2.2b): cancel reaches the handler. The cancel travels on the
/// control lane; the handler sees its token; the call ends `cancelled` (73)
/// and the cancel answers with the `CleanupReport`.
#[test]
fn cancel_reaches_the_handler() {
    let mut session = Session::open(Limits::default());
    let fetch = session.fetch("fetch-cancel", None);
    // Wait until the handler runs: its event is in the log of op-1.
    let read = session.read_events("op-1", 0);
    session.reply(read).unwrap();
    let cancel = session.cancel(fetch);
    let error = session.reply(fetch).unwrap_err();
    assert_eq!(error.code, crate::GwzErrorCode::Cancelled);
    let report = crate::CleanupReport::decode(&session.reply(cancel).unwrap()).unwrap();
    assert_eq!(report.pending_local_work, 0);
    assert!(session.script.cancelled_seen.load(Ordering::SeqCst));
    // The unary call has been answered: a second cancel finds it expired.
    let again = session.cancel(fetch);
    assert_eq!(
        session.reply(again).unwrap_err().code,
        crate::GwzErrorCode::OperationExpired
    );
}

/// Red row (step 2.2b): events arrive before the reply. A submitted fetch is
/// accepted with its `operation_id`; its progress event is read while it
/// runs, and only then does `operation.result` answer.
#[test]
fn events_arrive_before_the_reply() {
    let mut session = Session::open(Limits::default());
    let fetch = session.fetch("fetch-events", Some(crate::SessionMode::Submit));
    let accepted = crate::FetchResponse::decode(&session.reply(fetch).unwrap()).unwrap();
    let meta = accepted.response.meta;
    assert_eq!(meta.aggregate_status, crate::AggregateStatus::Accepted);
    let operation_id = meta.operation_id.unwrap();
    let read = session.read_events(&operation_id, 0);
    let events =
        taut_shape::generated::LogReadResponse::decode(&session.reply(read).unwrap()).unwrap();
    assert_eq!(events.records.len(), 1);
    let event = crate::OperationEvent::decode(&events.records[0].payload).unwrap();
    assert_eq!(event.message.as_deref(), Some("fetching"));
    let result_request = crate::OperationResultRequest {
        operation_id: operation_id.clone(),
    };
    let result = session.send(
        "operation.result",
        None,
        crate::encode(&result_request.to_cbor()),
        Lane::Call,
    );
    session.script.release();
    let result = crate::OperationResult::decode(&session.reply(result).unwrap()).unwrap();
    assert_eq!(result.operation_id, operation_id);
    assert_eq!(result.request_id, "fetch-events");
    assert_eq!(result.aggregate_status, crate::AggregateStatus::Ok);
}

/// Red row (step 2.2b): cancel during a saturated spool attachment
/// progresses. The client leaves its reply queue's call lane full of
/// `diff.output` replies, each about 900 KiB; `operation.cancel` still
/// travels on the control reserve, reaches the running handler and is
/// answered.
#[test]
#[allow(
    clippy::field_reassign_with_default,
    reason = "Limits is non_exhaustive outside its crate"
)]
fn cancel_during_a_saturated_spool_attachment_progresses() {
    let mut limits = Limits::default();
    limits.outstanding_calls = 8;
    limits.control_reserve = 4;
    let mut session = Session::open(limits);
    let spools = session.service.session().spools();
    let slot = spools.reserve().unwrap();
    let (inner, log) = spools.diffs.create();
    for _ in 0..8 {
        log.push(vec![7; 900 * 1024]);
    }
    log.seal();
    let log_id = slot.commit(SpoolKind::Diff, inner);
    let fetch = session.fetch("fetch-saturated", None);
    let read = session.read_events("op-1", 0);
    session.reply(read).unwrap();
    let mut spool_reads = Vec::new();
    for index in 0..7 {
        let request = taut_shape::generated::LogReadRequest {
            log_id: log_id.clone(),
            stream_id: format!("reader-{index}"),
            cursor: None,
            max_records: None,
            max_bytes: None,
            timeout_ms: Some(0),
        };
        spool_reads.push(session.send(
            "diff.output",
            None,
            taut_shape::cbor::encode(&request.to_cbor()),
            Lane::Call,
        ));
    }
    // The client reads nothing: the replies fill the call lane toward it.
    std::thread::sleep(Duration::from_millis(100));
    let cancel = session.cancel(fetch);
    let deadline = Instant::now() + Duration::from_secs(20);
    while !session.script.cancelled_seen.load(Ordering::SeqCst) {
        assert!(
            Instant::now() < deadline,
            "the cancel never reached the handler"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    // Let the host answer while the call lane toward the client stays full:
    // the cancel's reply must not need its room.
    std::thread::sleep(Duration::from_millis(300));
    let report = crate::CleanupReport::decode(&session.reply(cancel).unwrap()).unwrap();
    assert!(!report.peer_cleanup_confirmed);
    assert_eq!(
        session.reply(fetch).unwrap_err().code,
        crate::GwzErrorCode::Cancelled
    );
    for read in spool_reads {
        let bytes = session.reply(read).unwrap();
        let response = taut_shape::generated::LogReadResponse::decode(&bytes).unwrap();
        assert_eq!(response.records.len(), 1);
        assert!(
            bytes.len() <= 1024 * 1024 + 1024,
            "each spool reply stays within 1 MiB"
        );
    }
}

#[test]
fn a_repeated_call_id_ends_the_session_and_close_answers_with_its_report() {
    let mut session = Session::open(Limits::default());
    let close = crate::SessionCloseRequest {};
    let close = session.send(
        "session.close",
        None,
        crate::encode(&close.to_cbor()),
        Lane::Control,
    );
    let report = crate::CleanupReport::decode(&session.reply(close).unwrap()).unwrap();
    assert_eq!(
        (report.pending_local_work, report.peer_cleanup_confirmed),
        (0, false)
    );
    assert!(session.service.wait_ended(Duration::from_secs(5)));
    assert_eq!(session.client.recv().unwrap_err(), Closed::Peer);

    let session = Session::open(Limits::default());
    session.send_as(
        1,
        "transport_capabilities",
        None,
        Vec::new(),
        None,
        Lane::Call,
    );
    session.send_as(
        1,
        "transport_capabilities",
        None,
        Vec::new(),
        None,
        Lane::Call,
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match session.client.recv() {
            Ok(_) => {
                assert!(Instant::now() < deadline);
            }
            Err(closed) => {
                assert_ne!(closed, Closed::Local);
                break;
            }
        }
    }
    session.client.close();
}

/// C-P3-4: the worker that carries the token is `CoreDispatch`'s handler
/// context, and a cancelled call refuses at its gated step: here a `diff`
/// cancelled while it waited to reach `CoreDispatch` answers `cancelled` and
/// takes no open-log slot.
#[test]
fn core_dispatch_refuses_a_cancelled_call_at_its_gated_step() {
    let mut session = Session::open(Limits::default());
    let request = crate::DiffRequest {
        meta: meta(&session.root, "held-diff"),
        ..crate::DiffRequest::default()
    };
    let diff = session.send("diff", None, crate::encode(&request.to_cbor()), Lane::Call);
    let deadline = Instant::now() + Duration::from_secs(20);
    while !session.script.diff_waiting.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "the diff never ran");
        std::thread::sleep(Duration::from_millis(2));
    }
    let cancel = session.cancel(diff);
    session.script.diff_released.store(true, Ordering::SeqCst);
    let error = session.reply(diff).unwrap_err();
    assert_eq!(error.code, crate::GwzErrorCode::Cancelled);
    session.reply(cancel).unwrap();
}

/// S-P2-5: with 64 logs open and none releasable (sealed under 30 s ago),
/// the next `log` is refused with `transport_session_full` (77).
#[test]
fn the_open_log_after_the_limit_is_refused_with_transport_session_full() {
    let mut session = Session::open(Limits::default());
    for _ in 0..64 {
        session.diff_log(1);
    }
    let request = crate::LogRequest {
        meta: meta(&session.root, "log-65"),
        ..crate::LogRequest::default()
    };
    let log = session.send("log", None, crate::encode(&request.to_cbor()), Lane::Call);
    let error = session.reply(log).unwrap_err();
    assert_eq!(error.code, crate::GwzErrorCode::TransportSessionFull);
}

/// S-P2-5: a sealed `diff.output` log is released once its last reader
/// stream ends; a later read answers `operation_expired` (78), and a log id
/// this session never issued, another session's included,
/// `operation_not_found`.
#[test]
fn a_sealed_diff_log_is_released_by_its_last_reader_and_foreign_ids_are_not_found() {
    let mut session = Session::open(Limits::default());
    let log_id = session.diff_log(2);
    let read = session.read_spool("diff.output", &log_id, "reader");
    let response =
        taut_shape::generated::LogReadResponse::decode(&session.reply(read).unwrap()).unwrap();
    assert_eq!(response.records.len(), 2);
    let end = session.end_spool("diff.output", &log_id, "reader");
    session.reply(end).unwrap();
    let again = session.read_spool("diff.output", &log_id, "reader");
    assert_eq!(
        session.reply(again).unwrap_err().code,
        crate::GwzErrorCode::OperationExpired
    );

    let mut other = Session::open(Limits::default());
    let foreign = other.diff_log(1);
    let read = session.read_spool("diff.output", &foreign, "reader");
    assert_eq!(
        session.reply(read).unwrap_err().code,
        crate::GwzErrorCode::OperationNotFound
    );
    let own = other.read_spool("diff.output", &foreign, "reader");
    other.reply(own).unwrap();
}

/// S-P2-5: a `log.output` reply holds at least one record and at most the
/// session's read limit (1 MiB) of them, over commits with 512 KiB messages.
#[test]
fn a_log_output_reply_stays_within_the_read_limit() {
    let mut session = Session::open(Limits::default());
    let spools = session.service.session().spools();
    let record = crate::LogOutputRecord {
        kind: crate::LogOutputRecordKind::Entry,
        entry: Some(crate::LogEntry {
            subject: "s".to_owned(),
            body: Some("m".repeat(512 * 1024)),
            ..crate::LogEntry::default()
        }),
        degradation: None,
    };
    let slot = spools.reserve().unwrap();
    let inner = spools
        .logs
        .sealed_log_for_test(&[record.clone(), record.clone(), record])
        .unwrap();
    let log_id = slot.commit(SpoolKind::Log, inner);
    let mut seen = 0;
    let mut cursor = None;
    for _ in 0..3 {
        let read = taut_shape::generated::LogReadRequest {
            log_id: log_id.clone(),
            stream_id: "reader".to_owned(),
            cursor: cursor.clone(),
            max_records: None,
            max_bytes: Some(4 * 1024 * 1024),
            timeout_ms: Some(0),
        };
        let call = session.send(
            "log.output",
            None,
            taut_shape::cbor::encode(&read.to_cbor()),
            Lane::Call,
        );
        let bytes = session.reply(call).unwrap();
        assert!(bytes.len() <= 1024 * 1024 + 1024, "{} bytes", bytes.len());
        let response = taut_shape::generated::LogReadResponse::decode(&bytes).unwrap();
        assert!(!response.records.is_empty());
        seen += response.records.len();
        cursor = Some(response.next_cursor);
        if seen == 3 {
            break;
        }
    }
    assert_eq!(seen, 3);
}

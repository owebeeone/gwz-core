//! A session served over a real local carrier (a Unix socket, a Windows named pipe) with scripted dispatch (plan step
//! 2.3g1; `GwzSplitCommandChannel.md` §3.1, §3.3, §4.3; `GwzCoreSessionDesign.md` §8). Rows 1, 2, 5, 6, 7 and 9 of the
//! step's brief; rows 2 and 3 at the carrier itself are `carrier/tests/pre_open_rows.rs`.
//!
//! Two kinds of client: `dial`, the 2.3d client, whose carrier reads every byte the host writes; and `Raw`, the
//! bytes of a connection the row writes and reads itself, for a client that sends too early or stops reading.
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gwz_session_contract::{Frame, FrameKey, Tag, Target};
use gwz_session_host::{Limits, Worker};

use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::EventSink;
use crate::server::session::{SessionParts, SessionPolicy};
use crate::session_host::HostContext;
use crate::session_host::served::stream::{ServeFault, Served, StreamSession};
use crate::session_host::served::{
    Dispatch, Ended, Logged, OperationLog, ServedSession, SessionEnded, SpoolKind,
};
use crate::test_support::TempDir;
use crate::transport_host::carrier::{
    CarrierClosed, CarrierLimits, CloseReason, FrameClass, Lane, ProtocolFault, StreamCarrier,
    handshake::{
        Bounds, ClientConfig, ClientOutcome, Control, Hello, Hooks, HostFault, Intent, OpenRequest,
        OpenedInfo, Refusal, State, WireEnd, dial,
    },
    local::{LocalAddress, LocalListener, PeerPolicy, RawConnection, RuntimeDir, connect_stream},
};

/// The longest a row waits for anything before it fails naming it.
pub(super) const WAIT: Duration = Duration::from_secs(20);
/// The stall bound the stall row sets; production's is `SessionPolicy::STALL`.
pub(super) const STALL: Duration = Duration::from_millis(300);
/// The size of each record of a planted `diff.output` log: one reply each, near the 1 MiB read limit.
pub(super) const RECORD: usize = 900 * 1024;

pub(super) fn hello() -> Hello {
    Hello {
        protocol_min: 1,
        protocol_max: 1,
        server: "gwz rows".into(),
        core: "gwz-core rows".into(),
        server_pid: std::process::id(),
    }
}

#[allow(
    clippy::field_reassign_with_default,
    reason = "Limits is non_exhaustive outside its crate"
)]
pub(super) fn small_limits() -> Limits {
    let mut limits = Limits::default();
    limits.outstanding_calls = 8;
    limits.control_reserve = 4;
    limits
}

/// What the scripted handlers saw.
#[derive(Default)]
pub(super) struct Seen {
    pub(super) started: usize,
    /// A local-phase handler's gate refused it.
    gate_refused: bool,
    /// A handler waiting on its token saw it cancelled.
    pub(super) cancel_seen: bool,
}

#[derive(Default)]
pub(super) struct Script {
    pub(super) seen: Mutex<Seen>,
    changed: Condvar,
}

impl Script {
    fn note(&self, change: impl FnOnce(&mut Seen)) {
        change(&mut self.seen.lock().unwrap());
        self.changed.notify_all();
    }

    /// Waits until `check` holds of what was seen; fails naming `what` at the row's bound.
    pub(super) fn wait(&self, what: &str, check: impl Fn(&Seen) -> bool) {
        let (seen, _) = self
            .changed
            .wait_timeout_while(self.seen.lock().unwrap(), WAIT, |seen| !check(seen))
            .unwrap();
        assert!(check(&seen), "timed out waiting for {what}");
    }
}

/// `fetch` only, by its request ID: `net-` waits on its token as a network exchange does; `local-` is a local phase,
/// which only crosses its gate and never reads its token; any other answers at once.
struct RowDispatch {
    script: Arc<Script>,
}

impl Dispatch for RowDispatch {
    fn run(
        &self,
        method: &str,
        request: &[u8],
        _start: &Path,
        worker: &Worker<ServedSession>,
        _events: &dyn EventSink,
    ) -> ModelResult<Vec<u8>> {
        // Every scripted request is a `FetchRequest`'s bytes, whatever the method: resolution reads its field 1.
        let request = crate::FetchRequest::decode(request).unwrap();
        let id = request.meta.request_id.clone();
        self.script.note(|seen| seen.started += 1);
        if id.starts_with("panic-") {
            panic!("the scripted handler of {method} panics");
        }
        if method != "fetch" {
            return Err(ModelError::new(
                ErrorCode::UnsupportedOperation,
                "the rows script fetch only",
            ));
        }
        let deadline = Instant::now() + WAIT;
        if id.starts_with("net-") {
            while !worker.token().is_cancelled() && Instant::now() < deadline {
                let seen = self.script.seen.lock().unwrap();
                let _ = self
                    .script
                    .changed
                    .wait_timeout(seen, Duration::from_millis(5))
                    .unwrap();
            }
            // Only a cancel counts: a handler that reached the row's bound saw none.
            let cancelled = worker.token().is_cancelled();
            self.script.note(|seen| seen.cancel_seen |= cancelled);
            return Err(ModelError::new(
                ErrorCode::Cancelled,
                "the fetch saw its cancel",
            ));
        }
        if id.starts_with("local-") {
            while Instant::now() < deadline {
                if let Err(refused) = worker.effect(|_| ()) {
                    self.script.note(|seen| seen.gate_refused = true);
                    return Err(refused.into());
                }
                // The phase's own work between two crossings.
                thread::sleep(Duration::from_millis(1));
            }
        }
        let response = crate::FetchResponse {
            response: crate::ResponseEnvelope {
                meta: crate::ResponseMeta {
                    request_id: id,
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

/// The core-side log, as the rows read it.
#[derive(Default)]
pub(super) struct Recorded {
    entries: Mutex<Vec<Logged>>,
    ends: Mutex<Vec<SessionEnded>>,
    changed: Condvar,
}

impl OperationLog for Recorded {
    fn ended(&self, entry: Logged) {
        self.entries.lock().unwrap().push(entry);
        self.changed.notify_all();
    }

    fn session_ended(&self, end: SessionEnded) {
        self.ends.lock().unwrap().push(end);
        self.changed.notify_all();
    }
}

impl Recorded {
    pub(super) fn wait_for(&self, count: usize) -> Vec<Logged> {
        let (entries, _) = self
            .changed
            .wait_timeout_while(self.entries.lock().unwrap(), WAIT, |entries| {
                entries.len() < count
            })
            .unwrap();
        assert!(entries.len() >= count, "the log has {:?}", *entries);
        entries.clone()
    }

    /// The first session end logged, waiting for it at the row's bound.
    pub(super) fn wait_end(&self) -> SessionEnded {
        let (ends, _) = self
            .changed
            .wait_timeout_while(self.ends.lock().unwrap(), WAIT, |ends| ends.is_empty())
            .unwrap();
        ends.first().cloned().expect("no session end was logged")
    }
}

/// The composition's own hooks, as a server's would be: admits every open (the session count is step 2.3g2's) and
/// answers no control. With `hold`, its open waits for that carrier to end first, so a row fixes the order of what
/// the client sent against what the host has written.
#[derive(Default)]
pub(super) struct Admit {
    pub(super) hold: Option<Arc<StreamCarrier>>,
}

impl Hooks for Admit {
    fn open(&mut self, _open: &OpenRequest) -> Result<OpenedInfo, Refusal> {
        if let Some(carrier) = &self.hold {
            assert!(
                carrier.wait_closed(WAIT).is_some(),
                "the carrier did not end"
            );
        }
        Ok(OpenedInfo { open_sessions: 0 })
    }

    fn control(&mut self, _control: &Control) -> Result<State, Refusal> {
        Err(Refusal {
            code: 1,
            message: "the rows answer no control".into(),
        })
    }
}

/// A private directory for the listener (Unix); the pipe namespace (Windows).
pub(super) struct Dir {
    pub(super) runtime: RuntimeDir,
    _guard: Option<tempfile::TempDir>,
}

pub(super) fn dir() -> Dir {
    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            let guard = tempfile::Builder::new().prefix("gwz").tempdir().expect("tempdir");
            std::fs::set_permissions(guard.path(), std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
            let runtime = RuntimeDir::at(guard.path()).expect("a private directory");
            Dir { runtime, _guard: Some(guard) }
        } else {
            let env = crate::transport_host::carrier::local::RuntimeEnv::default();
            Dir { runtime: RuntimeDir::for_current_user(&env).expect("runtime"), _guard: None }
        }
    }
}

pub(super) fn meta(root: &Path, request_id: &str) -> crate::RequestMeta {
    crate::RequestMeta {
        request_id: request_id.to_owned(),
        schema_version: "gwz.v0".to_owned(),
        invocation: Some(crate::InvocationContext {
            caller_cwd: root.display().to_string(),
        }),
        ..crate::RequestMeta::default()
    }
}

/// A workspace for the session to serve, as resolution finds it.
fn workspace() -> (TempDir, PathBuf) {
    let temp = TempDir::new("served-stream");
    let root = temp.path().join("ws");
    std::fs::create_dir_all(&root).unwrap();
    crate::workspace_ops::handle_create_workspace(
        crate::CreateWorkspaceRequest {
            meta: meta(&root, "create"),
            workspace_root: root.display().to_string(),
            workspace_id: Some("ws_stream".to_owned()),
        },
        "op-create",
    )
    .unwrap();
    let start = crate::workspace_ops::caller_directory(&meta(&root, "x")).unwrap();
    let root =
        crate::workspace_ops::resolve_request_workspace_root(&start, &meta(&root, "x")).unwrap();
    (temp, root)
}

/// A server's side: one listener under the policy, one host context, one workspace.
pub(super) struct Host {
    pub(super) policy: SessionPolicy,
    listener: LocalListener,
    pub(super) context: HostContext,
    pub(super) root: PathBuf,
    pub(super) script: Arc<Script>,
    pub(super) log: Arc<Recorded>,
    _workspace: TempDir,
    _dir: Dir,
}

impl Host {
    pub(super) fn new(policy: SessionPolicy) -> Self {
        let dir = dir();
        let limits = policy.carrier_limits().expect("carrier limits");
        let listener =
            LocalListener::bind(&dir.runtime, PeerPolicy::current_user().unwrap(), limits)
                .expect("bind");
        let (workspace, root) = workspace();
        Self {
            policy,
            listener,
            context: HostContext::new(),
            root,
            script: Arc::default(),
            log: Arc::default(),
            _workspace: workspace,
            _dir: dir,
        }
    }

    pub(super) fn address(&self) -> &LocalAddress {
        self.listener.address()
    }

    pub(super) fn accept(&self) -> Arc<StreamCarrier> {
        Arc::new(self.listener.accept().expect("accepted"))
    }

    pub(super) fn serve(
        &self,
        carrier: Arc<StreamCarrier>,
        outer: &mut Admit,
    ) -> Result<Served, ServeFault> {
        let parts = SessionParts {
            dispatch: Arc::new(RowDispatch {
                script: Arc::clone(&self.script),
            }),
            log: Arc::clone(&self.log) as Arc<dyn OperationLog>,
            root: self.root.clone(),
            host: &self.context,
        };
        self.policy.serve(carrier, parts, outer)
    }

    /// Accepts the next connection and serves it as a session.
    pub(super) fn session(&self) -> StreamSession {
        match self.serve(self.accept(), &mut Admit::default()) {
            Ok(Served::Session(session)) => session,
            Ok(Served::Ended(outcome)) => panic!("no session: {outcome:?}"),
            Err(fault) => panic!("no session: {fault:?}"),
        }
    }

    /// A session with a `dial` client: the host's side and the client's carrier.
    pub(super) fn dialled(&self) -> (StreamSession, StreamCarrier) {
        thread::scope(|scope| {
            let host = scope.spawn(|| self.session());
            let config = ClientConfig {
                supported: 1..=1,
                client: "gwz rows".into(),
                limits: None,
                peer_pid: None,
                bounds: Bounds::DEFAULT,
            };
            let (client, outcome) = dial(
                self.address(),
                &PeerPolicy::current_user().unwrap(),
                CarrierLimits::DEFAULT,
                &config,
                &Intent::Open,
            )
            .expect("the dial");
            assert!(matches!(outcome, ClientOutcome::Opened { .. }));
            (host.join().unwrap(), client)
        })
    }

    /// A session with a raw client that has read `SessionOpened`.
    pub(super) fn raw_session(&self) -> (StreamSession, Raw) {
        thread::scope(|scope| {
            let host = scope.spawn(|| self.session());
            let mut raw = Raw::connect(self.address());
            raw.open();
            (host.join().unwrap(), raw)
        })
    }

    /// A `diff.output` log of `records` records of [`RECORD`] bytes in `session`, as `diff` registers one.
    pub(super) fn diff_log(session: &StreamSession, records: usize) -> String {
        Self::diff_log_of(session, records, RECORD)
    }

    /// A `diff.output` log of `records` records of `bytes` bytes in `session`.
    pub(super) fn diff_log_of(session: &StreamSession, records: usize, bytes: usize) -> String {
        let spools = session.service().session().spools();
        let slot = spools.reserve().unwrap();
        let (inner, log) = spools.diffs.create();
        for _ in 0..records {
            log.push(vec![7; bytes]);
        }
        log.seal();
        slot.commit(SpoolKind::Diff, inner)
    }
}

pub(super) fn fetch_call(root: &Path, call_id: i64, request_id: &str) -> Frame {
    let request = crate::FetchRequest {
        meta: meta(root, request_id),
    };
    call_frame(call_id, "fetch", crate::encode(&request.to_cbor()))
}

pub(super) fn call_frame(call_id: i64, method: &str, request: Vec<u8>) -> Frame {
    let call = crate::SessionCall {
        call_id,
        method: method.to_owned(),
        mode: None,
        request,
        log_verb: None,
    };
    Frame::new(Tag::SessionCall, crate::encode(&call.to_cbor()))
}

pub(super) fn spool_read(call_id: i64, log_id: &str, reader: usize) -> Frame {
    let read = taut_shape::generated::LogReadRequest {
        log_id: log_id.to_owned(),
        stream_id: format!("reader-{reader}"),
        cursor: None,
        max_records: None,
        max_bytes: None,
        timeout_ms: Some(0),
    };
    call_frame(
        call_id,
        "diff.output",
        taut_shape::cbor::encode(&read.to_cbor()),
    )
}

pub(super) fn cancel_call(call_id: i64, target: i64) -> Frame {
    let cancel = crate::OperationCancelRequest {
        call_id: Some(target),
        operation_id: None,
    };
    call_frame(
        call_id,
        "operation.cancel",
        crate::encode(&cancel.to_cbor()),
    )
}

pub(super) fn send(client: &StreamCarrier, frame: Frame, call_id: i64) {
    client
        .send_frame(
            frame,
            Lane::Data,
            FrameKey::ordinary(Target::new(call_id as u64)),
        )
        .expect("the call is queued");
}

pub(super) fn call_id_of(frame: &Frame) -> i64 {
    if frame.tag == Tag::SessionReply.byte() {
        crate::SessionReply::decode(&frame.body).unwrap().call_id
    } else {
        assert_eq!(frame.tag, Tag::SessionError.byte());
        crate::SessionError::decode(&frame.body).unwrap().call_id
    }
}

/// The client's next reply, which must answer `call_id`.
pub(super) fn reply(client: &StreamCarrier, call_id: i64) -> Result<Vec<u8>, Box<crate::GwzError>> {
    let frame = client
        .recv_registry_timeout(WAIT)
        .expect("the carrier is open")
        .unwrap_or_else(|| panic!("no reply to call {call_id}"));
    assert_eq!(call_id_of(&frame), call_id);
    if frame.tag == Tag::SessionReply.byte() {
        Ok(crate::SessionReply::decode(&frame.body).unwrap().response)
    } else {
        Err(Box::new(
            crate::SessionError::decode(&frame.body).unwrap().error,
        ))
    }
}

/// A connection whose bytes the row reads and writes itself.
pub(super) struct Raw {
    pub(super) connection: RawConnection,
}

impl Raw {
    pub(super) fn connect(address: &LocalAddress) -> Self {
        let policy = PeerPolicy::current_user().unwrap();
        let connection =
            connect_stream(address, &policy, Instant::now() + WAIT).expect("the connect");
        Self { connection }
    }

    pub(super) fn write(&mut self, frames: &[Frame]) {
        assert!(self.try_write(frames), "the write");
    }

    /// Writes `frames`; `false` when the host has already closed the stream.
    pub(super) fn try_write(&mut self, frames: &[Frame]) -> bool {
        let mut bytes = Vec::new();
        for frame in frames {
            let length = u32::try_from(frame.body.len() + 1).unwrap();
            bytes.extend_from_slice(&length.to_le_bytes());
            bytes.push(frame.tag);
            bytes.extend_from_slice(&frame.body);
        }
        self.connection.write.write_all(&bytes).is_ok() && self.connection.write.flush().is_ok()
    }

    /// The next frame; fails the row if none arrives within [`WAIT`].
    pub(super) fn read(&mut self) -> Frame {
        self.try_read().expect("a frame")
    }

    /// The next frame, or `None` at the end of the stream. A read that is still blocked at [`WAIT`] aborts the
    /// connection and fails the row naming the wait, so a host that never writes fails the row rather than hangs it.
    pub(super) fn try_read(&mut self) -> Option<Frame> {
        let RawConnection { read, abort, .. } = &mut self.connection;
        let abort: &(dyn Fn() + Send + Sync) = &**abort;
        let (frame, timed_out) = thread::scope(|scope| {
            let (done, finished) = std::sync::mpsc::channel();
            let reader = scope.spawn(move || {
                let frame = read_frame(&mut **read);
                let _ = done.send(());
                frame
            });
            let timed_out = finished.recv_timeout(WAIT).is_err();
            if timed_out {
                abort();
            }
            (reader.join().expect("the reader"), timed_out)
        });
        assert!(!timed_out, "no frame within {WAIT:?}");
        match frame {
            Ok(frame) => Some(frame),
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => None,
            Err(error) => panic!("the read: {error}"),
        }
    }

    pub(super) fn session_open() -> Frame {
        let open = OpenRequest {
            protocol: 1,
            client: "gwz rows".into(),
            limits: None,
        };
        Frame::new(Tag::SessionOpen, open.encode())
    }

    /// The handshake, as a client that keeps the rules: the hello, the open, `SessionOpened`.
    pub(super) fn open(&mut self) {
        assert_eq!(self.read().tag, Tag::SessionHello.byte());
        self.write(&[Self::session_open()]);
        assert_eq!(self.read().tag, Tag::SessionOpened.byte());
    }
}

/// One length-prefixed frame from `read`.
fn read_frame(read: &mut (dyn Read + Send)) -> std::io::Result<Frame> {
    let mut prefix = [0u8; 4];
    read.read_exact(&mut prefix)?;
    let mut rest = vec![0u8; u32::from_le_bytes(prefix) as usize];
    read.read_exact(&mut rest)?;
    Ok(Frame {
        tag: rest[0],
        body: rest[1..].to_vec(),
    })
}

/// Row 1: a call round-trips over a real local carrier with scripted dispatch, and `session.close` ends the session
/// and its serving thread. The service was opened before `SessionOpened` (C11): the call the client sends at once is
/// served, not a protocol error. The log records the call.
#[test]
fn a_call_round_trips_over_a_real_local_carrier_and_close_ends_the_session() {
    let host = Host::new(SessionPolicy::new(hello()));
    let (session, client) = host.dialled();
    send(&client, fetch_call(&host.root, 1, "plain-1"), 1);
    let response = crate::FetchResponse::decode(&reply(&client, 1).unwrap()).unwrap();
    assert_eq!(response.response.meta.request_id, "plain-1");
    let logged = host.log.wait_for(1);
    assert_eq!(
        (
            logged[0].method.as_str(),
            logged[0].call_id,
            logged[0].ended
        ),
        ("fetch", 1, Ended::Completed)
    );
    assert!(logged[0].operation_id.is_some() && logged[0].duration.is_some());
    let close = crate::SessionCloseRequest {};
    send(
        &client,
        call_frame(2, "session.close", crate::encode(&close.to_cbor())),
        2,
    );
    let report = crate::CleanupReport::decode(&reply(&client, 2).unwrap()).unwrap();
    assert_eq!(report.pending_local_work, 0);
    assert!(session.service().wait_ended(WAIT));
    let closed = session
        .carrier()
        .wait_closed(WAIT)
        .expect("the carrier ends");
    assert_eq!(
        closed.reason,
        CloseReason::Local(crate::transport_host::carrier::CloseCode::Normal)
    );
    let deadline = Instant::now() + WAIT;
    while !session.is_finished() {
        assert!(
            Instant::now() < deadline,
            "the serving thread did not return"
        );
        thread::sleep(Duration::from_millis(2));
    }
}

/// Row 2 (served): a client that writes a `SessionCall` right behind its `SessionOpen`, before it reads
/// `SessionOpened`, gets no session. The composition's hook waits for the carrier's end, so the call is read while the
/// host has not yet queued `SessionOpened`; the open is then a handshake fault, and nothing but the hello reaches the
/// client.
#[test]
fn a_call_before_session_opened_gets_no_session() {
    let host = Host::new(SessionPolicy::new(hello()));
    let (served, after_hello) = thread::scope(|scope| {
        let server = scope.spawn(|| {
            let carrier = host.accept();
            let mut outer = Admit {
                hold: Some(Arc::clone(&carrier)),
            };
            host.serve(carrier, &mut outer)
        });
        let mut raw = Raw::connect(host.address());
        assert_eq!(raw.read().tag, Tag::SessionHello.byte());
        raw.write(&[Raw::session_open(), fetch_call(&host.root, 1, "early-1")]);
        let served = server.join().unwrap();
        let mut rest = Vec::new();
        let _ = raw.connection.read.read_to_end(&mut rest);
        (served, rest)
    });
    let expected = CarrierClosed {
        reason: CloseReason::Protocol(ProtocolFault::BeforeOpened(FrameClass::Registry)),
    };
    match served {
        Err(ServeFault::Handshake(HostFault::Wire(WireEnd::Closed(closed)))) => {
            assert_eq!(closed, expected);
        }
        Err(other) => panic!("{other:?}"),
        Ok(_) => panic!("the early call was served"),
    }
    assert!(after_hello.is_empty(), "{} more bytes", after_hello.len());
    assert_eq!(host.script.seen.lock().unwrap().started, 0);
    // The rollback: the session the hook opened is closed, so its threads end and release what its ports hold.
    let deadline = Instant::now() + WAIT;
    while Arc::strong_count(&host.log) > 1 {
        assert!(
            Instant::now() < deadline,
            "the faulted session kept its threads: the open hook's session was not closed"
        );
        thread::sleep(Duration::from_millis(2));
    }
}

/// Row 5 (R1, C8): a client that stops reading, with no EOF, ends its session at the policy's stall bound, and the
/// next client of the same host context is served. Production's bound is 60 s.
#[test]
fn a_client_that_stops_reading_ends_at_the_stall_bound_and_the_next_client_is_served() {
    let production = SessionPolicy::new(hello());
    assert_eq!(
        production.stall_bounds().no_progress,
        Duration::from_secs(60)
    );
    assert_eq!(
        production.stall_bounds().close,
        production.limits.close_wait
    );
    let carrier_limits = production.carrier_limits().unwrap();
    assert_eq!(carrier_limits.stall(), production.stall_bounds());
    assert!(carrier_limits.is_awaiting_open());

    let mut policy = SessionPolicy::new(hello());
    policy.limits = small_limits();
    policy.stall = STALL;
    let host = Host::new(policy);
    let (session, mut raw) = host.raw_session();
    let log_id = Host::diff_log(&session, 8);
    let started = Instant::now();
    let reads: Vec<Frame> = (0..6)
        .map(|reader| spool_read(reader as i64 + 1, &log_id, reader))
        .collect();
    raw.write(&reads);
    // The client reads nothing more and keeps its end open.
    let closed = session
        .carrier()
        .wait_closed(WAIT)
        .expect("the session ends");
    let elapsed = started.elapsed();
    assert_eq!(closed.reason, CloseReason::Stalled);
    assert!(elapsed >= STALL && elapsed < WAIT, "{elapsed:?}");
    assert!(session.service().wait_ended(WAIT));

    let (_next, client) = host.dialled();
    send(&client, fetch_call(&host.root, 1, "next-1"), 1);
    let response = crate::FetchResponse::decode(&reply(&client, 1).unwrap()).unwrap();
    assert_eq!(response.response.meta.request_id, "next-1");
    drop(raw);
}

/// Row 6 (2.0 §4.3 "Carrier loss"): the carrier's loss cancels every live operation of the session: one that waits on
/// its token and one in a local phase, which only crosses its gate and is refused at the next crossing. The log gets
/// each one's method, IDs, outcome and duration.
#[test]
fn carrier_loss_cancels_every_live_operation_and_logs_each() {
    let host = Host::new(SessionPolicy::new(hello()));
    let (session, client) = host.dialled();
    send(&client, fetch_call(&host.root, 1, "net-1"), 1);
    send(&client, fetch_call(&host.root, 2, "local-1"), 2);
    host.script.wait("both handlers", |seen| seen.started == 2);
    client.abort();
    assert!(
        session.service().wait_ended(WAIT),
        "the session did not end"
    );
    let mut logged = host.log.wait_for(2);
    logged.sort_by_key(|entry| entry.call_id);
    for (entry, call_id) in logged.iter().zip([1, 2]) {
        assert_eq!(entry.method, "fetch");
        assert_eq!(entry.call_id, call_id);
        assert!(entry.operation_id.is_some(), "{entry:?}");
        assert_eq!(entry.ended, Ended::Cancelled, "{entry:?}");
        assert!(entry.duration.is_some(), "{entry:?}");
        assert!(entry.to_string().contains("outcome=cancelled"), "{entry}");
    }
    let seen = host.script.seen.lock().unwrap();
    assert!(seen.cancel_seen && seen.gate_refused);
    let end = host.log.wait_end();
    assert_eq!(end.ending, gwz_session_host::Ending::Channel, "{end}");
}

/// Row 7 (2.2b's saturated-spool row, over the real carrier): while the client reads nothing and the host's writer is
/// held behind spool replies, `operation.cancel` reaches the running handler and its reply is written ahead of spool
/// replies queued before it.
#[test]
fn a_cancel_progresses_under_a_saturated_spool_over_the_real_carrier() {
    let mut policy = SessionPolicy::new(hello());
    policy.limits = small_limits();
    let host = Host::new(policy);
    let (session, mut raw) = host.raw_session();
    let log_id = Host::diff_log(&session, 8);
    raw.write(&[fetch_call(&host.root, 1, "net-1")]);
    host.script.wait("the fetch", |seen| seen.started == 1);
    let reads: Vec<Frame> = (0..6)
        .map(|reader| spool_read(reader as i64 + 2, &log_id, reader))
        .collect();
    raw.write(&reads);
    // The writer holds the first reply in a full stream and queues the rest behind it.
    let deadline = Instant::now() + WAIT;
    while session.carrier().queued_out() < 2 {
        assert!(Instant::now() < deadline, "the spool replies never queued");
        thread::sleep(Duration::from_millis(2));
    }
    raw.write(&[cancel_call(8, 1)]);
    host.script
        .wait("the cancel at the handler", |seen| seen.cancel_seen);
    let mut order = Vec::new();
    while order.len() < 8 {
        order.push(call_id_of(&raw.read()));
    }
    let cancel = order.iter().position(|&id| id == 8).unwrap();
    let last_spool = order.iter().rposition(|&id| (2..=7).contains(&id)).unwrap();
    assert!(
        cancel < last_spool,
        "the cancel's reply came last: {order:?}"
    );
    assert!(session.carrier().wait_closed(Duration::ZERO).is_none());
}

/// Row 9 (carry-in from 2.3c): the byte-stream session's member lock manager is its host context's: a member the
/// host context holds is held in the session's manager.
#[test]
fn the_stream_session_shares_its_host_contexts_member_locks() {
    let host = Host::new(SessionPolicy::new(hello()));
    let (session, _client) = host.dialled();
    let members = session.service().session().members();
    assert_eq!(members.held_count(), 0);
    let held = host
        .context
        .members()
        .lock(&host.root, "mem_app", None)
        .unwrap();
    assert_eq!(members.held_count(), 1);
    drop(held);
    assert_eq!(members.held_count(), 0);
}

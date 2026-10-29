//! The operation service: the session host that serves one session's calls
//! (gwz-dev `dev-docs/GwzCoreSessionDesign.md` §4.1, §4.2 and §5, as
//! `GwzSplitCommandChannel.md` §3.2 adopts them for the served core; step 2.2b
//! of `GwzTransportReleaseConvergencePlan.md`).
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | Each call's record, token and gate are created when its frame is read, before resolution, admission, any reply or any worker | O7, §5.1 |
//! | A `call_id` not greater than every earlier one, a body that does not decode, and any call before the session has opened are protocol errors | §3, §4.1; server design §3 |
//! | Resolution and the class decision run on one admission thread, in receipt order; control frames and reads never wait for it | §5.1 |
//! | Classes: direct methods on a pool of direct workers, outside the running limit; fetch beside fetch and push; at most one push per workspace; a write beside nothing but direct methods | §4.2, §5.1, §5.2 |
//! | At most 8 operations run; 64 wait in one FIFO queue, started in queue order per workspace; 128 table entries, evicting the oldest delivered terminal record; 1024 outstanding calls and 64 control calls; each excess refused with `transport_session_full` before any effect | §5.1, §5.4 |
//! | A write or push on a workspace where a worker detached at a close bound still runs is refused, in every session sharing the host's record | `GwzSplitCommandChannel.md` §1, §3.2 |
//! | `unary` replies with the final response; `submit` replies `accepted` once admitted and keeps the record until release, eviction or the session's end | §4.2, §5.4 |
//! | `operation.cancel` by `call_id` or `operation_id`: settles a call not yet admitted, a queued operation or a waiting direct call before it runs, and otherwise cancels the token and replies once the target is terminal | §4.2, §5.3 |
//! | Events are appended through the operation's gate and read with `events.subscribe`; a read with nothing to return is held until an append, the log's completion, its wait or the session's end | §4.2, §5.4, §6 |
//! | `session.close` and channel closure: settle what has not run, cancel what runs, wait at most `close_wait`, revoke and detach what still runs, answer what is outstanding, and close | §8 |
//!
//! Bytes only, as the crate is: core's [`HostPorts`] decodes every call,
//! classifies its method, resolves its workspace, runs it and encodes every
//! reply, and this module never looks inside a body. It keeps no global
//! state: each service owns its threads and its table.

use std::fmt;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gwz_session_contract::{Frame, FrameKey, FrameSink, Lane, SendError, Target};

use crate::detached::DetachedWorkers;
use crate::gate::{CallControls, GateScope, OperationGate};
use crate::limits::{Limits, MAX_READ_WAIT};
use crate::lock;
use crate::token::{CancellationToken, Refused};

mod events;
mod table;
mod workers;

use events::EventLog;
use table::State;

/// One call as core decoded its `SessionCall` frame (§4.1).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Call {
    /// The client's identifier: at least 1, and greater than every earlier
    /// one in the session.
    pub call_id: u64,
    /// The method's name.
    pub method: String,
    /// How an operation method replies.
    pub mode: Mode,
    /// The method's request message, which the host never decodes.
    pub request: Vec<u8>,
    /// What a call to a log method does; `None` when the call carries no
    /// `log_verb`, which reads.
    pub verb: Option<Verb>,
}

/// How an operation method replies (§4.2).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Mode {
    /// With its final response, when the operation ends.
    Unary,
    /// With its `accepted` response, as soon as the host admits it.
    Submit,
}

/// What a call to a log method does (§4.2).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Verb {
    /// Reads records.
    Read,
    /// Ends the reader's stream.
    EndStream,
}

/// An operation method's class (§5.1). Direct methods are class R and are
/// [`Request::Direct`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Class {
    /// Class N, a fetch: runs beside direct methods and other class N
    /// operations.
    Fetch,
    /// Class N, a push: as a fetch, but at most one runs per workspace.
    Push,
    /// Class W, a workspace write, and any method core's table does not name:
    /// runs beside direct methods only.
    Write,
}

/// What a call asks of the host, as core's method registry classifies it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Request {
    /// A direct method (§4.2): it replies with its final response whatever
    /// its mode, and runs on a direct worker.
    Direct,
    /// An operation method of `class`.
    Operation {
        /// Its class.
        class: Class,
        /// Whether the method declares a response view beside its result,
        /// as `merge` does: its record is delivered only once both have been
        /// handed over (§5.4).
        response_view: bool,
    },
    /// An `events.subscribe` call.
    Events(EventsCall),
    /// A `diff.output` or `log.output` call, which core answers from the
    /// session's own registries on the reading thread.
    Spool,
    /// `operation.result`.
    Result {
        /// The operation.
        operation_id: String,
    },
    /// `operation.response`.
    Response {
        /// The operation.
        operation_id: String,
    },
    /// `operation.cancel`.
    Cancel(CancelTarget),
    /// `operation.release`.
    Release {
        /// The operation.
        operation_id: String,
    },
    /// `session.close`.
    Close,
}

/// The target of `operation.cancel` (§4.2).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CancelTarget {
    /// The call that created a unary, submitted or direct record.
    Call(u64),
    /// An operation.
    Operation(String),
}

/// An `events.subscribe` call (§4.2).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EventsCall {
    /// A read of the operation's event log.
    Read(EventsRead),
    /// The end of a reader's stream; it releases nothing.
    End {
        /// The operation.
        operation_id: String,
        /// The reader.
        stream_id: String,
    },
}

/// A read of an operation's event log (§4.2): the records after `cursor`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventsRead {
    /// The operation.
    pub operation_id: String,
    /// The reader.
    pub stream_id: String,
    /// The last record the reader consumed; `None` reads from the start.
    pub cursor: Option<u64>,
    /// At most this many records.
    pub max_records: Option<usize>,
    /// At most this many payload bytes, and never more than the session's
    /// read limit.
    pub max_bytes: Option<usize>,
    /// How long to hold the read when it has nothing to return: at most
    /// [`MAX_READ_WAIT`].
    pub wait: Duration,
}

/// What an events read returns.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EventsBatch {
    /// The records after the cursor, each with its sequence number.
    pub records: Vec<(u64, Vec<u8>)>,
    /// The cursor to read on from.
    pub next_cursor: u64,
    /// Whether the log is complete: its operation is terminal and every
    /// record has been returned.
    pub complete: bool,
}

/// What resolution found (§5.1).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Resolved {
    /// The workspace the request acts on, as a key: the served core's one
    /// root.
    pub workspace: String,
    /// The request's `RequestMeta.request_id`, unique among the session's
    /// live operations (§4.3).
    pub request_id: String,
}

/// The cleanup facts of an operation or a session: the fields of the
/// transport's cleanup report (§13 `CleanupReport`).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct Cleanup {
    /// Local work still running.
    pub pending_local_work: u32,
    /// Whether a peer confirmed its cleanup.
    pub peer_cleanup_confirmed: bool,
}

/// How a call ended (§5.2).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Terminal<F> {
    /// The handler produced a response, even one whose aggregate status is
    /// failed.
    Completed(Vec<u8>),
    /// The handler did not succeed and the token had been cancelled.
    Cancelled(F),
    /// Anything else.
    Failed(F),
}

/// What a worker reports through its gate, once.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Outcome<F> {
    /// The terminal.
    pub terminal: Terminal<F>,
    /// The encoded `OperationResult`, for an operation method.
    pub result: Vec<u8>,
    /// The cleanup facts.
    pub cleanup: Cleanup,
}

/// A refusal or failure the host itself produces. Core maps each code to its
/// wire code (`invalid_request`, `operation_not_found`, `open_operation`,
/// `operation_expired`, `transport_session_full`, `cancelled`,
/// `internal_error`, and `unsupported_operation` for a busy workspace).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostError {
    /// What kind of refusal.
    pub code: Code,
    /// Its text.
    pub message: String,
}

impl HostError {
    /// A refusal of `code` with `message`.
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for HostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for HostError {}

/// The kinds of [`HostError`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Code {
    /// A call the host refuses as malformed, or one after `session.close`.
    InvalidRequest,
    /// A target the session never issued.
    OperationNotFound,
    /// `operation.release` of a live operation.
    OpenOperation,
    /// A released or evicted record, or a unary call already answered.
    OperationExpired,
    /// A session limit.
    SessionFull,
    /// Cancelled before it ran, or detached at the close bound.
    Cancelled,
    /// A worker that could not start, or a handler that panicked.
    Internal,
    /// A write or push on a workspace where a detached worker still runs.
    WorkspaceBusy,
}

/// A frame the session cannot accept: it ends the session (§3, §8). No reply
/// is attributed to it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtocolError(pub String);

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "session protocol error: {}", self.0)
    }
}

impl std::error::Error for ProtocolError {}

/// What a worker holds while it runs a call: its gate, its token and, for an
/// operation method, its event log (O8, §5.2, §5.6).
///
/// It is `Sync`, so a handler that reports events from several threads
/// crosses its one gate from each in turn.
pub struct Worker<S> {
    gate: Mutex<OperationGate<S>>,
    token: CancellationToken,
    events: Option<Arc<EventLog>>,
    operation_id: Option<String>,
}

impl<S> Worker<S> {
    /// The call's token, which only `operation.cancel` and close cancel.
    pub fn token(&self) -> &CancellationToken {
        &self.token
    }

    /// The operation's ID, for an operation method.
    pub fn operation_id(&self) -> Option<&str> {
        self.operation_id.as_deref()
    }

    /// Appends an encoded event to the operation's log through the gate
    /// (§5.6, §6). A direct call has no log, and the event is dropped.
    ///
    /// # Errors
    ///
    /// [`Refused::Cancelled`] once the token is cancelled, so a producer
    /// stops, and [`Refused::Revoked`] after revocation.
    pub fn event(&self, record: Vec<u8>) -> Result<(), Refused> {
        let Some(events) = self.events.as_ref() else {
            return Ok(());
        };
        lock(&self.gate).append(|_| events.push(record))
    }

    /// An effectful request through the gate: a lock, a transport open or a
    /// log registration (§5.6).
    ///
    /// # Errors
    ///
    /// [`Refused::Cancelled`] or [`Refused::Revoked`], without running `f`.
    pub fn effect<R>(&self, f: impl FnOnce(GateScope<'_, S>) -> R) -> Result<R, Refused> {
        lock(&self.gate).effect(f)
    }
}

impl<S> fmt::Debug for Worker<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Worker")
            .field("operation_id", &self.operation_id)
            .finish_non_exhaustive()
    }
}

/// What core implements for the host (the crate map's `HostPorts`, §2):
/// decoding a call, classifying its method, resolving its workspace, running
/// it and encoding replies. Every body is core's, so the host stays bytes
/// only.
pub trait HostPorts: Send + Sync + 'static {
    /// Core's per-session data, which gates hold weakly.
    type Session: Send + Sync + 'static;
    /// A terminal's error as core reports it: its `GwzError` and, when there
    /// is one, its `ResponseMeta`.
    type Failure: Clone + Send + Sync + 'static;

    /// Decodes a `SessionCall` body.
    ///
    /// # Errors
    ///
    /// A body that does not decode, which is a protocol error (§3).
    fn decode(&self, body: &[u8]) -> Result<Call, ProtocolError>;

    /// The lane a call of `method`, and its reply, travel on (§3): control
    /// calls, `operation.cancel` and `session.close`, on the control lane,
    /// every other call on the call lane. It is decided from the method's name
    /// before anything is decoded, so a malformed control call's refusal
    /// still takes the control lane.
    fn lane(&self, method: &str) -> Lane;

    /// Classifies a call by its method (core's method registry and class
    /// table, §5.1), decoding the request messages of the methods declared
    /// with scalar parameters. Any method its table does not name is class W.
    ///
    /// # Errors
    ///
    /// A malformed call, such as a log verb on a method that is not a log
    /// method, refused with `invalid_request` before any effect.
    fn classify(&self, call: &Call) -> Result<Request, HostError>;

    /// Validates and resolves a direct or operation call on the admission
    /// thread (§5.1).
    ///
    /// # Errors
    ///
    /// The refusal, settled before any effect.
    fn resolve(&self, session: &Self::Session, call: &Call) -> Result<Resolved, Self::Failure>;

    /// Runs a direct or operation call on its worker (§5.2).
    fn run(
        &self,
        call: &Call,
        resolved: &Resolved,
        worker: &Worker<Self::Session>,
    ) -> Outcome<Self::Failure>;

    /// Answers a `diff.output` or `log.output` call from the session's own
    /// registries, without blocking (§4.2, §5.4).
    ///
    /// # Errors
    ///
    /// Its refusal.
    fn spool(&self, session: &Self::Session, call: &Call) -> Result<Vec<u8>, Self::Failure>;

    /// Releases the session's `diff.output` and `log.output` logs (§8 step 7).
    fn release_spools(&self, session: &Self::Session);

    /// Core's failure for a host refusal.
    fn failure(&self, error: HostError) -> Self::Failure;

    /// The encoded `OperationResult` of an operation that ended without
    /// running its handler, with `failure` (§6). `call` is the operation's own.
    fn settled_result(&self, call: &Call, operation_id: &str, failure: &Self::Failure) -> Vec<u8>;

    /// The terminal of an operation detached at the close bound: `Cancelled`,
    /// with the second error "detached at the close bound; outcome unknown"
    /// (§6, §8 step 5), and its encoded `OperationResult`.
    fn detached(&self, call: &Call, operation_id: Option<&str>) -> (Self::Failure, Vec<u8>);

    /// The `accepted` response of a submitted operation (§4.2).
    fn accepted(&self, call: &Call, resolved: &Resolved, operation_id: &str) -> Vec<u8>;

    /// The encoded reset event that stands for history an event log dropped
    /// (§5.4).
    fn reset_event(&self, operation_id: &str) -> Vec<u8>;

    /// An events read's reply body (taut-shape's `LogReadResponse`).
    fn events(&self, read: &EventsRead, batch: &EventsBatch) -> Vec<u8>;

    /// An ended events stream's reply body.
    fn events_ended(&self, operation_id: &str, stream_id: &str) -> Vec<u8>;

    /// A `CleanupReport` reply body.
    fn cleanup(&self, report: Cleanup) -> Vec<u8>;

    /// An `OperationReleaseResponse` reply body.
    fn released(&self) -> Vec<u8>;

    /// A `SessionReply` frame.
    fn reply(&self, call_id: u64, body: Vec<u8>) -> Frame;

    /// A `SessionError` frame.
    fn error(&self, call_id: u64, failure: &Self::Failure) -> Frame;
}

/// Where the host's replies go: the session's channel end.
///
/// `target` is the call's ID, and `control` the frame's class: true for a
/// control call's reply, false for every other reply and for the close
/// report, which is exempt from priority (`GwzSplitCommandChannel.md` §3.3).
/// An in-process end uses `lane` and ignores the rest; a byte stream orders by
/// class and target.
pub trait ReplySink: Send + Sync + 'static {
    /// Sends one reply.
    ///
    /// # Errors
    ///
    /// The channel's refusal; the frame comes back.
    fn send_reply(
        &self,
        frame: Frame,
        lane: Lane,
        target: u64,
        control: bool,
    ) -> Result<(), SendError>;

    /// Closes the channel (§8 step 9).
    fn close_channel(&self);
}

impl<T: FrameSink + Send + Sync + 'static> ReplySink for T {
    fn send_reply(
        &self,
        frame: Frame,
        lane: Lane,
        target: u64,
        control: bool,
    ) -> Result<(), SendError> {
        // The key carries the call's ID and class: a byte stream orders its
        // writes by it, and the in-process end ignores it.
        let target = Target::new(target);
        let key = if control {
            FrameKey::control(target)
        } else {
            FrameKey::ordinary(target)
        };
        self.send(frame, lane, key)
    }

    fn close_channel(&self) {
        self.close();
    }
}

/// One session's operation service.
///
/// [`Service::start`] starts its admission and read threads. The channel's
/// reader hands it each call frame's body with [`Service::receive`], which
/// never blocks, and reports the channel's end with
/// [`Service::channel_closed`]. A clone is another handle to the same
/// service.
pub struct Service<P: HostPorts> {
    inner: Arc<Inner<P>>,
}

impl<P: HostPorts> Clone for Service<P> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

pub(crate) struct Inner<P: HostPorts> {
    pub(crate) ports: P,
    pub(crate) session: Arc<P::Session>,
    pub(crate) sink: Arc<dyn ReplySink>,
    pub(crate) limits: Limits,
    pub(crate) detached: DetachedWorkers,
    /// Whether the handshake has opened the session; before it, every frame
    /// is a protocol error.
    opened: AtomicBool,
    /// The table lock.
    pub(crate) state: Mutex<State<P>>,
    /// Wakes the admission thread.
    pub(crate) admission: Condvar,
    /// Wakes close's wait for terminals.
    pub(crate) settled: Condvar,
    /// Wakes the read thread: an append, a completion, a parked read or the
    /// end. A separate lock, so a gate crossing never takes the table lock.
    pub(crate) reads: Arc<ReadSignal>,
    /// Replies the channel refused as closed: a dropped-reply fault, which
    /// the close path's ordering keeps at zero while the session is open.
    dropped: AtomicUsize,
}

/// The read thread's wake-up, apart from the table lock.
#[derive(Default)]
pub(crate) struct ReadSignal {
    pending: Mutex<bool>,
    changed: Condvar,
}

impl ReadSignal {
    pub(crate) fn notify(&self) {
        *lock(&self.pending) = true;
        self.changed.notify_all();
    }

    fn wait(&self, timeout: Duration) {
        let pending = lock(&self.pending);
        let (mut pending, _) = self
            .changed
            .wait_timeout_while(pending, timeout, |pending| !*pending)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *pending = false;
    }
}

/// A reply to send once the table lock is released.
pub(crate) struct Outgoing {
    pub(crate) frame: Frame,
    pub(crate) lane: Lane,
    pub(crate) target: u64,
    pub(crate) control: bool,
    /// The record view this reply delivers once it is handed over (§5.4).
    pub(crate) delivers: Option<(u64, View)>,
}

/// A view of an operation's terminal (§5.4).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum View {
    /// Its `OperationResult`.
    Result,
    /// Its final response.
    Response,
}

/// How the channel ended, for [`Service::channel_closed`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ending {
    /// The client closed or dropped its end, or the carrier failed.
    Channel,
    /// A protocol error.
    Protocol,
}

impl<P: HostPorts> Service<P> {
    /// A service for one session over `sink`, with `session` as core's
    /// per-session data and `detached` as the host's record of detached
    /// workers, which every session of one host shares. `opened` says whether
    /// the session is already open (in-process, `open` is the handshake);
    /// a byte-stream session opens with [`Service::open`].
    ///
    /// `limits` must have passed [`validate_limits`](crate::validate_limits).
    pub fn new(
        ports: P,
        session: Arc<P::Session>,
        sink: Arc<dyn ReplySink>,
        limits: Limits,
        detached: DetachedWorkers,
        opened: bool,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                ports,
                session,
                sink,
                limits,
                detached,
                opened: AtomicBool::new(opened),
                state: Mutex::new(State::default()),
                admission: Condvar::new(),
                settled: Condvar::new(),
                reads: Arc::default(),
                dropped: AtomicUsize::new(0),
            }),
        }
    }

    /// Starts the admission and read threads.
    ///
    /// # Errors
    ///
    /// A thread that could not be created; the service then serves nothing.
    pub fn start(&self) -> io::Result<()> {
        let admission = Arc::clone(&self.inner);
        thread::Builder::new()
            .name("gwz-session-admission".into())
            .spawn(move || table::admission_loop(&admission))?;
        let reads = Arc::clone(&self.inner);
        thread::Builder::new()
            .name("gwz-session-reads".into())
            .spawn(move || events::read_loop(&reads))?;
        Ok(())
    }

    /// Marks the session open, so its frames may be read. The handshake (step
    /// 2.3) calls it BEFORE it writes `SessionOpened`: a client may send its
    /// first call as soon as it reads `SessionOpened`, and a call read before
    /// `open` is a protocol error.
    pub fn open(&self) {
        self.inner.opened.store(true, Ordering::Release);
    }

    /// Whether the session is open, so its frames may be read.
    pub fn is_open(&self) -> bool {
        self.inner.opened.load(Ordering::Acquire)
    }

    /// Whether `request_id` names a live operation of this session: the
    /// live-call registry the carrier's router consults for a transport-lane
    /// frame (`GwzSplitCommandChannel.md` §3.3 "Scope"; §3.4). The router,
    /// step 2.1's, owns every per-binding state; this service owns none.
    pub fn is_live(&self, request_id: &str) -> bool {
        table::is_live(&lock(&self.inner.state), request_id)
    }

    /// Routes one session-class frame, tags 1 to 8, as the carrier's router
    /// hands it on without blocking: a `SessionCall` to [`Service::receive`];
    /// any other tag, or any frame before [`Service::open`], is a protocol
    /// error. Transport-lane frames (tags 16 and 17) are the router's own.
    ///
    /// # Errors
    ///
    /// A protocol error, which ends the session.
    pub fn route(&self, frame: &Frame) -> Result<(), ProtocolError> {
        if !self.is_open() {
            return Err(ProtocolError(format!(
                "a frame of tag {} before the session opened",
                frame.tag
            )));
        }
        if frame.tag != gwz_session_contract::Tag::SessionCall.byte() {
            return Err(ProtocolError(format!(
                "the host takes no frame of tag {} once the session is open",
                frame.tag
            )));
        }
        self.receive(&frame.body)
    }

    /// How many replies the channel refused as closed: a dropped-reply fault.
    pub fn dropped_replies(&self) -> usize {
        self.inner.dropped.load(Ordering::Acquire)
    }

    /// Core's per-session data.
    pub fn session(&self) -> &Arc<P::Session> {
        &self.inner.session
    }

    /// Handles one `SessionCall` body without blocking (§5.1): creates the
    /// call's record, handles control calls and reads, and queues the rest
    /// for admission.
    ///
    /// # Errors
    ///
    /// A protocol error, which ends the session: the caller then calls
    /// [`Service::channel_closed`] with [`Ending::Protocol`].
    pub fn receive(&self, body: &[u8]) -> Result<(), ProtocolError> {
        if !self.is_open() {
            return Err(ProtocolError("a call before the session opened".to_owned()));
        }
        let call = self.inner.ports.decode(body)?;
        // Counted before the table lock, so a close never overtakes the
        // replies this call produces (§8; 2.0 §3.3 "Close report").
        lock(&self.inner.state).unflushed += 1;
        let outgoing = table::receive(&self.inner, call);
        let (outgoing, result) = match outgoing {
            Ok(outgoing) => (outgoing, Ok(())),
            Err(error) => (Vec::new(), Err(error)),
        };
        self.inner.flush(outgoing);
        result
    }

    /// The channel has ended without `session.close` (§8): runs steps 2 to 5
    /// and 7 on a thread of its own, then ends. No reply is possible.
    pub fn channel_closed(&self, ending: Ending) {
        let _ = ending;
        workers::close(&self.inner, None);
    }

    /// Waits at most `timeout` for the session to end: its close or its
    /// channel's end has settled every call. Returns whether it has.
    pub fn wait_ended(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = lock(&self.inner.state);
        while !state.ended {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            state = self
                .inner
                .settled
                .wait_timeout(state, left)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        true
    }
}

impl<P: HostPorts> fmt::Debug for Service<P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Service").finish_non_exhaustive()
    }
}

impl<P: HostPorts> Inner<P> {
    /// Sends a batch of replies built under the table lock, whose section
    /// counted it in `unflushed`, and then uncounts it: close waits for no
    /// batch in flight before its report and the channel's close, so sends
    /// keep the order of settlement (§8 steps 6 to 9; 2.0 §3.3 "Close
    /// report"). A reply handed over marks the view it delivers (§5.4). A
    /// closed channel's refusal is counted as a dropped reply. A full lane
    /// means the client exceeded its own bounds, since a reply takes the lane
    /// of the call it answers (§3): the reply cannot be delivered, so the
    /// session ends.
    pub(crate) fn flush(&self, outgoing: Vec<Outgoing>) {
        let mut delivered = Vec::new();
        for reply in outgoing {
            let delivers = reply.delivers;
            let sent = self
                .sink
                .send_reply(reply.frame, reply.lane, reply.target, reply.control);
            match sent {
                Ok(()) => {
                    delivered.extend(delivers);
                }
                Err(SendError::Full(_)) => {
                    self.sink.close_channel();
                }
                Err(SendError::Closed(..)) => {
                    self.dropped.fetch_add(1, Ordering::AcqRel);
                }
            }
        }
        let mut state = lock(&self.state);
        table::mark_delivered(&mut state, &delivered);
        state.unflushed = state.unflushed.saturating_sub(1);
        drop(state);
        self.settled.notify_all();
    }

    /// A reply carrying `body` to `call_id`.
    pub(crate) fn reply(&self, call_id: u64, lane: Lane, body: Vec<u8>) -> Outgoing {
        Outgoing {
            frame: self.ports.reply(call_id, body),
            lane,
            target: call_id,
            control: lane == Lane::Control,
            delivers: None,
        }
    }

    /// An error reply carrying `failure` to `call_id`.
    pub(crate) fn error(&self, call_id: u64, lane: Lane, failure: &P::Failure) -> Outgoing {
        Outgoing {
            frame: self.ports.error(call_id, failure),
            lane,
            target: call_id,
            control: lane == Lane::Control,
            delivers: None,
        }
    }

    /// An error reply for a host refusal.
    pub(crate) fn refuse(&self, call_id: u64, lane: Lane, error: HostError) -> Outgoing {
        let failure = self.ports.failure(error);
        self.error(call_id, lane, &failure)
    }

    /// The longest a held read waits.
    pub(crate) fn read_wait(&self, asked: Duration) -> Duration {
        asked.min(MAX_READ_WAIT)
    }
}

/// The gate a worker starts with.
pub(crate) type Gate<P> = OperationGate<<P as HostPorts>::Session>;

/// The controls a record keeps.
pub(crate) type Controls<P> = CallControls<<P as HostPorts>::Session>;

mod tests;

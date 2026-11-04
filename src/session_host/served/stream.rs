//! A session served over a byte stream: the byte-stream counterpart of `serve_in_process` (plan step 2.3g1;
//! gwz-dev `dev-docs/GwzSplitCommandChannel.md` §3.1, §3.3, §4.3; `GwzCoreSessionDesign.md` §3, §8).
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | The host speaks first and the client's one frame is answered by step 2.3d's handshake ([`accept_open`]) | 2.0 §3.1 |
//! | The session's service is built with the limits the client states, started and opened inside the handshake's open hook, before `SessionOpened` is written: a client may send its first call as soon as it reads it (C11) | 2.0 §3.1; plan C11 |
//! | The client's `SessionLimits`: a value above the host's own is refused with `invalid_request`, one at or below it goes on to `open`'s validation (`validate_limits`), whose refusal is `invalid_request` with its text; with none stated the session has the host's limits | server design §3; plan 2.3g2 (Q1 (a)) |
//! | The carrier holds the session's frames until `SessionOpened` is queued: any frame but the handshake's before it ends the carrier as a protocol error (the carrier's pre-open gate, `CarrierLimits::awaiting_open`) | 2.0 §3.3 "When" |
//! | The session shares its host context's member lock manager and detached-worker record | §5.1; 2.0 §1 |
//! | A thread of the session drives the carrier's receive side: each registry frame goes to `Service::route`; a route error closes the carrier with a protocol fault and ends the session as `Protocol`; the carrier's end ends it as `Channel` (§8 channel closure: every live operation is cancelled, local phases included) | §3, §8; 2.0 §4.3 "Carrier loss" |
//! | Replies go back through the carrier keyed by their call and class, so the writer's priority rule applies; the session's close closes the carrier normally, after the close report | 2.0 §3.3 |
//! | Byte pressure never holds a thread of the session host: a reply the writer's queue has no bytes for is held by the session's [`outbox`], in order, and the session's pacer thread, the only one that waits for the writer, sends it as the writer takes frames. A control reply goes ahead on the control lane's own reserve, so `operation.cancel`, `session.close` and later calls are routed and answered while the client has replies past the byte bound unread. Only a lane past its frame count, the client's own overrun, is the session host's `Full`, and that ends the session at once: the carrier closes without draining what is held. A client that has stopped reading ends at the stall bound as carrier loss | §3 "Volume is bounded", D-N; §5.1; 2.0 §4.3 "Stuck peers" |
//! | The session's end is logged once, with why: the protocol error, or the carrier's close reason; it is logged once the session has ended (its close has settled every call), after what its admission held is released | 2.0 §4.3; plan 2.3g2 |
//!
//! The loop does not wait on `StreamCarrier::wait_closed`: after a peer's normal close the end reports its closure
//! only once a consumer takes from it (the 2.3d-2 probe), and this loop is that consumer.
//!
//! **Admission and control are the caller's.** `serve_stream` takes the composition's own [`Hooks`]: its `open` runs in
//! the open hook once the stated limits have passed (a server's session count, step 2.3g2) and its `control` answers a
//! `ServerControl` (step 2.3i). What the outer `open` admitted is released through [`StreamServing::held`] once the
//! session has ended, however it ends (its `session.close`, carrier loss, a protocol error): the carrier's end only
//! starts the close, and an operation that has not yet seen its cancel runs on until it returns or is detached at
//! `close_wait`. The session's serving thread waits for that end; with no session served, `serve_stream` drops it on
//! return, and a session the open hook opened that is not served ends before `serve_stream` returns.
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use gwz_session_host::{Ending, Limits, ReplySink, Service, validate_limits};

use super::{Dispatch, OperationLog, ServedPorts, SessionEnded, session_service};
use crate::model::{ErrorCode, ModelError};
use crate::session_host::context::HostContext;
use crate::transport_host::carrier::{
    Carrier, CloseCode, StreamCarrier,
    handshake::{
        BodyFault, Bounds, CarrierWire, Control, Hello, Hooks, HostFault, HostOutcome, OpenRequest,
        OpenedInfo, Refusal, SESSION_LIMIT_KEYS, State, accept_open, decode_session_limits,
    },
};

mod outbox;

use outbox::Outbox;

/// How long the serving loop waits on the carrier between two looks: it is woken by every frame and by the end, so
/// this bounds nothing a peer can see.
const IDLE_WAIT: Duration = Duration::from_secs(3600);

/// What serving a session over a stream needs besides its carrier.
pub(crate) struct StreamServing<'a> {
    pub(crate) dispatch: Arc<dyn Dispatch>,
    pub(crate) log: Arc<dyn OperationLog>,
    /// The one workspace the server serves.
    pub(crate) root: PathBuf,
    pub(crate) host: &'a HostContext,
    /// The host's limits: a session that states none has them, and a stated value above one of them is refused.
    pub(crate) limits: Limits,
    pub(crate) hello: &'a Hello,
    pub(crate) bounds: &'a Bounds,
    /// What the composition's admission holds for this connection (a server's session slot): released once the
    /// session has ended; with no session served it is dropped when `serve_stream` returns.
    pub(crate) held: Box<dyn Send>,
}

/// A session open over a stream carrier, served by its own thread until the carrier ends.
#[allow(
    dead_code,
    reason = "first production caller: plan step 2.3i, the accept loop"
)]
pub(crate) struct StreamSession {
    service: Service<ServedPorts>,
    carrier: Arc<StreamCarrier>,
    serving: JoinHandle<()>,
}

#[allow(
    dead_code,
    reason = "first production caller: plan step 2.3i, the accept loop"
)]
impl StreamSession {
    pub(crate) fn service(&self) -> &Service<ServedPorts> {
        &self.service
    }

    pub(crate) fn carrier(&self) -> &Arc<StreamCarrier> {
        &self.carrier
    }

    /// Whether the serving thread has returned: the carrier has ended, the session has ended and what its admission
    /// held is released.
    pub(crate) fn is_finished(&self) -> bool {
        self.serving.is_finished()
    }
}

/// How a connection's handshake ended.
#[allow(
    dead_code,
    reason = "first production caller: plan step 2.3i, the accept loop"
)]
pub(crate) enum Served {
    /// `SessionOpened` was written and the session is served.
    Session(StreamSession),
    /// No session: a `ServerControl` was answered, or the open was refused; the carrier is closed.
    Ended(HostOutcome),
}

/// Why a connection was not served; its carrier is closed, and a session the open hook had opened is closed too.
#[derive(Debug)]
#[allow(
    dead_code,
    reason = "first production caller: plan step 2.3i, the accept loop"
)]
pub(crate) enum ServeFault {
    /// Before the handshake: a carrier that does not hold its frames until the open, limits the session host refuses,
    /// a host context shut down. Nothing of the handshake was written.
    Setup(ModelError),
    /// The handshake failed: the client sent nothing in time, sent the wrong frame, or the carrier ended (a frame
    /// before `SessionOpened` among the reasons, `ProtocolFault::BeforeOpened`).
    Handshake(HostFault),
}

/// What the open hook builds the session's service from.
struct ServiceParts<'a> {
    dispatch: Arc<dyn Dispatch>,
    log: Arc<dyn OperationLog>,
    root: PathBuf,
    host: &'a HostContext,
    /// The host's limits.
    limits: Limits,
    sink: Arc<dyn ReplySink>,
}

/// The open hook: the stated limits, then the composition's own admission, then the session's service built, started
/// and opened (C11).
struct SessionHooks<'a> {
    outer: &'a mut dyn Hooks,
    parts: ServiceParts<'a>,
    /// The service, once started and opened: a fault after this is the session's to close.
    service: Option<Service<ServedPorts>>,
}

impl Hooks for SessionHooks<'_> {
    fn open(&mut self, open: &OpenRequest) -> Result<OpenedInfo, Refusal> {
        let limits = session_limits(open.limits.as_deref(), &self.parts.limits)?;
        let info = self.outer.open(open)?;
        let parts = &self.parts;
        let service = session_service(
            Arc::clone(&parts.dispatch),
            Arc::clone(&parts.log),
            parts.root.clone(),
            parts.host,
            limits,
            Arc::clone(&parts.sink),
            false,
        )
        .map_err(|error| Refusal {
            code: crate::GwzErrorCode::from(error.code).wire() as u64,
            message: error.message.into(),
        })?;
        if service.start().is_err() {
            // A thread that did start ends with the session.
            service.channel_closed(Ending::Channel);
            return Err(Refusal {
                code: crate::GwzErrorCode::InternalError.wire() as u64,
                message: "no session thread could start".into(),
            });
        }
        // Open before `SessionOpened` is written: the client's first call may follow it at once.
        service.open();
        self.service = Some(service);
        Ok(info)
    }

    fn control(&mut self, control: &Control) -> Result<State, Refusal> {
        self.outer.control(control)
    }
}

/// Serves one connection over `carrier`, an accepted end whose peer check has passed: the handshake, then, for an
/// open, the session on a thread of its own. `carrier` must hold its frames until the open
/// (`CarrierLimits::awaiting_open`), as the server's policy builds its listener: any other is refused before the hello.
///
/// # Errors
///
/// [`ServeFault`]; the carrier is closed either way.
pub(crate) fn serve_stream(
    carrier: Arc<StreamCarrier>,
    serving: StreamServing<'_>,
    outer: &mut dyn Hooks,
) -> Result<Served, ServeFault> {
    if !carrier.awaits_open() {
        // Without the gate a frame read before `SessionOpened` would reach a session the hook has already opened.
        carrier.close(CloseCode::Normal);
        return Err(ServeFault::Setup(ModelError::new(
            ErrorCode::InternalError,
            "the carrier does not hold its frames until the session opens",
        )));
    }
    // The host's own limits and context are checked before the hello, so a host that could serve no session writes
    // nothing.
    let setup = validate_limits(&serving.limits)
        .map_err(ModelError::from)
        .and_then(|_| {
            if serving.host.is_shut_down() {
                Err(ModelError::new(
                    ErrorCode::InvalidRequest,
                    "the host context has been shut down",
                ))
            } else {
                Ok(())
            }
        });
    if let Err(error) = setup {
        carrier.close(CloseCode::Normal);
        return Err(ServeFault::Setup(error));
    }
    let log = Arc::clone(&serving.log);
    let held = serving.held;
    let outbox = Arc::new(Outbox::new(Arc::clone(&carrier)));
    let mut hooks = SessionHooks {
        outer,
        parts: ServiceParts {
            dispatch: serving.dispatch,
            log: serving.log,
            root: serving.root,
            host: serving.host,
            limits: serving.limits,
            sink: Arc::clone(&outbox) as Arc<dyn ReplySink>,
        },
        service: None,
    };
    let outcome = accept_open(
        &mut CarrierWire(&carrier),
        serving.hello,
        serving.bounds,
        &mut hooks,
    );
    let opened = hooks.service.take();
    match outcome {
        Ok(HostOutcome::Opened(_)) => {
            // The host writes `SessionOpened` only once the hook has opened the service.
            let service = opened.expect("the open hook opened the session's service");
            // The pacer ends with the channel: its paced send answers `Closed` once the carrier ends, and the session's
            // close, which every ending of the session reaches (§8 step 9), ends it once nothing is held.
            let pacer_outbox = Arc::clone(&outbox);
            let pacer = std::thread::Builder::new()
                .name("gwz-session-pacer".into())
                .spawn(move || pacer_outbox.pace());
            let thread_service = service.clone();
            let thread_carrier = Arc::clone(&carrier);
            // Shared so that a thread that cannot start leaves it here, for the session's end below.
            let held = Arc::new(Mutex::new(Some(held)));
            let thread_held = Arc::clone(&held);
            let spawned = pacer.and_then(|_| {
                std::thread::Builder::new()
                    .name("gwz-session-stream".into())
                    .spawn(move || {
                        let end = serve_frames(&thread_carrier, &thread_service);
                        release_when_ended(&thread_service, take_held(&thread_held));
                        log.session_ended(end);
                    })
            });
            match spawned {
                Ok(serving) => Ok(Served::Session(StreamSession {
                    service,
                    carrier,
                    serving,
                })),
                Err(error) => {
                    carrier.close(CloseCode::Normal);
                    service.channel_closed(Ending::Channel);
                    release_when_ended(&service, take_held(&held));
                    Err(ServeFault::Setup(ModelError::new(
                        ErrorCode::InternalError,
                        format!("no session thread could start: {error}"),
                    )))
                }
            }
        }
        // A refused open or a control exchange: the hook built no service.
        Ok(ended) => Ok(Served::Ended(ended)),
        Err(fault) => {
            // `accept_open` closed the carrier; the session the hook opened, if it did, ends with it, and holds what
            // its admission took until then.
            if let Some(service) = opened {
                service.channel_closed(Ending::Channel);
                release_when_ended(&service, Some(held));
            }
            Err(ServeFault::Handshake(fault))
        }
    }
}

/// What the admission held, taken from where the serving thread and `serve_stream` share it: `None` once taken.
fn take_held(held: &Mutex<Option<Box<dyn Send>>>) -> Option<Box<dyn Send>> {
    held.lock().unwrap_or_else(PoisonError::into_inner).take()
}

/// Releases `held`, what the session's admission took, once `service`'s session has ended. Its close has begun (the
/// carrier's end, a protocol error or the session's own close started it), so the wait ends: the close detaches what
/// still runs at `close_wait`, which `validate_limits` bounds.
fn release_when_ended(service: &Service<ServedPorts>, held: Option<Box<dyn Send>>) {
    while !service.wait_ended(IDLE_WAIT) {}
    drop(held);
}

/// The limits of a session whose client stated `stated` (an encoded `SessionLimits`) to a host whose limits are `host`
/// (server design §3; plan step 2.3g2): none stated gives the host's; a value above the host's is refused, and one at
/// or below it is taken and the whole goes to `open`'s validation. Every refusal is `invalid_request`, and names only
/// keys and numbers, never text the client sent.
pub(crate) fn session_limits(stated: Option<&[u8]>, host: &Limits) -> Result<Limits, Refusal> {
    let invalid = |message: String| Refusal {
        code: crate::GwzErrorCode::InvalidRequest.wire() as u64,
        message: message.into(),
    };
    let Some(stated) = stated else {
        return Ok(host.clone());
    };
    let values = decode_session_limits(stated).map_err(|fault| {
        invalid(match fault {
            BodyFault::Unknown(key) => format!(
                "the session limits hold key {key}, which is none of keys 1 to {SESSION_LIMIT_KEYS}"
            ),
            BodyFault::Missing(key) => format!(
                "the session limits lack key {key}; each of keys 1 to {SESSION_LIMIT_KEYS} is required"
            ),
            BodyFault::WrongType(key) | BodyFault::Range(key) => {
                format!("session limit key {key} is not an unsigned integer")
            }
            _ => "the session limits are not a canonical map of unsigned integers".to_owned(),
        })
    })?;
    let [
        running,
        queued,
        table,
        workers,
        events,
        logs,
        calls,
        reserve,
        read,
        close_ms,
    ] = values;
    let mut limits = host.clone();
    let counts: [(&str, u64, &mut usize); 9] = [
        (
            "running_operations",
            running,
            &mut limits.running_operations,
        ),
        ("queued_operations", queued, &mut limits.queued_operations),
        ("operation_table", table, &mut limits.operation_table),
        ("direct_workers", workers, &mut limits.direct_workers),
        ("event_log", events, &mut limits.event_log),
        ("open_logs", logs, &mut limits.open_logs),
        ("outstanding_calls", calls, &mut limits.outstanding_calls),
        ("control_reserve", reserve, &mut limits.control_reserve),
        ("read_bytes", read, &mut limits.read_bytes),
    ];
    let above = |name: &str, value: u64, most: u64| {
        invalid(format!(
            "session limit {name} of {value} is above this host's {most}"
        ))
    };
    for (name, value, field) in counts {
        let most = u64::try_from(*field).unwrap_or(u64::MAX);
        if value > most {
            return Err(above(name, value, most));
        }
        // At most the host's own value, so it fits.
        *field = value as usize;
    }
    let most = u64::try_from(host.close_wait.as_millis()).unwrap_or(u64::MAX);
    if close_ms > most {
        return Err(above("close_wait_ms", close_ms, most));
    }
    limits.close_wait = Duration::from_millis(close_ms);
    validate_limits(&limits).map_err(|error| invalid(error.to_string()))?;
    Ok(limits)
}

/// The session's receive side, until the carrier ends: returns how the session ended, for the log.
fn serve_frames(carrier: &StreamCarrier, service: &Service<ServedPorts>) -> SessionEnded {
    loop {
        match carrier.recv_registry_timeout(IDLE_WAIT) {
            Ok(Some(frame)) => {
                if let Err(error) = service.route(&frame) {
                    carrier.close(CloseCode::ProtocolFault);
                    service.channel_closed(Ending::Protocol);
                    return SessionEnded {
                        ending: Ending::Protocol,
                        detail: error.0,
                    };
                }
            }
            Ok(None) => {}
            Err(closed) => {
                service.channel_closed(Ending::Channel);
                return SessionEnded {
                    ending: Ending::Channel,
                    detail: format!("{:?}", closed.reason),
                };
            }
        }
    }
}

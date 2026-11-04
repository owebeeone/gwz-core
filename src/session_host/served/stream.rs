//! A session served over a byte stream: the byte-stream counterpart of `serve_in_process` (plan step 2.3g1;
//! gwz-dev `dev-docs/GwzSplitCommandChannel.md` §3.1, §3.3, §4.3; `GwzCoreSessionDesign.md` §3, §8).
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | The host speaks first and the client's one frame is answered by step 2.3d's handshake ([`accept_open`]) | 2.0 §3.1 |
//! | The session's service is started and opened inside the handshake's open hook, before `SessionOpened` is written: a client may send its first call as soon as it reads it (C11) | 2.0 §3.1; plan C11 |
//! | The carrier holds the session's frames until `SessionOpened` is queued: any frame but the handshake's before it ends the carrier as a protocol error (the carrier's pre-open gate, `CarrierLimits::awaiting_open`) | 2.0 §3.3 "When" |
//! | The session shares its host context's member lock manager and detached-worker record | §5.1; 2.0 §1 |
//! | A thread of the session drives the carrier's receive side: each registry frame goes to `Service::route`; a route error closes the carrier with a protocol fault and ends the session as `Protocol`; the carrier's end ends it as `Channel` (§8 channel closure: every live operation is cancelled, local phases included) | §3, §8; 2.0 §4.3 "Carrier loss" |
//! | Replies go back through the carrier keyed by their call and class, so the writer's priority rule applies; the session's close closes the carrier normally, after the close report | 2.0 §3.3 |
//! | Byte pressure never holds a thread of the session host: a reply the writer's queue has no bytes for is held by the session's [`outbox`], in order, and the session's pacer thread, the only one that waits for the writer, sends it as the writer takes frames. A control reply goes ahead on the control lane's own reserve, so `operation.cancel`, `session.close` and later calls are routed and answered while the client has replies past the byte bound unread. Only a lane past its frame count, the client's own overrun, is the session host's `Full`, and that ends the session at once: the carrier closes without draining what is held. A client that has stopped reading ends at the stall bound as carrier loss | §3 "Volume is bounded", D-N; §5.1; 2.0 §4.3 "Stuck peers" |
//! | The session's end is logged once, with why: the protocol error, or the carrier's close reason | 2.0 §4.3 |
//!
//! The loop does not wait on `StreamCarrier::wait_closed`: after a peer's normal close the end reports its closure
//! only once a consumer takes from it (the 2.3d-2 probe), and this loop is that consumer.
//!
//! **Admission and control are the caller's.** `serve_stream` takes the composition's own [`Hooks`]: its `open` runs
//! first in the open hook (a server's session count, step 2.3g2) and its `control` answers a `ServerControl` (step
//! 2.3i). What the outer `open` admitted is the caller's to release unless a [`Served::Session`] comes back, and then
//! when that session ends.
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use gwz_session_host::{Ending, Limits, ReplySink, Service, validate_limits};

use super::{Dispatch, OperationLog, ServedPorts, SessionEnded, session_service};
use crate::model::{ErrorCode, ModelError};
use crate::session_host::context::HostContext;
use crate::transport_host::carrier::{
    Carrier, CloseCode, StreamCarrier,
    handshake::{
        Bounds, CarrierWire, Control, Hello, Hooks, HostFault, HostOutcome, OpenRequest,
        OpenedInfo, Refusal, State, accept_open,
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
    pub(crate) limits: Limits,
    pub(crate) hello: &'a Hello,
    pub(crate) bounds: &'a Bounds,
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

    /// Whether the serving thread has returned: the carrier has ended and the session's closure has begun.
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

/// The open hook: the composition's own first, then the session's service started and opened (C11).
struct SessionHooks<'a> {
    outer: &'a mut dyn Hooks,
    service: &'a Service<ServedPorts>,
    /// The service was started and opened: a fault after this is the session's to close.
    opened: bool,
}

impl Hooks for SessionHooks<'_> {
    fn open(&mut self, open: &OpenRequest) -> Result<OpenedInfo, Refusal> {
        let info = self.outer.open(open)?;
        if self.service.start().is_err() {
            // A thread that did start ends with the session.
            self.service.channel_closed(Ending::Channel);
            return Err(Refusal {
                code: crate::GwzErrorCode::InternalError.wire() as u64,
                message: "no session thread could start".into(),
            });
        }
        // Open before `SessionOpened` is written: the client's first call may follow it at once.
        self.service.open();
        self.opened = true;
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
    let log = Arc::clone(&serving.log);
    let outbox = Arc::new(Outbox::new(Arc::clone(&carrier)));
    let setup = validate_limits(&serving.limits)
        .map_err(ModelError::from)
        .and_then(|_| {
            let sink = Arc::clone(&outbox) as Arc<dyn ReplySink>;
            session_service(
                serving.dispatch,
                serving.log,
                serving.root,
                serving.host,
                serving.limits,
                sink,
                false,
            )
        });
    let service = match setup {
        Ok(service) => service,
        Err(error) => {
            carrier.close(CloseCode::Normal);
            return Err(ServeFault::Setup(error));
        }
    };
    let mut hooks = SessionHooks {
        outer,
        service: &service,
        opened: false,
    };
    let outcome = accept_open(
        &mut CarrierWire(&carrier),
        serving.hello,
        serving.bounds,
        &mut hooks,
    );
    let opened = hooks.opened;
    match outcome {
        Ok(HostOutcome::Opened(_)) => {
            // The pacer ends with the channel: its paced send answers `Closed` once the carrier ends, and the session's
            // close, which every ending of the session reaches (§8 step 9), ends it once nothing is held.
            let pacer_outbox = Arc::clone(&outbox);
            let pacer = std::thread::Builder::new()
                .name("gwz-session-pacer".into())
                .spawn(move || pacer_outbox.pace());
            let thread_service = service.clone();
            let thread_carrier = Arc::clone(&carrier);
            let spawned = pacer.and_then(|_| {
                std::thread::Builder::new()
                    .name("gwz-session-stream".into())
                    .spawn(move || {
                        let end = serve_frames(&thread_carrier, &thread_service);
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
                    Err(ServeFault::Setup(ModelError::new(
                        ErrorCode::InternalError,
                        format!("no session thread could start: {error}"),
                    )))
                }
            }
        }
        Ok(ended) => Ok(Served::Ended(ended)),
        Err(fault) => {
            // `accept_open` closed the carrier; the session the hook opened, if it did, ends with it.
            if opened {
                service.channel_closed(Ending::Channel);
            }
            Err(ServeFault::Handshake(fault))
        }
    }
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

//! The policy a server applies to every connection it serves, and the call that serves one (plan step 2.3g1; gwz-dev
//! `dev-docs/GwzSplitCommandChannel.md` §4.3 "Stuck peers"; plan C8).
//!
//! The carrier supplies the mechanisms (its lanes, its inbound bounds, its stall watchdog, its pre-open gate) and this
//! policy their values: the lanes and the registry bound come from the session's limits, so a client that keeps
//! within its own bounds never meets a lane at its frame count; the writer's byte bound (`QUEUE_BYTES`) is not a
//! session limit but backpressure: replies past it are held by the session, within its lanes' frame counts, and sent
//! by its pacer thread as the writer takes frames, so no thread of the session host waits on it and control replies go
//! ahead on the control lane's reserve (`served::stream`'s outbox); the no-progress bound is 60 seconds, after which a peer that has stopped reading ends its session as carrier loss,
//! whatever it has queued; the close bound is the session's own `close_wait`; and an accepted end holds every frame but
//! the handshake's until it queues `SessionOpened`.
//!
//! **Admission (plan step 2.3g2, C20).** The server's sessions are counted ([`Sessions`]): a `SessionOpen` beyond
//! `--max-sessions` N is refused with one `transport_session_full` naming N (server design §6 and §17), before any
//! session is built; a `ServerControl` is answered whatever the count. A session's slot is released once the session
//! has ended, however it ends (its close, carrier loss, a protocol error: the close has settled every call, and what ran
//! on past its cancel has returned or been detached at `close_wait`), so the next client after one session ends is
//! served, and the count bounds the sessions not yet ended. The client's `SessionLimits` are judged
//! against this policy's limits, the socket host's defaults (`served::stream::session_limits`).
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use gwz_session_host::Limits;

use crate::session_host::HostContext;
use crate::session_host::served::stream::{ServeFault, Served, StreamServing, serve_stream};
use crate::session_host::served::{Dispatch, OperationLog};
use crate::transport_host::carrier::{
    CarrierLimits, InboundLimits, StallBounds, StreamCarrier,
    handshake::{Bounds, Control, Hello, Hooks, OpenRequest, OpenedInfo, Refusal, State},
};

/// What a server applies to each session it serves.
#[derive(Clone, Debug)]
#[allow(
    dead_code,
    reason = "first production caller: plan step 2.3i, the accept loop"
)]
pub(crate) struct SessionPolicy {
    /// The session's limits (session design §1 defaults).
    pub(crate) limits: Limits,
    /// How long the host's carrier may have frames queued with no byte accepted before the session ends as carrier
    /// loss.
    pub(crate) stall: Duration,
    /// The handshake's bounds.
    pub(crate) bounds: Bounds,
    /// What the host says first.
    pub(crate) hello: Hello,
    /// The server's open sessions, counted against its `--max-sessions`: every connection served under this policy
    /// (and every clone of it) counts against the one count.
    sessions: Arc<Sessions>,
}

#[allow(
    dead_code,
    reason = "first production caller: plan step 2.3i, the accept loop"
)]
impl SessionPolicy {
    /// The no-progress bound of `GwzSplitCommandChannel.md` §4.3: 60 seconds.
    pub(crate) const STALL: Duration = Duration::from_secs(60);

    /// The production policy: the session design's limits, the 60 s stall bound, the handshake's own bounds and
    /// `--max-sessions` 64.
    pub(crate) fn new(hello: Hello) -> Self {
        Self {
            limits: Limits::default(),
            stall: Self::STALL,
            bounds: Bounds::DEFAULT,
            hello,
            sessions: Arc::new(Sessions::with_max(Sessions::DEFAULT_MAX)),
        }
    }

    /// This policy with `--max-sessions` `max` and a count of its own; `None` outside 1 to 1024 (server design §17).
    pub(crate) fn with_max_sessions(self, max: u32) -> Option<Self> {
        (1..=Sessions::MOST).contains(&max).then(|| Self {
            sessions: Arc::new(Sessions::with_max(max)),
            ..self
        })
    }

    /// The server's session count.
    pub(crate) fn sessions(&self) -> &Arc<Sessions> {
        &self.sessions
    }

    /// The carrier's stall bounds: the no-progress bound, and the session's close bound for a closing end's flush.
    pub(crate) fn stall_bounds(&self) -> StallBounds {
        StallBounds {
            no_progress: self.stall,
            close: self.limits.close_wait,
        }
    }

    /// The limits of every carrier the server accepts: the call lane and the control reserve of the session, the
    /// registry bound of both, these stall bounds, and the pre-open gate. The frame counts are the session's; the
    /// writer's byte bound stays the carrier's, and the session holds the replies past it rather than fail them or wait
    /// for them on a thread of the session host. `None` when the session's limits leave a lane without room.
    pub(crate) fn carrier_limits(&self) -> Option<CarrierLimits> {
        let registry_frames = self
            .limits
            .outstanding_calls
            .checked_add(self.limits.control_reserve)?;
        let limits =
            CarrierLimits::new(self.limits.outstanding_calls, self.limits.control_reserve)?
                .with_inbound(InboundLimits {
                    registry_frames,
                    ..InboundLimits::DEFAULT
                })
                .with_stall(self.stall_bounds())
                .awaiting_open();
        Some(limits)
    }

    /// Serves one accepted connection under this policy (`serve_stream`): an open takes a slot of the server's
    /// session count first, then `outer`, the composition's own admission and control, is asked. The slot is held by
    /// the session until it ends, and released at once when no session is served.
    ///
    /// # Errors
    ///
    /// `serve_stream`'s; the carrier is closed either way.
    pub(crate) fn serve(
        &self,
        carrier: Arc<StreamCarrier>,
        session: SessionParts<'_>,
        outer: &mut dyn Hooks,
    ) -> Result<Served, ServeFault> {
        let admitted = Arc::new(Mutex::new(None));
        let mut counted = Counted {
            sessions: &self.sessions,
            outer,
            admitted: Arc::clone(&admitted),
        };
        serve_stream(
            carrier,
            StreamServing {
                dispatch: session.dispatch,
                log: session.log,
                root: session.root,
                host: session.host,
                limits: self.limits.clone(),
                hello: &self.hello,
                bounds: &self.bounds,
                held: Box::new(admitted),
            },
            &mut counted,
        )
    }
}

/// A server's open sessions and its `--max-sessions`.
#[derive(Debug)]
pub(crate) struct Sessions {
    max: u32,
    open: Mutex<u32>,
}

impl Sessions {
    /// `--max-sessions` when none is given (server design §6).
    pub(crate) const DEFAULT_MAX: u32 = 64;
    /// The largest `--max-sessions` (server design §17).
    pub(crate) const MOST: u32 = 1024;

    fn with_max(max: u32) -> Self {
        Self {
            max,
            open: Mutex::new(0),
        }
    }

    /// The server's `--max-sessions`.
    pub(crate) fn max(&self) -> u32 {
        self.max
    }

    /// Sessions open now.
    #[allow(
        dead_code,
        reason = "first production caller: plan step 2.3i, the ServerState answer"
    )]
    pub(crate) fn open(&self) -> u32 {
        *self.open.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A slot for one more session, or `None` at the cap.
    fn take(self: &Arc<Self>) -> Option<Slot> {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        if *open >= self.max {
            return None;
        }
        let others = *open;
        *open += 1;
        Some(Slot {
            sessions: Arc::clone(self),
            others,
        })
    }
}

/// One open session's place in the count, given back when it is dropped.
#[derive(Debug)]
pub(crate) struct Slot {
    sessions: Arc<Sessions>,
    /// The sessions that were open when this one took its slot: `SessionOpened`'s `open_sessions`.
    others: u32,
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut open = self
            .sessions
            .open
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *open = open.saturating_sub(1);
    }
}

/// The open hook's count: a slot first, then the composition's own hooks. The slot it took goes to `admitted`, which
/// the session holds until it ends.
struct Counted<'a> {
    sessions: &'a Arc<Sessions>,
    outer: &'a mut dyn Hooks,
    admitted: Arc<Mutex<Option<Slot>>>,
}

impl Hooks for Counted<'_> {
    /// `SessionOpened`'s `open_sessions` is the count's: the sessions open when this one took its slot.
    fn open(&mut self, open: &OpenRequest) -> Result<OpenedInfo, Refusal> {
        let Some(slot) = self.sessions.take() else {
            let max = self.sessions.max();
            return Err(Refusal {
                code: crate::GwzErrorCode::TransportSessionFull.wire() as u64,
                message: format!("the server has {max} open sessions, its --max-sessions; wait")
                    .into(),
            });
        };
        // A refusal of the composition's drops the slot.
        self.outer.open(open)?;
        let open_sessions = slot.others;
        *self.admitted.lock().unwrap_or_else(PoisonError::into_inner) = Some(slot);
        Ok(OpenedInfo { open_sessions })
    }

    fn control(&mut self, control: &Control) -> Result<State, Refusal> {
        self.outer.control(control)
    }
}

/// What a session is served with: the handlers, the log, the workspace and the host context it shares.
#[allow(
    dead_code,
    reason = "first production caller: plan step 2.3i, the accept loop"
)]
pub(crate) struct SessionParts<'a> {
    pub(crate) dispatch: Arc<dyn Dispatch>,
    pub(crate) log: Arc<dyn OperationLog>,
    pub(crate) root: PathBuf,
    pub(crate) host: &'a HostContext,
}

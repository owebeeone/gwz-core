//! The transport lane's routing on the core's side (gwz-dev
//! `dev-docs/GwzSplitCommandChannel.md` §3.3; step 2.2b): frame tags 16
//! (transport data) and 17 (transport control) carry transport envelopes for
//! a live call, beside its call and reply on tags 1 to 3.
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | Tag 17 carries every kind charged to the transport's control reserve; tag 16 carries `data`, `open` and `check_identity`; a kind under the other tag faults its binding | §3.3 "Tag by kind", "Tag alignment" |
//! | Only after the session has opened: a lane frame before it, like any frame of a tag the host does not take, is a protocol error ending the session | §3.3 "When"; session design §3 |
//! | A frame names a live call by `request_id`; one for a call that is not live is discarded, creating no state | §3.3 "Scope" |
//! | Per stream, bounded by quantities the sender controls: `window` coalesces to the highest offset, `data` is bounded by the binding's receive window, every other kind holds one slot; an overrun faults that binding only | §3.3 "Transport-class bound", "Fault scope" |
//! | The router never blocks: a consumer that is not ready holds back its own binding only | §3.3 "Byte-stream reader" |
//!
//! Bytes only, as the crate is: core decodes each lane frame's body into a
//! [`LaneFrame`] header and encodes what it sends; this module routes by the
//! header and hands each frame's body to its call's consumer unread.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use gwz_session_contract::{Frame, Lane};

use crate::lock;
use crate::service::{HostPorts, ProtocolError, ReplySink, Service};

/// The tag of transport data: `data`, `open` and `check_identity` (§3.3).
pub const TRANSPORT_DATA: u8 = 16;
/// The tag of transport control: every kind the transport charges to its
/// control reserve (§3.3).
pub const TRANSPORT_CONTROL: u8 = 17;

/// The tag a kind travels under: [`TRANSPORT_CONTROL`] for a control kind,
/// [`TRANSPORT_DATA`] otherwise.
pub const fn lane_tag(control: bool) -> u8 {
    if control {
        TRANSPORT_CONTROL
    } else {
        TRANSPORT_DATA
    }
}

/// A lane frame's target, which the byte-stream writer compares for
/// equality: its call's `request_id`, its binding's `session_id` and its
/// `stream_id` (§3.3 "Writer interface").
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct StreamTarget {
    /// The call that bound it.
    pub request_id: String,
    /// The binding.
    pub session_id: String,
    /// The stream.
    pub stream_id: u64,
}

/// What core decoded from a lane frame's body.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaneFrame {
    /// The frame's tag, 16 or 17.
    pub tag: u8,
    /// Where it goes.
    pub target: StreamTarget,
    /// What kind of envelope it carries.
    pub kind: LaneKind,
    /// The frame's body, handed to the consumer unread.
    pub body: Vec<u8>,
}

/// An envelope's kind, as far as routing needs it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LaneKind {
    /// `data`, carrying this many payload bytes.
    Data(usize),
    /// `window`, granting credit up to this offset.
    Window(u64),
    /// Any other kind: its wire number, and whether the transport charges it
    /// to its control reserve.
    Other {
        /// The kind's wire number.
        kind: i64,
        /// Whether it is a control kind.
        control: bool,
    },
}

impl LaneKind {
    /// Whether the kind belongs under tag 17.
    pub fn control(self) -> bool {
        match self {
            LaneKind::Data(_) => false,
            LaneKind::Window(_) => true,
            LaneKind::Other { control, .. } => control,
        }
    }
}

/// What became of a routed lane frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Routed {
    /// Queued for its call's consumer.
    Delivered,
    /// For no live call, or for a binding already faulted: dropped, no state
    /// created.
    Discarded,
    /// It broke its binding's rules: that binding is faulted, nothing else.
    Faulted(String),
}

/// Why a call's lane gave no frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LaneError {
    /// The binding broke the lane's rules.
    Faulted(String),
    /// Nothing arrived in time.
    TimedOut,
    /// The call's lane is closed.
    Closed,
}

impl fmt::Display for LaneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LaneError::Faulted(reason) => {
                write!(formatter, "the transport binding faulted: {reason}")
            }
            LaneError::TimedOut => formatter.write_str("no transport frame arrived in time"),
            LaneError::Closed => formatter.write_str("the call's transport lane is closed"),
        }
    }
}

impl std::error::Error for LaneError {}

/// The session's live-call registry for the lane: each call that may bind a
/// transport registers here, and frames for it are routed to it.
#[derive(Clone)]
pub struct Lanes {
    sink: Arc<dyn ReplySink>,
    calls: Arc<Mutex<HashMap<String, Arc<CallState>>>>,
}

struct CallState {
    receive_window: usize,
    inbound: Mutex<Inbound>,
    arrived: Condvar,
}

#[derive(Default)]
struct Inbound {
    queue: VecDeque<(StreamTarget, LaneKind, Vec<u8>)>,
    data: HashMap<StreamTarget, usize>,
    faulted: Option<String>,
    closed: bool,
}

impl Lanes {
    /// The registry of a session whose lane frames go to `sink`.
    pub fn new(sink: Arc<dyn ReplySink>) -> Self {
        Self {
            sink,
            calls: Arc::default(),
        }
    }

    /// Registers the live call `request_id`, whose binding receives at most
    /// `receive_window` bytes of `data` per stream unconsumed. The call's
    /// lane ends when the returned handle drops.
    ///
    /// # Errors
    ///
    /// A call already registered under `request_id`.
    pub fn register(&self, request_id: &str, receive_window: usize) -> Result<CallLane, LaneError> {
        let mut calls = lock(&self.calls);
        if calls.contains_key(request_id) {
            return Err(LaneError::Faulted(format!(
                "request {request_id} already has a transport lane"
            )));
        }
        let state = Arc::new(CallState {
            receive_window,
            inbound: Mutex::default(),
            arrived: Condvar::new(),
        });
        calls.insert(request_id.to_owned(), Arc::clone(&state));
        Ok(CallLane {
            request_id: request_id.to_owned(),
            state,
            lanes: self.clone(),
        })
    }

    /// Routes one decoded lane frame without blocking (§3.3).
    pub fn route(&self, frame: LaneFrame) -> Routed {
        let state = lock(&self.calls).get(&frame.target.request_id).cloned();
        let Some(state) = state else {
            return Routed::Discarded;
        };
        let mut inbound = lock(&state.inbound);
        if inbound.faulted.is_some() || inbound.closed {
            return Routed::Discarded;
        }
        let fault = if frame.tag != lane_tag(frame.kind.control()) {
            Some(format!(
                "a kind under tag {} that belongs under the other tag",
                frame.tag
            ))
        } else {
            admit(&mut inbound, state.receive_window, frame)
        };
        if let Some(reason) = fault.as_ref() {
            inbound.faulted = Some(reason.clone());
            inbound.queue.clear();
        }
        drop(inbound);
        state.arrived.notify_all();
        match fault {
            Some(reason) => Routed::Faulted(reason),
            None => Routed::Delivered,
        }
    }
}

/// Queues a frame under the transport-class bound, or says why it breaks it.
fn admit(inbound: &mut Inbound, receive_window: usize, frame: LaneFrame) -> Option<String> {
    let LaneFrame {
        target, kind, body, ..
    } = frame;
    match kind {
        LaneKind::Data(bytes) => {
            let pending = inbound.data.entry(target.clone()).or_default();
            if pending.saturating_add(bytes) > receive_window {
                return Some(format!(
                    "data beyond the binding's receive window of {receive_window} bytes"
                ));
            }
            *pending += bytes;
        }
        LaneKind::Window(offset) => {
            let pending = inbound.queue.iter_mut().find(|(queued, queued_kind, _)| {
                *queued == target && matches!(queued_kind, LaneKind::Window(_))
            });
            if let Some((_, queued_kind, queued_body)) = pending {
                if let LaneKind::Window(held) = *queued_kind
                    && offset > held
                {
                    *queued_kind = LaneKind::Window(offset);
                    *queued_body = body;
                }
                return None;
            }
        }
        LaneKind::Other { kind: wire, .. } => {
            let taken = inbound.queue.iter().any(|(queued, queued_kind, _)| {
                *queued == target
                    && matches!(queued_kind, LaneKind::Other { kind, .. } if *kind == wire)
            });
            if taken {
                return Some(format!(
                    "a second kind {wire} before the first was consumed"
                ));
            }
        }
    }
    inbound.queue.push_back((target, kind, body));
    None
}

/// A live call's lane: what its consumer, the carrier-backed transport
/// handle, sends and receives (§3.4).
pub struct CallLane {
    request_id: String,
    state: Arc<CallState>,
    lanes: Lanes,
}

impl CallLane {
    /// The call's `request_id`.
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// Sends a lane frame core encoded for `target` (§3.3): tag 17 on the
    /// control lane, tag 16 on the call lane.
    ///
    /// # Errors
    ///
    /// The channel's refusal.
    pub fn send(&self, frame: Frame, target: &StreamTarget) -> Result<(), LaneError> {
        let control = frame.tag == TRANSPORT_CONTROL;
        let lane = if control { Lane::Control } else { Lane::Call };
        self.lanes
            .sink
            .send_lane(frame, lane, target, control)
            .map_err(|_| LaneError::Closed)
    }

    /// The next frame for this call's binding, waiting at most `timeout`.
    ///
    /// # Errors
    ///
    /// The binding's fault, the lane's close, or the timeout.
    pub fn recv(&self, timeout: Duration) -> Result<Vec<u8>, LaneError> {
        let deadline = Instant::now() + timeout;
        let mut inbound = lock(&self.state.inbound);
        loop {
            if let Some(reason) = inbound.faulted.as_ref() {
                return Err(LaneError::Faulted(reason.clone()));
            }
            if let Some((target, kind, body)) = inbound.queue.pop_front() {
                if let LaneKind::Data(bytes) = kind
                    && let Some(pending) = inbound.data.get_mut(&target)
                {
                    *pending = pending.saturating_sub(bytes);
                }
                return Ok(body);
            }
            if inbound.closed {
                return Err(LaneError::Closed);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(LaneError::TimedOut);
            }
            inbound = self
                .state
                .arrived
                .wait_timeout(inbound, left)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
    }
}

impl Drop for CallLane {
    fn drop(&mut self) {
        lock(&self.state.inbound).closed = true;
        self.state.arrived.notify_all();
        let mut calls = lock(&self.lanes.calls);
        if calls
            .get(&self.request_id)
            .is_some_and(|state| Arc::ptr_eq(state, &self.state))
        {
            calls.remove(&self.request_id);
        }
    }
}

impl fmt::Debug for CallLane {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CallLane")
            .field("request_id", &self.request_id)
            .finish_non_exhaustive()
    }
}

/// Routes one frame the host received: a call to the service, a lane frame
/// to its call (after decoding it with `decode_lane`), anything else, or
/// anything before the session has opened, a protocol error (§3; §3.3). It
/// never blocks; core's byte-stream `FrameRouter` and the in-process reader
/// both call it.
///
/// # Errors
///
/// A protocol error, which ends the session.
pub fn route_frame<P: HostPorts>(
    service: &Service<P>,
    lanes: &Lanes,
    frame: &Frame,
    decode_lane: impl FnOnce(&Frame) -> Result<LaneFrame, ProtocolError>,
) -> Result<Routed, ProtocolError> {
    if !service.is_open() {
        return Err(ProtocolError(format!(
            "a frame of tag {} before the session opened",
            frame.tag
        )));
    }
    match frame.tag {
        1 => service.receive(&frame.body).map(|()| Routed::Delivered),
        TRANSPORT_DATA | TRANSPORT_CONTROL => decode_lane(frame).map(|lane| lanes.route(lane)),
        tag => Err(ProtocolError(format!(
            "the host takes no frame of tag {tag}"
        ))),
    }
}

mod tests;

//! The host's side: speak first, wait for the client's one frame, answer or refuse. The host runs over an accepted
//! carrier whose peer check has passed, and writes nothing before its hello.
//!
//! **The hook (C11).** For a `SessionOpen`, [`Hooks::open`] runs after the shape, canonical form and protocol range
//! are checked and before `SessionOpened` is written: the composition opens its session there, and an `Err` becomes
//! the one refusal. [`Hooks::control`] answers a `ServerControl` with the `ServerState` to write; a control exchange
//! is not a session and the open hook does not run for it.
//!
//! **What this module does not hold.** The host reads only the first frame. A later `SessionOpen` or `ServerControl`
//! on an opened session is the session's protocol error (server design §3, "each frame appears at most once"), which
//! the router of the session enforces. A stream carrier also refuses a malformed `SessionOpen` itself before it
//! delivers it (`stream/inbound.rs`); the check here is for a wire that does not.
use super::super::{
    Carrier, CarrierClosed, CloseCode, FrameRefused, Lane, StreamCarrier, validate_session_open,
    wire::{INVALID_REQUEST, session_error_frame},
};
use super::{
    Bounds, Control, FrameWire, Hello, OpenRequest, Opened, SERVER_VERSION_MISMATCH, State, WireEnd,
};
use gwz_session_contract::{Frame, FrameKey, Tag, Target};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

const OPEN: u8 = Tag::SessionOpen.byte();
const CONTROL: u8 = Tag::ServerControl.byte();
/// The refusal texts: fixed, and never repeating anything the peer sent.
const NOT_OF_SHAPE: &str = "the session open is not of the client placement's shape";
const NOT_CANONICAL: &str = "the request is not canonical";

/// Why the composition refuses: the code and a fixed text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Refusal {
    pub(crate) code: u64,
    pub(crate) message: &'static str,
}

/// What the composition tells the host when it has opened the session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OpenedInfo {
    /// The host's other open sessions, for `SessionOpened`.
    pub(crate) open_sessions: u32,
}

/// The composition's side of the handshake.
pub(crate) trait Hooks {
    /// Opens the session for a validated `SessionOpen`, before `SessionOpened` is written.
    fn open(&mut self, open: &OpenRequest) -> Result<OpenedInfo, Refusal>;
    /// The state to answer a `ServerControl` with; a stdio host refuses it.
    fn control(&mut self, control: &Control) -> Result<State, Refusal>;
}

/// How the host's handshake ended without a fault.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum HostOutcome {
    /// `SessionOpened` was written; the carrier stays open for the session.
    Opened(OpenRequest),
    /// `ServerState` was written and the connection closed.
    Controlled(Control),
    /// The one `SessionError` was written and the connection closed.
    Refused { code: u64 },
}

/// Why the host's handshake failed; the connection is closed and nothing more was written.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum HostFault {
    /// The hello the host was given is not one it can send: a range that does not start at 1 or is empty.
    BadHello,
    /// The client sent no frame within the bound.
    Expired,
    /// The first frame is not a `SessionOpen` or a `ServerControl`: the tag.
    Unexpected(u8),
    Wire(WireEnd),
}

/// The host's handshake over `wire`: writes `hello`, reads the client's frame, answers. Closes the wire on every
/// outcome but [`HostOutcome::Opened`].
pub(crate) fn accept_open(
    wire: &mut dyn FrameWire,
    hello: &Hello,
    bounds: &Bounds,
    hooks: &mut dyn Hooks,
) -> Result<HostOutcome, HostFault> {
    let outcome = converse(wire, hello, bounds, hooks);
    if !matches!(outcome, Ok(HostOutcome::Opened(_))) {
        wire.close();
    }
    outcome
}

fn converse(
    wire: &mut dyn FrameWire,
    hello: &Hello,
    bounds: &Bounds,
    hooks: &mut dyn Hooks,
) -> Result<HostOutcome, HostFault> {
    let started = wire.now();
    let body = hello.encode();
    if Hello::decode(&body).is_err() {
        return Err(HostFault::BadHello);
    }
    wire.send(Frame::new(Tag::SessionHello, body))
        .map_err(HostFault::Wire)?;
    let left = bounds
        .host_wait
        .saturating_sub(wire.now().saturating_duration_since(started));
    let mut frame = wire
        .recv(left)
        .map_err(HostFault::Wire)?
        .ok_or(HostFault::Expired)?;
    let outcome = match frame.tag {
        OPEN => open(wire, hello, hooks, &frame.body),
        CONTROL => control(wire, hooks, &frame.body),
        other => Err(HostFault::Unexpected(other)),
    };
    // The body may have carried what a refusal must not keep.
    frame.body.fill(0);
    black_box(&frame.body);
    outcome
}

fn refuse(wire: &mut dyn FrameWire, code: u64, message: &str) -> Result<HostOutcome, HostFault> {
    wire.send(session_error_frame(code, message))
        .map_err(HostFault::Wire)?;
    Ok(HostOutcome::Refused { code })
}

fn open(
    wire: &mut dyn FrameWire,
    hello: &Hello,
    hooks: &mut dyn Hooks,
    body: &[u8],
) -> Result<HostOutcome, HostFault> {
    if validate_session_open(body).is_err() {
        return refuse(wire, INVALID_REQUEST, NOT_OF_SHAPE);
    }
    let Ok(request) = OpenRequest::decode(body) else {
        return refuse(wire, INVALID_REQUEST, NOT_CANONICAL);
    };
    if !(hello.protocol_min..=hello.protocol_max).contains(&request.protocol) {
        let text = format!(
            "protocol {} is outside the {}..={} this host serves",
            request.protocol, hello.protocol_min, hello.protocol_max
        );
        return refuse(wire, SERVER_VERSION_MISMATCH, &text);
    }
    match hooks.open(&request) {
        Err(refusal) => refuse(wire, refusal.code, refusal.message),
        Ok(info) => {
            let opened = Opened {
                protocol: request.protocol,
                server: hello.server.clone(),
                core: hello.core.clone(),
                server_pid: hello.server_pid,
                open_sessions: info.open_sessions,
            };
            wire.send(Frame::new(Tag::SessionOpened, opened.encode()))
                .map_err(HostFault::Wire)?;
            Ok(HostOutcome::Opened(request))
        }
    }
}

fn control(
    wire: &mut dyn FrameWire,
    hooks: &mut dyn Hooks,
    body: &[u8],
) -> Result<HostOutcome, HostFault> {
    let Ok(control) = Control::decode(body) else {
        return refuse(wire, INVALID_REQUEST, NOT_CANONICAL);
    };
    match hooks.control(&control) {
        Err(refusal) => refuse(wire, refusal.code, refusal.message),
        Ok(state) => {
            wire.send(Frame::new(Tag::ServerState, state.encode()))
                .map_err(HostFault::Wire)?;
            Ok(HostOutcome::Controlled(control))
        }
    }
}

/// An accepted [`StreamCarrier`] as the host's wire. Its clock is the system's; the handshake frames travel in
/// arrival order (an ordinary key) on the control reserve.
pub(crate) struct CarrierWire<'a>(pub(crate) &'a StreamCarrier);

impl FrameWire for CarrierWire<'_> {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn send(&mut self, frame: Frame) -> Result<(), WireEnd> {
        let key = FrameKey::ordinary(Target::new(0));
        self.0
            .send_frame(frame, Lane::Control, key)
            .map_err(|refused| match refused {
                FrameRefused::Full(_) => WireEnd::Full,
                FrameRefused::Closed(_, closed) => WireEnd::Closed(closed),
            })
    }

    fn recv(&mut self, wait: Duration) -> Result<Option<Frame>, WireEnd> {
        self.0
            .recv_registry_timeout(wait)
            .map_err(|closed: CarrierClosed| WireEnd::Closed(closed))
    }

    fn close(&mut self) {
        self.0.close(CloseCode::Normal);
    }
}

//! The handshake on a byte stream: who speaks first, in what order, within what bounds (parked server design §3;
//! `GwzSplitCommandChannel.md` §3.1). The bodies are [`body`]; the host's side is [`host`] and the client's [`client`].
//!
//! ```text
//! host                                client
//!  | -- SessionHello (tag 6) -------->  |   after the peer check, nothing before it
//!  |                                    |   the client sends nothing until it holds a well-formed hello
//!  | <--- SessionOpen (4) | ServerControl (7)
//!  | -- SessionOpened (5) | ServerState (8) then close, or one SessionError (call ID 0) then close
//! ```
//!
//! **Bounds.** The host waits 30 s for the client's frame. The client waits 30 s for `SessionOpened` and 5 s for
//! `ServerState`, each counted from the start of its connect, so the wait covers the connect and the hello. A bound
//! that passes ends the handshake: the client aborts the stream and fails `server_unavailable`; the host closes.
//!
//! **The client reads the first frame by its length first.** A first frame whose length is zero or over 4 KiB, or
//! whose tag is not 6, fails `server_unavailable` after the five header bytes, with those bytes escaped: the client
//! does not read on, so a shell's banner is not taken for a frame. The same goes for a hello that does not decode.
//! Later frames from the host are held to [`REPLY_FRAME_MAX`].
//!
//! **No clock and no stream of its own.** The two sides run over a [`FrameWire`] (the host, whole frames: the
//! accepted [`StreamCarrier`](super::StreamCarrier)) and a [`ByteWire`] (the client, raw bytes before a carrier
//! exists). Both supply `now`, so the rows set the time and the bounds never wait for real.
//!
//! **Refusals.** A refusal is exactly one `SessionError` for call ID 0, with a fixed message that repeats nothing the
//! peer sent, and then the close. The host never writes after it.
use super::CarrierClosed;
use gwz_session_contract::Frame;
use std::{
    io,
    time::{Duration, Instant},
};

mod body;
mod client;
mod dial;
mod host;

pub(crate) use body::{Action, BodyFault, Control, Hello, OpenRequest, Opened, State};
pub(crate) use client::{
    Cause, ClientConfig, ClientFault, ClientOutcome, Intent, Stage, Unavailable,
    run as client_handshake,
};
pub(crate) use dial::{DialFault, StreamWire, dial};
pub(crate) use host::{
    CarrierWire, Hooks, HostFault, HostOutcome, OpenedInfo, Refusal, accept_open,
};

/// Error code `server_unavailable` of `GwzErrorCode` (`GwzErrorCode::ServerUnavailable.wire()`).
pub(crate) const SERVER_UNAVAILABLE: u64 = 79;
/// Error code `server_version_mismatch` of `GwzErrorCode`.
pub(crate) const SERVER_VERSION_MISMATCH: u64 = 80;
/// The largest first frame a client reads: its tag byte and its body.
pub(crate) const FIRST_FRAME_MAX: usize = 4096;
/// The largest later frame a client reads from the host.
pub(crate) const REPLY_FRAME_MAX: usize = 16 * 1024;
/// Bytes of a frame's length prefix and tag, all a client reads before it judges a first frame.
pub(crate) const FRAME_HEAD: usize = 5;
/// The most bytes of a peer's first frame that an error message shows.
pub(crate) const SHOWN_BYTES: usize = 32;
/// The most bytes of a host's refusal text that a client shows.
pub(crate) const SHOWN_MESSAGE: usize = 256;

/// How long each side waits (`GwzSplitCommandChannel.md` §3.1; server design §3).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Bounds {
    /// The host, for the client's one frame.
    pub(crate) host_wait: Duration,
    /// The client, for `SessionOpened`, from the start of its connect.
    pub(crate) open_wait: Duration,
    /// The client, for `ServerState`, from the start of its connect.
    pub(crate) state_wait: Duration,
}

impl Bounds {
    pub(crate) const DEFAULT: Bounds = Bounds {
        host_wait: Duration::from_secs(30),
        open_wait: Duration::from_secs(30),
        state_wait: Duration::from_secs(5),
    };
}

/// Why a wire could not send or receive.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum WireEnd {
    /// The carrier ended.
    Closed(CarrierClosed),
    /// The carrier's lane is full: not expected of a handshake frame.
    Full,
    /// The raw stream failed.
    Io(io::ErrorKind),
}

/// The host's end: whole frames, a clock and a close.
pub(crate) trait FrameWire {
    fn now(&self) -> Instant;
    fn send(&mut self, frame: Frame) -> Result<(), WireEnd>;
    /// The next frame, or `None` once `wait` has passed with none.
    fn recv(&mut self, wait: Duration) -> Result<Option<Frame>, WireEnd>;
    fn close(&mut self);
}

/// What a raw read of exactly `count` bytes gave.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Heard {
    Bytes(Vec<u8>),
    /// `wait` passed before `count` bytes arrived.
    Silent,
    /// The stream ended first, after the bytes given.
    Ended(Vec<u8>),
}

/// The client's end: raw bytes, a clock and an abort that shuts the stream.
pub(crate) trait ByteWire {
    fn now(&self) -> Instant;
    fn send(&mut self, bytes: &[u8]) -> Result<(), WireEnd>;
    /// Exactly `count` bytes, read from the stream and no more.
    fn recv(&mut self, count: usize, wait: Duration) -> Result<Heard, WireEnd>;
    fn abort(&mut self);
}

/// `bytes` for a message: printable ASCII as it is, a backslash doubled, anything else `\xNN`, at most the first
/// `limit` bytes and `...` when there were more.
pub(crate) fn escape(bytes: &[u8], limit: usize) -> String {
    let mut shown = String::new();
    for &byte in bytes.iter().take(limit) {
        match byte {
            b'\\' => shown.push_str("\\\\"),
            0x20..=0x7e => shown.push(char::from(byte)),
            _ => shown.push_str(&format!("\\x{byte:02x}")),
        }
    }
    if bytes.len() > limit {
        shown.push_str("...");
    }
    shown
}

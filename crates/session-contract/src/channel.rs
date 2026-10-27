//! Lanes, closure, and the two traits every channel adapter implements (§3;
//! crate map §2 and §7).

use std::error::Error;
use std::fmt;
use std::io;

use crate::frame::{Frame, FrameError};

/// The lane a frame travels on. Core classifies each frame it sends (crate
/// map §3 and §7).
///
/// Each queue of an in-process channel holds the outstanding-call limit on
/// the call lane, plus the control reserve on the control lane
/// ([`Limits`](crate::Limits)), and a frame holds its lane's room until the
/// receiver takes it. A reply travels on the lane of the call it answers, so
/// a lane of the reply queue never holds more replies than the client has
/// calls outstanding on it (§3). The in-process lanes share one order: frames
/// arrive in the order they were sent, whatever their lanes. A byte stream's
/// writer may put the oldest control frame ahead of ordinary frames of other
/// targets that wait with it, never ahead of a frame of its own target or of a
/// `SessionCall` (operator decision D-N, gwz-session-channel's `WriteQueue`).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Lane {
    /// Ordinary calls, and the replies that answer them, within the
    /// outstanding-call limit.
    Call,
    /// Control calls, `operation.cancel` and `session.close`, and the replies
    /// that answer them, within the control reserve. The outstanding-call
    /// limit never refuses them (§3).
    Control,
}

/// Whether a byte stream's writer may write a frame ahead of ordinary frames (operator decision D-N). Core sets it
/// when it encodes the frame; the channel does not look into the body.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Class {
    /// `operation.cancel`, a transport `window`, `cancel` or terminal frame: written ahead of ordinary frames, except
    /// those of its own target and any `SessionCall`.
    Control,
    /// Everything else, and the close report, which core keys ordinary although it travels on the control lane so that
    /// the writer writes it after every frame queued before it.
    Ordinary,
}

/// What a frame's order is kept for: an opaque value that the channel compares for equality only. Core derives it from
/// the frame's `call_id` (tags 1 to 3) or its `(request_id, session_id, stream_id)` (tags 16 and 17); the channel holds
/// no GWZ or Taut type and decodes no body. Two targets that collide only keep more order than needed.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Target(u64);

impl Target {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

/// How a byte stream's writer orders a frame: its [`Class`] and its [`Target`] (operator decision D-N). The writer
/// orders by the key and the frame's tag byte only. The in-process adapter takes the key and ignores it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FrameKey {
    pub class: Class,
    pub target: Target,
}

impl FrameKey {
    pub const fn control(target: Target) -> Self {
        Self {
            class: Class::Control,
            target,
        }
    }

    pub const fn ordinary(target: Target) -> Self {
        Self {
            class: Class::Ordinary,
            target,
        }
    }

    /// The key a sender states where the channel ignores it (the in-process adapter).
    pub const IN_PROCESS: FrameKey = FrameKey::ordinary(Target::new(0));
}

/// Why a channel has ended, as one end sees it (§3: the channel reports
/// closure to both ends; §8).
///
/// Once an end reports a reason, it keeps reporting the same one: every later
/// `send` fails with it, and every later `recv` returns it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Closed {
    /// This end closed the channel ([`FrameSink::close`]).
    Local,
    /// The peer closed the channel: its end closed or was dropped, or its
    /// byte stream ended between frames.
    Peer,
    /// A protocol error ended the session (§3). The end that met the frame
    /// reports it, and so may its peer, where the adapter shares the reason.
    Protocol(FrameError),
    /// The byte stream failed, or ended inside a frame
    /// ([`io::ErrorKind::UnexpectedEof`]).
    Stream(io::ErrorKind),
}

impl fmt::Display for Closed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Closed::Local => formatter.write_str("this end closed the channel"),
            Closed::Peer => formatter.write_str("the peer closed the channel"),
            Closed::Protocol(error) => {
                write!(formatter, "a protocol error ended the session: {error}")
            }
            Closed::Stream(kind) => write!(formatter, "the byte stream failed: {kind}"),
        }
    }
}

impl Error for Closed {}

/// Why [`FrameSink::send`] did not send a frame. The frame comes back, so a
/// channel never drops one (§3).
#[derive(Debug)]
pub enum SendError {
    /// The frame's lane is full. Nothing was sent, and the channel stays
    /// open: the lane has room again once the receiver takes frames. Core
    /// reports this as `transport_session_full` (§3, §9).
    Full(Frame),
    /// The channel has ended, or this frame ended it with a protocol error.
    /// The frame was not delivered.
    Closed(Frame, Closed),
}

impl SendError {
    /// The frame that was not sent.
    pub fn into_frame(self) -> Frame {
        match self {
            SendError::Full(frame) | SendError::Closed(frame, _) => frame,
        }
    }

    /// Why the channel ended, or `None` for a full lane.
    pub fn closed(&self) -> Option<Closed> {
        match self {
            SendError::Full(_) => None,
            SendError::Closed(_, closed) => Some(*closed),
        }
    }
}

impl fmt::Display for SendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendError::Full(_) => formatter.write_str("the frame's lane is full"),
            SendError::Closed(_, closed) => write!(formatter, "the channel has ended: {closed}"),
        }
    }
}

impl Error for SendError {}

/// The sending side of a channel end (§3; crate map §2 and §7).
pub trait FrameSink {
    /// Sends `frame` on `lane`, keyed by `key`. The in-process adapter uses the lane and ignores the key; a byte
    /// stream ignores the lane and orders its writes by the key (operator decision D-N).
    ///
    /// Frames arrive in the order their sends succeeded, whatever their
    /// lanes, and each arrives once and whole, unless the receiving end has
    /// closed. A frame its carrier does not carry, or larger than
    /// [`MAX_FRAME_BYTES`](crate::MAX_FRAME_BYTES), is a protocol error: it
    /// ends the session at both ends and comes back in [`SendError::Closed`]
    /// with [`Closed::Protocol`].
    ///
    /// # Errors
    ///
    /// [`SendError::Full`] when the lane is full, leaving the channel open;
    /// [`SendError::Closed`] once the channel has ended. Either way the frame
    /// comes back.
    ///
    /// Whether a send can wait is the adapter's to state: the in-process
    /// adapter's never waits, and a byte stream's waits while its writer
    /// does.
    fn send(&self, frame: Frame, lane: Lane, key: FrameKey) -> Result<(), SendError>;

    /// Ends the session at this end: §8's "closes the channel".
    ///
    /// This end sends and receives nothing more, not even frames the peer
    /// sent before the close: `send` fails, and `recv` returns
    /// [`Closed::Local`], or the reason the channel had already ended for.
    /// The peer receives every frame this end sent before closing, then
    /// [`Closed::Peer`]. Closing again changes nothing, and dropping an end
    /// closes it.
    fn close(&self);
}

/// The receiving side of a channel end (§3).
pub trait FrameSource {
    /// Takes the next frame, waiting until one arrives or the channel has
    /// ended (§3).
    ///
    /// Frames come in the order the peer sent them, each once and whole.
    /// After the peer ends the channel, the frames it sent first are still
    /// delivered, and then the closure.
    ///
    /// # Errors
    ///
    /// Why the channel has ended; every later call returns the same reason.
    fn recv(&self) -> Result<Frame, Closed>;
}

/// Where a byte-stream end delivers the frames it receives (operator decision D-N). A byte-stream end implements
/// [`FrameSink`] only: its reader checks each frame's length, tag and size, and hands the frame, as a tag and a body,
/// to the router that core supplied when it built the end. Core decodes the body there and owns every per-class and
/// per-binding queue, so the reader never waits for a consumer.
pub trait FrameRouter: Send + Sync {
    /// Takes one received frame. It never blocks. An error ends the session at this end with that reason.
    fn route(&self, frame: Frame) -> Result<(), Closed>;

    /// The session has ended at this end, for the reason that stands: called once, after the last `route`.
    fn ended(&self, reason: Closed);
}

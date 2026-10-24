//! The carrier seam: what moves the transport's discrete messages between the core side and the client side of a
//! split (convergence plan 2.1; Connection API contract §6, "the carrier adapter").
//!
//! A [`Carrier`] is one party's end of a connection. It moves whole messages, in order, in each direction, through a
//! bounded queue that keeps a reserve for control traffic, and it ends with a stated [`CarrierClosed`] reason that
//! both ends can see. Three things implement it:
//!
//! | End | Moves | Peer |
//! | --- | --- | --- |
//! | [`InProcessCarrier`] (the default) | typed messages, nothing encoded | the other end in this process |
//! | [`StreamCarrier`] | `session-channel` frames over a byte stream | another process, after the peer check |
//! | the local carrier ([`local`]) | a `StreamCarrier` over a Unix domain socket or a Windows named pipe | same user, same machine |
//!
//! **Lanes.** A message is `Data` or `Control` by its kind, by the mux's own rule ([`Lane::of`]). The queue of one
//! direction is one FIFO (frames arrive in the order they were sent, whatever their lanes) with a capacity per lane:
//! a full `Data` lane refuses a data message, giving it back, and never refuses a control message while the control
//! reserve has room. So a flood of data cannot keep `Window`, `Cancel` or a terminal message from being accepted. A
//! control message does not overtake the data queued before it: the transport's terminal rules need that order.
//!
//! **Binding.** [`CarrierBinding`] is the capability a carrier end hands the composition, which hands it to a session.
//! Every [`Arrival`] carries the binding of the end it arrived on, and a session admits a message only when its own
//! binding [`admits`](CarrierBinding::admits) the arrival. A binding is valid only on the carrier connection that
//! bound it: a message that arrived on another connection is discarded with no state created (contract §6).
//!
//! **Close.** An end closes with a [`CloseCode`]. Its peer reads the code in [`CloseReason::Peer`], after the messages
//! sent before the close. A dropped end closes with `Dropped`. A stream that ends without the announcement reads as
//! [`CloseReason::PeerGone`], a failed one as [`CloseReason::Stream`], and a frame the carrier refuses as
//! [`CloseReason::Protocol`]. Nothing waits on a timeout to learn that the other end has gone.
//!
//! Bytes: the stream carrier owns the framing of `session-channel` (length prefix, size cap) and adds the tags of the
//! transport lane ([`wire`]). It writes no framing of its own.
use crate::transport_host::Attachment;
use gwz_transport::protocol::MessageKind;
use std::{fmt, io, task::Waker};

mod binding;
mod in_process;
mod link;
mod queue;
mod stream;
mod wire;
// The local carrier's first production caller is `gwz server` (plan 2.3), which this step does not write.
#[allow(
    dead_code,
    reason = "first production caller: plan step 2.3, the server command"
)]
pub(crate) mod local;
cfg_if::cfg_if! { if #[cfg(test)] { mod tests; } }

pub(crate) use binding::{CarrierBinding, CarrierId, Peer, Side};
pub(crate) use in_process::{InProcessCarrier, in_process};
pub(crate) use link::CarrierLink;
pub(crate) use stream::{BindingRefused, FrameRefused, StreamCarrier};
pub(crate) use wire::{SessionOpenRefusal, validate_session_open};

/// Which lane of a carrier's queue a message takes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Lane {
    /// Data, and the opens and identity checks that start work: bounded by the data lane.
    Data,
    /// Window, cancel, terminal and bootstrap messages: bounded by the control reserve, which data never uses.
    Control,
}

impl Lane {
    /// The mux's own rule (`gwz-transport` `mux::enqueue`): everything but `Data`, `Open` and `CheckIdentity` is
    /// control. A carrier that classified differently would let a lane fill that the mux keeps room in.
    pub(crate) fn of(kind: MessageKind) -> Lane {
        match kind {
            MessageKind::Data | MessageKind::Open | MessageKind::CheckIdentity => Lane::Data,
            _ => Lane::Control,
        }
    }
}

/// A direction's queue capacity: `data_frames` on the data lane and `control_reserve` on the control lane, which only
/// control messages use. Whole messages bound the queue; a message is at most the transport's own 128 KiB frame, so
/// the bound in bytes is the count times that.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CarrierLimits {
    data_frames: usize,
    control_reserve: usize,
    inbound: InboundLimits,
}

impl CarrierLimits {
    /// The mux's own queue: 64 frames, 8 of them reserved for control. The carrier keeps a wider control reserve
    /// because it queues both directions' control traffic of every binding in its connection.
    pub(crate) const DEFAULT: CarrierLimits = CarrierLimits {
        data_frames: 64,
        control_reserve: 16,
        inbound: InboundLimits::DEFAULT,
    };

    /// `None` when either lane would have no room, or the capacity would overflow.
    pub(crate) fn new(data_frames: usize, control_reserve: usize) -> Option<Self> {
        (data_frames > 0
            && control_reserve > 0
            && data_frames.checked_add(control_reserve).is_some())
        .then_some(Self {
            data_frames,
            control_reserve,
            inbound: InboundLimits::DEFAULT,
        })
    }

    /// These limits with other bounds on what the peer may have queued here.
    pub(crate) fn with_inbound(self, inbound: InboundLimits) -> Self {
        Self { inbound, ..self }
    }

    pub(crate) fn inbound(self) -> InboundLimits {
        self.inbound
    }

    pub(crate) fn capacity(self, lane: Lane) -> usize {
        match lane {
            Lane::Data => self.data_frames,
            Lane::Control => self.control_reserve,
        }
    }
}

impl Default for CarrierLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// What a carrier end keeps for what its peer sends, with no consumer needed: the reader of a byte stream hands every
/// frame to a router that never blocks (operator decision D-N). A peer must keep within these bounds, which the
/// composition sets from the limits of the session the carrier serves. The session registry's class is a frame count,
/// the call limit and the control reserve of the session; the transport lane is not: its state is per binding and per
/// stream ([`BindingLimits`]). A session-class overrun ends the carrier; a binding's faults only that binding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InboundLimits {
    /// Frames under tags 1 to 8 queued for the consumer: the session's outstanding calls and control reserve.
    pub(crate) registry_frames: usize,
    /// Bytes queued for the consumer across all classes, one frame larger than this aside when the queue is empty. A
    /// transport frame past it faults its binding; a registry frame ends the carrier.
    pub(crate) bytes: usize,
    /// Bindings open at once on the carrier.
    pub(crate) max_bindings: usize,
}

impl InboundLimits {
    /// Room for the session contract's defaults (1,024 outstanding calls and a control reserve of 64) in the registry
    /// class, and for 2 MiB-frames of two full queues in bytes.
    pub(crate) const DEFAULT: InboundLimits = InboundLimits {
        registry_frames: 1024 + 64,
        bytes: 2 * gwz_session_contract::MAX_FRAME_BYTES,
        max_bindings: 1024,
    };
}

/// What one binding's receiving side keeps, per stream, in quantities its sender controls (`GwzSplitCommandChannel.md`
/// §3.3, "Transport-class bound"): `data` is bounded in bytes by `receive_window` whatever the size of its frames (a
/// frame is charged at least one byte); a `window` coalesces into the stream's one queued window at the highest
/// `max_offset`; every other kind holds at most `per_kind` queued frames of that kind on a stream; and a binding holds
/// at most `max_streams` streams with queued state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BindingLimits {
    pub(crate) receive_window: usize,
    pub(crate) max_streams: usize,
    pub(crate) per_kind: u8,
}

impl BindingLimits {
    /// The binding's negotiated transport `Limits`: its receive window; the stream bound is the 4,096 sequenced
    /// streams a binding lives for.
    pub(crate) fn from_transport(limits: &gwz_transport::protocol::Limits) -> Self {
        Self {
            receive_window: usize::try_from(limits.receive_window).unwrap_or(usize::MAX),
            max_streams: 4096,
            per_kind: 4,
        }
    }
}

impl Default for BindingLimits {
    fn default() -> Self {
        Self::from_transport(&gwz_transport::binding::default_limits())
    }
}

/// Why a binding was faulted: the carrier's other bindings and calls go on (contract §6).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BindingFault {
    /// The peer queued more than the binding's bound for a class of its state.
    Overrun(FrameClass),
    /// A frame that breaks the binding's rules: a kind under the other class's tag.
    Invalid,
}

/// The class of an inbound frame, by its tag.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum FrameClass {
    /// Tag 16.
    TransportData,
    /// Tag 17.
    TransportControl,
    /// Tags 1 to 8: the session contract's.
    Registry,
}

/// Why an end closed, as one byte on the wire. A code the peer sends that is not here ends the carrier with a
/// protocol fault.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum CloseCode {
    /// An orderly close.
    Normal,
    /// The end was dropped.
    Dropped,
    /// The operation the carrier served was cancelled.
    Cancelled,
    /// The end met a protocol fault and closes on it.
    ProtocolFault,
}

impl CloseCode {
    pub(crate) fn wire(self) -> u8 {
        match self {
            CloseCode::Normal => 0,
            CloseCode::Dropped => 1,
            CloseCode::Cancelled => 2,
            CloseCode::ProtocolFault => 3,
        }
    }

    pub(crate) fn from_wire(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(CloseCode::Normal),
            1 => Some(CloseCode::Dropped),
            2 => Some(CloseCode::Cancelled),
            3 => Some(CloseCode::ProtocolFault),
            _ => None,
        }
    }
}

/// What a carrier refused on the wire or in a message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProtocolFault {
    /// The framing: a length prefix or a tag the carrier does not carry (`session-channel`'s refusal).
    Frame(gwz_session_contract::FrameError),
    /// A transport message that does not encode or decode under the transport's own limits.
    Message,
    /// A close announcement with a code this carrier does not define, or one with the wrong shape.
    Close,
    /// A frame arrived that the carrier's consumer cannot take (a registry frame at a message-level end).
    Unexpected,
    /// A `SessionOpen` that carries more than the client placement's shape allows.
    SessionOpen(SessionOpenRefusal),
    /// The peer queued more frames of a class, or more bytes, than the bound it must respect (D-N).
    Overrun(FrameClass),
}

/// Why a carrier has ended, as one end sees it. The first reason stands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CloseReason {
    /// This end closed with the code.
    Local(CloseCode),
    /// The peer closed with the code, after sending what it had sent.
    Peer(CloseCode),
    /// The peer's stream ended without an announcement: the peer's process is gone, or killed.
    PeerGone,
    /// The byte stream failed.
    Stream(io::ErrorKind),
    /// A fault in what the peer sent, or in what this end was asked to send.
    Protocol(ProtocolFault),
}

/// A carrier has ended. Every later call on the end reports the same reason.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CarrierClosed {
    pub(crate) reason: CloseReason,
}

impl fmt::Display for CarrierClosed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.reason {
            CloseReason::Local(code) => write!(f, "this end closed the carrier ({code:?})"),
            CloseReason::Peer(code) => write!(f, "the peer closed the carrier ({code:?})"),
            CloseReason::PeerGone => f.write_str("the peer's end of the carrier is gone"),
            CloseReason::Stream(kind) => write!(f, "the carrier's stream failed: {kind}"),
            CloseReason::Protocol(fault) => {
                write!(f, "the carrier met a protocol fault: {fault:?}")
            }
        }
    }
}

impl std::error::Error for CarrierClosed {}

/// Why [`Carrier::try_send`] did not send. The message comes back; a carrier never drops one.
#[derive(Debug)]
pub(crate) enum SendRefused {
    /// The message's lane is full. Nothing was queued and the carrier stays open: try again once the lane has room,
    /// which wakes the registered waker.
    Full(Box<Attachment>),
    /// The carrier has ended, or this message ended it (one that does not encode).
    Closed(Box<Attachment>, CarrierClosed),
}

/// A message that arrived on a carrier end, with the binding of that end.
#[derive(Debug)]
pub(crate) struct Arrival {
    pub(crate) binding: CarrierBinding,
    pub(crate) message: Attachment,
}

impl Arrival {
    /// The message, if `bound` admits it; otherwise it is discarded here, and nothing is created for it.
    pub(crate) fn into_message_for(self, bound: &CarrierBinding) -> Option<Attachment> {
        bound.admits(&self).then_some(self.message)
    }
}

/// One party's end of a carrier connection. Calls never wait: a full lane or an empty queue is an answer, and the
/// waker registered with [`register`](Carrier::register) is woken when the answer may have changed.
pub(crate) trait Carrier: Send + Sync {
    /// Queues `message` for the peer on its lane.
    fn try_send(&self, message: Attachment) -> Result<(), SendRefused>;

    /// The next message from the peer in the order it was sent, with this end's binding, or `None` when nothing has
    /// arrived. After the peer's close, the messages it sent before still arrive, then the closure.
    fn try_recv(&self) -> Result<Option<Arrival>, CarrierClosed>;

    /// Wakes `waker` when a message arrives, a full lane gets room, or the carrier ends. One waker per end: a later
    /// registration replaces the earlier.
    fn register(&self, waker: &Waker);

    /// Ends the carrier at this end with `code`. This end sends and receives nothing more; the peer receives what was
    /// sent before, then the code. Closing again changes nothing.
    fn close(&self, code: CloseCode);

    /// This end's binding: the capability the composition gives a session.
    fn binding(&self) -> CarrierBinding;
}

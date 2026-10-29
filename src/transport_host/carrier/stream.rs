//! The stream carrier: frames over a byte stream, behind the seam.
//!
//! The framing and the reader are `session-channel`'s byte-stream adapter: a little-endian `u32` length, a tag byte, the
//! body; a length of zero or over 64 MiB ends the carrier before a body is allocated. This module is the **core side**
//! of the adapter's two interfaces (operator decision D-N, `GwzSplitCommandChannel.md` §3.3): it keys every frame it
//! sends, and it is the [`FrameRouter`](gwz_session_contract::FrameRouter) that receives every frame the adapter reads.
//! It is split by direction: [`state`] (what the end keeps under its lock), [`inbound`] (the router and admission) and
//! [`writer`] (admission to the writer, the writer thread and the stall watchdog).
//!
//! **Writing.** Callers never wait: [`Carrier::try_send`] queues a frame or answers `Full`. A writer thread takes the
//! next frame by `session-channel`'s [`WriteQueue`](gwz_session_channel::WriteQueue) rule: at each frame boundary the
//! oldest *unblocked* control-class frame (a transport frame under tag 17) goes ahead of the ordinary frames before it,
//! except ordinary frames of its own target (the same call, or the same request, session and stream) and any
//! `SessionCall`, which it never overtakes. So a cancel or a window is not held behind a spool of another call or
//! stream, and per-target order and the increasing call-ID rule survive. Admission is bounded per lane (the data lane,
//! and a control reserve that data never uses), so a full data lane never keeps a control frame from being accepted.
//!
//! **Reading.** The adapter's reader never waits for this side's consumer; the router never blocks. What it reads
//! waits in an arrival-ordered record that is **taken by class**: the registry's frames (tags 1 to 8: the session's
//! calls, replies and handshake) by [`StreamCarrier::try_recv_registry`], and the transport frames of each *binding*
//! (the composition's per-operation capability, [`StreamCarrier::open_binding`]) by
//! [`Carrier::try_recv_binding`]; an end with one consumer takes the whole record in arrival order
//! ([`Carrier::try_recv`]). A frame's charge stays against its class or binding until its own consumer takes it, so a
//! binding whose consumer is not ready holds back that binding only: its whole window can sit queued and untaken
//! while a registry frame and another binding's frames are taken. The registry class is bounded by the call limit,
//! the control reserve and its own byte cap ([`InboundLimits`](super::InboundLimits)); an overrun ends the carrier. A
//! binding's state is per stream and bounded in quantities the sender controls ([`BindingLimits`]): `data` in bytes by
//! the binding's receive window, a `window` coalesced into the stream's one queued window at the highest offset, every
//! other kind in a few slots per stream. A peer that breaks a binding's bound, sends it a kind under the wrong tag, or
//! sends a frame that names it and does not decode, faults that binding only: its queued frames are dropped, its later
//! frames are discarded, and the carrier's other bindings and calls go on. A frame for a binding that is not open is
//! discarded and creates no state. Only a registry overrun or a protocol fault of the session ends the carrier.
//!
//! **A refused `SessionOpen`.** A `SessionOpen` that is not of the client placement's frozen shape is not delivered:
//! the end writes exactly one `SessionError` (`invalid_request`, call ID 0) and closes. A sender's own refusal happens
//! before any byte of the frame is queued.
//!
//! **Endings.** A peer that stops listening ends this end's writing, not its reading; a peer that stops reading ends
//! the carrier as `Stalled` after the no-progress bound; a closing end gives up flushing after the close bound
//! ([`writer`]).
//!
//! **Bytes written** are bounded as well: at most [`QUEUE_BYTES`] queued for the writer, and a frame larger than that
//! is admitted only into an empty queue.
use super::{
    Arrival, Attachment, BindingFault, BindingLimits, Carrier, CarrierBinding, CarrierClosed,
    CarrierLimits, CloseCode, CloseReason, Lane, ProtocolFault, SendRefused, TakeEnd, wire,
};
use gwz_session_channel::{ByteStreamEnd, byte_stream};
use gwz_session_contract::{Frame, FrameKey};
use std::{
    collections::HashMap,
    io::{self, Read, Write},
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    task::Waker,
    thread,
    time::{Duration, Instant},
};

mod inbound;
mod state;
mod writer;

use state::{BindingInbound, Ending, Scope, State, Taken};
use writer::{Counting, Progress, watch_loop, write_loop};

pub(crate) use writer::QUEUE_BYTES;

/// Why [`StreamCarrier::send_frame`] did not send; the frame comes back.
#[derive(Debug)]
pub(crate) enum FrameRefused {
    Full(Frame),
    Closed(Frame, CarrierClosed),
}

/// Why [`StreamCarrier::open_binding`] did not open a binding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BindingRefused {
    /// The binding's name is empty, too long or has control characters, or the binding is open already.
    Name,
    /// The carrier holds its limit of bindings.
    TooMany,
    /// The carrier has ended.
    Closed(CarrierClosed),
}

/// Everything but the byte-stream end: what the router, the writer thread and the watchdog share.
struct Core {
    state: Mutex<State>,
    /// The writer waits here for a frame or an ending.
    out_ready: Condvar,
    /// Blocking receivers wait here for a frame or an end.
    in_ready: Condvar,
    /// The writer waits here, after its announcement, for the reader to return.
    reader_gone: Condvar,
    /// The watchdog waits here for work, a deadline or the end.
    watch_ready: Condvar,
    /// Forces the stream shut: shuts a socket down so that a blocked read or write returns.
    abort: Box<dyn Fn() + Send + Sync>,
    binding: CarrierBinding,
    limits: CarrierLimits,
    /// When the stream last accepted a byte.
    progress: Arc<Progress>,
}

struct Inner {
    core: Arc<Core>,
    end: ByteStreamEnd<Counting<Box<dyn Write + Send>>>,
}

/// One end of a carrier over a byte stream.
pub(crate) struct StreamCarrier {
    inner: Arc<Inner>,
}

fn lock(core: &Core) -> MutexGuard<'_, State> {
    core.state.lock().unwrap_or_else(PoisonError::into_inner)
}

fn closed(reason: CloseReason) -> CarrierClosed {
    CarrierClosed { reason }
}

/// The session channel's name for a lane.
fn wire_lane(lane: Lane) -> gwz_session_contract::Lane {
    match lane {
        Lane::Data => gwz_session_contract::Lane::Call,
        Lane::Control => gwz_session_contract::Lane::Control,
    }
}

fn lane_index(lane: Lane) -> usize {
    match lane {
        Lane::Data => 0,
        Lane::Control => 1,
    }
}

/// Overwrites a refused frame's body: it may have carried a secret.
fn wipe(mut frame: Frame) {
    frame.body.fill(0);
    std::hint::black_box(&frame.body);
}

pub(super) fn binding_name_ok(name: &str) -> bool {
    !name.is_empty() && name.len() <= 128 && !name.chars().any(char::is_control)
}

/// The router the byte-stream adapter hands every frame to.
struct CarrierRouter(Arc<Core>);

/// The stream's reading half, which the adapter's reader thread owns: when that thread returns it drops the half, which
/// is how this end learns the thread is gone ([`StreamCarrier::wait_stopped`]).
struct ReadGuard {
    inner: Box<dyn Read + Send>,
    core: Arc<Core>,
}

impl Read for ReadGuard {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buffer)
    }
}

impl Drop for ReadGuard {
    fn drop(&mut self) {
        lock(&self.core).reader_exited = true;
        self.core.in_ready.notify_all();
    }
}

enum Admission {
    Full(Frame),
    Closed(Frame, CarrierClosed),
}

impl Core {
    fn wake(&self, waker: Option<Waker>) {
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

impl StreamCarrier {
    /// A carrier over `read` and `write`. `abort` shuts the stream so that a blocked read or write returns (a socket's
    /// `shutdown`, a pipe's cancel and disconnect); it may do nothing for a stream whose halves end by being dropped.
    pub(crate) fn new(
        read: Box<dyn Read + Send>,
        write: Box<dyn Write + Send>,
        abort: Box<dyn Fn() + Send + Sync>,
        binding: CarrierBinding,
        limits: CarrierLimits,
    ) -> io::Result<Self> {
        let progress = Arc::new(Progress::new());
        let core = Arc::new(Core {
            state: Mutex::new(State::new()),
            out_ready: Condvar::new(),
            in_ready: Condvar::new(),
            reader_gone: Condvar::new(),
            watch_ready: Condvar::new(),
            abort,
            binding,
            limits,
            progress: progress.clone(),
        });
        let counted = Counting {
            inner: write,
            progress,
        };
        let watched = ReadGuard {
            inner: read,
            core: core.clone(),
        };
        let end = byte_stream(watched, counted, Arc::new(CarrierRouter(core.clone())))?;
        let inner = Arc::new(Inner { core, end });
        let writing = inner.clone();
        thread::Builder::new()
            .name("gwz-carrier-write".into())
            .spawn(move || write_loop(&writing))?;
        let watching = inner.core.clone();
        thread::Builder::new()
            .name("gwz-carrier-watch".into())
            .spawn(move || watch_loop(&watching))?;
        Ok(Self { inner })
    }

    fn core(&self) -> &Core {
        &self.inner.core
    }

    /// Opens a binding: the carrier will route the transport frames whose envelope names `id` to it, within `limits`.
    /// A frame for a binding that is not open is discarded and creates no state (contract §6).
    pub(crate) fn open_binding(
        &self,
        id: &str,
        limits: BindingLimits,
    ) -> Result<(), BindingRefused> {
        let mut state = lock(self.core());
        if let Some(reason) = state.gone() {
            return Err(BindingRefused::Closed(reason));
        }
        state.open_binding(id, limits, self.core().limits.inbound().max_bindings)
    }

    /// Retires a binding: its queued frames are dropped and its later frames are discarded without a record. Its
    /// consumer is woken to learn that the binding is over ([`TakeEnd::Binding`]).
    pub(crate) fn retire_binding(&self, id: &str) {
        let mut state = lock(self.core());
        let waker = state.remove_binding(id);
        drop(state);
        self.core().in_ready.notify_all();
        self.core().wake(waker);
    }

    /// Why the binding was faulted, if it has been. The fault stays until the binding is retired.
    pub(crate) fn binding_fault(&self, id: &str) -> Option<BindingFault> {
        lock(self.core()).bindings.get(id).and_then(|b| b.fault)
    }

    /// How many frames were discarded because their binding was not open or had been faulted, or because they arrived
    /// after the peer's close announcement.
    pub(crate) fn discarded_frames(&self) -> usize {
        lock(self.core()).discarded
    }

    /// Queues a registry frame (tags 1 to 8) on `lane`, keyed by `key`. A `SessionOpen` is checked first.
    pub(crate) fn send_frame(
        &self,
        frame: Frame,
        lane: Lane,
        key: FrameKey,
    ) -> Result<(), FrameRefused> {
        if let Some(fault) = frame_fault(&frame) {
            let reason = CloseReason::Protocol(fault);
            self.core()
                .finish(reason, Ending::Flush(CloseCode::ProtocolFault));
            return Err(FrameRefused::Closed(frame, closed(reason)));
        }
        let mut state = lock(self.core());
        match self.core().admit(&mut state, frame, lane, key) {
            Ok(()) => {
                drop(state);
                self.core().out_ready.notify_one();
                self.core().watch_ready.notify_one();
                Ok(())
            }
            Err(Admission::Closed(frame, reason)) => Err(FrameRefused::Closed(frame, reason)),
            Err(Admission::Full(frame)) => Err(FrameRefused::Full(frame)),
        }
    }

    /// The next registry frame (tags 1 to 8), if one has arrived. Transport frames are not its business.
    pub(crate) fn try_recv_registry(&self) -> Result<Option<Frame>, CarrierClosed> {
        match self.take_scoped(Scope::Registry)? {
            Some(Taken::Registry(frame) | Taken::Message(frame)) => Ok(Some(frame)),
            None => Ok(None),
        }
    }

    /// The next registry frame, waiting up to `wait`.
    pub(crate) fn recv_registry_timeout(
        &self,
        wait: Duration,
    ) -> Result<Option<Frame>, CarrierClosed> {
        match self.recv_item(Scope::Registry, wait)? {
            Some(Taken::Registry(frame) | Taken::Message(frame)) => Ok(Some(frame)),
            None => Ok(None),
        }
    }

    /// The next transport message of `binding`, waiting up to `wait`. A binding that was faulted, retired or never
    /// opened ends the take with [`TakeEnd::Binding`] at once; the carrier's own end is [`TakeEnd::Carrier`].
    pub(crate) fn recv_binding_timeout(
        &self,
        binding: &str,
        wait: Duration,
    ) -> Result<Option<Arrival>, TakeEnd> {
        match self.recv_with(wait, |core, state| core.take_binding(state, binding))? {
            None => Ok(None),
            Some(item) => self.arrival(item).map(Some).map_err(TakeEnd::Carrier),
        }
    }

    /// The next transport message of the whole end, in arrival order, waiting up to `wait`.
    pub(crate) fn recv_timeout(&self, wait: Duration) -> Result<Option<Arrival>, CarrierClosed> {
        match self.recv_item(Scope::Any, wait)? {
            None => Ok(None),
            Some(item) => self.arrival(item).map(Some),
        }
    }

    /// Wakes `waker` when a registry frame arrives or the carrier ends. One waker for the registry.
    pub(crate) fn register_registry(&self, waker: &Waker) {
        lock(self.core()).registry.waker = Some(waker.clone());
    }

    /// Waits up to `wait` for the carrier to end at this end.
    pub(crate) fn wait_closed(&self, wait: Duration) -> Option<CarrierClosed> {
        let deadline = Instant::now() + wait;
        let mut state = lock(self.core());
        loop {
            if let Some(reason) = state.closed {
                return Some(reason);
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            state = self
                .core()
                .in_ready
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Waits up to `wait` for the end's threads to have returned: the reader, the writer and the watchdog. They return
    /// once the carrier has ended and its stream is shut; a peer that never reads cannot keep them past the close or
    /// stall bound.
    pub(crate) fn wait_stopped(&self, wait: Duration) -> bool {
        let deadline = Instant::now() + wait;
        let mut state = lock(self.core());
        loop {
            if state.reader_exited && state.writer_done {
                return true;
            }
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            state = self
                .core()
                .in_ready
                .wait_timeout(state, left.min(Duration::from_millis(20)))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Entries queued for the consumers, across the registry and every binding.
    pub(crate) fn queued_in(&self) -> usize {
        lock(self.core()).inn.len()
    }

    /// Frames queued for the writer, the one it is writing excluded.
    pub(crate) fn queued_out(&self) -> usize {
        lock(self.core()).out.len()
    }

    /// How long frames have waited with no byte accepted by the stream, or `None` while nothing waits: the progress
    /// signal the composition's own stuck-peer policy reads (`GwzSplitCommandChannel.md` §4.3).
    pub(crate) fn stalled_for(&self) -> Option<Duration> {
        lock(self.core())
            .busy()
            .then(|| self.core().progress.idle())
    }

    /// Ends the carrier now: queued frames are dropped, the stream is shut and the peer reads the stream's end.
    pub(crate) fn abort(&self) {
        self.core()
            .finish(CloseReason::Local(CloseCode::Cancelled), Ending::Abort);
    }

    fn take_scoped(&self, scope: Scope<'_>) -> Result<Option<Taken>, CarrierClosed> {
        let mut state = lock(self.core());
        self.core().take(&mut state, scope)
    }

    fn arrival(&self, item: Taken) -> Result<Arrival, CarrierClosed> {
        match item {
            Taken::Message(frame) => match wire::decode_attachment(&frame) {
                Ok(message) => Ok(Arrival {
                    binding: self.core().binding.clone(),
                    message,
                }),
                Err(fault) => Err(self.fail(fault)),
            },
            Taken::Registry(frame) => {
                wipe(frame);
                Err(self.fail(ProtocolFault::Unexpected))
            }
        }
    }

    fn fail(&self, fault: ProtocolFault) -> CarrierClosed {
        let reason = CloseReason::Protocol(fault);
        self.core()
            .finish(reason, Ending::Flush(CloseCode::ProtocolFault));
        closed(reason)
    }

    fn recv_item(&self, scope: Scope<'_>, wait: Duration) -> Result<Option<Taken>, CarrierClosed> {
        self.recv_with(wait, |core, state| core.take(state, scope))
    }

    /// Waits up to `wait` for `take` to give an item or an end.
    fn recv_with<E>(
        &self,
        wait: Duration,
        mut take: impl FnMut(&Core, &mut State) -> Result<Option<Taken>, E>,
    ) -> Result<Option<Taken>, E> {
        let deadline = Instant::now() + wait;
        let mut state = lock(self.core());
        loop {
            if let Some(item) = take(self.core(), &mut state)? {
                return Ok(Some(item));
            }
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return Ok(None);
            };
            state = self
                .core()
                .in_ready
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

/// What is wrong with a frame a caller asks to send, if anything.
fn frame_fault(frame: &Frame) -> Option<ProtocolFault> {
    match frame.tag {
        4 => wire::validate_session_open(&frame.body)
            .err()
            .map(ProtocolFault::SessionOpen),
        1..=8 => None,
        _ => Some(ProtocolFault::Unexpected),
    }
}

impl Carrier for StreamCarrier {
    fn try_send(&self, message: Attachment) -> Result<(), SendRefused> {
        let frame = match wire::encode_attachment(&message) {
            Ok(frame) => frame,
            Err(fault) => {
                let reason = CloseReason::Protocol(fault);
                self.core()
                    .finish(reason, Ending::Flush(CloseCode::ProtocolFault));
                return Err(SendRefused::Closed(Box::new(message), closed(reason)));
            }
        };
        let lane = Lane::of(message.1.kind);
        let key = wire::key_of(&message);
        let mut state = lock(self.core());
        match self.core().admit(&mut state, frame, lane, key) {
            Ok(()) => {
                drop(state);
                self.core().out_ready.notify_one();
                self.core().watch_ready.notify_one();
                Ok(())
            }
            Err(Admission::Closed(_, reason)) => {
                Err(SendRefused::Closed(Box::new(message), reason))
            }
            Err(Admission::Full(_)) => Err(SendRefused::Full(Box::new(message))),
        }
    }

    fn try_recv(&self) -> Result<Option<Arrival>, CarrierClosed> {
        match self.take_scoped(Scope::Any)? {
            None => Ok(None),
            Some(item) => self.arrival(item).map(Some),
        }
    }

    fn try_recv_transport(&self) -> Result<Option<Arrival>, CarrierClosed> {
        match self.take_scoped(Scope::Transport)? {
            None => Ok(None),
            Some(item) => self.arrival(item).map(Some),
        }
    }

    fn bind_fault(&self, session: &str) -> Option<BindingFault> {
        self.binding_fault(session)
    }

    fn try_recv_binding(&self, binding: &str) -> Result<Option<Arrival>, TakeEnd> {
        let taken = {
            let mut state = lock(self.core());
            self.core().take_binding(&mut state, binding)?
        };
        match taken {
            None => Ok(None),
            Some(item) => self.arrival(item).map(Some).map_err(TakeEnd::Carrier),
        }
    }

    fn register(&self, waker: &Waker) {
        lock(self.core()).waker = Some(waker.clone());
    }

    fn bind_open(&self, session: &str) -> Result<(), BindingRefused> {
        self.open_binding(session, BindingLimits::default())
    }

    fn bind_retire(&self, session: &str) {
        self.retire_binding(session);
    }

    fn accept_binds(&self) {
        lock(self.core()).accept_binds = true;
    }

    fn register_binding(&self, binding: &str, waker: &Waker) {
        if let Some(open) = lock(self.core()).bindings.get_mut(binding) {
            open.waker = Some(waker.clone());
        }
    }

    fn close(&self, code: CloseCode) {
        self.core()
            .finish(CloseReason::Local(code), Ending::Flush(code));
    }

    fn binding(&self) -> CarrierBinding {
        self.core().binding.clone()
    }
}

impl Drop for StreamCarrier {
    fn drop(&mut self) {
        self.close(CloseCode::Dropped);
    }
}

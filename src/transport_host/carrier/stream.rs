//! The stream carrier: frames over a byte stream, behind the seam.
//!
//! The framing and the reader are `session-channel`'s byte-stream adapter: a little-endian `u32` length, a tag byte, the
//! body; a length of zero or over 64 MiB ends the carrier before a body is allocated. This module is the **core side**
//! of the adapter's two interfaces (operator decision D-N, `GwzSplitCommandChannel.md` §3.3): it keys every frame it
//! sends, and it is the [`FrameRouter`] that receives every frame the adapter reads.
//!
//! **Writing.** Callers never wait: [`Carrier::try_send`] queues a frame or answers `Full`. A writer thread takes the
//! next frame by `session-channel`'s [`WriteQueue`] rule: at each frame boundary the oldest control-class frame (a
//! transport frame under tag 17) goes ahead of the ordinary frames before it, except ordinary frames of its own target
//! (the same call, or the same request, session and stream) and any `SessionCall`, which it never overtakes. So a
//! cancel or a window is not held behind a spool of another call or stream, and per-target order and the increasing
//! call-ID rule survive. Admission is bounded per lane (the data lane, and a control reserve that data never uses), so
//! a full data lane never keeps a control frame from being accepted.
//!
//! **Reading.** The adapter's reader never waits for this side's consumer; [`CarrierRouter::route`] never blocks.
//! A registry frame (tags 1 to 8) joins the session's queue, bounded by the call limit and control reserve
//! ([`InboundLimits`]). A transport frame belongs to a *binding*, the composition's per-operation capability
//! ([`StreamCarrier::open_binding`]); its state is per stream and bounded in quantities the sender controls
//! ([`BindingLimits`]): `data` in bytes by the binding's receive window, a `window` coalesced into the stream's one
//! queued window at the highest offset, every other kind in a few slots per stream. A peer that breaks a binding's
//! bound, or sends it a kind under the wrong tag, faults that binding only: its queued frames are dropped, its later
//! frames are discarded, and the carrier's other bindings and calls go on. A frame for a binding that is not open is
//! discarded and creates no state. Only a session-class overrun or a protocol fault of the session ends the carrier.
//!
//! **Bytes written** are bounded as well: at most [`QUEUE_BYTES`] queued for the writer, and a frame larger than that
//! is admitted only into an empty queue.
use super::{
    Arrival, Attachment, BindingFault, BindingLimits, Carrier, CarrierBinding, CarrierClosed,
    CarrierLimits, CloseCode, CloseReason, FrameClass, Lane, ProtocolFault, SendRefused,
    wire::{self, CONTROL_TAG, DATA_TAG, MAX_TRANSPORT_FRAME},
};
use gwz_session_channel::{ByteStreamEnd, WriteQueue, byte_stream};
use gwz_session_contract::{Closed, Frame, FrameKey, FrameRouter, FrameSink};
use gwz_transport::protocol::MessageKind;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io::{self, ErrorKind, Read, Write},
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    task::Waker,
    thread,
    time::{Duration, Instant},
};

/// How long an end that has announced its close waits for its peer to answer by closing.
const LINGER: Duration = Duration::from_secs(2);

/// The most bytes the writer's queue holds, a single larger frame aside.
pub(crate) const QUEUE_BYTES: usize = 8 * 1024 * 1024;

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

/// What the consumer takes next: a registry frame, or a transport frame still encoded (it is decoded as it is taken).
enum Taken {
    Registry(Frame),
    Message(Frame),
}

/// How the writer is to finish when the carrier ends.
#[derive(Clone, Copy)]
enum Ending {
    /// Write what is queued, then the announcement of the code, then shut the stream.
    Flush(CloseCode),
    /// Shut the stream now; queued frames are dropped.
    Abort,
}

/// A frame queued for the writer.
struct Outgoing {
    frame: Frame,
    lane: Lane,
    key: FrameKey,
}

/// One queued inbound entry, in arrival order.
enum Entry {
    Registry(Frame),
    Message(Queued),
}

struct Queued {
    binding: String,
    stream: i64,
    kind: MessageKind,
    /// What the entry holds against its binding's data bound: a data frame's payload (at least one byte), else 0.
    charge: usize,
    frame: Frame,
    /// A queued window's offset, if it may still be coalesced into.
    window: Option<i64>,
}

#[derive(Default)]
struct StreamInbound {
    data_bytes: usize,
    window_seq: Option<u64>,
    /// Queued frames of each non-data, non-window kind, by the kind's wire number.
    kinds: [u8; 24],
    queued: usize,
}

struct BindingInbound {
    limits: BindingLimits,
    fault: Option<BindingFault>,
    streams: HashMap<i64, StreamInbound>,
    seqs: BTreeSet<u64>,
    bytes: usize,
}

struct State {
    out: WriteQueue<Outgoing>,
    out_lanes: [usize; 2],
    out_bytes: usize,
    inn: BTreeMap<u64, Entry>,
    next_seq: u64,
    registry_queued: usize,
    inn_bytes: usize,
    bindings: HashMap<String, BindingInbound>,
    discarded: usize,
    /// Why this end can do nothing more.
    closed: Option<CarrierClosed>,
    /// Why the peer ended, once it has: delivered after the inbound queue drains.
    peer_ended: Option<CloseReason>,
    ending: Option<Ending>,
    /// The adapter has reported the end of its session: its reader has returned.
    reader_done: bool,
    waker: Option<Waker>,
}

/// Everything but the byte-stream end: what the router and the writer thread share.
struct Core {
    state: Mutex<State>,
    /// The writer waits here for a frame or an ending.
    out_ready: Condvar,
    /// Blocking receivers wait here for a frame or an end.
    in_ready: Condvar,
    /// The writer waits here, after its announcement, for the reader to return.
    reader_gone: Condvar,
    /// Forces the stream shut: shuts a socket down so that a blocked read returns.
    abort: Box<dyn Fn() + Send + Sync>,
    binding: CarrierBinding,
    limits: CarrierLimits,
}

struct Inner {
    core: Arc<Core>,
    end: ByteStreamEnd<Box<dyn Write + Send>>,
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

fn binding_name_ok(name: &str) -> bool {
    !name.is_empty() && name.len() <= 128 && !name.chars().any(char::is_control)
}

/// The router the byte-stream adapter hands every frame to.
struct CarrierRouter(Arc<Core>);

impl StreamCarrier {
    /// A carrier over `read` and `write`. `abort` shuts the stream so that a blocked read returns (a socket's
    /// `shutdown`); it may do nothing for a stream whose halves end by being dropped.
    pub(crate) fn new(
        read: Box<dyn Read + Send>,
        write: Box<dyn Write + Send>,
        abort: Box<dyn Fn() + Send + Sync>,
        binding: CarrierBinding,
        limits: CarrierLimits,
    ) -> io::Result<Self> {
        let core = Arc::new(Core {
            state: Mutex::new(State {
                out: WriteQueue::new(),
                out_lanes: [0; 2],
                out_bytes: 0,
                inn: BTreeMap::new(),
                next_seq: 0,
                registry_queued: 0,
                inn_bytes: 0,
                bindings: HashMap::new(),
                discarded: 0,
                closed: None,
                peer_ended: None,
                ending: None,
                reader_done: false,
                waker: None,
            }),
            out_ready: Condvar::new(),
            in_ready: Condvar::new(),
            reader_gone: Condvar::new(),
            abort,
            binding,
            limits,
        });
        let end = byte_stream(read, write, Arc::new(CarrierRouter(core.clone())))?;
        let inner = Arc::new(Inner { core, end });
        let writing = inner.clone();
        thread::Builder::new()
            .name("gwz-carrier-write".into())
            .spawn(move || write_loop(&writing))?;
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
        if !binding_name_ok(id) || state.bindings.contains_key(id) {
            return Err(BindingRefused::Name);
        }
        if state.bindings.len() >= self.core().limits.inbound().max_bindings {
            return Err(BindingRefused::TooMany);
        }
        state.bindings.insert(
            id.to_owned(),
            BindingInbound {
                limits,
                fault: None,
                streams: HashMap::new(),
                seqs: BTreeSet::new(),
                bytes: 0,
            },
        );
        Ok(())
    }

    /// Retires a binding: its queued frames are dropped and its later frames are discarded without a record.
    pub(crate) fn retire_binding(&self, id: &str) {
        let mut state = lock(self.core());
        state.remove_binding(id);
    }

    /// Why the binding was faulted, if it has been. The fault stays until the binding is retired.
    pub(crate) fn binding_fault(&self, id: &str) -> Option<BindingFault> {
        lock(self.core()).bindings.get(id).and_then(|b| b.fault)
    }

    /// How many frames were discarded because their binding was not open or had been faulted.
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
                Ok(())
            }
            Err(Admission::Closed(frame, reason)) => Err(FrameRefused::Closed(frame, reason)),
            Err(Admission::Full(frame)) => Err(FrameRefused::Full(frame)),
        }
    }

    /// The next registry frame, waiting up to `wait`. A transport message in its place comes back as its frame.
    pub(crate) fn recv_frame_timeout(
        &self,
        wait: Duration,
    ) -> Result<Option<Frame>, CarrierClosed> {
        match self.recv_item(wait)? {
            None => Ok(None),
            Some(Taken::Registry(frame) | Taken::Message(frame)) => Ok(Some(frame)),
        }
    }

    /// The next transport message, waiting up to `wait`.
    pub(crate) fn recv_timeout(&self, wait: Duration) -> Result<Option<Arrival>, CarrierClosed> {
        match self.recv_item(wait)? {
            None => Ok(None),
            Some(item) => self.arrival(item).map(Some),
        }
    }

    /// Waits up to `wait` for the carrier to end at this end.
    pub(crate) fn wait_closed(&self, wait: Duration) -> Option<CarrierClosed> {
        let deadline = Instant::now() + wait;
        let mut state = lock(self.core());
        loop {
            if let Some(reason) = state.gone() {
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

    /// Entries queued for the consumer, across the session's queue and every binding.
    pub(crate) fn queued_in(&self) -> usize {
        lock(self.core()).inn.len()
    }

    /// Frames queued for the writer, the one it is writing excluded.
    pub(crate) fn queued_out(&self) -> usize {
        lock(self.core()).out.len()
    }

    /// Ends the carrier now: queued frames are dropped, the stream is shut and the peer reads the stream's end.
    pub(crate) fn abort(&self) {
        self.core()
            .finish(CloseReason::Local(CloseCode::Cancelled), Ending::Abort);
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

    fn recv_item(&self, wait: Duration) -> Result<Option<Taken>, CarrierClosed> {
        let deadline = Instant::now() + wait;
        let mut state = lock(self.core());
        loop {
            if let Some(item) = self.core().take(&mut state)? {
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

enum Admission {
    Full(Frame),
    Closed(Frame, CarrierClosed),
}

impl State {
    /// Why nothing more can be sent: this end's end, or the peer's.
    fn gone(&self) -> Option<CarrierClosed> {
        self.closed.or(self.peer_ended.map(closed))
    }

    fn fits_out(&self, bytes: usize) -> bool {
        self.out.is_empty() || self.out_bytes + bytes <= QUEUE_BYTES
    }

    /// Drops a binding's queued frames and its record.
    fn remove_binding(&mut self, id: &str) {
        if let Some(binding) = self.bindings.remove(id) {
            self.drop_entries(&binding);
        }
    }

    fn drop_entries(&mut self, binding: &BindingInbound) {
        for seq in &binding.seqs {
            self.inn.remove(seq);
        }
        self.inn_bytes -= binding.bytes;
    }

    /// Faults a binding: its queued frames are dropped, its record stays so that its later frames are counted and
    /// discarded, and nothing else changes.
    fn fault_binding(&mut self, id: &str, fault: BindingFault) {
        let Some(mut binding) = self.bindings.remove(id) else {
            return;
        };
        self.drop_entries(&binding);
        binding.seqs.clear();
        binding.streams.clear();
        binding.bytes = 0;
        binding.fault = Some(fault);
        self.bindings.insert(id.to_owned(), binding);
    }

    /// Settles what taking `entry` (queued under `seq`) leaves behind in its binding's state.
    fn release(&mut self, seq: u64, entry: &Queued) {
        self.inn_bytes -= entry.frame.size();
        let Some(binding) = self.bindings.get_mut(&entry.binding) else {
            return;
        };
        binding.seqs.remove(&seq);
        binding.bytes -= entry.frame.size();
        let Some(stream) = binding.streams.get_mut(&entry.stream) else {
            return;
        };
        stream.queued -= 1;
        match entry.kind {
            MessageKind::Data => stream.data_bytes -= entry.charge,
            MessageKind::Window => {
                if stream.window_seq == Some(seq) {
                    stream.window_seq = None;
                }
            }
            kind => {
                let slot = &mut stream.kinds[kind_slot(kind)];
                *slot = slot.saturating_sub(1);
            }
        }
        if stream.queued == 0 {
            binding.streams.remove(&entry.stream);
        }
    }
}

fn kind_slot(kind: MessageKind) -> usize {
    usize::try_from(kind.wire()).unwrap_or(0).min(23)
}

impl Core {
    fn wake(&self, waker: Option<Waker>) {
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    /// Queues `frame` for the writer on `lane`, or says why not. The lane's bound is its own: the control reserve is
    /// never used by data.
    fn admit(
        &self,
        state: &mut State,
        frame: Frame,
        lane: Lane,
        key: FrameKey,
    ) -> Result<(), Admission> {
        if let Some(reason) = state.gone() {
            return Err(Admission::Closed(frame, reason));
        }
        let bytes = frame.size();
        if state.out_lanes[lane_index(lane)] >= self.limits.capacity(lane) || !state.fits_out(bytes)
        {
            return Err(Admission::Full(frame));
        }
        state.out_bytes += bytes;
        state.out_lanes[lane_index(lane)] += 1;
        let tag = frame.tag;
        state.out.push(Outgoing { frame, lane, key }, tag, key);
        Ok(())
    }

    /// Ends this end for `reason` unless it has ended, and says how the writer is to finish.
    fn finish(&self, reason: CloseReason, ending: Ending) {
        let mut state = lock(self);
        if state.closed.is_some() {
            return;
        }
        state.closed = Some(closed(reason));
        state.ending = Some(ending);
        state.inn.clear();
        state.registry_queued = 0;
        state.inn_bytes = 0;
        state.bindings.clear();
        if matches!(ending, Ending::Abort) {
            state.out.clear();
            state.out_bytes = 0;
            state.out_lanes = [0; 2];
        }
        let waker = state.waker.clone();
        drop(state);
        self.out_ready.notify_all();
        self.in_ready.notify_all();
        self.wake(waker);
        if matches!(ending, Ending::Abort) {
            (self.abort)();
        }
    }

    /// After the announcement: waits, up to [`LINGER`], for the peer to close its end in answer (the adapter's reader
    /// then reports the end), so that the announcement is read before this end's handle goes. A wedged peer does not
    /// hold the writer longer than that.
    fn linger(&self) {
        let deadline = Instant::now() + LINGER;
        let mut state = lock(self);
        while !state.reader_done {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return;
            };
            state = self
                .reader_gone
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// The peer has ended: what it sent is still delivered, then `reason`.
    fn peer_ended(&self, reason: CloseReason) {
        let mut state = lock(self);
        if state.closed.is_some() || state.peer_ended.is_some() {
            return;
        }
        state.peer_ended = Some(reason);
        // Nothing more goes to a peer that has ended: the writer stops.
        state.ending.get_or_insert(Ending::Abort);
        state.out.clear();
        state.out_bytes = 0;
        state.out_lanes = [0; 2];
        let waker = state.waker.clone();
        drop(state);
        self.out_ready.notify_all();
        self.in_ready.notify_all();
        self.wake(waker);
    }

    /// The next inbound item, or why there will be none.
    fn take(&self, state: &mut State) -> Result<Option<Taken>, CarrierClosed> {
        if let Some(reason) = state.closed {
            return Err(reason);
        }
        if let Some((seq, entry)) = state.inn.pop_first() {
            return Ok(Some(match entry {
                Entry::Registry(frame) => {
                    state.registry_queued -= 1;
                    state.inn_bytes -= frame.size();
                    Taken::Registry(frame)
                }
                Entry::Message(queued) => {
                    state.release(seq, &queued);
                    Taken::Message(queued.frame)
                }
            }));
        }
        match state.peer_ended {
            Some(reason) => {
                state.closed = Some(closed(reason));
                self.in_ready.notify_all();
                self.out_ready.notify_all();
                Err(closed(reason))
            }
            None => Ok(None),
        }
    }

    /// The session ends here for a protocol fault of its own: the carrier closes, and the adapter's session with it.
    fn session_fault(&self, fault: ProtocolFault) -> Closed {
        self.finish(CloseReason::Protocol(fault), Ending::Abort);
        Closed::Stream(ErrorKind::InvalidData)
    }

    fn notify_consumer(&self, state: MutexGuard<'_, State>) {
        let waker = state.waker.clone();
        drop(state);
        self.in_ready.notify_all();
        self.wake(waker);
    }

    /// Routes a registry frame: the session's queue, bounded by the call limit and control reserve.
    fn route_registry(&self, frame: Frame) -> Result<(), Closed> {
        let bytes = frame.size();
        let mut state = lock(self);
        if state.closed.is_some() {
            return Ok(());
        }
        let over = state.registry_queued >= self.limits.inbound().registry_frames
            || (!state.inn.is_empty() && state.inn_bytes + bytes > self.limits.inbound().bytes);
        if over {
            drop(state);
            return Err(self.session_fault(ProtocolFault::Overrun(FrameClass::Registry)));
        }
        let seq = state.next_seq;
        state.next_seq += 1;
        state.registry_queued += 1;
        state.inn_bytes += bytes;
        state.inn.insert(seq, Entry::Registry(frame));
        self.notify_consumer(state);
        Ok(())
    }

    /// Routes a transport frame to its binding's per-stream state.
    fn route_transport(&self, frame: Frame) -> Result<(), Closed> {
        if frame.size() > MAX_TRANSPORT_FRAME {
            let size = frame.size();
            wipe(frame);
            return Err(self.session_fault(ProtocolFault::Frame(
                gwz_session_contract::FrameError::TooLarge { size },
            )));
        }
        let message = match wire::decode_attachment_any_class(&frame) {
            Ok(message) => message,
            Err(fault) => {
                wipe(frame);
                return Err(self.session_fault(fault));
            }
        };
        let envelope = message.1;
        let binding_id = envelope.session_id.as_str();
        let bytes = frame.size();
        let mut state = lock(self);
        if state.closed.is_some() {
            return Ok(());
        }
        let known = state.bindings.get(binding_id).map(|b| b.fault.is_none());
        if known != Some(true) {
            // Not open, or faulted: discarded, and no state is created for it.
            state.discarded += 1;
            return Ok(());
        }
        let inbound = self.limits.inbound();
        let global_over = !state.inn.is_empty() && state.inn_bytes + bytes > inbound.bytes;
        let seq = state.next_seq;
        let verdict = {
            let binding = state.bindings.get_mut(binding_id).expect("a known binding");
            admit_transport(binding, &envelope, seq, frame.tag, global_over)
        };
        match verdict {
            Verdict::Fault(fault) => {
                state.fault_binding(binding_id, fault);
            }
            Verdict::Coalesced(window_seq, offset) => {
                // The stream's one queued window takes the higher offset, in the place it already holds.
                if let Some(Entry::Message(queued)) = state.inn.get_mut(&window_seq)
                    && queued.window.is_some_and(|old| offset > old)
                {
                    let old_size = queued.frame.size();
                    queued.window = Some(offset);
                    queued.frame = frame;
                    let new_size = queued.frame.size();
                    state.inn_bytes = state.inn_bytes - old_size + new_size;
                    if let Some(binding) = state.bindings.get_mut(binding_id) {
                        binding.bytes = binding.bytes - old_size + new_size;
                    }
                }
            }
            Verdict::Queue { charge, window } => {
                state.next_seq += 1;
                state.inn_bytes += bytes;
                if let Some(binding) = state.bindings.get_mut(binding_id) {
                    binding.bytes += bytes;
                    binding.seqs.insert(seq);
                }
                state.inn.insert(
                    seq,
                    Entry::Message(Queued {
                        binding: binding_id.to_owned(),
                        stream: envelope.stream_id,
                        kind: envelope.kind,
                        charge,
                        frame,
                        window,
                    }),
                );
            }
        }
        self.notify_consumer(state);
        Ok(())
    }
}

/// What a transport frame does to its binding.
enum Verdict {
    Fault(BindingFault),
    /// A window folded into the queued one at this sequence number, carrying this offset.
    Coalesced(u64, i64),
    Queue {
        charge: usize,
        window: Option<i64>,
    },
}

/// Applies a decoded transport frame to its binding's stream state, within the binding's bounds. `seq` is the number
/// the frame would be queued under.
fn admit_transport(
    binding: &mut BindingInbound,
    envelope: &gwz_transport::protocol::Envelope,
    seq: u64,
    tag: u8,
    global_over: bool,
) -> Verdict {
    // A kind under the other class's tag breaks the binding's rules.
    if wire::tag_of(Lane::of(envelope.kind)) != tag {
        return Verdict::Fault(BindingFault::Invalid);
    }
    let class = if tag == DATA_TAG {
        FrameClass::TransportData
    } else {
        FrameClass::TransportControl
    };
    if global_over || envelope.stream_id < 0 {
        return Verdict::Fault(if envelope.stream_id < 0 {
            BindingFault::Invalid
        } else {
            BindingFault::Overrun(class)
        });
    }
    let limits = binding.limits;
    if !binding.streams.contains_key(&envelope.stream_id)
        && binding.streams.len() >= limits.max_streams
    {
        return Verdict::Fault(BindingFault::Overrun(class));
    }
    let stream = binding.streams.entry(envelope.stream_id).or_default();
    let mut verdict = Verdict::Queue {
        charge: 0,
        window: None,
    };
    match envelope.kind {
        MessageKind::Data => {
            let payload = envelope.data.as_ref().map_or(0, |data| data.payload.len());
            let charge = payload.max(1);
            if stream.data_bytes + charge > limits.receive_window {
                return Verdict::Fault(BindingFault::Overrun(FrameClass::TransportData));
            }
            stream.data_bytes += charge;
            verdict = Verdict::Queue {
                charge,
                window: None,
            };
        }
        MessageKind::Window => {
            let offset = envelope
                .window
                .as_ref()
                .map_or(0, |window| window.max_offset);
            // A window folds into the stream's queued one, unless a sequence number rides on either: the sequenced
            // kernel needs every numbered frame, so none is dropped ahead of it.
            if let (Some(window_seq), None) = (stream.window_seq, envelope.message_seq) {
                return Verdict::Coalesced(window_seq, offset);
            }
            stream.window_seq = Some(seq);
            verdict = Verdict::Queue {
                charge: 0,
                window: envelope.message_seq.is_none().then_some(offset),
            };
        }
        kind => {
            let slot = &mut stream.kinds[kind_slot(kind)];
            if *slot >= limits.per_kind {
                return Verdict::Fault(BindingFault::Overrun(FrameClass::TransportControl));
            }
            *slot += 1;
        }
    }
    stream.queued += 1;
    verdict
}

impl FrameRouter for CarrierRouter {
    fn route(&self, frame: Frame) -> Result<(), Closed> {
        let core = &self.0;
        match frame.tag {
            CONTROL_TAG if wire::is_close(&frame) => {
                match wire::parse_close(&frame) {
                    Ok(code) => core.peer_ended(CloseReason::Peer(code)),
                    Err(fault) => return Err(core.session_fault(fault)),
                }
                Ok(())
            }
            DATA_TAG | CONTROL_TAG => core.route_transport(frame),
            4 => match wire::validate_session_open(&frame.body) {
                Ok(()) => core.route_registry(frame),
                Err(refusal) => {
                    // The frame carried more than the placement allows: it is not delivered, and its bytes are wiped.
                    wipe(frame);
                    Err(core.session_fault(ProtocolFault::SessionOpen(refusal)))
                }
            },
            _ => core.route_registry(frame),
        }
    }

    fn ended(&self, reason: Closed) {
        let core = &self.0;
        match reason {
            Closed::Local => {}
            Closed::Peer => core.peer_ended(CloseReason::PeerGone),
            Closed::Stream(kind) => core.finish(CloseReason::Stream(kind), Ending::Abort),
            Closed::Protocol(fault) => core.finish(
                CloseReason::Protocol(ProtocolFault::Frame(fault)),
                Ending::Abort,
            ),
        }
        lock(core).reader_done = true;
        core.reader_gone.notify_all();
    }
}

/// Writes frames until the carrier ends, then the announcement, then shuts the stream.
fn write_loop(inner: &Inner) {
    let core = &*inner.core;
    loop {
        let mut state = lock(core);
        let next = loop {
            if let Some(outgoing) = state.out.pop() {
                state.out_bytes -= outgoing.frame.size();
                state.out_lanes[lane_index(outgoing.lane)] -= 1;
                break Some(outgoing);
            }
            match state.ending {
                Some(_) => break None,
                None => {
                    state = core
                        .out_ready
                        .wait(state)
                        .unwrap_or_else(PoisonError::into_inner);
                }
            }
        };
        let ending = state.ending;
        let waker = state.waker.clone();
        drop(state);
        match next {
            Some(outgoing) => {
                if let Err(error) =
                    inner
                        .end
                        .send(outgoing.frame, wire_lane(outgoing.lane), outgoing.key)
                {
                    let reason = match error.closed() {
                        Some(Closed::Stream(kind)) => CloseReason::Stream(kind),
                        Some(Closed::Protocol(fault)) => {
                            CloseReason::Protocol(ProtocolFault::Frame(fault))
                        }
                        _ => CloseReason::PeerGone,
                    };
                    core.finish(reason, Ending::Abort);
                    return;
                }
                // A lane has room again.
                core.wake(waker);
            }
            None => {
                if let Some(Ending::Flush(code)) = ending {
                    // The peer may already be gone; the announcement is best effort.
                    let _ = inner.end.send(
                        wire::close_frame(code),
                        gwz_session_contract::Lane::Control,
                        FrameKey::ordinary(gwz_session_contract::Target::new(0)),
                    );
                    core.linger();
                }
                inner.end.close();
                (core.abort)();
                return;
            }
        }
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
                Ok(())
            }
            Err(Admission::Closed(_, reason)) => {
                Err(SendRefused::Closed(Box::new(message), reason))
            }
            Err(Admission::Full(_)) => Err(SendRefused::Full(Box::new(message))),
        }
    }

    fn try_recv(&self) -> Result<Option<Arrival>, CarrierClosed> {
        let item = {
            let mut state = lock(self.core());
            self.core().take(&mut state)?
        };
        match item {
            None => Ok(None),
            Some(item) => self.arrival(item).map(Some),
        }
    }

    fn register(&self, waker: &Waker) {
        lock(self.core()).waker = Some(waker.clone());
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

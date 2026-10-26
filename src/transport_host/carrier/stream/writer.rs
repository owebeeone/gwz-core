//! The writing side of the stream carrier: admission to the writer's queue, the writer thread, the watchdog that bounds
//! a peer that has stopped taking bytes, and how the end finishes.
//!
//! **A peer that stops listening is not the end of what it sent.** A write that meets the far side's departure (a broken
//! pipe, a reset) stops this end's writing and leaves its reading alone: the reader goes on to the stream's end, what
//! the peer sent before is delivered, and the one reason reported is the peer's announcement if one arrived, else
//! `PeerGone`. Only a local close or abort, or a failure of this end's own, drops what is queued inbound.
//!
//! **A peer that stops reading is bounded.** The writer's stream is counted: every byte the stream accepts is progress.
//! The watchdog thread ends the carrier as `Stalled`, and shuts the stream so that the blocked write returns, when
//! frames are queued (or one is mid-write) and no byte has been accepted for [`StallBounds::no_progress`]. A closing
//! end flushes for at most [`StallBounds::close`] and then shuts the stream anyway, so `close` and `Drop` cannot leave
//! the threads and the descriptor behind a peer that never reads.
use super::super::{CarrierClosed, CloseReason, Lane, ProtocolFault, wire};
use super::{
    Admission, Core, Inner, closed, lane_index, lock,
    state::{Ending, Outgoing, Scope, State, Taken, close_deadline},
    wire_lane,
};
use gwz_session_channel::peer_side_failure;
use gwz_session_contract::{Closed, Frame, FrameKey, FrameSink, Lane as WireLane, Target};
use std::{
    io::{self, Write},
    sync::{
        Arc, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

/// How long an end that has announced its close waits for its peer to answer by closing.
const LINGER: Duration = Duration::from_secs(2);

/// The most bytes the writer's queue holds, a single larger frame aside.
pub(crate) const QUEUE_BYTES: usize = 8 * 1024 * 1024;

/// The instant a byte was last accepted by the stream, as milliseconds from the carrier's start.
pub(super) struct Progress {
    start: Instant,
    last_ms: AtomicU64,
}

impl Progress {
    pub(super) fn new() -> Self {
        Self {
            start: Instant::now(),
            last_ms: AtomicU64::new(0),
        }
    }

    pub(super) fn touch(&self) {
        let ms = u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.last_ms.fetch_max(ms, Ordering::AcqRel);
    }

    /// How long since a byte was last accepted (or the writer last became busy).
    pub(super) fn idle(&self) -> Duration {
        let now = u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX);
        Duration::from_millis(now.saturating_sub(self.last_ms.load(Ordering::Acquire)))
    }
}

/// The stream's writer, counting what it accepts.
pub(super) struct Counting<W> {
    pub(super) inner: W,
    pub(super) progress: Arc<Progress>,
}

impl<W: Write> Write for Counting<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(bytes)?;
        if written > 0 {
            self.progress.touch();
        }
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl Core {
    /// Queues `frame` for the writer on `lane`, or says why not. The lane's bound is its own: the control reserve is
    /// never used by data.
    pub(super) fn admit(
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
        if state.out_lanes[lane_index(lane)] >= self.limits.capacity(lane)
            || !state.fits_out(bytes, QUEUE_BYTES)
        {
            return Err(Admission::Full(frame));
        }
        if !state.busy() {
            // The stall bound counts from the moment there is work, not from the last byte of an earlier burst.
            self.progress.touch();
        }
        state.out_bytes += bytes;
        state.out_lanes[lane_index(lane)] += 1;
        let tag = frame.tag;
        state.out.push(Outgoing { frame, lane, key }, tag, key);
        Ok(())
    }

    /// Ends this end for `reason` unless it has ended, and says how the writer is to finish.
    pub(super) fn finish(&self, reason: CloseReason, ending: Ending) {
        let mut state = lock(self);
        if state.closed.is_some() {
            return;
        }
        state.closed = Some(closed(reason));
        if !matches!(state.ending, Some(Ending::Abort)) {
            state.ending = Some(ending.clone());
        }
        let wakers = state.wakers(Scope::Any);
        state.clear_inbound();
        match ending {
            Ending::Flush(_) => {
                state.close_deadline = Some(close_deadline(self.limits.stall()));
            }
            Ending::Refuse(_) => {
                state.close_deadline = Some(close_deadline(self.limits.stall()));
                state.clear_outbound();
            }
            Ending::Abort => state.clear_outbound(),
        }
        drop(state);
        self.out_ready.notify_all();
        self.in_ready.notify_all();
        self.watch_ready.notify_all();
        for waker in wakers {
            waker.wake();
        }
        if matches!(ending, Ending::Abort) {
            (self.abort)();
        }
    }

    /// After the announcement: waits, up to [`LINGER`], for the peer to close its end in answer (the adapter's reader
    /// then reports the end), so that the announcement is read before this end's handle goes. A wedged peer does not
    /// hold the writer longer than that, and a closing end's deadline cuts it short.
    pub(super) fn linger(&self) {
        let mut deadline = Instant::now() + LINGER;
        let mut state = lock(self);
        if let Some(close) = state.close_deadline {
            deadline = deadline.min(close);
        }
        while !state.reader_done && state.peer_ended.is_none() {
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

    /// The peer has ended: what it sent is still delivered, then `reason`. `last` says that everything it will send
    /// has arrived (its announcement, or the end of its stream); a write that merely broke is not last, since the
    /// peer may still have bytes in flight to this end. An announcement stands over the stream's end as the reason.
    pub(super) fn peer_ended(&self, reason: CloseReason, last: bool) {
        let mut state = lock(self);
        if state.closed.is_some() || state.peer_final {
            return;
        }
        let first = state.peer_ended.is_none();
        let announcement = matches!(reason, CloseReason::Peer(_));
        if first || announcement {
            state.peer_ended = Some(reason);
        }
        if announcement {
            state.peer_announced = true;
        }
        if last {
            state.peer_final = true;
        }
        if first {
            // Nothing more goes to a peer that has ended: the writer stops.
            state.ending.get_or_insert(Ending::Abort);
            state.clear_outbound();
        }
        let wakers = state.wakers(Scope::Any);
        drop(state);
        self.out_ready.notify_all();
        self.in_ready.notify_all();
        self.watch_ready.notify_all();
        for waker in wakers {
            waker.wake();
        }
    }

    /// The next inbound item of `scope`, or why there will be none. After the peer's end, what it sent still comes
    /// out, class by class; a class that has nothing left learns the closure, and the end is sealed once nothing is
    /// queued anywhere.
    pub(super) fn take(
        &self,
        state: &mut State,
        scope: Scope<'_>,
    ) -> Result<Option<Taken>, CarrierClosed> {
        if let Some(reason) = state.closed {
            return Err(reason);
        }
        if let Some(item) = state.pop(scope) {
            return Ok(Some(item));
        }
        match state.peer_ended {
            Some(reason) if state.peer_final => {
                if state.inn.is_empty() {
                    state.closed = Some(closed(reason));
                    self.in_ready.notify_all();
                    self.out_ready.notify_all();
                }
                Err(closed(reason))
            }
            _ => Ok(None),
        }
    }
}

/// Writes frames until the carrier ends, then the announcement, then shuts the stream.
pub(super) fn write_loop(inner: &Inner) {
    let core = &*inner.core;
    loop {
        let mut state = lock(core);
        let next = loop {
            if let Some(outgoing) = state.out.pop() {
                state.out_bytes -= outgoing.frame.size();
                state.out_lanes[lane_index(outgoing.lane)] -= 1;
                state.writing = true;
                break Some(outgoing);
            }
            // An ending that waits for the reader (the peer stopped listening but may still be sending) is not yet
            // the end of this thread's work.
            if state.ending.is_some() && !state.awaiting_reader() {
                break None;
            }
            state = core
                .out_ready
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        };
        let ending = state.ending.clone();
        let peer_gone = state.peer_ended.is_some();
        let waker = state.waker.clone();
        drop(state);
        match next {
            Some(outgoing) => {
                let sent = inner
                    .end
                    .send(outgoing.frame, wire_lane(outgoing.lane), outgoing.key);
                lock(core).writing = false;
                if let Err(error) = sent {
                    match error.closed() {
                        // The far side stopped listening: this end stops writing and reads on to its end.
                        Some(Closed::Stream(kind)) if peer_side_failure(kind) => {
                            core.peer_ended(CloseReason::PeerGone, false);
                        }
                        other => {
                            let reason = match other {
                                Some(Closed::Stream(kind)) => CloseReason::Stream(kind),
                                Some(Closed::Protocol(fault)) => {
                                    CloseReason::Protocol(ProtocolFault::Frame(fault))
                                }
                                _ => CloseReason::PeerGone,
                            };
                            core.finish(reason, Ending::Abort);
                            finish_thread(inner);
                            return;
                        }
                    }
                    continue;
                }
                // A lane has room again.
                core.wake(waker);
            }
            None => {
                match ending {
                    Some(Ending::Flush(code)) if !peer_gone => {
                        // The peer may already be gone; the announcement is best effort.
                        let _ = inner.end.send(
                            wire::close_frame(code),
                            WireLane::Control,
                            FrameKey::ordinary(Target::new(0)),
                        );
                        core.linger();
                    }
                    Some(Ending::Refuse(frame)) => {
                        let _ = inner.end.send(
                            frame,
                            WireLane::Control,
                            FrameKey::ordinary(Target::new(0)),
                        );
                        core.linger();
                    }
                    _ => {}
                }
                finish_thread(inner);
                return;
            }
        }
    }
}

/// The writer thread's last act: the stream is shut and the watchdog released.
fn finish_thread(inner: &Inner) {
    let core = &*inner.core;
    inner.end.close();
    (core.abort)();
    let mut state = lock(core);
    state.writer_done = true;
    drop(state);
    core.watch_ready.notify_all();
    core.in_ready.notify_all();
}

/// Ends the carrier when the peer stops taking bytes, and shuts a closing end that cannot flush.
pub(super) fn watch_loop(core: &Core) {
    let bounds = core.limits.stall();
    let mut state = lock(core);
    loop {
        if state.writer_done {
            return;
        }
        let now = Instant::now();
        // A closing end has the close bound; any other has the no-progress bound.
        let wait = match (&state.ending, state.close_deadline) {
            (Some(Ending::Flush(_) | Ending::Refuse(_)), Some(deadline)) => {
                match deadline.checked_duration_since(now) {
                    Some(left) => Some(left),
                    None => {
                        drop(state);
                        // The reason that stands is the close; the stream is shut so the blocked write returns.
                        (core.abort)();
                        state = lock(core);
                        // Keep watching: shutting a stream cannot be undone, and the writer returns on its own.
                        state = core
                            .watch_ready
                            .wait_timeout(state, Duration::from_millis(50))
                            .unwrap_or_else(PoisonError::into_inner)
                            .0;
                        continue;
                    }
                }
            }
            _ if state.busy() && state.ending.is_none() => {
                let idle = core.progress.idle();
                match bounds.no_progress.checked_sub(idle) {
                    Some(left) if !left.is_zero() => Some(left),
                    _ => {
                        drop(state);
                        core.finish(CloseReason::Stalled, Ending::Abort);
                        state = lock(core);
                        continue;
                    }
                }
            }
            _ => None,
        };
        state = match wait {
            Some(left) => {
                core.watch_ready
                    .wait_timeout(state, left)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0
            }
            None => core
                .watch_ready
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner),
        };
    }
}

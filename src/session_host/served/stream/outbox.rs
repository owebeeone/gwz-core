//! A served session's channel end over its stream carrier: the replies byte pressure holds, and the pacer that sends
//! them (plan step 2.3g1, remediation F9; `GwzCoreSessionDesign.md` §3 "Volume is bounded" and D-N, §5.1, §8;
//! `GwzSplitCommandChannel.md` §3.3).
//!
//! The session host sends replies from its reading, admission, events, worker and close threads, inline, and §5.1
//! needs the reading thread never to block: it handles `operation.cancel`, `session.close` and log reads. So no sender
//! waits for the carrier's writer here. A frame the writer's queue has no bytes for is held, in order, and one pacer
//! thread of the session sends the held frames as the writer takes frames
//! ([`StreamCarrier::send_frame_paced`], bounded by the carrier's stall watchdog).
//!
//! | Rule | Why |
//! | --- | --- |
//! | While any frame is held, an ordinary frame is held behind it | per-target order, and the close report after every frame before it (2.0 §3.3 "Close report") |
//! | A control frame with no held frame of its own target goes to the carrier at once, on the control lane, whose frame and byte reserves data never uses; only past those reserves is it held, and then the pacer sends it ahead of held ordinary frames, behind any earlier held frame of its own target | D-N: a cancel's reply does not wait behind spool replies |
//! | A lane's held frames and its frames queued for the writer stay within the lane's frame count (one more while the pacer hands a frame over); a frame past that, counted by the outbox or refused by the carrier on the ahead path, is the session host's `Full`, and the channel and carrier close at once, without draining what is held, so no call after the overrun is routed | §3: a reply takes its call's lane, so only a client past its own bounds meets a full lane; what is held is bounded by the session's legal volume |
//! | The channel's close with frames held closes the carrier once the pacer has queued the last of them | §8 step 9, after the close report |
//! | The carrier's end, met by the pacer's send, drops what is held and refuses later frames `Closed`; the channel's close ends the pacer once nothing is held, and every ending of the session reaches it | §8 channel closure, step 9 |
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use gwz_session_contract::{Class, Closed, Frame, FrameKey, FrameSink, Lane, SendError, Target};

use crate::transport_host::carrier::{
    Carrier, CarrierClosed, CloseCode, CloseReason, FrameRefused, Lane as CarrierLane, Offered,
    StreamCarrier,
};

/// The channel's view of a carrier's end.
fn channel_closed(closed: CarrierClosed) -> Closed {
    match closed.reason {
        CloseReason::Local(_) => Closed::Local,
        CloseReason::Peer(_) | CloseReason::PeerGone => Closed::Peer,
        CloseReason::Stream(kind) => Closed::Stream(kind),
        CloseReason::Stalled => Closed::Stream(std::io::ErrorKind::TimedOut),
        CloseReason::Protocol(_) => Closed::Stream(std::io::ErrorKind::InvalidData),
    }
}

fn index(lane: CarrierLane) -> usize {
    match lane {
        CarrierLane::Data => 0,
        CarrierLane::Control => 1,
    }
}

/// A frame waiting for the writer's room.
struct Held {
    frame: Frame,
    lane: CarrierLane,
    key: FrameKey,
}

#[derive(Default)]
struct Waiting {
    frames: VecDeque<Held>,
    /// The target of the frame the pacer is handing to the carrier: still ahead of every later frame, and still
    /// counted on its lane until the carrier has it.
    in_hand: Option<Target>,
    /// Frames held per lane, the one in hand included.
    counts: [usize; 2],
    /// The session host closed the channel: the pacer closes the carrier after the last held frame.
    closing: bool,
    /// The channel has ended: nothing is held, and every later frame is refused with this.
    ended: Option<Closed>,
}

impl Waiting {
    fn holds_any(&self) -> bool {
        self.in_hand.is_some() || !self.frames.is_empty()
    }

    fn holds(&self, target: Target) -> bool {
        self.in_hand == Some(target) || self.frames.iter().any(|held| held.key.target == target)
    }

    /// Takes the frame the pacer sends next: the oldest held control frame that no earlier held frame of its own
    /// target blocks, ahead of held ordinary frames (the writer's own rule, `GwzSplitCommandChannel.md` §3.3: a reply
    /// the control room had no bytes for does not wait behind replies of other targets); else the oldest.
    fn take_next(&mut self) -> Option<Held> {
        let ahead = self.frames.iter().enumerate().position(|(at, held)| {
            held.key.class == Class::Control
                && !self
                    .frames
                    .iter()
                    .take(at)
                    .any(|earlier| earlier.key.target == held.key.target)
        });
        match ahead {
            Some(at) => self.frames.remove(at),
            None => self.frames.pop_front(),
        }
    }

    /// Ends the channel: drops what is held. The frame in hand stays counted until the pacer is back.
    fn end(&mut self, closed: Closed) {
        if self.ended.is_none() {
            self.ended = Some(closed);
        }
        for held in self.frames.drain(..) {
            self.counts[index(held.lane)] -= 1;
        }
    }
}

/// The session's channel end: the session host's `FrameSink` over the stream carrier, with its held replies.
pub(super) struct Outbox {
    carrier: Arc<StreamCarrier>,
    waiting: Mutex<Waiting>,
    /// The pacer waits here for a held frame, the close or the end.
    ready: Condvar,
}

impl Outbox {
    pub(super) fn new(carrier: Arc<StreamCarrier>) -> Self {
        Self {
            carrier,
            waiting: Mutex::new(Waiting::default()),
            ready: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Waiting> {
        self.waiting.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Hands `frame` to the carrier, or holds it for the pacer; never waits for the writer. Holds the outbox's lock
    /// across the carrier's own (which never waits), so two senders' frames keep one order.
    fn offer(&self, frame: Frame, lane: CarrierLane, key: FrameKey) -> Result<(), SendError> {
        let mut waiting = self.lock();
        if let Some(closed) = waiting.ended {
            return Err(SendError::Closed(frame, closed));
        }
        if waiting.closing {
            return Err(SendError::Closed(frame, Closed::Local));
        }
        let ahead =
            !waiting.holds_any() || (key.class == Class::Control && !waiting.holds(key.target));
        let frame = if ahead {
            match self.carrier.offer_frame(frame, lane, key) {
                Ok(Offered::Queued) => return Ok(()),
                Ok(Offered::Pressed(frame)) => frame,
                Err(FrameRefused::Full(frame)) => return self.overrun(waiting, frame),
                Err(FrameRefused::Closed(frame, closed)) => {
                    return Err(SendError::Closed(frame, channel_closed(closed)));
                }
            }
        } else {
            frame
        };
        // A client within its bounds has at most its lane's frame count of replies unread, and every held or queued
        // frame is one of them; the frame in the pacer's hand may be counted twice, once queued. So past the count
        // (`held > free` before this frame) the client has exceeded its own bounds: the session host's full lane.
        if waiting.counts[index(lane)] > self.carrier.free_frames(lane) {
            return self.overrun(waiting, frame);
        }
        waiting.counts[index(lane)] += 1;
        waiting.frames.push_back(Held { frame, lane, key });
        drop(waiting);
        self.ready.notify_all();
        Ok(())
    }

    /// A frame past a lane's frame count, from either source (the outbox's own count of held frames, or the carrier's
    /// refusal of a frame sent ahead): the client exceeded its own bounds. The session ends now, as the session host's
    /// close of a full lane does anywhere: the carrier closes at once, without draining what is held, so the serving
    /// thread meets its end and cancels what runs, and no call after the overrun is routed (a close that waited for the
    /// drain would keep the session running). Returns the session host's `Full`.
    fn overrun(&self, mut waiting: MutexGuard<'_, Waiting>, frame: Frame) -> Result<(), SendError> {
        waiting.end(Closed::Local);
        drop(waiting);
        self.ready.notify_all();
        self.carrier.close(CloseCode::Normal);
        Err(SendError::Full(frame))
    }

    /// The pacer: sends held frames as the writer takes frames, until the channel ends. Runs on a thread of its own,
    /// the carrier's only sender that waits for its writer.
    pub(super) fn pace(&self) {
        loop {
            let held = {
                let mut waiting = self.lock();
                loop {
                    if waiting.ended.is_some() {
                        return;
                    }
                    if let Some(held) = waiting.take_next() {
                        waiting.in_hand = Some(held.key.target);
                        break held;
                    }
                    if waiting.closing {
                        waiting.end(Closed::Local);
                        drop(waiting);
                        self.carrier.close(CloseCode::Normal);
                        return;
                    }
                    waiting = self
                        .ready
                        .wait(waiting)
                        .unwrap_or_else(PoisonError::into_inner);
                }
            };
            let lane = held.lane;
            let sent = self.carrier.send_frame_paced(held.frame, lane, held.key);
            let mut waiting = self.lock();
            waiting.in_hand = None;
            waiting.counts[index(lane)] -= 1;
            match sent {
                Ok(()) => {}
                Err(FrameRefused::Full(_)) => {
                    // The client exceeded its own bounds: as the session host does with a full lane, the channel
                    // closes.
                    waiting.end(Closed::Local);
                    drop(waiting);
                    self.carrier.close(CloseCode::Normal);
                    return;
                }
                Err(FrameRefused::Closed(_, closed)) => {
                    waiting.end(channel_closed(closed));
                    return;
                }
            }
        }
    }
}

impl FrameSink for Outbox {
    fn send(&self, frame: Frame, lane: Lane, key: FrameKey) -> Result<(), SendError> {
        let lane = match lane {
            Lane::Control => CarrierLane::Control,
            Lane::Call => CarrierLane::Data,
        };
        self.offer(frame, lane, key)
    }

    /// §8 step 9: the carrier closes in order, after every frame sent before; at once when nothing is held.
    fn close(&self) {
        let mut waiting = self.lock();
        waiting.closing = true;
        let now = !waiting.holds_any();
        if now {
            waiting.end(Closed::Local);
        }
        drop(waiting);
        self.ready.notify_all();
        if now {
            self.carrier.close(CloseCode::Normal);
        }
    }
}

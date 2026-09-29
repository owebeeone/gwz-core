//! The in-process adapter (§3): two bounded queues of frames inside one
//! process, one for each direction.

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use gwz_session_contract::{
    Carrier, Closed, Frame, FrameError, FrameSink, FrameSource, Lane, Limits, SendError,
};

/// A session's in-process channel: its client end and its host end.
///
/// Each direction has its own queue, which holds `capacity`'s outstanding
/// calls on the call lane and its control reserve on the control lane (§3).
/// The queues are bounded by counters and start empty: nothing is
/// pre-allocated by a limit.
pub fn pair(capacity: Limits) -> (InProcessEnd, InProcessEnd) {
    let shared = Arc::new(Shared {
        capacity,
        state: Mutex::new(State::default()),
        arrived: [Condvar::new(), Condvar::new()],
    });
    let client = InProcessEnd {
        shared: Arc::clone(&shared),
        side: Side::Client,
    };
    let host = InProcessEnd {
        shared,
        side: Side::Host,
    };
    (client, host)
}

/// One end of an in-process channel: the client's or the host's. The two
/// behave alike.
///
/// - `send` never waits. A full lane refuses the frame with
///   [`SendError::Full`] and gives it back; the channel stays open. A frame
///   holds its lane's room until the peer's `recv` takes it.
/// - The carrier is [`Carrier::InProcess`]: a frame under a tag other than 1
///   to 3, or over 64 MiB, ends the session at both ends, and both report
///   the [`Closed::Protocol`] error.
/// - `recv` waits until a frame arrives or the channel has ended. A close at
///   either end wakes every waiting `recv`.
/// - After the peer ends the channel, this end still receives what the peer
///   sent before, then the closure. The end that closed, or met the protocol
///   error, discards what it had not yet received.
///
/// An end is `Send` and `Sync`: one thread may wait in `recv` while others
/// send. Dropping an end closes it.
pub struct InProcessEnd {
    shared: Arc<Shared>,
    side: Side,
}

struct Shared {
    capacity: Limits,
    state: Mutex<State>,
    /// Signalled when a frame arrives for a side, or the channel ends.
    arrived: [Condvar; 2],
}

#[derive(Default)]
struct State {
    /// The frames waiting for each side, by [`Side::index`].
    inboxes: [Inbox; 2],
    /// The first ending, which both ends then report.
    ended: Option<Ending>,
    /// Whether each side has stopped receiving: it closed, or ended the
    /// session with a protocol error.
    stopped: [bool; 2],
}

/// Frames in transit to one side, with each lane's count.
#[derive(Default)]
struct Inbox {
    frames: VecDeque<(Frame, Lane)>,
    calls: usize,
    controls: usize,
}

impl Inbox {
    fn count(&mut self, lane: Lane) -> &mut usize {
        match lane {
            Lane::Call => &mut self.calls,
            Lane::Control => &mut self.controls,
        }
    }
}

#[derive(Clone, Copy)]
struct Ending {
    by: Side,
    protocol: Option<FrameError>,
}

impl Ending {
    /// The reason `side` reports.
    fn seen_by(self, side: Side) -> Closed {
        match self.protocol {
            Some(error) => Closed::Protocol(error),
            None if self.by == side => Closed::Local,
            None => Closed::Peer,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Side {
    Client,
    Host,
}

impl Side {
    fn index(self) -> usize {
        match self {
            Side::Client => 0,
            Side::Host => 1,
        }
    }

    fn peer(self) -> Side {
        match self {
            Side::Client => Side::Host,
            Side::Host => Side::Client,
        }
    }
}

impl InProcessEnd {
    fn lock(&self) -> MutexGuard<'_, State> {
        // Nothing panics while holding the lock, so the state stays whole.
        self.shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Ends the channel if nothing has yet; this side stops receiving either
    /// way. Returns the reason this side reports.
    fn stop(&self, state: &mut State, protocol: Option<FrameError>) -> Closed {
        let ending = *state.ended.get_or_insert(Ending {
            by: self.side,
            protocol,
        });
        state.stopped[self.side.index()] = true;
        // Nothing this side has not received will be delivered now.
        state.inboxes[self.side.index()] = Inbox::default();
        for arrived in &self.shared.arrived {
            arrived.notify_all();
        }
        ending.seen_by(self.side)
    }
}

impl FrameSink for InProcessEnd {
    fn send(&self, frame: Frame, lane: Lane) -> Result<(), SendError> {
        let mut state = self.lock();
        if let Some(ending) = state.ended {
            return Err(SendError::Closed(frame, ending.seen_by(self.side)));
        }
        if let Err(error) = frame.check(Carrier::InProcess) {
            let closed = self.stop(&mut state, Some(error));
            return Err(SendError::Closed(frame, closed));
        }
        let peer = self.side.peer().index();
        let inbox = &mut state.inboxes[peer];
        let count = inbox.count(lane);
        if *count >= self.shared.capacity.lane_capacity(lane) {
            return Err(SendError::Full(frame));
        }
        *count += 1;
        inbox.frames.push_back((frame, lane));
        drop(state);
        self.shared.arrived[peer].notify_one();
        Ok(())
    }

    fn close(&self) {
        let mut state = self.lock();
        self.stop(&mut state, None);
    }
}

impl FrameSource for InProcessEnd {
    fn recv(&self) -> Result<Frame, Closed> {
        let side = self.side.index();
        let mut state = self.lock();
        loop {
            let ended = state.ended;
            if !state.stopped[side] {
                let inbox = &mut state.inboxes[side];
                if let Some((frame, lane)) = inbox.frames.pop_front() {
                    *inbox.count(lane) -= 1;
                    return Ok(frame);
                }
            }
            if let Some(ending) = ended {
                return Err(ending.seen_by(self.side));
            }
            state = self.shared.arrived[side]
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

impl Drop for InProcessEnd {
    fn drop(&mut self) {
        self.close();
    }
}

impl fmt::Debug for InProcessEnd {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InProcessEnd")
            .field("side", &self.side)
            .field("capacity", &self.shared.capacity)
            .finish_non_exhaustive()
    }
}

mod tests;

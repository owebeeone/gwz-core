//! The default carrier: both ends in one process, typed messages handed from one queue to the other with nothing
//! encoded. This is what `LocalLink` pumped between its two ports, with the bound and the reserve of the seam.
use super::{
    Arrival, Attachment, Carrier, CarrierBinding, CarrierClosed, CarrierId, CarrierLimits,
    CloseCode, CloseReason, Lane, Peer, SendRefused, Side, queue::LaneQueue,
};
use std::{
    io,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    task::Waker,
};

/// Both ends' state under one lock. Index 0 is the core's end and 1 the client's; `queues[i]` holds what end `i` sent.
struct Shared {
    queues: [LaneQueue<Attachment>; 2],
    closed: [Option<CloseCode>; 2],
    wakers: [Option<Waker>; 2],
}

impl Shared {
    fn wakers_of(&self, end: usize) -> Option<Waker> {
        self.wakers[end].clone()
    }
}

/// One end of an in-process carrier.
pub(crate) struct InProcessCarrier {
    shared: Arc<Mutex<Shared>>,
    me: usize,
    binding: CarrierBinding,
}

/// A connected pair: the core's end and the client's end.
pub(crate) fn in_process(
    limits: CarrierLimits,
) -> io::Result<(InProcessCarrier, InProcessCarrier)> {
    let id = CarrierId::random()?;
    let shared = Arc::new(Mutex::new(Shared {
        queues: [LaneQueue::new(limits), LaneQueue::new(limits)],
        closed: [None, None],
        wakers: [None, None],
    }));
    let end = |me, side| InProcessCarrier {
        shared: shared.clone(),
        me,
        binding: CarrierBinding::new(id, side, Peer::InProcess),
    };
    Ok((end(0, Side::Core), end(1, Side::Client)))
}

fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

fn closed(reason: CloseReason) -> CarrierClosed {
    CarrierClosed { reason }
}

impl InProcessCarrier {
    /// Why this end can do nothing more, if it can't: its own close first, else the peer's.
    fn ended(&self, state: &Shared) -> Option<CarrierClosed> {
        match (state.closed[self.me], state.closed[1 - self.me]) {
            (Some(code), _) => Some(closed(CloseReason::Local(code))),
            (None, Some(code)) => Some(closed(CloseReason::Peer(code))),
            (None, None) => None,
        }
    }
}

impl Carrier for InProcessCarrier {
    fn try_send(&self, message: Attachment) -> Result<(), SendRefused> {
        let lane = Lane::of(message.1.kind);
        let mut state = lock(&self.shared);
        if let Some(reason) = self.ended(&state) {
            return Err(SendRefused::Closed(Box::new(message), reason));
        }
        state.queues[self.me]
            .push(lane, message)
            .map_err(|message| SendRefused::Full(Box::new(message)))?;
        let waker = state.wakers_of(1 - self.me);
        drop(state);
        if let Some(waker) = waker {
            waker.wake();
        }
        Ok(())
    }

    fn try_recv(&self) -> Result<Option<Arrival>, CarrierClosed> {
        let mut state = lock(&self.shared);
        // This end's own close ends its receiving at once; the peer's close lets what it sent first arrive.
        if let Some(code) = state.closed[self.me] {
            return Err(closed(CloseReason::Local(code)));
        }
        let popped = state.queues[1 - self.me].pop();
        match popped {
            Some((_, message)) => {
                // The peer's lane has room again.
                let waker = state.wakers_of(1 - self.me);
                drop(state);
                if let Some(waker) = waker {
                    waker.wake();
                }
                Ok(Some(Arrival {
                    binding: self.binding.clone(),
                    message,
                }))
            }
            None => match self.ended(&state) {
                Some(reason) => Err(reason),
                None => Ok(None),
            },
        }
    }

    fn register(&self, waker: &Waker) {
        lock(&self.shared).wakers[self.me] = Some(waker.clone());
    }

    fn close(&self, code: CloseCode) {
        let mut state = lock(&self.shared);
        if state.closed[self.me].is_some() {
            return;
        }
        state.closed[self.me] = Some(code);
        // What the peer sent and this end never took is dropped with the end; what this end sent stays for the peer.
        state.queues[1 - self.me].clear();
        let waker = state.wakers_of(1 - self.me);
        drop(state);
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    fn binding(&self) -> CarrierBinding {
        self.binding.clone()
    }
}

impl Drop for InProcessCarrier {
    fn drop(&mut self) {
        self.close(CloseCode::Dropped);
    }
}

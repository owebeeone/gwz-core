//! The bounded queue of an in-process carrier's direction: one FIFO with a capacity per lane. (A byte stream's writer
//! takes its next frame by `session-channel`'s `WriteQueue` rule instead.)
use super::{CarrierLimits, Lane};
use std::collections::VecDeque;

/// A FIFO whose items count against the capacity of their lane.
pub(super) struct LaneQueue<T> {
    items: VecDeque<(Lane, T)>,
    data: usize,
    control: usize,
    limits: CarrierLimits,
}

impl<T> LaneQueue<T> {
    pub(super) fn new(limits: CarrierLimits) -> Self {
        Self {
            items: VecDeque::new(),
            data: 0,
            control: 0,
            limits,
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub(super) fn len(&self, lane: Lane) -> usize {
        match lane {
            Lane::Data => self.data,
            Lane::Control => self.control,
        }
    }

    pub(super) fn has_room(&self, lane: Lane) -> bool {
        self.len(lane) < self.limits.capacity(lane)
    }

    /// Queues `item` on `lane`, or gives it back when the lane is full. The other lane's fullness does not matter.
    pub(super) fn push(&mut self, lane: Lane, item: T) -> Result<(), T> {
        if !self.has_room(lane) {
            return Err(item);
        }
        match lane {
            Lane::Data => self.data += 1,
            Lane::Control => self.control += 1,
        }
        self.items.push_back((lane, item));
        Ok(())
    }

    /// Queues `item` past the lane's capacity: for the one close announcement an end owes its peer.
    pub(super) fn push_reserved(&mut self, lane: Lane, item: T) {
        match lane {
            Lane::Data => self.data += 1,
            Lane::Control => self.control += 1,
        }
        self.items.push_back((lane, item));
    }

    pub(super) fn pop(&mut self) -> Option<(Lane, T)> {
        let popped = self.items.pop_front();
        if let Some((lane, _)) = &popped {
            match lane {
                Lane::Data => self.data -= 1,
                Lane::Control => self.control -= 1,
            }
        }
        popped
    }

    pub(super) fn clear(&mut self) {
        self.items.clear();
        self.data = 0;
        self.control = 0;
    }
}

//! The byte-stream writer's order (operator decision D-N).
//!
//! A byte stream carries one frame at a time, so frames wait for the writer. The writer picks the next frame at each
//! frame boundary by one rule: **the oldest control frame goes ahead of the ordinary frames before it, except the
//! ordinary frames of its own target and any `SessionCall`, which it never overtakes.** Everything else leaves in the
//! order it arrived. So a `Cancel`, a `Window` or a terminal frame is not held behind megabytes of an in-band spool of
//! another call or stream, while the order the protocol depends on is kept: a stream's frames in the order sent, a
//! call's frames in the order sent, and the calls in increasing call-ID order.
//!
//! The channel decodes no body. Core states each frame's [`FrameKey`] when it encodes the frame: its
//! [`Class`](gwz_session_contract::Class) (control or ordinary) and an opaque [`Target`](gwz_session_contract::Target)
//! derived from the `call_id` (tags 1 to 3) or the `(request_id, session_id, stream_id)` (tags 16 and 17), which this
//! queue compares for equality only. The only other thing it looks at is the tag byte: a frame under tag 1 is a
//! `SessionCall`. Core keys the close report ordinary although it travels on the control lane, so the writer writes it
//! after every frame queued before it.
//!
//! The queue is plain data: the byte-stream adapter uses it to take turns among concurrent senders, and a carrier that
//! queues frames ahead of its own writer thread uses it for that queue. Control frames are bounded by the control
//! reserve of whoever queues them, so no ordinary frame waits forever.
use gwz_session_contract::{Class, FrameKey};
use std::collections::VecDeque;

const SESSION_CALL_TAG: u8 = 1;

struct Entry<T> {
    item: T,
    tag: u8,
    key: FrameKey,
}

impl<T> Entry<T> {
    fn control(&self) -> bool {
        self.key.class == Class::Control
    }
}

/// Frames waiting for the writer, in arrival order, taken out by the rule above.
pub struct WriteQueue<T> {
    entries: VecDeque<Entry<T>>,
}

impl<T> Default for WriteQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> WriteQueue<T> {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::new(),
        }
    }

    /// Queues `item`, a frame under `tag`, keyed by `key`.
    pub fn push(&mut self, item: T, tag: u8, key: FrameKey) {
        self.entries.push_back(Entry { item, tag, key });
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Where the next frame sits: the oldest control frame if nothing before it must stay ahead of it, else the
    /// oldest frame, which nothing precedes.
    fn next_index(&self) -> Option<usize> {
        if self.entries.is_empty() {
            return None;
        }
        let Some((index, control)) = self
            .entries
            .iter()
            .enumerate()
            .find(|(_, entry)| entry.control())
        else {
            return Some(0);
        };
        let blocked = self.entries.iter().take(index).any(|before| {
            before.tag == SESSION_CALL_TAG || before.key.target == control.key.target
        });
        Some(if blocked { 0 } else { index })
    }

    /// The frame the writer takes next, without taking it.
    pub fn peek(&self) -> Option<&T> {
        self.entries
            .get(self.next_index()?)
            .map(|entry| &entry.item)
    }

    /// Takes the frame the writer writes next.
    pub fn pop(&mut self) -> Option<T> {
        let index = self.next_index()?;
        self.entries.remove(index).map(|entry| entry.item)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

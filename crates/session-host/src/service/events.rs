//! An operation's event log and the held reads of `events.subscribe` (§4.2,
//! §5.4, §6).
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | A ring of at most `event_log` records; on overflow the oldest records are dropped and one reset event stands for them, ahead of the later ones | §5.4 |
//! | Records are numbered from 1; a read returns the records after its cursor, within its record and byte limits, and at least one record when one is there | §4.2 |
//! | A read with nothing to return is held, without a thread, until an append, the log's completion, its wait or the session's end | §4.2, §5.1 |
//! | An append is short and takes no table lock: it wakes the read thread through its own signal | §5.6; gate module |

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{EventsBatch, EventsRead, HostPorts, Inner, ReadSignal};
use crate::lock;

/// One operation's event log.
pub(crate) struct EventLog {
    ring: Mutex<Ring>,
    signal: Arc<ReadSignal>,
}

struct Ring {
    capacity: usize,
    next_seq: u64,
    entries: VecDeque<Entry>,
    reset: Vec<u8>,
    complete: bool,
}

struct Entry {
    seq: u64,
    payload: Vec<u8>,
    reset: bool,
}

impl EventLog {
    /// A log of at most `capacity` records, at least 2, whose dropped history
    /// `reset` stands for.
    pub(crate) fn new(capacity: usize, reset: Vec<u8>, signal: Arc<ReadSignal>) -> Self {
        Self {
            ring: Mutex::new(Ring {
                capacity: capacity.max(2),
                next_seq: 1,
                entries: VecDeque::new(),
                reset,
                complete: false,
            }),
            signal,
        }
    }

    /// Appends a record and wakes held reads.
    pub(crate) fn push(&self, payload: Vec<u8>) {
        {
            let mut ring = lock(&self.ring);
            if ring.complete {
                return;
            }
            let seq = ring.next_seq;
            ring.next_seq += 1;
            ring.entries.push_back(Entry {
                seq,
                payload,
                reset: false,
            });
            if ring.entries.len() > ring.capacity {
                let mut last = 0;
                if ring.entries.front().is_some_and(|entry| entry.reset)
                    && let Some(entry) = ring.entries.pop_front()
                {
                    last = entry.seq;
                }
                while ring.entries.len() > ring.capacity - 1 {
                    match ring.entries.pop_front() {
                        Some(entry) => {
                            last = entry.seq;
                        }
                        None => {
                            break;
                        }
                    }
                }
                let reset = ring.reset.clone();
                ring.entries.push_front(Entry {
                    seq: last,
                    payload: reset,
                    reset: true,
                });
            }
        }
        self.signal.notify();
    }

    /// Marks the log complete: its operation is terminal.
    pub(crate) fn complete(&self) {
        lock(&self.ring).complete = true;
        self.signal.notify();
    }

    /// The records after `read.cursor`, within its limits and `bytes`.
    pub(crate) fn read(&self, read: &EventsRead, bytes: usize) -> EventsBatch {
        let ring = lock(&self.ring);
        let after = read.cursor.unwrap_or(0);
        let max_records = read.max_records.unwrap_or(usize::MAX).max(1);
        let max_bytes = read.max_bytes.unwrap_or(bytes).min(bytes);
        let mut records = Vec::new();
        let mut used = 0usize;
        let mut next_cursor = after;
        for entry in ring.entries.iter().filter(|entry| entry.seq > after) {
            let size = entry.payload.len();
            let over = records.len() >= max_records || used.saturating_add(size) > max_bytes;
            if over && !records.is_empty() {
                break;
            }
            used = used.saturating_add(size);
            next_cursor = entry.seq;
            records.push((entry.seq, entry.payload.clone()));
            if over {
                break;
            }
        }
        let rest = ring.entries.iter().any(|entry| entry.seq > next_cursor);
        EventsBatch {
            records,
            next_cursor,
            complete: ring.complete && !rest,
        }
    }
}

/// A read held until it has something to return.
pub(crate) struct Held {
    pub(crate) call_id: u64,
    pub(crate) read: EventsRead,
    pub(crate) log: Arc<EventLog>,
    pub(crate) deadline: Instant,
}

/// The read thread: answers each held read once it has records, its log is
/// complete, its wait has elapsed or the session is closing, and ends with
/// the session.
pub(crate) fn read_loop<P: HostPorts>(inner: &Arc<Inner<P>>) {
    loop {
        let (outgoing, next, ended) = {
            let mut state = lock(&inner.state);
            state.unflushed += 1;
            let now = Instant::now();
            let closing = state.closing || state.ended;
            let mut outgoing = Vec::new();
            let mut kept = Vec::new();
            for held in std::mem::take(&mut state.held) {
                let batch = held.log.read(&held.read, inner.limits.read_bytes);
                let ready = !batch.records.is_empty() || batch.complete;
                if ready || closing || now >= held.deadline {
                    let body = inner.ports.events(&held.read, &batch);
                    state.answered(held.call_id);
                    outgoing.push(inner.reply(
                        held.call_id,
                        gwz_session_contract::Lane::Call,
                        body,
                    ));
                } else {
                    kept.push(held);
                }
            }
            let next = kept.iter().map(|held| held.deadline).min();
            state.held = kept;
            (outgoing, next, state.ended)
        };
        inner.flush(outgoing);
        if ended {
            return;
        }
        let wait = next.map_or(Duration::from_secs(1), |deadline| {
            deadline.saturating_duration_since(Instant::now())
        });
        inner.reads.wait(wait.min(Duration::from_secs(1)));
    }
}

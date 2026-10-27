//! What a stream carrier end keeps under its one lock: the writer's queue, the inbound record of what the peer sent,
//! and the per-class and per-binding indexes over that record (convergence plan 2.1, remediation round 1).
//!
//! **The inbound record is one arrival-ordered map, taken three ways.** [`State::pop`] takes the oldest entry of the
//! whole end (an end with one consumer), the oldest registry frame (tags 1 to 8), or the oldest frame of one binding.
//! An entry's charge stays against its class or binding until it is taken by its own consumer, so a consumer that
//! is not ready holds back only the entries it owns, and a window's worth of one binding queued and untaken never
//! keeps a registry frame or another binding's frame from being taken.
use super::super::{
    BindingEnd, BindingFault, BindingLimits, CarrierClosed, CloseCode, CloseReason, Lane,
    StallBounds,
};
use gwz_session_channel::WriteQueue;
use gwz_session_contract::{Frame, FrameKey};
use gwz_transport::protocol::MessageKind;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    task::Waker,
    time::Instant,
};

/// How the writer is to finish when the carrier ends.
#[derive(Clone)]
pub(super) enum Ending {
    /// Write what is queued, then the announcement of the code, then shut the stream.
    Flush(CloseCode),
    /// Write this one frame (a refusal the peer is owed), then shut the stream; queued frames are dropped.
    Refuse(Frame),
    /// Shut the stream now; queued frames are dropped.
    Abort,
}

/// A frame queued for the writer.
pub(super) struct Outgoing {
    pub(super) frame: Frame,
    pub(super) lane: Lane,
    pub(super) key: FrameKey,
}

/// One queued inbound entry, in arrival order.
pub(super) enum Entry {
    Registry(Frame),
    Message(Queued),
}

pub(super) struct Queued {
    pub(super) binding: String,
    pub(super) stream: i64,
    pub(super) kind: MessageKind,
    /// What the entry holds against its binding's data bound: a data frame's payload (at least one byte), else 0.
    pub(super) charge: usize,
    pub(super) frame: Frame,
    /// A queued window's offset, if it may still be coalesced into.
    pub(super) window: Option<i64>,
}

#[derive(Default)]
pub(super) struct StreamInbound {
    pub(super) data_bytes: usize,
    pub(super) window_seq: Option<u64>,
    /// Queued frames of each non-data, non-window kind, by the kind's wire number.
    pub(super) kinds: [u8; 24],
    pub(super) queued: usize,
}

pub(super) struct BindingInbound {
    pub(super) limits: BindingLimits,
    pub(super) fault: Option<BindingFault>,
    pub(super) streams: HashMap<i64, StreamInbound>,
    pub(super) seqs: BTreeSet<u64>,
    pub(super) bytes: usize,
    pub(super) waker: Option<Waker>,
}

/// The registry class (tags 1 to 8): the session's calls, replies and handshake frames.
#[derive(Default)]
pub(super) struct RegistryInbound {
    pub(super) seqs: BTreeSet<u64>,
    pub(super) bytes: usize,
    pub(super) waker: Option<Waker>,
}

/// What a consumer takes: the whole end's next entry, the registry's, or one binding's.
#[derive(Clone, Copy)]
pub(super) enum Scope<'a> {
    Any,
    Registry,
    Binding(&'a str),
}

/// What the consumer takes next: a registry frame, or a transport frame still encoded (it is decoded as it is taken).
pub(super) enum Taken {
    Registry(Frame),
    Message(Frame),
}

pub(super) struct State {
    pub(super) out: WriteQueue<Outgoing>,
    pub(super) out_lanes: [usize; 2],
    pub(super) out_bytes: usize,
    /// The writer is inside a send.
    pub(super) writing: bool,
    pub(super) inn: BTreeMap<u64, Entry>,
    pub(super) next_seq: u64,
    pub(super) registry: RegistryInbound,
    pub(super) bindings: HashMap<String, BindingInbound>,
    pub(super) discarded: usize,
    /// Why this end can do nothing more.
    pub(super) closed: Option<CarrierClosed>,
    /// Why the peer ended, once it has: delivered after the inbound queue drains. An announcement is the reason that
    /// stands over the stream's end.
    pub(super) peer_ended: Option<CloseReason>,
    /// Everything the peer will ever send has arrived: its announcement, or the end of its stream. Until then a
    /// peer that merely stopped listening (a write that broke) has not ended its side.
    pub(super) peer_final: bool,
    /// The peer's close announcement arrived; whatever follows it is discarded.
    pub(super) peer_announced: bool,
    pub(super) ending: Option<Ending>,
    /// When a closing end gives up on flushing.
    pub(super) close_deadline: Option<Instant>,
    /// The adapter has reported the end of its session: its reader has returned.
    pub(super) reader_done: bool,
    /// The adapter's reader thread has returned (it dropped the reading half).
    pub(super) reader_exited: bool,
    /// The writer thread has returned.
    pub(super) writer_done: bool,
    pub(super) waker: Option<Waker>,
}

impl State {
    pub(super) fn new() -> Self {
        Self {
            out: WriteQueue::new(),
            out_lanes: [0; 2],
            out_bytes: 0,
            writing: false,
            inn: BTreeMap::new(),
            next_seq: 0,
            registry: RegistryInbound::default(),
            bindings: HashMap::new(),
            discarded: 0,
            closed: None,
            peer_ended: None,
            peer_final: false,
            peer_announced: false,
            ending: None,
            close_deadline: None,
            reader_done: false,
            reader_exited: false,
            writer_done: false,
            waker: None,
        }
    }

    /// Why nothing more can be sent: this end's end, or the peer's.
    pub(super) fn gone(&self) -> Option<CarrierClosed> {
        self.closed
            .or(self.peer_ended.map(|reason| CarrierClosed { reason }))
    }

    /// The peer has stopped listening but may still be sending: the writer is done, the reader is not.
    pub(super) fn awaiting_reader(&self) -> bool {
        self.closed.is_none() && self.peer_ended.is_some() && !self.peer_final
    }

    pub(super) fn fits_out(&self, bytes: usize, limit: usize) -> bool {
        self.out.is_empty() || self.out_bytes + bytes <= limit
    }

    /// Whether frames wait for the writer or one is being written.
    pub(super) fn busy(&self) -> bool {
        self.writing || !self.out.is_empty()
    }

    /// Drops a binding's queued frames and its record. Returns the binding's waker, for the caller to wake once the
    /// lock is released: its consumer learns that the binding is gone.
    pub(super) fn remove_binding(&mut self, id: &str) -> Option<Waker> {
        let binding = self.bindings.remove(id)?;
        self.drop_entries(&binding);
        binding.waker
    }

    /// Why `id` has nothing more to give while the carrier goes on: it was faulted, or it is not open.
    pub(super) fn binding_end(&self, id: &str) -> Option<BindingEnd> {
        match self.bindings.get(id) {
            None => Some(BindingEnd::NotOpen),
            Some(binding) => binding.fault.map(BindingEnd::Faulted),
        }
    }

    fn drop_entries(&mut self, binding: &BindingInbound) {
        for seq in &binding.seqs {
            self.inn.remove(seq);
        }
    }

    /// Faults a binding: its queued frames are dropped, its record stays so that its later frames are counted and
    /// discarded, and nothing else changes. Returns the binding's waker, for the caller to wake once the lock is
    /// released.
    pub(super) fn fault_binding(&mut self, id: &str, fault: BindingFault) -> Option<Waker> {
        let mut binding = self.bindings.remove(id)?;
        self.drop_entries(&binding);
        binding.seqs.clear();
        binding.streams.clear();
        binding.bytes = 0;
        binding.fault = Some(fault);
        let waker = binding.waker.clone();
        self.bindings.insert(id.to_owned(), binding);
        waker
    }

    /// The oldest entry of `scope`, taken out with its charge settled.
    pub(super) fn pop(&mut self, scope: Scope<'_>) -> Option<Taken> {
        let seq = match scope {
            Scope::Any => *self.inn.first_key_value()?.0,
            Scope::Registry => *self.registry.seqs.first()?,
            Scope::Binding(id) => *self.bindings.get(id)?.seqs.first()?,
        };
        Some(match self.inn.remove(&seq)? {
            Entry::Registry(frame) => {
                self.registry.seqs.remove(&seq);
                self.registry.bytes -= frame.size();
                Taken::Registry(frame)
            }
            Entry::Message(queued) => {
                self.release(seq, &queued);
                Taken::Message(queued.frame)
            }
        })
    }

    /// Settles what taking `entry` (queued under `seq`) leaves behind in its binding's state.
    fn release(&mut self, seq: u64, entry: &Queued) {
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

    /// The wakers a new entry of `scope` should wake: the end's own and the scope's.
    pub(super) fn wakers(&self, scope: Scope<'_>) -> Vec<Waker> {
        let mut wakers: Vec<Waker> = self.waker.iter().cloned().collect();
        match scope {
            Scope::Any => {
                wakers.extend(self.registry.waker.iter().cloned());
                wakers.extend(self.bindings.values().filter_map(|b| b.waker.clone()));
            }
            Scope::Registry => wakers.extend(self.registry.waker.iter().cloned()),
            Scope::Binding(id) => {
                wakers.extend(self.bindings.get(id).and_then(|b| b.waker.clone()));
            }
        }
        wakers
    }

    /// Resets the end's inbound record for a final close: everything queued is dropped, wakers stay.
    pub(super) fn clear_inbound(&mut self) {
        self.inn.clear();
        self.registry.seqs.clear();
        self.registry.bytes = 0;
        self.bindings.clear();
    }

    pub(super) fn clear_outbound(&mut self) {
        self.out.clear();
        self.out_bytes = 0;
        self.out_lanes = [0; 2];
    }
}

pub(super) fn kind_slot(kind: MessageKind) -> usize {
    usize::try_from(kind.wire()).unwrap_or(0).min(23)
}

/// The stall bounds' close deadline for an ending that flushes.
pub(super) fn close_deadline(bounds: StallBounds) -> Instant {
    Instant::now() + bounds.close
}

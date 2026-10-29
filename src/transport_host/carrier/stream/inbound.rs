//! The reading side of the stream carrier: the [`FrameRouter`] the byte-stream adapter hands every frame to, and the
//! admission of each frame to its class or binding.
//!
//! `route` never blocks and never waits for a consumer. A registry frame (tags 1 to 8) joins the registry class,
//! bounded by the session's call limit and control reserve and by the registry's own byte cap. A transport frame is
//! attributed to its binding by a shallow walk of its envelope (the `session_id`), before the transport's own full
//! decode: a frame for a binding that is not open (or was faulted) is discarded with no state created, even if it
//! would not decode, and a frame that fails the full decode faults the binding it names and no other. Only a frame
//! that cannot be attributed (the request-name framing, a body that is not a map) ends the session.
use super::super::{
    BindingFault, BindingLimits, CloseReason, FrameClass, ProtocolFault,
    wire::{self, CONTROL_TAG, DATA_TAG, INVALID_REQUEST, MAX_TRANSPORT_FRAME},
};
use super::{
    CarrierRouter, Core, lock,
    state::{BindingInbound, Entry, Queued, Scope, State, kind_slot},
    wipe,
};
use gwz_session_contract::{Closed, Frame, FrameRouter};
use gwz_transport::protocol::{Envelope, MessageKind};
use std::{io::ErrorKind, sync::MutexGuard, task::Waker};

/// What the peer is told when its `SessionOpen` is refused. Fixed text: nothing the peer sent is repeated.
const SESSION_OPEN_REFUSED: &str = "the SessionOpen is not of the client placement's shape";

impl Core {
    /// Wakes the consumers of `scope` (and the end's own waker) once the lock is released.
    pub(super) fn notify(&self, state: MutexGuard<'_, State>, scope: Scope<'_>) {
        let wakers = state.wakers(scope);
        drop(state);
        self.in_ready.notify_all();
        wake_all(wakers);
    }

    /// The session ends here for a protocol fault of its own: the carrier closes, and the adapter's session with it.
    pub(super) fn session_fault(&self, fault: ProtocolFault) -> Closed {
        self.finish(CloseReason::Protocol(fault), super::state::Ending::Abort);
        Closed::Stream(ErrorKind::InvalidData)
    }

    /// Discards a frame that is not delivered: counted, and its bytes wiped.
    fn discard(&self, mut state: MutexGuard<'_, State>, frame: Frame) {
        state.discarded += 1;
        drop(state);
        wipe(frame);
    }

    /// Routes a registry frame: the registry class, bounded by the call limit, the control reserve and the byte cap.
    fn route_registry(&self, frame: Frame) -> Result<(), Closed> {
        let bytes = frame.size();
        let mut state = lock(self);
        if state.closed.is_some() {
            drop(state);
            wipe(frame);
            return Ok(());
        }
        if state.peer_announced {
            {
                self.discard(state, frame);
                return Ok(());
            }
        }
        let limits = self.limits.inbound();
        let over = state.registry.seqs.len() >= limits.registry_frames
            || (!state.registry.seqs.is_empty()
                && state.registry.bytes + bytes > limits.registry_bytes);
        if over {
            drop(state);
            wipe(frame);
            return Err(self.session_fault(ProtocolFault::Overrun(FrameClass::Registry)));
        }
        let seq = state.next_seq;
        state.next_seq += 1;
        state.registry.seqs.insert(seq);
        state.registry.bytes += bytes;
        state.inn.insert(seq, Entry::Registry(frame));
        self.notify(state, Scope::Registry);
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
        // Attribution first, from the envelope's `session_id` alone.
        let attributed = wire::split_lane(&frame).and_then(|parts| {
            wire::envelope_session_id(parts.encoded)
                .map(|id| id.map(|id| (id.to_owned(), parts.encoded.len())))
        });
        let (binding_id, _) = match attributed {
            Ok(Some(found)) => found,
            // Nothing says which binding it belongs to: discarded with no state.
            Ok(None) => {
                let state = lock(self);
                {
                    self.discard(state, frame);
                    return Ok(());
                }
            }
            Err(fault) => {
                wipe(frame);
                return Err(self.session_fault(fault));
            }
        };
        // An end that accepts binds admits the peer's `bind` for a binding it has not opened (and no other frame of one):
        // the client end of a served session, whose bindings the core mints (GwzSplitCommandChannel 3.3).
        let may_bind = {
            let state = lock(self);
            state.accept_binds.is_some()
                && !state.peer_announced
                && frame.tag == CONTROL_TAG
                && !state.bindings.contains_key(&binding_id)
        };
        let is_bind = may_bind
            && decode_envelope(&frame).is_ok_and(|envelope| {
                envelope.kind == MessageKind::Bind && envelope.stream_id == 0
            });
        {
            let mut state = lock(self);
            if state.closed.is_some() {
                drop(state);
                wipe(frame);
                return Ok(());
            }
            if state.peer_announced || !live(&state, &binding_id) {
                let opened = is_bind
                    && !state.peer_announced
                    && state.accept_binds.is_some_and(|limits| {
                        state
                            .open_binding(&binding_id, limits, self.limits.inbound().max_bindings)
                            .is_ok()
                    });
                if !opened {
                    // Not open, or faulted, or the peer has said it is done: discarded, and no state is created for it.
                    self.discard(state, frame);
                    return Ok(());
                }
            }
        }
        let envelope = match decode_envelope(&frame) {
            Ok(envelope) => envelope,
            Err(_) => {
                // The binding is live and its frame does not decode: that binding's fault, not the session's.
                let mut state = lock(self);
                let waker = state.fault_binding(&binding_id, BindingFault::Invalid);
                drop(state);
                self.in_ready.notify_all();
                wake_all(waker.into_iter().collect());
                wipe(frame);
                return Ok(());
            }
        };
        let bytes = frame.size();
        let mut state = lock(self);
        if state.closed.is_some() {
            drop(state);
            wipe(frame);
            return Ok(());
        }
        if !live(&state, &binding_id) {
            {
                self.discard(state, frame);
                return Ok(());
            }
        }
        let seq = state.next_seq;
        let verdict = {
            let binding = state.bindings.get_mut(&binding_id).expect("a live binding");
            admit_transport(binding, &envelope, seq, frame.tag)
        };
        match verdict {
            Verdict::Fault(fault) => {
                let waker = state.fault_binding(&binding_id, fault);
                drop(state);
                self.in_ready.notify_all();
                wake_all(waker.into_iter().collect());
                wipe(frame);
                return Ok(());
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
                    if let Some(binding) = state.bindings.get_mut(&binding_id) {
                        binding.bytes = binding.bytes - old_size + new_size;
                    }
                } else {
                    wipe(frame);
                }
            }
            Verdict::Queue { charge, window } => {
                state.next_seq += 1;
                if let Some(binding) = state.bindings.get_mut(&binding_id) {
                    binding.bytes += bytes;
                    binding.seqs.insert(seq);
                }
                state.inn.insert(
                    seq,
                    Entry::Message(Queued {
                        binding: binding_id.clone(),
                        stream: envelope.stream_id,
                        kind: envelope.kind,
                        charge,
                        frame,
                        window,
                    }),
                );
            }
        }
        self.notify(state, Scope::Binding(&binding_id));
        Ok(())
    }

    /// Refuses a received `SessionOpen`: the frame is not delivered, the peer is sent exactly one `SessionError`
    /// (`invalid_request`, call ID 0, fixed text) and the stream is shut. The router does not return an error, so the
    /// adapter's reader goes on until the writer has written the answer and shut the stream.
    fn refuse_session_open(&self, refusal: wire::SessionOpenRefusal, frame: Frame) {
        wipe(frame);
        self.finish(
            CloseReason::Protocol(ProtocolFault::SessionOpen(refusal)),
            super::state::Ending::Refuse(wire::session_error_frame(
                INVALID_REQUEST,
                SESSION_OPEN_REFUSED,
            )),
        );
    }
}

fn wake_all(wakers: Vec<Waker>) {
    for waker in wakers {
        waker.wake();
    }
}

/// Whether `id` names an open binding that has not been faulted.
fn live(state: &State, id: &str) -> bool {
    state.bindings.get(id).is_some_and(|b| b.fault.is_none())
}

fn decode_envelope(frame: &Frame) -> Result<Envelope, ProtocolFault> {
    let parts = wire::split_lane(frame)?;
    wire::decode_envelope(parts.encoded)
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
    envelope: &Envelope,
    seq: u64,
    tag: u8,
) -> Verdict {
    // A kind under the other class's tag breaks the binding's rules.
    if wire::tag_of(super::super::Lane::of(envelope.kind)) != tag {
        return Verdict::Fault(BindingFault::Invalid);
    }
    let class = if tag == DATA_TAG {
        FrameClass::TransportData
    } else {
        FrameClass::TransportControl
    };
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
                    Ok(code) => core.peer_ended(CloseReason::Peer(code), true),
                    Err(fault) => return Err(core.session_fault(fault)),
                }
                Ok(())
            }
            DATA_TAG | CONTROL_TAG => core.route_transport(frame),
            4 => match wire::validate_session_open(&frame.body) {
                Ok(()) => core.route_registry(frame),
                Err(refusal) => {
                    core.refuse_session_open(refusal, frame);
                    Ok(())
                }
            },
            _ => core.route_registry(frame),
        }
    }

    fn ended(&self, reason: Closed) {
        let core = &self.0;
        match reason {
            Closed::Local => {}
            Closed::Peer => core.peer_ended(CloseReason::PeerGone, true),
            // The far side going away mid-stream is the peer's departure, not this end's trouble.
            Closed::Stream(kind) if gwz_session_channel::peer_side_failure(kind) => {
                core.peer_ended(CloseReason::PeerGone, true);
            }
            Closed::Stream(kind) => {
                core.finish(CloseReason::Stream(kind), super::state::Ending::Abort)
            }
            Closed::Protocol(fault) => core.finish(
                CloseReason::Protocol(ProtocolFault::Frame(fault)),
                super::state::Ending::Abort,
            ),
        }
        let mut state = lock(core);
        state.reader_done = true;
        drop(state);
        core.reader_gone.notify_all();
        core.out_ready.notify_all();
        core.in_ready.notify_all();
    }
}

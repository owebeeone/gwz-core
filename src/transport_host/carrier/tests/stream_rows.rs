//! The stream carrier over OS pipes: the rows that need no socket and so run the same everywhere. The local
//! carrier's rows, over the real socket and pipe, are in `local_rows`.
use super::super::{
    BindingFault, BindingLimits, Carrier, CarrierClosed, CarrierLimits, CloseCode, CloseReason,
    FrameClass, FrameRefused, InboundLimits, Lane, ProtocolFault, SendRefused, SessionOpenRefusal,
    StreamCarrier,
};
use super::{support::*, wire_rows::*};
use crate::transport_host::Attachment;
use gwz_session_contract::{Frame, FrameError, FrameKey, Target};
use gwz_transport::protocol::MessageKind;
use std::{thread, time::Duration};

const SHORT: Duration = Duration::from_millis(50);

/// The key the rows state for registry frames they do not order.
const ZERO_KEY: FrameKey = FrameKey::ordinary(Target::new(0));

#[test]
fn a_frame_set_round_trips_between_two_threads() {
    let (core, client) = over_pipes(limits(64, 16));
    let sent_messages = [
        data(1, 0, vec![1; 70_000 / 2]),
        window(1, 9),
        data(1, 35_000, vec![2]),
        close(1, 35_001),
    ];
    let registry = [
        Frame {
            tag: 1,
            body: b"call".to_vec(),
        },
        Frame {
            tag: 2,
            body: vec![0; 3000],
        },
        Frame {
            tag: 4,
            body: session_open_without_environment(),
        },
    ];
    let expected_messages = sent_messages.clone();
    let expected_registry = registry.clone();
    let sender = thread::spawn(move || {
        for frame in registry {
            core.send_frame(frame, Lane::Data, ZERO_KEY).unwrap();
        }
        for message in sent_messages {
            core.try_send(message).unwrap();
        }
        core
    });
    // The registry frames come first, then the messages, as sent.
    for expected in &expected_registry {
        let frame = eventually("a frame", || client.recv_frame_timeout(SHORT).unwrap());
        assert_eq!(&frame, expected);
    }
    for expected in &expected_messages {
        let arrival = eventually("a message", || client.recv_timeout(SHORT).unwrap());
        assert_eq!(&arrival.message, expected);
        assert!(client.binding().admits(&arrival));
    }
    let _core = sender.join().unwrap();
}

#[test]
fn a_dropped_end_gives_the_peer_the_dropped_code_and_not_a_timeout() {
    let (core, client) = over_pipes(limits(8, 4));
    core.try_send(window(1, 1)).unwrap();
    drop(core);
    // What was sent before the drop arrives, then the reason.
    assert_eq!(tag(&next(&client)), tag(&window(1, 1)));
    let closed = eventually("the closure", || client.try_recv().err());
    assert_eq!(closed.reason, CloseReason::Peer(CloseCode::Dropped));
    assert_eq!(client.try_recv().unwrap_err(), closed);
}

#[test]
fn an_explicit_close_gives_the_peer_its_code_after_the_queued_frames_and_this_end_its_own() {
    let (core, client) = over_pipes(limits(8, 4));
    for offset in 0..3 {
        core.try_send(data(1, offset, vec![0; 100])).unwrap();
    }
    core.close(CloseCode::Cancelled);
    assert_eq!(
        core.try_recv().unwrap_err(),
        CarrierClosed {
            reason: CloseReason::Local(CloseCode::Cancelled)
        }
    );
    assert!(matches!(
        core.try_send(window(1, 1)),
        Err(SendRefused::Closed(..))
    ));
    for offset in 0..3 {
        assert_eq!(next(&client).1.data.unwrap().offset, offset);
    }
    let closed = eventually("the closure", || client.try_recv().err());
    assert_eq!(closed.reason, CloseReason::Peer(CloseCode::Cancelled));
}

#[test]
fn a_stream_that_ends_without_an_announcement_reads_as_the_peer_gone() {
    let (carrier, from_carrier, to_carrier) = against_raw(limits(8, 4));
    drop(to_carrier);
    drop(from_carrier);
    let closed = eventually("the closure", || carrier.try_recv().err());
    assert_eq!(closed.reason, CloseReason::PeerGone);
}

#[test]
fn an_oversized_length_is_refused_before_anything_is_allocated_or_read() {
    for prefix in [u32::MAX, 64 * 1024 * 1024 + 1, 0] {
        let (carrier, _from_carrier, mut to_carrier) = against_raw(limits(8, 4));
        // Only the four prefix bytes ever arrive: a carrier that tried to read the body would wait for it, and one that
        // allocated for it would have done so by now. The refusal comes with the stream still open.
        write_all(&mut to_carrier, &prefix.to_le_bytes());
        let closed = eventually("the refusal", || carrier.try_recv().err());
        let CloseReason::Protocol(ProtocolFault::Frame(fault)) = closed.reason else {
            panic!("expected a framing fault for {prefix}: {closed}");
        };
        match prefix {
            0 => assert_eq!(fault, FrameError::Empty),
            _ => assert!(matches!(fault, FrameError::TooLarge { .. }), "{fault:?}"),
        }
    }
}

#[test]
fn a_transport_frame_over_the_transports_own_frame_size_ends_the_carrier() {
    let (carrier, _from_carrier, mut to_carrier) = against_raw(limits(8, 4));
    // 300 KiB under the data tag: far under the contract's 64 MiB, over the transport's 128 KiB frame. The router
    // refuses it by its size, before it decodes a byte of it.
    let mut bytes = (300u32 * 1024).to_le_bytes().to_vec();
    bytes.push(16);
    bytes.extend(vec![0u8; 300 * 1024 - 1]);
    write_all(&mut to_carrier, &bytes);
    let closed = eventually("the refusal", || carrier.try_recv().err());
    assert!(matches!(
        closed.reason,
        CloseReason::Protocol(ProtocolFault::Frame(FrameError::TooLarge { .. }))
    ));
}

#[test]
fn a_tag_the_carrier_does_not_carry_ends_it() {
    let (carrier, _from_carrier, mut to_carrier) = against_raw(limits(8, 4));
    write_all(&mut to_carrier, &wire_frame(20, b"x"));
    let closed = eventually("the refusal", || carrier.try_recv().err());
    assert_eq!(
        closed.reason,
        CloseReason::Protocol(ProtocolFault::Frame(FrameError::ReservedTag(20)))
    );
}

#[test]
fn a_session_open_with_an_environment_is_refused_on_send_and_the_canary_never_crosses() {
    let (carrier, from_carrier, _to_carrier) = against_raw(limits(8, 4));
    let refused = carrier
        .send_frame(
            Frame {
                tag: 4,
                body: session_open_with_environment(),
            },
            Lane::Data,
            ZERO_KEY,
        )
        .unwrap_err();
    let FrameRefused::Closed(frame, closed) = refused else {
        panic!("a session open with an environment must end the carrier");
    };
    assert_eq!(frame.tag, 4);
    assert_eq!(
        closed.reason,
        CloseReason::Protocol(ProtocolFault::SessionOpen(
            SessionOpenRefusal::ForbiddenField(3)
        ))
    );
    // Whatever else crossed, the canary did not: read the wire until the carrier has shut it.
    let crossed = read_to_end(from_carrier);
    assert!(
        !crossed.windows(CANARY.len()).any(|window| window == CANARY),
        "the canary crossed the carrier"
    );
    assert!(!crossed.windows(8).any(|window| window == b"GH_TOKEN"));
}

#[test]
fn a_session_open_with_an_environment_from_a_raw_peer_is_refused_and_not_delivered() {
    let (carrier, _from_carrier, mut to_carrier) = against_raw(limits(8, 4));
    // A peer that does not use this carrier's sender: the receiving side refuses by itself.
    write_all(
        &mut to_carrier,
        &wire_frame(4, &session_open_with_environment()),
    );
    let closed = eventually("the refusal", || carrier.recv_frame_timeout(SHORT).err());
    assert_eq!(
        closed.reason,
        CloseReason::Protocol(ProtocolFault::SessionOpen(
            SessionOpenRefusal::ForbiddenField(3)
        ))
    );
    assert_eq!(carrier.recv_frame_timeout(SHORT).unwrap_err(), closed);
}

/// Fills a carrier's writer: large data frames on stream 2 until the data lane answers `Full`. The far end reads
/// nothing, so the pipe fills, the writer blocks inside one frame, and the rest wait in the queue.
fn saturate(carrier: &StreamCarrier) -> usize {
    let mut queued = 0usize;
    loop {
        match carrier.try_send(data(2, (queued * 60_000) as i64, vec![0; 60_000])) {
            Ok(()) => queued += 1,
            Err(SendRefused::Full(back)) => {
                assert_eq!(back.1.kind, MessageKind::Data);
                return queued;
            }
            Err(other) => panic!("{other:?}"),
        }
        assert!(queued < 1000, "an unread flood must hit a bound");
    }
}

#[test]
fn control_is_accepted_while_the_data_lane_is_full() {
    let (carrier, _from_carrier, _to_carrier) = against_raw(limits(4, 4));
    let queued = saturate(&carrier);
    assert!(queued >= 4);
    carrier.try_send(window(1, 1)).unwrap();
    carrier.try_send(cancel(1)).unwrap();
}

/// D-N. The writer is blocked inside a frame with a queue of large data frames of stream 2 behind it; a cancel of
/// stream 1 is accepted and is the next frame the writer takes, not the last.
#[test]
fn cancel_under_saturated_data_progresses() {
    let (carrier, mut from_carrier, _to_carrier) = against_raw(limits(8, 4));
    let queued = saturate(&carrier);
    carrier.try_send(cancel(1)).unwrap();
    let kinds: Vec<_> = (0..=queued)
        .map(|_| read_message(&mut from_carrier).1.kind)
        .collect();
    let position = kinds
        .iter()
        .position(|kind| *kind == MessageKind::Cancel)
        .unwrap();
    // The writer had taken at most two frames (one blocked in the pipe, one the pipe held) when the cancel arrived.
    assert!(
        position <= 2,
        "the cancel was frame {position} of {}",
        kinds.len()
    );
    assert!(
        kinds[..position]
            .iter()
            .all(|kind| *kind == MessageKind::Data)
    );
    assert_eq!(
        kinds
            .iter()
            .filter(|kind| **kind == MessageKind::Data)
            .count(),
        queued
    );
}

#[test]
fn a_control_frame_never_overtakes_a_frame_of_its_own_stream() {
    let (carrier, mut from_carrier, _to_carrier) = against_raw(limits(16, 4));
    for offset in 0..4 {
        carrier
            .try_send(data(1, offset * 60_000, vec![0; 60_000]))
            .unwrap();
    }
    for offset in 0..4 {
        carrier
            .try_send(data(2, offset * 60_000, vec![0; 60_000]))
            .unwrap();
    }
    eventually("frames queued behind the writer", || {
        (carrier.queued_out() >= 5).then_some(())
    });
    carrier.try_send(close(1, 240_000)).unwrap();
    let wire: Vec<_> = (0..9).map(|_| read_message(&mut from_carrier)).collect();
    let at = |kind: MessageKind, stream: i64, from: usize| {
        wire.iter()
            .enumerate()
            .skip(from)
            .find(|(_, m)| m.1.kind == kind && m.1.stream_id == stream)
            .map(|(index, _)| index)
    };
    let close_at = at(MessageKind::Close, 1, 0).unwrap();
    // Every data frame of stream 1 is written before its close; stream 1's frames keep their order.
    let stream_one: Vec<_> = wire.iter().filter(|m| m.1.stream_id == 1).collect();
    assert_eq!(stream_one.last().unwrap().1.kind, MessageKind::Close);
    assert_eq!(stream_one.len(), 5);
    let last_data_one = wire
        .iter()
        .rposition(|m| m.1.stream_id == 1 && m.1.kind == MessageKind::Data)
        .unwrap();
    assert!(last_data_one < close_at);
    // And it did go ahead of some of stream 2's.
    assert!(wire[close_at + 1..].iter().any(|m| m.1.stream_id == 2));
}

#[test]
fn a_session_call_is_never_overtaken_by_a_control_frame() {
    let (carrier, mut from_carrier, _to_carrier) = against_raw(limits(16, 4));
    for offset in 0..4 {
        carrier
            .try_send(data(2, offset * 60_000, vec![0; 60_000]))
            .unwrap();
    }
    carrier
        .send_frame(
            Frame {
                tag: 1,
                body: b"call-4".to_vec(),
            },
            Lane::Data,
            FrameKey::ordinary(Target::new(4)),
        )
        .unwrap();
    eventually("frames queued behind the writer", || {
        (carrier.queued_out() >= 2).then_some(())
    });
    carrier.try_send(cancel(1)).unwrap();
    let mut call_at = None;
    let mut cancel_at = None;
    for index in 0..6 {
        let (tag, body) = read_wire_frame(&mut from_carrier);
        match tag {
            1 => call_at = Some(index),
            17 => {
                let frame = Frame { tag, body };
                assert!(decode_attachment_kind(&frame, MessageKind::Cancel));
                cancel_at = Some(index);
            }
            _ => {}
        }
    }
    assert!(
        call_at.unwrap() < cancel_at.unwrap(),
        "{call_at:?} {cancel_at:?}"
    );
}

fn decode_attachment_kind(frame: &Frame, kind: MessageKind) -> bool {
    super::super::wire::decode_attachment(frame).is_ok_and(|m| m.1.kind == kind)
}

fn small_registry() -> CarrierLimits {
    limits(8, 4).with_inbound(InboundLimits {
        registry_frames: 2,
        bytes: 4 * 1024 * 1024,
        max_bindings: 4,
    })
}

#[test]
fn a_peer_past_the_session_bound_ends_the_carrier_with_the_class_named() {
    let (carrier, _from_carrier, mut to_carrier) = against_raw(small_registry());
    // Nothing consumes: the reader takes the two the bound allows, and the third is the peer's overrun.
    for _ in 0..3 {
        write_all(&mut to_carrier, &wire_frame(2, b"reply"));
    }
    let closed = eventually("the refusal", || carrier.wait_closed(SHORT));
    assert_eq!(
        closed.reason,
        CloseReason::Protocol(ProtocolFault::Overrun(FrameClass::Registry))
    );
}

#[test]
fn the_reader_does_not_wait_for_a_consumer() {
    let (carrier, _from_carrier, mut to_carrier) = against_raw(limits(8, 4));
    // 1,000 registry frames and no consumer: the default bound is 1,088, all are taken, and they are all there
    // afterwards, in the order sent.
    for index in 0..1000u32 {
        write_all(&mut to_carrier, &wire_frame(2, &index.to_le_bytes()));
    }
    for index in 0..1000u32 {
        let frame = eventually("a frame", || carrier.recv_frame_timeout(SHORT).unwrap());
        assert_eq!(frame.body, index.to_le_bytes());
    }
}

/// The binding the rows open on both ends of a pair, and on the carrier they write to.
const BINDING: &str = "session-1";

fn envelope_in(
    session: &str,
    mut message: gwz_transport::protocol::Envelope,
) -> gwz_transport::protocol::Envelope {
    message.session_id = session.into();
    message
}

fn in_binding(session: &str, message: Attachment) -> Attachment {
    (message.0, envelope_in(session, message.1))
}

#[test]
fn a_conforming_senders_whole_receive_window_in_minimum_size_frames_does_not_fault_the_binding() {
    let window_bytes = 4096usize;
    let limits = limits(8, 4);
    let (carrier, _from_carrier, mut to_carrier) = against_raw(limits);
    carrier
        .open_binding(
            "paused",
            BindingLimits {
                receive_window: window_bytes,
                ..BindingLimits::default()
            },
        )
        .unwrap();
    // The whole window as one-byte data frames, and a window after each, while the consumer is paused.
    for offset in 0..window_bytes as i64 {
        write_all(
            &mut to_carrier,
            &wire_message(&in_binding("paused", data(1, offset, vec![9]))),
        );
        write_all(
            &mut to_carrier,
            &wire_message(&in_binding("paused", window(1, offset + 10))),
        );
    }
    // Every frame is in; nothing faulted. (A message the consumer has not asked for yet is not in question.)
    eventually("every frame routed", || {
        (carrier.discarded_frames() == 0 && carrier.queued_in() > window_bytes).then_some(())
    });
    assert_eq!(carrier.binding_fault("paused"), None);
    // The consumer then gets the window's data in order, and the stream's one window at the highest offset.
    let mut offsets = Vec::new();
    let mut window_max = None;
    for _ in 0..=window_bytes {
        let message = next(&carrier);
        match message.1.kind {
            MessageKind::Data => offsets.push(message.1.data.unwrap().offset),
            MessageKind::Window => window_max = Some(message.1.window.unwrap().max_offset),
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(offsets, (0..window_bytes as i64).collect::<Vec<_>>());
    assert_eq!(window_max, Some(window_bytes as i64 - 1 + 10));
    assert!(carrier.try_recv().unwrap().is_none());
    assert_eq!(carrier.binding_fault("paused"), None);
}

#[test]
fn the_default_receive_window_in_one_byte_frames_does_not_fault_the_binding() {
    let (carrier, _from_carrier, to_carrier) = against_raw(limits(8, 4));
    let window_bytes = BindingLimits::default().receive_window;
    carrier
        .open_binding("full", BindingLimits::default())
        .unwrap();
    let writer = std::thread::spawn(move || {
        let mut to_carrier = to_carrier;
        for offset in 0..window_bytes as i64 {
            write_all(
                &mut to_carrier,
                &wire_message(&in_binding("full", data(1, offset, vec![9]))),
            );
            write_all(
                &mut to_carrier,
                &wire_message(&in_binding("full", window(1, offset + 1))),
            );
        }
        to_carrier
    });
    let _keep = writer.join().unwrap();
    eventually("every data frame routed", || {
        (carrier.queued_in() > window_bytes).then_some(())
    });
    assert_eq!(carrier.binding_fault("full"), None);
    assert_eq!(carrier.discarded_frames(), 0);
}

#[test]
fn a_deliberate_overrun_faults_only_its_binding_while_a_second_binding_completes() {
    let (carrier, _from_carrier, mut to_carrier) = against_raw(limits(8, 4));
    let small = BindingLimits {
        receive_window: 8,
        ..BindingLimits::default()
    };
    carrier.open_binding("bad", small).unwrap();
    carrier.open_binding("good", small).unwrap();
    // `bad` sends nine one-byte frames to a window of eight, with `good` interleaved.
    for offset in 0..9 {
        write_all(
            &mut to_carrier,
            &wire_message(&in_binding("bad", data(1, offset, vec![1]))),
        );
        if offset < 8 {
            write_all(
                &mut to_carrier,
                &wire_message(&in_binding("good", data(1, offset, vec![2]))),
            );
        }
    }
    // One more for `bad` after its fault, which is discarded.
    write_all(
        &mut to_carrier,
        &wire_message(&in_binding("bad", data(1, 9, vec![1]))),
    );
    write_all(
        &mut to_carrier,
        &wire_message(&in_binding("good", close(1, 8))),
    );
    eventually("the fault", || carrier.binding_fault("bad"));
    assert_eq!(
        carrier.binding_fault("bad"),
        Some(BindingFault::Overrun(FrameClass::TransportData))
    );
    assert_eq!(carrier.binding_fault("good"), None);
    // The carrier is open, and the consumer gets `good`'s eight data frames and its close, and nothing of `bad`'s.
    let mut seen = Vec::new();
    for _ in 0..9 {
        let message = next(&carrier);
        assert_eq!(message.1.session_id, "good");
        seen.push(message.1.kind);
    }
    assert_eq!(
        seen.iter()
            .filter(|kind| **kind == MessageKind::Data)
            .count(),
        8
    );
    assert_eq!(seen.last(), Some(&MessageKind::Close));
    assert!(carrier.try_recv().unwrap().is_none());
    eventually("the late frame discarded", || {
        (carrier.discarded_frames() >= 1).then_some(())
    });
}

#[test]
fn a_kind_past_its_slots_on_a_stream_faults_the_binding() {
    let (carrier, _from_carrier, mut to_carrier) = against_raw(limits(8, 4));
    for _ in 0..5 {
        write_all(&mut to_carrier, &wire_message(&cancel(3)));
    }
    eventually("the fault", || carrier.binding_fault(BINDING));
    assert_eq!(
        carrier.binding_fault(BINDING),
        Some(BindingFault::Overrun(FrameClass::TransportControl))
    );
    assert!(
        carrier.wait_closed(Duration::from_millis(1)).is_none(),
        "the carrier goes on"
    );
}

#[test]
fn a_kind_under_the_other_classs_tag_faults_its_binding_and_not_the_session() {
    let (carrier, _from_carrier, mut to_carrier) = against_raw(limits(8, 4));
    // A data message under the control tag.
    let frame = super::super::wire::encode_attachment(&data(1, 0, vec![1])).unwrap();
    write_all(&mut to_carrier, &wire_frame(17, &frame.body));
    eventually("the fault", || carrier.binding_fault(BINDING));
    assert_eq!(carrier.binding_fault(BINDING), Some(BindingFault::Invalid));
    assert!(carrier.wait_closed(Duration::from_millis(1)).is_none());
}

#[test]
fn a_frame_for_a_binding_that_is_not_open_creates_no_state() {
    let (carrier, _from_carrier, mut to_carrier) = against_raw(limits(8, 4));
    write_all(
        &mut to_carrier,
        &wire_message(&in_binding("nobody", data(1, 0, vec![1]))),
    );
    write_all(
        &mut to_carrier,
        &wire_message(&in_binding("nobody", cancel(1))),
    );
    eventually("both discarded", || {
        (carrier.discarded_frames() == 2).then_some(())
    });
    assert_eq!(carrier.binding_fault("nobody"), None);
    assert_eq!(carrier.queued_in(), 0);
    // Opening it afterwards starts clean, and a retired binding is forgotten.
    carrier
        .open_binding("nobody", BindingLimits::default())
        .unwrap();
    write_all(
        &mut to_carrier,
        &wire_message(&in_binding("nobody", data(1, 0, vec![1]))),
    );
    eventually("queued", || (carrier.queued_in() == 1).then_some(()));
    carrier.retire_binding("nobody");
    assert_eq!(carrier.queued_in(), 0);
    assert!(carrier.try_recv().unwrap().is_none());
}

#[test]
fn a_cancel_queued_after_the_open_and_data_of_its_stream_arrives_after_them() {
    let (core, client) = over_pipes(limits(16, 8));
    core.try_send(open_stream(1)).unwrap();
    for offset in 0..3 {
        core.try_send(data(1, offset, vec![0; 100])).unwrap();
    }
    core.try_send(data(2, 0, vec![0; 100])).unwrap();
    core.try_send(cancel(1)).unwrap();
    let mut stream_one = Vec::new();
    for _ in 0..6 {
        let message = next(&client);
        if message.1.stream_id == 1 {
            stream_one.push(message.1.kind);
        }
    }
    assert_eq!(
        stream_one,
        [
            MessageKind::Open,
            MessageKind::Data,
            MessageKind::Data,
            MessageKind::Data,
            MessageKind::Cancel
        ]
    );
}

#[test]
fn frames_of_one_target_leave_in_the_order_they_were_sent() {
    let (core, client) = over_pipes(limits(16, 8));
    let sent = [
        data(1, 0, vec![1]),
        window(1, 9),
        data(1, 1, vec![2]),
        close(1, 2),
    ];
    for message in sent.clone() {
        core.try_send(message).unwrap();
    }
    for expected in &sent {
        assert_eq!(tag(&next(&client)), tag(expected));
    }
}

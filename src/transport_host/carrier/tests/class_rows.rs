//! The inbound record taken by class and by binding (remediation round 1: C-P2-1 = S-P2-6), and the attribution of a
//! transport frame to its binding before it is decoded (C-P2-2).
use super::super::{
    BindingFault, BindingLimits, Carrier, CarrierLimits, CloseCode, CloseReason, FrameClass,
    InboundLimits, ProtocolFault,
};
use super::support::*;
use gwz_transport::protocol::MessageKind;
use std::time::Duration;

const SHORT: Duration = Duration::from_millis(50);

fn small_window(bytes: usize) -> BindingLimits {
    BindingLimits {
        receive_window: bytes,
        ..BindingLimits::default()
    }
}

fn pause_until(what: &str, mut check: impl FnMut() -> bool) {
    eventually(what, || check().then_some(()));
}

#[test]
fn a_bindings_whole_window_queued_and_untaken_does_not_hold_back_the_registry_or_another_binding() {
    let (carrier, _from_carrier, mut to_carrier) = against_raw(limits(8, 4));
    carrier.open_binding("a", small_window(64)).unwrap();
    carrier.open_binding("b", small_window(64)).unwrap();
    // A's whole window, as 64 one-byte frames, is queued first; then a registry call, then one frame for B.
    for offset in 0..64 {
        write_all(&mut to_carrier, &wire_for("a", data(1, offset, vec![9])));
    }
    write_all(&mut to_carrier, &wire_frame(1, b"call"));
    write_all(&mut to_carrier, &wire_for("b", window(1, 7)));
    pause_until("every frame routed", || carrier.queued_in() == 66);

    // Both are taken now, though 64 older frames of A sit ahead of them in arrival order.
    assert_eq!(
        carrier.try_recv_registry().unwrap().map(|frame| frame.body),
        Some(b"call".to_vec())
    );
    let b = carrier.try_recv_binding("b").unwrap().expect("B's frame");
    assert_eq!(
        (b.message.1.kind, b.message.1.session_id.as_str()),
        (MessageKind::Window, "b")
    );
    // A's frames were not touched: all still queued, no fault, no discard.
    assert_eq!(carrier.queued_in(), 64);
    assert_eq!(carrier.binding_fault("a"), None);
    assert_eq!(carrier.binding_fault("b"), None);
    assert_eq!(carrier.discarded_frames(), 0);
    // A then drains in order, and only then is its window free again.
    for offset in 0..64 {
        let arrival = carrier.try_recv_binding("a").unwrap().expect("A's frame");
        assert_eq!(arrival.message.1.data.unwrap().offset, offset);
    }
    assert!(carrier.try_recv_binding("a").unwrap().is_none());
    assert_eq!(carrier.queued_in(), 0);
}

#[test]
fn a_frames_charge_stays_until_its_own_consumer_takes_it() {
    let (carrier, _from_carrier, mut to_carrier) = against_raw(limits(8, 4));
    carrier.open_binding("a", small_window(4)).unwrap();
    carrier.open_binding("b", small_window(4)).unwrap();
    for offset in 0..4 {
        write_all(&mut to_carrier, &wire_for("a", data(1, offset, vec![9])));
    }
    write_all(&mut to_carrier, &wire_for("b", window(1, 1)));
    write_all(&mut to_carrier, &wire_frame(1, b"call"));
    pause_until("routed", || carrier.queued_in() == 6);
    // Taking other consumers' frames frees nothing of A's window: one more A frame is over it.
    assert!(carrier.try_recv_binding("b").unwrap().is_some());
    assert!(carrier.try_recv_registry().unwrap().is_some());
    write_all(&mut to_carrier, &wire_for("a", data(1, 4, vec![9])));
    pause_until("the overrun", || carrier.binding_fault("a").is_some());
    assert_eq!(
        carrier.binding_fault("a"),
        Some(BindingFault::Overrun(FrameClass::TransportData))
    );
    assert_eq!(carrier.binding_fault("b"), None);
    assert!(carrier.wait_closed(Duration::from_millis(1)).is_none());

    // A consumer's own take frees its own window.
    carrier.open_binding("c", small_window(4)).unwrap();
    for offset in 0..4 {
        write_all(&mut to_carrier, &wire_for("c", data(1, offset, vec![9])));
    }
    pause_until("routed", || carrier.queued_in() == 4);
    assert!(carrier.try_recv_binding("c").unwrap().is_some());
    write_all(&mut to_carrier, &wire_for("c", data(1, 4, vec![9])));
    pause_until("queued", || carrier.queued_in() == 4);
    assert_eq!(carrier.binding_fault("c"), None);
}

#[test]
fn three_bindings_fill_their_windows_with_no_consumer_and_nothing_faults() {
    // A registry byte cap far below what the three windows hold: the cap is the registry's, not a pool the transport
    // class shares with it.
    let limits = limits(8, 4).with_inbound(InboundLimits {
        registry_frames: 8,
        registry_bytes: 16 * 1024,
        max_bindings: 8,
    });
    let (carrier, _from_carrier, mut to_carrier) = against_raw(limits);
    for id in ["a", "b", "c"] {
        carrier.open_binding(id, small_window(8 * 1024)).unwrap();
    }
    for offset in 0..8 {
        for id in ["a", "b", "c"] {
            write_all(
                &mut to_carrier,
                &wire_for(id, data(1, offset * 1024, vec![5; 1024])),
            );
        }
    }
    write_all(&mut to_carrier, &wire_frame(1, b"cancel-this-call"));
    pause_until("every frame routed", || carrier.queued_in() == 25);
    for id in ["a", "b", "c"] {
        assert_eq!(carrier.binding_fault(id), None, "{id}");
    }
    assert!(carrier.wait_closed(Duration::from_millis(1)).is_none());
    // The registry frame is still taken, ahead of 24 older transport frames.
    assert_eq!(
        carrier.try_recv_registry().unwrap().map(|frame| frame.body),
        Some(b"cancel-this-call".to_vec())
    );
    assert_eq!(carrier.queued_in(), 24);
}

#[test]
fn one_binding_over_its_window_faults_only_that_binding_and_the_registry_goes_on() {
    let (carrier, _from_carrier, mut to_carrier) = against_raw(limits(8, 4));
    carrier.open_binding("a", small_window(2)).unwrap();
    carrier.open_binding("b", small_window(2)).unwrap();
    for offset in 0..3 {
        write_all(&mut to_carrier, &wire_for("a", data(1, offset, vec![1])));
    }
    write_all(&mut to_carrier, &wire_for("b", data(1, 0, vec![2])));
    write_all(&mut to_carrier, &wire_frame(1, b"call"));
    pause_until("a's fault", || carrier.binding_fault("a").is_some());
    assert_eq!(carrier.binding_fault("b"), None);
    assert_eq!(
        carrier.try_recv_registry().unwrap().map(|frame| frame.body),
        Some(b"call".to_vec())
    );
    assert!(carrier.try_recv_binding("a").unwrap().is_none());
    assert!(carrier.try_recv_binding("b").unwrap().is_some());
}

#[test]
fn the_registry_class_has_its_own_byte_cap_and_the_transport_class_does_not_count_against_it() {
    let limits = limits(8, 4).with_inbound(InboundLimits {
        registry_frames: 100,
        registry_bytes: 1000,
        max_bindings: 8,
    });
    let (carrier, _from_carrier, mut to_carrier) = against_raw(limits);
    // Far more than 1,000 bytes of transport state is fine.
    for offset in 0..4 {
        write_all(
            &mut to_carrier,
            &wire_for("session-1", data(1, offset * 4096, vec![0; 4096])),
        );
    }
    pause_until("routed", || carrier.queued_in() == 4);
    assert!(carrier.wait_closed(Duration::from_millis(1)).is_none());
    // Registry frames past the cap end the carrier, naming the class.
    for _ in 0..3 {
        write_all(&mut to_carrier, &wire_frame(2, &[0u8; 400]));
    }
    let closed = eventually("the overrun", || carrier.wait_closed(SHORT));
    assert_eq!(
        closed.reason,
        CloseReason::Protocol(ProtocolFault::Overrun(FrameClass::Registry))
    );
}

#[test]
fn a_frame_wakes_the_waker_of_its_own_binding_or_the_registry_and_not_the_others() {
    let (carrier, _from_carrier, mut to_carrier) = against_raw(limits(8, 4));
    carrier.open_binding("a", BindingLimits::default()).unwrap();
    carrier.open_binding("b", BindingLimits::default()).unwrap();
    let ((wake_a, count_a), (wake_b, count_b), (wake_r, count_r)) =
        (counting_waker(), counting_waker(), counting_waker());
    carrier.register_binding("a", &wake_a);
    carrier.register_binding("b", &wake_b);
    carrier.register_registry(&wake_r);

    write_all(&mut to_carrier, &wire_for("b", window(1, 3)));
    pause_until("b's wake", || count_b.count() == 1);
    write_all(&mut to_carrier, &wire_frame(1, b"call"));
    pause_until("the registry's wake", || count_r.count() == 1);
    write_all(&mut to_carrier, &wire_for("a", window(1, 3)));
    pause_until("a's wake", || count_a.count() == 1);
    assert_eq!(
        (count_a.count(), count_b.count(), count_r.count()),
        (1, 1, 1)
    );
}

#[test]
fn a_blocking_take_of_one_binding_returns_when_its_frame_arrives_whatever_else_is_queued() {
    let (carrier, _from_carrier, mut to_carrier) = against_raw(limits(8, 4));
    carrier.open_binding("a", BindingLimits::default()).unwrap();
    carrier.open_binding("b", BindingLimits::default()).unwrap();
    for offset in 0..3 {
        write_all(&mut to_carrier, &wire_for("a", data(1, offset, vec![1])));
    }
    pause_until("a's frames", || carrier.queued_in() == 3);
    assert!(
        carrier
            .recv_binding_timeout("b", Duration::from_millis(30))
            .unwrap()
            .is_none()
    );
    let writer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        write_all(&mut to_carrier, &wire_for("b", window(1, 3)));
        to_carrier
    });
    let arrival = carrier
        .recv_binding_timeout("b", WAIT)
        .unwrap()
        .expect("b's frame");
    assert_eq!(arrival.message.1.kind, MessageKind::Window);
    assert_eq!(carrier.queued_in(), 3);
    let _keep = writer.join().unwrap();
}

#[test]
fn after_the_peers_close_each_class_drains_what_it_has_and_then_learns_the_closure() {
    let (carrier, _from_carrier, mut to_carrier) = against_raw(limits(8, 4));
    carrier.open_binding("a", BindingLimits::default()).unwrap();
    write_all(&mut to_carrier, &wire_for("a", data(1, 0, vec![1])));
    write_all(&mut to_carrier, &wire_frame(1, b"call"));
    write_all(
        &mut to_carrier,
        &super::super::wire::close_frame(CloseCode::Normal).pipe_wire(),
    );
    drop(to_carrier);
    // The registry class has one frame, then the closure; binding `a` still has its frame afterwards.
    let frame = eventually("the call", || carrier.recv_registry_timeout(SHORT).unwrap());
    assert_eq!(frame.body, b"call");
    let closure = eventually("the registry's closure", || {
        carrier.recv_registry_timeout(SHORT).err()
    });
    assert_eq!(closure.reason, CloseReason::Peer(CloseCode::Normal));
    let arrival = carrier
        .try_recv_binding("a")
        .unwrap()
        .expect("a's frame survives");
    assert_eq!(arrival.message.1.data.unwrap().offset, 0);
    assert_eq!(
        carrier.try_recv_binding("a").unwrap_err().reason,
        CloseReason::Peer(CloseCode::Normal)
    );
}

trait PipeWire {
    fn pipe_wire(&self) -> Vec<u8>;
}

impl PipeWire for gwz_session_contract::Frame {
    fn pipe_wire(&self) -> Vec<u8> {
        wire_frame(self.tag, &self.body)
    }
}

#[test]
fn an_envelope_that_does_not_decode_faults_the_binding_it_names_and_no_other() {
    let (carrier, _from_carrier, mut to_carrier) = against_raw(limits(8, 4));
    carrier.open_binding("a", BindingLimits::default()).unwrap();
    carrier.open_binding("b", BindingLimits::default()).unwrap();
    write_all(
        &mut to_carrier,
        &wire_lane_raw(16, "request-1", &undecodable_envelope("a")),
    );
    write_all(&mut to_carrier, &wire_for("b", data(1, 0, vec![7])));
    pause_until("a's fault", || carrier.binding_fault("a").is_some());
    assert_eq!(carrier.binding_fault("a"), Some(BindingFault::Invalid));
    // B completes, the carrier stays open.
    let arrival = carrier
        .recv_binding_timeout("b", WAIT)
        .unwrap()
        .expect("b's frame");
    assert_eq!(arrival.message.1.data.unwrap().payload, vec![7]);
    assert_eq!(carrier.binding_fault("b"), None);
    assert!(carrier.wait_closed(Duration::from_millis(1)).is_none());
}

#[test]
fn an_undecodable_envelope_for_a_binding_that_is_not_open_is_counted_and_creates_nothing() {
    let (carrier, _from_carrier, mut to_carrier) = against_raw(limits(8, 4));
    write_all(
        &mut to_carrier,
        &wire_lane_raw(16, "request-1", &undecodable_envelope("closed-one")),
    );
    pause_until("the discard", || carrier.discarded_frames() == 1);
    assert_eq!(carrier.binding_fault("closed-one"), None);
    assert_eq!(carrier.queued_in(), 0);
    assert!(carrier.wait_closed(Duration::from_millis(1)).is_none());
}

#[test]
fn a_frame_that_names_no_binding_at_all_is_a_session_fault_only_when_it_cannot_be_walked() {
    // A CBOR map with no text `session_id` is unattributable but well-framed: discarded, counted.
    let (carrier, _from_carrier, mut to_carrier) = against_raw(limits(8, 4));
    write_all(
        &mut to_carrier,
        &wire_lane_raw(16, "r", &[0xa1, 0x01, 0x01]),
    );
    pause_until("the discard", || carrier.discarded_frames() == 1);
    assert!(carrier.wait_closed(Duration::from_millis(1)).is_none());
    // A body that is not a map ends the session.
    write_all(&mut to_carrier, &wire_lane_raw(16, "r", &[0x01]));
    let closed = eventually("the fault", || carrier.wait_closed(SHORT));
    assert_eq!(closed.reason, CloseReason::Protocol(ProtocolFault::Message));
    // So does a broken request-name framing.
    let (carrier, _from_carrier, mut to_carrier) = against_raw(limits(8, 4));
    write_all(&mut to_carrier, &wire_frame(16, &[9, b'x']));
    let closed = eventually("the fault", || carrier.wait_closed(SHORT));
    assert_eq!(closed.reason, CloseReason::Protocol(ProtocolFault::Message));
}

#[test]
fn the_in_process_carrier_takes_its_one_stream_by_binding_too() {
    let (core, client) = super::super::in_process(CarrierLimits::DEFAULT).unwrap();
    core.try_send(data(1, 0, vec![1])).unwrap();
    // No bindings of its own: every name takes the one stream.
    let arrival = client
        .try_recv_binding("anything")
        .unwrap()
        .expect("the message");
    assert_eq!(arrival.message.1.kind, MessageKind::Data);
}

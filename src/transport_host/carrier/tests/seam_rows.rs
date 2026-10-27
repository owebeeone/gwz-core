//! The seam on the in-process carrier: order, bounds, the control reserve, close reasons and the binding.
use super::super::{
    BindingEnd, BindingFault, BindingLimits, Carrier, CarrierClosed, CarrierLimits, CarrierLink,
    CloseCode, CloseReason, FrameClass, Join, Lane, SendRefused, StreamCarrier, in_process,
};
use super::support::*;
use crate::transport_host::{TransportPort, session::Session};
use gwz_transport::protocol::MessageKind;
use std::{
    future::Future,
    pin::pin,
    sync::Arc,
    task::{Context, Poll, Waker},
    time::Duration,
};

#[test]
fn messages_arrive_in_the_order_sent_in_both_directions() {
    let (core, client) = in_process(CarrierLimits::DEFAULT).unwrap();
    let sent = [
        data(1, 0, vec![1]),
        window(1, 10),
        data(1, 1, vec![2]),
        close(1, 2),
    ];
    for message in sent.clone() {
        core.try_send(message).unwrap();
    }
    client.try_send(window(2, 5)).unwrap();
    for expected in &sent {
        assert_eq!(tag(&next(&client)), tag(expected));
    }
    assert_eq!(tag(&next(&core)), tag(&window(2, 5)));
    assert!(client.try_recv().unwrap().is_none());
}

#[test]
fn a_full_data_lane_gives_the_message_back_and_control_still_goes() {
    let (core, client) = in_process(limits(2, 2)).unwrap();
    core.try_send(data(1, 0, vec![0])).unwrap();
    core.try_send(data(1, 1, vec![0])).unwrap();
    let refused = core.try_send(data(1, 2, vec![0])).unwrap_err();
    let SendRefused::Full(back) = refused else {
        panic!("a full data lane must answer Full");
    };
    assert_eq!(back.1.data.as_ref().unwrap().offset, 2);
    // The control reserve is separate: window and cancel are accepted while data is full.
    core.try_send(window(1, 99)).unwrap();
    core.try_send(cancel(1)).unwrap();
    // And the reserve is itself bounded.
    assert!(matches!(
        core.try_send(close(1, 2)),
        Err(SendRefused::Full(_))
    ));
    // Order is the order sent, across lanes.
    let kinds: Vec<_> = (0..4).map(|_| next(&client).1.kind).collect();
    assert_eq!(
        kinds,
        [
            MessageKind::Data,
            MessageKind::Data,
            MessageKind::Window,
            MessageKind::Cancel
        ]
    );
    // Taking messages gives the lane room again.
    core.try_send(data(1, 2, vec![0])).unwrap();
}

#[test]
fn lanes_follow_the_mux_rule() {
    use MessageKind::*;
    for kind in [Data, Open, CheckIdentity] {
        assert_eq!(Lane::of(kind), Lane::Data, "{kind:?}");
    }
    for kind in [
        Bind,
        Bound,
        BindRejected,
        Opened,
        OpenFailed,
        Window,
        Flush,
        Flushed,
        EndWrite,
        Close,
        Closed,
        Cancel,
        Failed,
        IdentityChecked,
        IdentityCheckFailed,
    ] {
        assert_eq!(Lane::of(kind), Lane::Control, "{kind:?}");
    }
}

#[test]
fn a_close_reaches_the_peer_with_its_code_after_what_was_sent() {
    let (core, client) = in_process(CarrierLimits::DEFAULT).unwrap();
    core.try_send(data(1, 0, vec![1])).unwrap();
    core.close(CloseCode::Cancelled);
    assert_eq!(tag(&next(&client)), tag(&data(1, 0, vec![1])));
    let closed = client.try_recv().unwrap_err();
    assert_eq!(closed.reason, CloseReason::Peer(CloseCode::Cancelled));
    // The same reason, every time.
    assert_eq!(client.try_recv().unwrap_err(), closed);
    assert!(matches!(client.try_send(window(1, 1)), Err(SendRefused::Closed(_, c)) if c == closed));
    // The closing end sends and receives nothing more and says why.
    let own = CarrierClosed {
        reason: CloseReason::Local(CloseCode::Cancelled),
    };
    assert_eq!(core.try_recv().unwrap_err(), own);
    assert!(matches!(core.try_send(window(1, 1)), Err(SendRefused::Closed(_, c)) if c == own));
}

#[test]
fn a_dropped_end_closes_the_peer_with_the_dropped_code() {
    let (core, client) = in_process(CarrierLimits::DEFAULT).unwrap();
    drop(core);
    assert_eq!(
        client.try_recv().unwrap_err().reason,
        CloseReason::Peer(CloseCode::Dropped)
    );
}

#[test]
fn a_binding_admits_only_arrivals_of_its_own_connection_and_side() {
    let (core_a, client_a) = in_process(CarrierLimits::DEFAULT).unwrap();
    let (core_b, client_b) = in_process(CarrierLimits::DEFAULT).unwrap();
    client_a.try_send(window(1, 1)).unwrap();
    client_b.try_send(window(1, 1)).unwrap();
    let from_a = core_a.try_recv().unwrap().unwrap();
    let from_b = core_b.try_recv().unwrap().unwrap();
    assert!(core_a.binding().admits(&from_a));
    // Frames under another connection's binding are discarded, and nothing is created for them.
    assert!(!core_a.binding().admits(&from_b));
    assert!(!core_b.binding().admits(&from_a));
    // The other side of the same connection is not this side.
    assert!(!client_a.binding().admits(&from_a));
    assert!(from_b.into_message_for(&core_a.binding()).is_none());
    assert!(from_a.into_message_for(&core_a.binding()).is_some());
    assert_ne!(core_a.binding().id(), core_b.binding().id());
    assert_eq!(core_a.binding().id(), client_a.binding().id());
}

#[test]
fn the_registered_waker_is_woken_by_arrival_by_room_and_by_close() {
    let (core, client) = in_process(limits(1, 1)).unwrap();
    let (waker, wakes) = counting_waker();
    client.register(&waker);
    core.try_send(data(1, 0, vec![0])).unwrap();
    assert!(wakes.count() >= 1, "an arrival must wake the receiver");
    let (core_waker, core_wakes) = counting_waker();
    core.register(&core_waker);
    assert!(matches!(
        core.try_send(data(1, 1, vec![0])),
        Err(SendRefused::Full(_))
    ));
    let before = core_wakes.count();
    next(&client);
    assert!(
        core_wakes.count() > before,
        "room in a full lane must wake the sender"
    );
    let before = wakes.count();
    core.close(CloseCode::Normal);
    assert!(wakes.count() > before, "a close must wake the peer");
}

#[test]
fn a_links_end_closes_the_surviving_carriers_with_a_code_that_never_calls_a_failure_normal() {
    use super::super::LinkEnd;
    let closed = |reason| LinkEnd::Carrier(CarrierClosed { reason });
    // Orderly ends close in order.
    assert_eq!(LinkEnd::Stopped.code(), CloseCode::Normal);
    assert_eq!(LinkEnd::PortClosed.code(), CloseCode::Normal);
    assert_eq!(
        closed(CloseReason::Peer(CloseCode::Normal)).code(),
        CloseCode::Normal
    );
    assert_eq!(
        closed(CloseReason::Local(CloseCode::Cancelled)).code(),
        CloseCode::Cancelled
    );
    // Every failure closes with a failure code, whichever way it arrived.
    for end in [
        LinkEnd::PortFailed,
        closed(CloseReason::PeerGone),
        closed(CloseReason::Stalled),
        closed(CloseReason::Stream(std::io::ErrorKind::BrokenPipe)),
        closed(CloseReason::Protocol(super::super::ProtocolFault::Message)),
        closed(CloseReason::Peer(CloseCode::ProtocolFault)),
    ] {
        assert_eq!(end.code(), CloseCode::ProtocolFault, "{end:?}");
    }
    assert_eq!(
        closed(CloseReason::Peer(CloseCode::Dropped)).code(),
        CloseCode::Cancelled
    );
}

/// A scoped link over the bindings `a` and `b` of one stream carrier (and `ghost`, never opened, when asked), with a
/// port per join: (the link, the carrier, the ports in join order, the peer's raw writer, the ports' sessions).
struct Scoped {
    link: CarrierLink,
    carrier: Arc<StreamCarrier>,
    ports: Vec<TransportPort>,
    to_carrier: std::io::PipeWriter,
    _sessions: Vec<Arc<Session>>,
}

fn scoped(names: &[&str]) -> Scoped {
    let (carrier, _from_carrier, to_carrier) = against_raw(limits(8, 4));
    for name in ["a", "b"] {
        let limits = BindingLimits {
            receive_window: 4,
            ..BindingLimits::default()
        };
        carrier.open_binding(name, limits).unwrap();
    }
    let carrier = Arc::new(carrier);
    let mut ports = Vec::new();
    let mut sessions = Vec::new();
    let mut joins = Vec::new();
    for name in names {
        let (session, port) = Session::driver(3000, 3000, None).expect("a driver session");
        sessions.push(session);
        ports.push(port.clone());
        let end: Arc<dyn Carrier> = carrier.clone();
        joins.push(Join {
            port,
            end,
            binding: Some((*name).to_owned()),
        });
    }
    let link = CarrierLink::start_joined(joins).unwrap();
    Scoped {
        link,
        carrier,
        ports,
        to_carrier,
        _sessions: sessions,
    }
}

/// Whether the port has been disconnected: its next message is the end.
fn disconnected(port: &TransportPort) -> bool {
    let mut context = Context::from_waker(Waker::noop());
    let mut next = pin!(port.next_message());
    matches!(next.as_mut().poll(&mut context), Poll::Ready(Ok(None)))
}

/// What a scoped link must show once join 0 (`a`) has ended alone: its port disconnected and its reason recorded, while
/// join 1 (`b`) is untouched, the link has not ended and the carrier is open.
fn only_join_zero_ended(scoped: &Scoped, why: BindingEnd) {
    eventually("a's join to end", || {
        scoped.link.join_ended(0).map(|ended| {
            assert_eq!(ended, why);
        })
    });
    eventually("a's port to be disconnected", || {
        disconnected(&scoped.ports[0]).then_some(())
    });
    assert!(!disconnected(&scoped.ports[1]), "b's port was disconnected");
    assert_eq!(scoped.link.join_ended(1), None);
    assert_eq!(scoped.link.ended(), None);
    assert!(
        scoped
            .carrier
            .wait_closed(Duration::from_millis(1))
            .is_none()
    );
    // The link keeps serving `b`: a quiet wait changes nothing.
    std::thread::sleep(Duration::from_millis(60));
    assert!(!disconnected(&scoped.ports[1]));
    assert_eq!(scoped.link.ended(), None);
}

#[test]
fn a_scoped_link_ends_only_the_join_whose_binding_was_faulted() {
    let mut link = scoped(&["a", "b"]);
    // One frame larger than a's whole window (4 bytes): the overrun is decided when the frame is read, whatever the
    // link has taken meanwhile. A burst of small frames overruns only if the link has not drained them as they came.
    write_all(&mut link.to_carrier, &wire_for("a", data(1, 0, vec![9; 5])));
    eventually("a's overrun", || link.carrier.binding_fault("a"));
    only_join_zero_ended(
        &link,
        BindingEnd::Faulted(BindingFault::Overrun(FrameClass::TransportData)),
    );
}

#[test]
fn a_scoped_link_ends_only_the_join_whose_binding_was_retired() {
    let link = scoped(&["a", "b"]);
    link.carrier.retire_binding("a");
    only_join_zero_ended(&link, BindingEnd::NotOpen);
}

#[test]
fn a_scoped_link_ends_only_the_join_whose_binding_was_never_opened() {
    let link = scoped(&["ghost", "b"]);
    only_join_zero_ended(&link, BindingEnd::NotOpen);
}

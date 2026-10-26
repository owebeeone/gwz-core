//! The seam on the in-process carrier: order, bounds, the control reserve, close reasons and the binding.
use super::super::{
    Carrier, CarrierClosed, CarrierLimits, CloseCode, CloseReason, Lane, SendRefused, in_process,
};
use super::support::*;
use gwz_transport::protocol::MessageKind;

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

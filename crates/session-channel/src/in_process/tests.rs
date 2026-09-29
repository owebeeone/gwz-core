#![cfg(test)]
//! The in-process adapter's own guarantees, beyond the shared suite (§3; plan
//! CS1.2; §15.10): two bounded queues, each holding the outstanding-call limit
//! on the call lane plus the control reserve on the control lane; a `send`
//! that never waits, and refuses a full lane without dropping a frame; and a
//! close that wakes a receiver waiting at the closing end.

use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use crate::{InProcessEnd, pair};
use gwz_session_contract::{Closed, Frame, FrameSink, FrameSource, Lane, Limits, SendError, Tag};

fn call(marker: u16) -> Frame {
    Frame::new(Tag::SessionCall, marker.to_le_bytes().to_vec())
}

fn small_limits() -> Limits {
    Limits::new(3, 2).expect("three calls and two control frames")
}

/// Sends from another thread and fails if the send has not returned within
/// the watchdog's bound, so a send that waited fails the test instead of
/// hanging it. A send that does not wait returns at once; the bound only
/// catches a hang, and decides no ordering.
fn send_promptly(end: &Arc<InProcessEnd>, frame: Frame, lane: Lane) -> Result<(), SendError> {
    let (done, returned) = mpsc::channel();
    let sender = Arc::clone(end);
    thread::spawn(move || {
        let _ = done.send(sender.send(frame, lane));
    });
    returned
        .recv_timeout(Duration::from_secs(10))
        .expect("send returned without waiting for the receiver")
}

#[track_caller]
fn assert_full(result: Result<(), SendError>, frame: &Frame) {
    match result {
        Err(SendError::Full(returned)) => assert_eq!(&returned, frame, "the frame comes back"),
        other => panic!("expected a full lane, got {other:?}"),
    }
}

#[test]
fn a_full_call_lane_refuses_at_once_and_drops_nothing() {
    let (client, host) = pair(small_limits());
    let client = Arc::new(client);
    for marker in 0..3 {
        send_promptly(&client, call(marker), Lane::Call).expect("room on the call lane");
    }
    assert_full(send_promptly(&client, call(3), Lane::Call), &call(3));
    assert_full(send_promptly(&client, call(3), Lane::Call), &call(3));
    // The channel stays open, and every accepted frame arrives.
    for marker in 0..3 {
        assert_eq!(host.recv(), Ok(call(marker)));
    }
    // A frame holds its slot until the receiver takes it.
    for marker in 3..6 {
        send_promptly(&client, call(marker), Lane::Call).expect("the receiver freed the slots");
    }
    assert_full(send_promptly(&client, call(6), Lane::Call), &call(6));
    for marker in 3..6 {
        assert_eq!(host.recv(), Ok(call(marker)));
    }
}

#[test]
fn a_control_frame_goes_through_on_the_reserve_when_the_call_lane_is_full() {
    let (client, host) = pair(small_limits());
    for marker in 0..3 {
        client
            .send(call(marker), Lane::Call)
            .expect("room on the call lane");
    }
    assert_full(client.send(call(3), Lane::Call), &call(3));
    let cancel = Frame::new(Tag::SessionCall, b"operation.cancel".to_vec());
    let close = Frame::new(Tag::SessionCall, b"session.close".to_vec());
    client
        .send(cancel.clone(), Lane::Control)
        .expect("the control reserve takes a control frame");
    client
        .send(close.clone(), Lane::Control)
        .expect("the control reserve takes a second");
    // The reserve is bounded too, and a full reserve leaves the call lane as
    // it was.
    assert_full(client.send(call(9), Lane::Control), &call(9));
    assert_full(client.send(call(3), Lane::Call), &call(3));
    // The lanes share one order.
    for expected in [call(0), call(1), call(2), cancel, close] {
        assert_eq!(host.recv(), Ok(expected));
    }
}

#[test]
fn the_call_lane_never_takes_the_reserves_room() {
    let (client, host) = pair(small_limits());
    for marker in 0..2 {
        client
            .send(call(marker), Lane::Control)
            .expect("room in the reserve");
    }
    assert_full(client.send(call(2), Lane::Control), &call(2));
    for marker in 10..13 {
        client
            .send(call(marker), Lane::Call)
            .expect("a full reserve leaves the call lane its room");
    }
    assert_full(client.send(call(13), Lane::Call), &call(13));
    for expected in [0, 1, 10, 11, 12] {
        assert_eq!(host.recv(), Ok(call(expected)));
    }
}

#[test]
fn each_direction_has_its_own_queue() {
    let (client, host) = pair(small_limits());
    for marker in 0..3 {
        client
            .send(call(marker), Lane::Call)
            .expect("room toward the host");
    }
    let reply = |marker: u16| Frame::new(Tag::SessionReply, marker.to_le_bytes().to_vec());
    for marker in 0..3 {
        host.send(reply(marker), Lane::Call)
            .expect("the reply queue has its own room");
    }
    assert_full(host.send(reply(3), Lane::Call), &reply(3));
    for marker in 0..3 {
        assert_eq!(client.recv(), Ok(reply(marker)));
        assert_eq!(host.recv(), Ok(call(marker)));
    }
}

#[test]
fn the_default_queues_hold_1024_calls_and_64_control_frames() {
    let (client, host) = pair(Limits::default());
    for marker in 0..1024 {
        client
            .send(call(marker), Lane::Call)
            .expect("room on the call lane");
    }
    assert_full(client.send(call(1024), Lane::Call), &call(1024));
    for marker in 0..64 {
        client
            .send(call(marker), Lane::Control)
            .expect("room in the reserve");
    }
    assert_full(client.send(call(64), Lane::Control), &call(64));
    for marker in 0..1024 {
        assert_eq!(host.recv(), Ok(call(marker)));
    }
    for marker in 0..64 {
        assert_eq!(host.recv(), Ok(call(marker)));
    }
}

#[test]
fn closure_is_reported_to_both_ends_whichever_closes() {
    for host_closes in [false, true] {
        let (client, host) = pair(small_limits());
        let (closing, other) = if host_closes {
            (&host, &client)
        } else {
            (&client, &host)
        };
        closing
            .send(call(1), Lane::Call)
            .expect("an open channel sends");
        closing.close();
        assert_eq!(closing.recv(), Err(Closed::Local));
        assert_eq!(other.recv(), Ok(call(1)), "sent before the close");
        assert_eq!(other.recv(), Err(Closed::Peer));
        match other.send(call(2), Lane::Call) {
            Err(SendError::Closed(frame, Closed::Peer)) => assert_eq!(frame, call(2)),
            result => panic!("the peer's sends fail once closed, got {result:?}"),
        }
    }
}

#[test]
fn a_closed_channel_reports_its_closure_before_a_full_lane() {
    let (client, host) = pair(small_limits());
    for marker in 0..3 {
        client
            .send(call(marker), Lane::Call)
            .expect("room on the call lane");
    }
    host.close();
    match client.send(call(3), Lane::Call) {
        Err(SendError::Closed(frame, Closed::Peer)) => assert_eq!(frame, call(3)),
        result => panic!("expected the closure, got {result:?}"),
    }
}

#[test]
fn closing_wakes_a_receiver_waiting_at_the_closing_end() {
    let (client, host) = pair(small_limits());
    thread::scope(|scope| {
        let waiting = scope.spawn(|| client.recv());
        client.close();
        let received = waiting.join().expect("the receiver does not panic");
        assert_eq!(received, Err(Closed::Local));
    });
    assert_eq!(host.recv(), Err(Closed::Peer));
}

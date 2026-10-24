#![cfg(test)]
//! The lane's routing rules on decoded headers (`GwzSplitCommandChannel.md`
//! §3.3): scope, tag by kind, the transport-class bound and fault scope. The
//! rows that drive the lane through a service are in the service's tests.

use std::sync::Arc;
use std::time::Duration;

use gwz_session_contract::{Frame, Lane, SendError};

use super::*;

struct NoSink;

impl ReplySink for NoSink {
    fn send_reply(&self, frame: Frame, _: Lane, _: u64, _: bool) -> Result<(), SendError> {
        Err(SendError::Full(frame))
    }

    fn send_lane(&self, frame: Frame, _: Lane, _: &StreamTarget, _: bool) -> Result<(), SendError> {
        Err(SendError::Full(frame))
    }

    fn close_channel(&self) {}
}

fn target(request_id: &str, stream_id: u64) -> StreamTarget {
    StreamTarget {
        request_id: request_id.to_owned(),
        session_id: format!("s-{request_id}"),
        stream_id,
    }
}

fn frame(request_id: &str, stream_id: u64, kind: LaneKind, body: &str) -> LaneFrame {
    LaneFrame {
        tag: lane_tag(kind.control()),
        target: target(request_id, stream_id),
        kind,
        body: body.as_bytes().to_vec(),
    }
}

const OPENED: LaneKind = LaneKind::Other {
    kind: 5,
    control: true,
};
const CLOSED: LaneKind = LaneKind::Other {
    kind: 13,
    control: true,
};

fn next(lane: &CallLane) -> Result<String, LaneError> {
    lane.recv(Duration::from_millis(20))
        .map(|body| String::from_utf8(body).unwrap())
}

#[test]
fn control_kinds_take_tag_17_and_data_open_and_identity_checks_tag_16() {
    assert_eq!(lane_tag(true), 17);
    assert_eq!(lane_tag(false), 16);
    assert_eq!(lane_tag(LaneKind::Data(1).control()), TRANSPORT_DATA);
    assert_eq!(lane_tag(LaneKind::Window(1).control()), TRANSPORT_CONTROL);
    assert_eq!(lane_tag(OPENED.control()), TRANSPORT_CONTROL);
}

#[test]
fn a_frame_for_no_live_call_is_discarded_and_creates_no_state() {
    let lanes = Lanes::new(Arc::new(NoSink));
    assert_eq!(lanes.route(frame("r1", 0, OPENED, "x")), Routed::Discarded);
    let lane = lanes.register("r1", 16).unwrap();
    assert!(matches!(next(&lane), Err(LaneError::TimedOut)));
    assert!(lanes.register("r1", 16).is_err(), "one lane per live call");
    drop(lane);
    assert_eq!(lanes.route(frame("r1", 0, OPENED, "x")), Routed::Discarded);
    assert!(
        lanes.register("r1", 16).is_ok(),
        "a retired call's lane is gone"
    );
}

#[test]
fn frames_arrive_in_order_and_windows_coalesce_to_the_highest_offset() {
    let lanes = Lanes::new(Arc::new(NoSink));
    let lane = lanes.register("r1", 16).unwrap();
    assert_eq!(
        lanes.route(frame("r1", 1, OPENED, "opened")),
        Routed::Delivered
    );
    assert_eq!(
        lanes.route(frame("r1", 1, LaneKind::Window(10), "w10")),
        Routed::Delivered
    );
    assert_eq!(
        lanes.route(frame("r1", 1, LaneKind::Data(4), "data")),
        Routed::Delivered
    );
    assert_eq!(
        lanes.route(frame("r1", 1, LaneKind::Window(30), "w30")),
        Routed::Delivered
    );
    assert_eq!(
        lanes.route(frame("r1", 1, LaneKind::Window(20), "w20")),
        Routed::Delivered
    );
    assert_eq!(next(&lane).unwrap(), "opened");
    assert_eq!(next(&lane).unwrap(), "w30");
    assert_eq!(next(&lane).unwrap(), "data");
    assert!(matches!(next(&lane), Err(LaneError::TimedOut)));
}

#[test]
fn an_overrun_or_a_misplaced_kind_faults_that_binding_only() {
    let lanes = Lanes::new(Arc::new(NoSink));
    let first = lanes.register("r1", 8).unwrap();
    let second = lanes.register("r2", 8).unwrap();
    assert_eq!(
        lanes.route(frame("r1", 1, LaneKind::Data(8), "eight")),
        Routed::Delivered
    );
    // Within the window once consumed, beyond it while unconsumed.
    assert!(matches!(
        lanes.route(frame("r1", 1, LaneKind::Data(1), "one")),
        Routed::Faulted(_)
    ));
    assert!(matches!(next(&first), Err(LaneError::Faulted(_))));
    assert_eq!(
        lanes.route(frame("r1", 1, OPENED, "late")),
        Routed::Discarded
    );
    // The other call goes on.
    assert_eq!(
        lanes.route(frame("r2", 1, LaneKind::Data(8), "eight")),
        Routed::Delivered
    );
    assert_eq!(next(&second).unwrap(), "eight");
    assert_eq!(
        lanes.route(frame("r2", 1, LaneKind::Data(8), "again")),
        Routed::Delivered
    );
    // One slot per other kind and stream.
    assert_eq!(
        lanes.route(frame("r2", 2, CLOSED, "closed")),
        Routed::Delivered
    );
    assert!(matches!(
        lanes.route(frame("r2", 2, CLOSED, "closed")),
        Routed::Faulted(_)
    ));

    let third = lanes.register("r3", 8).unwrap();
    let mut misplaced = frame("r3", 1, OPENED, "opened");
    misplaced.tag = TRANSPORT_DATA;
    assert!(matches!(lanes.route(misplaced), Routed::Faulted(_)));
    assert!(matches!(next(&third), Err(LaneError::Faulted(_))));
}

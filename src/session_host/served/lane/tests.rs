//! The lane body with the transport's own envelopes (client placement
//! projection): tag by kind, the decoded target, and routing to a live
//! call's lane (`GwzSplitCommandChannel.md` §3.3).

use std::sync::Arc;
use std::time::Duration;

use gwz_session_contract::{Frame, Lane, SendError};
use gwz_session_host::{LaneKind, Lanes, ReplySink, Routed, StreamTarget};
use gwz_transport::protocol::{Envelope, MessageKind, Open, Opened};

use super::{decode_lane, encode_lane, envelope};

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

fn open() -> Envelope {
    Envelope {
        version: 1,
        session_id: "binding-1".to_owned(),
        stream_id: 1,
        kind: MessageKind::Open,
        open: Some(Open::default()),
        ..Envelope::default()
    }
}

fn opened() -> Envelope {
    Envelope {
        version: 1,
        session_id: "binding-1".to_owned(),
        stream_id: 1,
        kind: MessageKind::Opened,
        opened: Some(Opened::default()),
        ..Envelope::default()
    }
}

#[test]
fn an_open_takes_tag_16_and_its_answer_tag_17_and_both_round_trip() {
    let (frame, target) = encode_lane("request-1", open());
    assert_eq!(frame.tag, 16);
    assert_eq!(
        target,
        StreamTarget {
            request_id: "request-1".to_owned(),
            session_id: "binding-1".to_owned(),
            stream_id: 1,
        }
    );
    let decoded = decode_lane(&frame).unwrap();
    assert_eq!(decoded.target, target);
    assert_eq!(
        decoded.kind,
        LaneKind::Other {
            kind: MessageKind::Open.wire(),
            control: false,
        }
    );
    assert_eq!(envelope(&frame.body).unwrap(), open());
    let (answer, _) = encode_lane("request-1", opened());
    assert_eq!(answer.tag, 17);
    assert!(decode_lane(&answer).unwrap().kind.control());
}

#[test]
fn a_live_calls_answer_reaches_its_lane_and_a_body_that_does_not_decode_is_refused() {
    let lanes = Lanes::new(Arc::new(NoSink));
    let lane = lanes.register("request-1", 65536).unwrap();
    let (answer, _) = encode_lane("request-1", opened());
    assert_eq!(
        lanes.route(decode_lane(&answer).unwrap()),
        Routed::Delivered
    );
    let body = lane.recv(Duration::from_secs(5)).unwrap();
    assert_eq!(envelope(&body).unwrap(), opened());
    let (other, _) = encode_lane("request-2", opened());
    assert_eq!(lanes.route(decode_lane(&other).unwrap()), Routed::Discarded);
    let mut misplaced = answer.clone();
    misplaced.tag = 16;
    assert!(matches!(
        lanes.route(decode_lane(&misplaced).unwrap()),
        Routed::Faulted(_)
    ));
    let garbage = Frame {
        tag: 17,
        body: vec![0xff],
    };
    assert!(decode_lane(&garbage).is_err());
}

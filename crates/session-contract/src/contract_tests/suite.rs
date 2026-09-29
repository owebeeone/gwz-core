//! The cases. Each takes fresh channels from the fixture, so they run in any
//! order.

use std::thread;

use crate::{
    Carrier, Closed, Frame, FrameError, FrameSink, FrameSource, Lane, MAX_FRAME_BYTES, SendError,
    Tag,
};

/// What the suite needs from an adapter.
pub trait ChannelFixture {
    /// One end of the adapter's channel. The suite shares ends between
    /// threads, so an end is `Send` and `Sync`.
    type End: FrameSink + FrameSource + Send + Sync;

    /// Which part of the tag registry the adapter carries.
    fn carrier(&self) -> Carrier;

    /// A fresh channel: two connected ends. Each lane of each direction must
    /// hold the frames the suite keeps in flight: 256 on the call lane and 8
    /// on the control lane.
    fn pair(&mut self) -> (Self::End, Self::End);
}

/// A case's name, and the case.
type Case<F> = (&'static str, fn(&mut F));

/// Runs every case against `fixture`. A failing case's name is on standard
/// error, which the test harness shows for a failing test.
pub fn run_all<F: ChannelFixture>(fixture: &mut F) {
    let cases: [Case<F>; 8] = [
        (
            "frames_arrive_in_order_on_both_lanes",
            frames_arrive_in_order_on_both_lanes,
        ),
        (
            "concurrent_senders_keep_each_frame_whole",
            concurrent_senders_keep_each_frame_whole,
        ),
        (
            "closing_delivers_what_was_sent_then_the_closure",
            closing_delivers_what_was_sent_then_the_closure,
        ),
        (
            "dropping_an_end_closes_the_channel",
            dropping_an_end_closes_the_channel,
        ),
        (
            "a_waiting_receiver_returns_when_the_peer_closes",
            a_waiting_receiver_returns_when_the_peer_closes,
        ),
        (
            "a_tag_the_carrier_does_not_carry_ends_the_session",
            a_tag_the_carrier_does_not_carry_ends_the_session,
        ),
        (
            "a_frame_over_the_size_limit_ends_the_session",
            a_frame_over_the_size_limit_ends_the_session,
        ),
        (
            "a_frame_at_the_size_limit_arrives_whole",
            a_frame_at_the_size_limit_arrives_whole,
        ),
    ];
    for (name, case) in cases {
        eprintln!("gwz-session-contract conformance case: {name}");
        case(fixture);
    }
}

/// Frames arrive in the order they were sent, on both lanes and in both
/// directions, each once and byte for byte, whatever their size: the channel
/// is reliable and ordered in each direction while open (§3).
pub fn frames_arrive_in_order_on_both_lanes<F: ChannelFixture>(fixture: &mut F) {
    let carrier = fixture.carrier();
    let (a, b) = fixture.pair();
    let frames = sample_frames(carrier);
    for (from, to) in [(&a, &b), (&b, &a)] {
        thread::scope(|scope| {
            let sent = frames.clone();
            // A byte stream's writer waits for its reader, so the frames are
            // sent from another thread.
            scope.spawn(move || {
                for (index, frame) in sent.into_iter().enumerate() {
                    let lane = if index % 2 == 0 {
                        Lane::Call
                    } else {
                        Lane::Control
                    };
                    from.send(frame, lane)
                        .expect("an open channel sends each frame");
                }
            });
            for expected in &frames {
                assert_eq!(to.recv().as_ref(), Ok(expected));
            }
        });
    }
}

/// Frames sent from several threads at once each arrive whole, and each
/// thread's frames arrive in its own order.
pub fn concurrent_senders_keep_each_frame_whole<F: ChannelFixture>(fixture: &mut F) {
    const SENDERS: u8 = 4;
    const FRAMES: u8 = 50;
    let (a, b) = fixture.pair();
    thread::scope(|scope| {
        for sender in 0..SENDERS {
            let a = &a;
            scope.spawn(move || {
                for sequence in 0..FRAMES {
                    // Each body is long enough that a byte stream writes it
                    // in several pieces if nothing keeps frames apart.
                    let mut body = vec![sender; 4096];
                    body[0] = sequence;
                    a.send(Frame::new(Tag::SessionCall, body), Lane::Call)
                        .expect("an open channel sends each frame");
                }
            });
        }
        let mut next = [0u8; SENDERS as usize];
        for _ in 0..u32::from(SENDERS) * u32::from(FRAMES) {
            let frame = b.recv().expect("every frame arrives");
            assert_eq!(frame.tag, Tag::SessionCall.byte());
            assert_eq!(frame.body.len(), 4096);
            let sender = frame.body[1];
            assert!(
                frame.body[1..].iter().all(|byte| *byte == sender),
                "a frame arrived whole"
            );
            let expected = &mut next[usize::from(sender)];
            assert_eq!(frame.body[0], *expected, "sender {sender}'s order");
            *expected += 1;
        }
    });
}

/// An end that closes has every frame it sent before the close delivered,
/// then the closure. The closed end sends and receives nothing more, not even
/// what the peer had in flight to it, and a frame it cannot send comes back.
/// Closing twice changes nothing (§3: closure is reported to both ends; §8).
pub fn closing_delivers_what_was_sent_then_the_closure<F: ChannelFixture>(fixture: &mut F) {
    let (a, b) = fixture.pair();
    let in_flight_to_a = frame_of(Tag::SessionReply, 3);
    b.send(in_flight_to_a, Lane::Call)
        .expect("an open channel sends");
    let sent = [
        frame_of(Tag::SessionCall, 0),
        frame_of(Tag::SessionCall, 1),
        frame_of(Tag::SessionCall, 2),
    ];
    for (frame, lane) in sent.iter().zip([Lane::Call, Lane::Control, Lane::Call]) {
        a.send(frame.clone(), lane).expect("an open channel sends");
    }
    a.close();
    a.close();
    assert_eq!(a.recv(), Err(Closed::Local));
    assert_eq!(a.recv(), Err(Closed::Local), "closure is reported again");
    let refused = frame_of(Tag::SessionCall, 9);
    assert_closed(
        a.send(refused.clone(), Lane::Control),
        &refused,
        Closed::Local,
    );

    for frame in &sent {
        assert_eq!(b.recv().as_ref(), Ok(frame));
    }
    assert_eq!(b.recv(), Err(Closed::Peer));
    assert_eq!(b.recv(), Err(Closed::Peer), "closure is reported again");
    assert_closed(b.send(refused.clone(), Lane::Call), &refused, Closed::Peer);
    b.close();
    assert_eq!(b.recv(), Err(Closed::Peer), "the first reason stands");
}

/// Dropping an end closes the channel as `close` does.
pub fn dropping_an_end_closes_the_channel<F: ChannelFixture>(fixture: &mut F) {
    let (a, b) = fixture.pair();
    let frame = frame_of(Tag::SessionCall, 1);
    a.send(frame.clone(), Lane::Call)
        .expect("an open channel sends");
    drop(a);
    assert_eq!(b.recv(), Ok(frame.clone()));
    assert_eq!(b.recv(), Err(Closed::Peer));
    assert_closed(b.send(frame.clone(), Lane::Call), &frame, Closed::Peer);
}

/// A receiver waiting on an idle channel returns once the peer closes.
pub fn a_waiting_receiver_returns_when_the_peer_closes<F: ChannelFixture>(fixture: &mut F) {
    let (a, b) = fixture.pair();
    thread::scope(|scope| {
        let waiting = scope.spawn(|| b.recv());
        a.close();
        let received = waiting.join().expect("the receiver does not panic");
        assert_eq!(received, Err(Closed::Peer));
    });
}

/// A frame whose tag the carrier does not carry ends the session (§3: "any
/// other tag"; the transport lane's reserved tags; and, in-process, the
/// byte-stream tags 4 to 8, server design §8). The frame comes back with the
/// protocol error, which the sending end reports from then on; the frames sent
/// before it still arrive, and then the peer sees the channel closed.
pub fn a_tag_the_carrier_does_not_carry_ends_the_session<F: ChannelFixture>(fixture: &mut F) {
    let carrier = fixture.carrier();
    let mut tags = vec![0, 9, 15, 16, 24, 31, 32, 255];
    if carrier == Carrier::InProcess {
        tags.extend(4..=8);
    }
    for tag in tags {
        let error = carrier
            .tag(tag)
            .expect_err("the tag is outside the carrier's registry");
        let (a, b) = fixture.pair();
        let before = frame_of(Tag::SessionCall, 1);
        a.send(before.clone(), Lane::Call)
            .expect("an open channel sends");
        let bad = Frame {
            tag,
            body: vec![0xa0],
        };
        assert_closed(
            a.send(bad.clone(), Lane::Call),
            &bad,
            Closed::Protocol(error),
        );
        assert_eq!(a.recv(), Err(Closed::Protocol(error)), "tag {tag}");
        let after = frame_of(Tag::SessionCall, 2);
        assert_closed(
            a.send(after.clone(), Lane::Call),
            &after,
            Closed::Protocol(error),
        );

        assert_eq!(b.recv(), Ok(before), "tag {tag}");
        assert_peer_ended(b.recv(), error);
    }
}

/// A frame over [`MAX_FRAME_BYTES`] ends the session (§15.10).
pub fn a_frame_over_the_size_limit_ends_the_session<F: ChannelFixture>(fixture: &mut F) {
    let (a, b) = fixture.pair();
    let error = FrameError::TooLarge {
        size: MAX_FRAME_BYTES + 1,
    };
    let over = Frame::new(Tag::SessionReply, vec![0; MAX_FRAME_BYTES]);
    match a.send(over, Lane::Call) {
        Err(SendError::Closed(frame, Closed::Protocol(reported))) => {
            assert_eq!(reported, error);
            assert_eq!(frame.body.len(), MAX_FRAME_BYTES, "the frame comes back");
        }
        other => panic!("an oversize frame must end the session, got {other:?}"),
    }
    assert_eq!(a.recv(), Err(Closed::Protocol(error)));
    assert_peer_ended(b.recv(), error);
}

/// A frame of exactly [`MAX_FRAME_BYTES`] arrives whole (§1: frames of at most
/// 64 MiB).
pub fn a_frame_at_the_size_limit_arrives_whole<F: ChannelFixture>(fixture: &mut F) {
    let (a, b) = fixture.pair();
    let mut body = vec![0x5a; MAX_FRAME_BYTES - 1];
    body[0] = 1;
    body[MAX_FRAME_BYTES / 2] = 2;
    body[MAX_FRAME_BYTES - 2] = 3;
    let frame = Frame::new(Tag::SessionReply, body);
    assert_eq!(frame.size(), MAX_FRAME_BYTES);
    thread::scope(|scope| {
        let sent = frame.clone();
        let a = &a;
        scope.spawn(move || {
            a.send(sent, Lane::Call)
                .expect("a frame at the limit is sent");
        });
        let received = b.recv().expect("a frame at the limit arrives");
        // Compared as frames, whose `Debug` leaves the 64 MiB body out.
        assert_eq!(received, frame);
    });
}

/// One frame for each tag the carrier carries, with bodies from empty to
/// larger than 65,535 bytes, so a length prefix's upper bytes are used too.
fn sample_frames(carrier: Carrier) -> Vec<Frame> {
    let lengths = [0, 1, 23, 255, 256, 4096, 65_536, 70_000];
    Tag::ALL
        .iter()
        .filter(|tag| carrier.tag(tag.byte()).is_ok())
        .zip(lengths.iter().cycle())
        .map(|(tag, length)| {
            let body = (0..*length).map(|index| (index % 251) as u8).collect();
            Frame::new(*tag, body)
        })
        .chain([
            Frame::new(Tag::SessionReply, vec![0x42; 70_000]),
            Frame::new(Tag::SessionCall, Vec::new()),
        ])
        .collect()
}

fn frame_of(tag: Tag, marker: u8) -> Frame {
    Frame::new(tag, vec![marker, 0xa0])
}

/// `result` is a refused send that gave `frame` back with `reason`.
#[track_caller]
fn assert_closed(result: Result<(), SendError>, frame: &Frame, reason: Closed) {
    match result {
        Err(SendError::Closed(returned, reported)) => {
            assert_eq!(&returned, frame, "the frame comes back");
            assert_eq!(reported, reason);
        }
        other => panic!("expected the frame back with {reason:?}, got {other:?}"),
    }
}

/// The peer of an end that met protocol error `error` sees the channel
/// closed: by the peer, as a byte stream shows it, or by the same error, as
/// an adapter that shares its reason shows it.
#[track_caller]
fn assert_peer_ended(received: Result<Frame, Closed>, error: FrameError) {
    match received {
        Err(Closed::Peer) => {}
        Err(Closed::Protocol(reported)) if reported == error => {}
        other => panic!("the peer must see the session end, got {other:?}"),
    }
}

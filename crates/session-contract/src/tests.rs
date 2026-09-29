#![cfg(test)]
//! The contract's own values: the tag registry, frame checks, lanes, limits
//! and the errors' texts. They live in the library, so that the Tier A
//! command, `cargo test -p gwz-session-contract --lib`, runs them (gwz-dev `dev-docs/GwzCoreSessionDesign.md` §1 and §3,
//! as `GwzCoreServerDesign.md` §3 and §8 amend §3). The behaviour of channel
//! ends is the conformance suite's (`contract_tests`), which each adapter runs.

use std::io;

use crate::{
    Carrier, Closed, Frame, FrameError, Lane, Limits, LimitsError, MAX_FRAME_BYTES, RESERVED_TAGS,
    SendError, Tag,
};

#[test]
fn the_frame_limit_is_64_mib() {
    assert_eq!(MAX_FRAME_BYTES, 64 * 1024 * 1024);
}

#[test]
fn the_registry_holds_tags_1_to_8_in_byte_order() {
    let bytes: Vec<u8> = Tag::ALL.iter().map(|tag| tag.byte()).collect();
    assert_eq!(bytes, (1..=8).collect::<Vec<u8>>());
    let named = [
        (Tag::SessionCall, 1),
        (Tag::SessionReply, 2),
        (Tag::SessionError, 3),
        (Tag::SessionOpen, 4),
        (Tag::SessionOpened, 5),
        (Tag::SessionHello, 6),
        (Tag::ServerControl, 7),
        (Tag::ServerState, 8),
    ];
    for (tag, byte) in named {
        assert_eq!(tag.byte(), byte);
        assert_eq!(Tag::from_byte(byte), Ok(tag));
    }
}

#[test]
fn every_byte_has_exactly_one_registry_answer() {
    assert_eq!(RESERVED_TAGS, 16..=31);
    for byte in 0..=u8::MAX {
        let expected = match byte {
            1..=8 => Ok(Tag::ALL[usize::from(byte - 1)]),
            16..=31 => Err(FrameError::ReservedTag(byte)),
            _ => Err(FrameError::UnknownTag(byte)),
        };
        assert_eq!(Tag::from_byte(byte), expected, "byte {byte}");
        assert_eq!(Carrier::ByteStream.tag(byte), expected, "byte {byte}");
    }
}

#[test]
fn only_byte_streams_carry_the_handshake_and_control_tags() {
    for tag in Tag::ALL {
        let byte_stream_only = (4..=8).contains(&tag.byte());
        assert_eq!(tag.byte_stream_only(), byte_stream_only, "{tag:?}");
        assert_eq!(Carrier::ByteStream.tag(tag.byte()), Ok(tag));
        let in_process = if byte_stream_only {
            Err(FrameError::ByteStreamOnly(tag.byte()))
        } else {
            Ok(tag)
        };
        assert_eq!(Carrier::InProcess.tag(tag.byte()), in_process, "{tag:?}");
    }
    // Outside the registry, the in-process answer is the registry's own.
    for byte in [0, 9, 15, 16, 31, 32, 255] {
        assert_eq!(Carrier::InProcess.tag(byte), Tag::from_byte(byte));
    }
}

#[test]
fn a_frames_size_counts_its_tag_byte_and_its_body() {
    assert_eq!(Frame::new(Tag::SessionCall, Vec::new()).size(), 1);
    assert_eq!(Frame::new(Tag::SessionCall, vec![0; 300]).size(), 301);
    let frame = Frame::new(Tag::SessionReply, vec![7, 8]);
    assert_eq!(
        frame,
        Frame {
            tag: 2,
            body: vec![7, 8]
        }
    );
}

#[test]
fn the_size_limit_admits_64_mib_and_refuses_one_byte_more() {
    assert_eq!(Frame::check_size(1), Ok(()));
    assert_eq!(Frame::check_size(MAX_FRAME_BYTES), Ok(()));
    assert_eq!(
        Frame::check_size(MAX_FRAME_BYTES + 1),
        Err(FrameError::TooLarge {
            size: MAX_FRAME_BYTES + 1
        })
    );
    // A byte stream's zero length prefix leaves no room for the tag byte.
    assert_eq!(Frame::check_size(0), Err(FrameError::Empty));

    let at_limit = Frame::new(Tag::SessionReply, vec![0; MAX_FRAME_BYTES - 1]);
    assert_eq!(at_limit.check(Carrier::InProcess), Ok(Tag::SessionReply));
    assert_eq!(at_limit.check(Carrier::ByteStream), Ok(Tag::SessionReply));
    let over = Frame::new(Tag::SessionReply, vec![0; MAX_FRAME_BYTES]);
    let too_large = Err(FrameError::TooLarge {
        size: MAX_FRAME_BYTES + 1,
    });
    assert_eq!(over.check(Carrier::InProcess), too_large);
    assert_eq!(over.check(Carrier::ByteStream), too_large);
}

#[test]
fn a_frame_is_checked_for_its_size_before_its_tag() {
    // A byte stream meets the length prefix before the tag byte, so both
    // carriers report the size first.
    let frame = Frame {
        tag: 0,
        body: vec![0; MAX_FRAME_BYTES],
    };
    for carrier in [Carrier::InProcess, Carrier::ByteStream] {
        assert_eq!(
            frame.check(carrier),
            Err(FrameError::TooLarge {
                size: MAX_FRAME_BYTES + 1
            })
        );
    }
    let small = Frame {
        tag: 16,
        body: Vec::new(),
    };
    assert_eq!(
        small.check(Carrier::ByteStream),
        Err(FrameError::ReservedTag(16))
    );
    let handshake = Frame::new(Tag::SessionHello, Vec::new());
    assert_eq!(
        handshake.check(Carrier::InProcess),
        Err(FrameError::ByteStreamOnly(6))
    );
    assert_eq!(handshake.check(Carrier::ByteStream), Ok(Tag::SessionHello));
}

#[test]
fn debug_output_never_shows_a_body() {
    // A body can carry the environment snapshot (the server design's
    // `SessionOpen`), which is secret-bearing.
    let frame = Frame::new(Tag::SessionOpen, b"SECRET=hunter2".to_vec());
    assert_eq!(format!("{frame:?}"), "Frame { tag: 4, body_len: 14 }");
    let full = format!("{:?}", SendError::Full(frame.clone()));
    let closed = format!("{:?}", SendError::Closed(frame, Closed::Local));
    for text in [full, closed] {
        assert!(!text.contains("SECRET"), "{text}");
        assert!(!text.contains("104, 117"), "{text}");
        assert!(text.contains("body_len: 14"), "{text}");
    }
}

#[test]
fn a_send_error_gives_its_frame_back() {
    let frame = Frame::new(Tag::SessionCall, vec![1, 2, 3]);
    let full = SendError::Full(frame.clone());
    assert_eq!(full.closed(), None);
    assert_eq!(full.into_frame(), frame);
    let reason = Closed::Protocol(FrameError::ReservedTag(20));
    let closed = SendError::Closed(frame.clone(), reason);
    assert_eq!(closed.closed(), Some(reason));
    assert_eq!(closed.into_frame(), frame);
}

#[test]
fn errors_say_what_happened() {
    let texts = [
        (
            FrameError::UnknownTag(9).to_string(),
            "frame tag 9 is not in the session's tag registry",
        ),
        (
            FrameError::ReservedTag(16).to_string(),
            "frame tag 16 is reserved for a transport lane",
        ),
        (
            FrameError::ByteStreamOnly(4).to_string(),
            "frame tag 4 is carried only by byte streams",
        ),
        (
            FrameError::TooLarge { size: 67_108_865 }.to_string(),
            "a frame of 67108865 bytes exceeds the 64 MiB frame limit",
        ),
        (
            FrameError::Empty.to_string(),
            "a frame length of zero leaves no room for the tag byte",
        ),
        (Closed::Local.to_string(), "this end closed the channel"),
        (Closed::Peer.to_string(), "the peer closed the channel"),
        (
            Closed::Protocol(FrameError::UnknownTag(0)).to_string(),
            "a protocol error ended the session: frame tag 0 is not in the session's tag registry",
        ),
        (
            Closed::Stream(io::ErrorKind::UnexpectedEof).to_string(),
            "the byte stream failed: unexpected end of file",
        ),
        (
            SendError::Full(Frame::new(Tag::SessionCall, Vec::new())).to_string(),
            "the frame's lane is full",
        ),
        (
            SendError::Closed(Frame::new(Tag::SessionCall, Vec::new()), Closed::Peer).to_string(),
            "the channel has ended: the peer closed the channel",
        ),
    ];
    for (text, expected) in texts {
        assert_eq!(text, expected);
    }
}

#[test]
fn the_default_limits_are_the_contracts() {
    let limits = Limits::default();
    assert_eq!(Limits::DEFAULT_OUTSTANDING_CALLS, 1024);
    assert_eq!(Limits::DEFAULT_CONTROL_RESERVE, 64);
    assert_eq!(limits.outstanding_calls(), 1024);
    assert_eq!(limits.control_reserve(), 64);
    assert_eq!(limits.lane_capacity(Lane::Call), 1024);
    assert_eq!(limits.lane_capacity(Lane::Control), 64);
    assert_eq!(limits.queue_capacity(), 1024 + 64);
    assert_eq!(Limits::new(1024, 64), Ok(limits));
}

#[test]
fn limits_refuse_what_would_leave_a_lane_no_room_or_overflow_a_queue() {
    // Without an outstanding call nothing could be sent, and without a control
    // reserve `session.close` could never be sent (§3).
    assert_eq!(Limits::new(0, 64), Err(LimitsError::NoOutstandingCalls));
    assert_eq!(Limits::new(1024, 0), Err(LimitsError::NoControlReserve));
    assert_eq!(Limits::new(0, 0), Err(LimitsError::NoOutstandingCalls));
    assert_eq!(Limits::new(usize::MAX, 1), Err(LimitsError::Overflow));
    let largest = Limits::new(usize::MAX - 1, 1).expect("a queue of usize::MAX frames");
    assert_eq!(largest.queue_capacity(), usize::MAX);
    let smallest = Limits::new(1, 1).expect("one frame on each lane");
    assert_eq!(smallest.queue_capacity(), 2);
    // The texts are core's `open` refusals for the same two limits.
    assert_eq!(
        LimitsError::NoOutstandingCalls.to_string(),
        "session limit outstanding_calls must be at least 1"
    );
    assert_eq!(
        LimitsError::NoControlReserve.to_string(),
        "session limit control_reserve must be at least 1"
    );
    assert_eq!(
        LimitsError::Overflow.to_string(),
        "session limits outstanding_calls plus control_reserve overflow a channel queue"
    );
}

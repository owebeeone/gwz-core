#![cfg(test)]
//! The byte-stream adapter's own guarantees, beyond the shared suite (§3; plan
//! CS1.3): frames round-trip against checked-in vectors that taut-shape's
//! interop tool framed; a length prefix over 64 MiB ends the session before
//! anything is allocated; end of stream is closure; and closing an end ends
//! the peer's stream (§15.9, the adapter's half).
//!
//! # The vectors
//!
//! Generated once, on 2026-09-28, from a copy of `taut-shape-rs` 0.9.2 whose
//! `crates/taut-shape-tool/src/framing.rs` has SHA-256
//! `b205c07cab85b99dc789e6a144cabef49be695c8a7706799b0ab5fa33ed4f0ca`, built
//! offline with `cargo build --offline --locked -p taut-shape-tool`. That
//! framing writes `u32-LE (1 + body.len())`, the tag byte, then the body.
//!
//! - `tool_client_read.bin`, 28 bytes: the tool's own output from
//!   `taut-shape-tool client --stream-id gwz --from 0 < /dev/null`, one
//!   `LogReadRequest` frame under the log shape's tag 3.
//! - `tool_client_push.bin`, 47 bytes: the same command with `--script`
//!   naming `[{"after_frames": 1, "inputs": [{"type": "push", "payload":
//!   "aGVsbG8="}, {"type": "seal"}]}]`. After the request come the log
//!   shape's push (tag 0) and seal (tag 1) frames.
//! - `tool_node_read_response.bin`, 38 bytes: the tool's output from
//!   `taut-shape-tool node --script <that script> < tool_client_read.bin`,
//!   one `LogReadResponse` frame under the log shape's tag 7.
//! - `tool_framing_session_tags.bin`, 70,535 bytes: the frames of
//!   [`session_tag_frames`], written by the tool's own `framing::write_frame`,
//!   compiled verbatim (`#[path]`) into a one-off generator against the same
//!   taut-shape, with bodies from taut-shape's `cbor::encode`. It covers the
//!   eight registry tags and length prefixes over 255 and over 65,535.
//!
//! The expected frames below were read off `cbor::encode`'s output, before
//! framing, so the test checks this adapter's framing against the tool's.
//! The vectors stay in `tests/vectors/`, read through `CARGO_MANIFEST_DIR`;
//! the tests live in the library, so that the Tier A command,
//! `cargo test -p gwz-session-channel --lib`, runs them.

use std::io::{self, ErrorKind, Read, Write};
use std::panic::{self, AssertUnwindSafe};

use crate::byte_stream;
use gwz_session_contract::{
    Closed, Frame, FrameError, FrameSink, FrameSource, Lane, MAX_FRAME_BYTES, SendError, Tag,
};

const TOOL_CLIENT_READ: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/vectors/tool_client_read.bin"
));
const TOOL_CLIENT_PUSH: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/vectors/tool_client_push.bin"
));
const TOOL_NODE_READ_RESPONSE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/vectors/tool_node_read_response.bin"
));
const TOOL_FRAMING_SESSION_TAGS: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/vectors/tool_framing_session_tags.bin"
));

fn hex(text: &str) -> Vec<u8> {
    let digits: Vec<u8> = text
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .map(|byte| match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            _ => panic!("not a hex digit: {byte}"),
        })
        .collect();
    digits
        .chunks(2)
        .map(|pair| (pair[0] << 4) | pair[1])
        .collect()
}

fn frame(tag: u8, body: Vec<u8>) -> Frame {
    Frame { tag, body }
}

/// The body bytes 0, 1, ..., 250, 0, 1, ... of the generator's byte strings.
fn pattern(length: usize) -> Vec<u8> {
    (0..length).map(|index| (index % 251) as u8).collect()
}

/// `LogReadRequest {1: "log-A", 2: "gwz", 3: {1: 0}, 4: null, 5: null,
/// 6: null}`.
fn tool_read_request() -> Frame {
    frame(
        3,
        hex("a6 01 65 6c6f672d41 02 63 67777a 03 a1 01 00 04 f6 05 f6 06 f6"),
    )
}

/// The generator's frames: one shaped like each registry frame, then the
/// smallest body and a 70,000-byte one.
fn session_tag_frames() -> Vec<Frame> {
    let mut reply = hex("a2 01 01 02 59 012c");
    reply.extend(pattern(300));
    let mut large = hex("5a 00011170");
    large.extend(pattern(70_000));
    vec![
        // {1: 1, 2: "status", 4: h'a0'}
        frame(1, hex("a3 01 01 02 66 737461747573 04 41 a0")),
        // {1: 1, 2: <300 bytes>}
        frame(2, reply),
        // {1: 2, 2: {1: 75, 2: "transport_session_full"}}
        frame(
            3,
            hex("a2 01 02 02 a2 01 18 4b 02 76 7472616e73706f72745f73657373696f6e5f66756c6c"),
        ),
        // {1: 1, 2: "gwz 1.1.0", 5: {2: false}}
        frame(4, hex("a3 01 01 02 69 67777a20312e312e30 05 a1 02 f4")),
        // {1: 1, 2: "gwz 1.1.0", 3: "gwz-core 1.1.0 transport", 4: 4242, 5: 0}
        frame(
            5,
            hex(
                "a5 01 01 02 69 67777a20312e312e30 03 78 18 67777a2d636f726520312e312e30207472616e73706f7274 04 19 1092 05 00",
            ),
        ),
        // {1: 1, 2: 1, 3: "gwz 1.1.0", 4: "gwz-core 1.1.0 transport", 5: 4242}
        frame(
            6,
            hex(
                "a5 01 01 02 01 03 69 67777a20312e312e30 04 78 18 67777a2d636f726520312e312e30207472616e73706f7274 05 19 1092",
            ),
        ),
        // {1: 0}
        frame(7, hex("a1 01 00")),
        // {1: 4242, 2: 0, 3: 16, 6: false}
        frame(8, hex("a4 01 19 1092 02 00 03 10 06 f4")),
        // {}
        frame(1, hex("a0")),
        // <70,000 bytes>
        frame(2, large),
    ]
}

/// Every frame an end reads from `bytes`, and the closure that ends them.
fn read_all(bytes: impl Read) -> (Vec<Frame>, Closed) {
    let end = byte_stream(bytes, io::sink());
    let mut frames = Vec::new();
    loop {
        match end.recv() {
            Ok(frame) => frames.push(frame),
            Err(closed) => {
                assert_eq!(end.recv(), Err(closed), "the closure is reported again");
                return (frames, closed);
            }
        }
    }
}

/// The bytes an end writes for `frames`.
fn write_all(frames: &[Frame]) -> Vec<u8> {
    let mut written = Vec::new();
    {
        let end = byte_stream(io::empty(), &mut written);
        for (index, frame) in frames.iter().enumerate() {
            let lane = if index % 2 == 0 {
                Lane::Call
            } else {
                Lane::Control
            };
            end.send(frame.clone(), lane)
                .expect("a Vec takes every frame");
        }
    }
    written
}

#[track_caller]
fn assert_round_trip(vector: &[u8], expected: &[Frame]) {
    let (frames, closed) = read_all(vector);
    assert_eq!(frames, expected, "the frames read");
    assert_eq!(closed, Closed::Peer, "the vector ends between frames");
    assert!(write_all(expected) == vector, "the bytes written");
}

#[test]
fn the_tools_request_frame_round_trips() {
    assert_round_trip(TOOL_CLIENT_READ, &[tool_read_request()]);
}

#[test]
fn the_tools_response_frame_round_trips_under_a_byte_stream_tag() {
    // `LogReadResponse {1: "log-A", 2: "gwz", 3: [{1: 1, 2: h'68656c6c6f'}],
    // 4: {1: 1}, 5: 0, 6: null}`, under tag 7, which byte streams carry.
    let response = frame(
        7,
        hex(
            "a6 01 65 6c6f672d41 02 63 67777a 03 81 a2 01 01 02 45 68656c6c6f 04 a1 01 01 05 00 06 f6",
        ),
    );
    assert_round_trip(TOOL_NODE_READ_RESPONSE, &[response]);
}

#[test]
fn every_registry_tag_round_trips_as_the_tool_frames_it() {
    let frames = session_tag_frames();
    let tags: Vec<u8> = frames.iter().map(|frame| frame.tag).collect();
    assert_eq!(tags, [1, 2, 3, 4, 5, 6, 7, 8, 1, 2]);
    assert_round_trip(TOOL_FRAMING_SESSION_TAGS, &frames);
}

#[test]
fn a_frame_is_its_little_endian_length_its_tag_and_its_body() {
    let frames = [frame(1, vec![0xa0]), frame(8, vec![0; 0x01_02_03])];
    let written = write_all(&frames);
    assert_eq!(written[..6], [2, 0, 0, 0, 1, 0xa0]);
    assert_eq!(written[6..11], [0x04, 0x02, 0x01, 0, 8]);
    assert_eq!(written.len(), 6 + 5 + 0x01_02_03);
}

#[test]
fn a_tool_frame_under_a_tag_outside_the_registry_ends_the_session() {
    // The tool's push frame carries the log shape's tag 0, which the session's
    // registry does not hold; its seal frame after it is never read.
    let (frames, closed) = read_all(TOOL_CLIENT_PUSH);
    assert_eq!(frames, [tool_read_request()]);
    assert_eq!(closed, Closed::Protocol(FrameError::UnknownTag(0)));
}

/// Serves `bytes`, then fails the test if it is read again: the adapter must
/// stop at the point the test names.
struct ThenStop<'a> {
    bytes: &'a [u8],
    what: &'static str,
}

impl Read for ThenStop<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        assert!(!self.bytes.is_empty(), "the adapter read {}", self.what);
        let length = buffer.len().min(self.bytes.len());
        buffer[..length].copy_from_slice(&self.bytes[..length]);
        self.bytes = &self.bytes[length..];
        Ok(length)
    }
}

#[test]
fn a_length_prefix_over_64_mib_ends_the_session_before_anything_is_allocated() {
    // The prefix is read into four bytes and checked; the buffer for a body
    // exists only after the check. Here the check is seen to come first: the
    // reader serves the prefix and fails the test if the adapter reads on.
    for size in [MAX_FRAME_BYTES as u32 + 1, u32::MAX] {
        let prefix = size.to_le_bytes();
        let reader = ThenStop {
            bytes: &prefix,
            what: "past an over-cap length prefix",
        };
        let end = byte_stream(reader, io::sink());
        let error = FrameError::TooLarge {
            size: size as usize,
        };
        assert_eq!(end.recv(), Err(Closed::Protocol(error)), "prefix {size}");
        assert_eq!(end.recv(), Err(Closed::Protocol(error)));
        let refused = frame(1, vec![0xa0]);
        match end.send(refused.clone(), Lane::Call) {
            Err(SendError::Closed(returned, Closed::Protocol(reported))) => {
                assert_eq!(returned, refused);
                assert_eq!(reported, error);
            }
            result => panic!("the session has ended, got {result:?}"),
        }
    }
}

#[test]
fn a_length_prefix_of_exactly_64_mib_is_read_whole() {
    let mut bytes = Vec::with_capacity(4 + MAX_FRAME_BYTES);
    bytes.extend((MAX_FRAME_BYTES as u32).to_le_bytes());
    bytes.push(Tag::SessionReply.byte());
    bytes.resize(4 + MAX_FRAME_BYTES, 0x5a);
    let end = byte_stream(&bytes[..], io::sink());
    let received = end.recv().expect("a 64 MiB frame is accepted");
    assert_eq!(received.tag, 2);
    assert_eq!(received.size(), MAX_FRAME_BYTES);
    assert!(received.body == bytes[5..], "the body arrives whole");
    assert_eq!(end.recv(), Err(Closed::Peer));
}

#[test]
fn a_zero_length_prefix_ends_the_session() {
    let (frames, closed) = read_all(&[0u8, 0, 0, 0, 1, 0xa0][..]);
    assert!(frames.is_empty());
    assert_eq!(closed, Closed::Protocol(FrameError::Empty));
}

#[test]
fn a_tag_outside_the_registry_ends_the_session_before_its_body_is_read() {
    for tag in [0, 9, 15, 16, 24, 31, 32, 255] {
        let expected = Tag::from_byte(tag).expect_err("outside the registry");
        let good = write_all(&[frame(1, vec![0xa0])]);
        let bad_head = [5, 0, 0, 0, tag];
        let reader = good.as_slice().chain(ThenStop {
            bytes: &bad_head,
            what: "the body of a frame whose tag ends the session",
        });
        let (frames, closed) = read_all(reader);
        assert_eq!(frames, [frame(1, vec![0xa0])], "tag {tag}");
        assert_eq!(closed, Closed::Protocol(expected), "tag {tag}");
    }
}

#[test]
fn end_of_stream_between_frames_is_the_peers_closure_and_inside_one_a_truncation() {
    let whole = write_all(&[frame(1, vec![1, 2, 3])]);
    assert_eq!(whole.len(), 8);
    for cut in 0..whole.len() {
        let (frames, closed) = read_all(&whole[..cut]);
        assert!(frames.is_empty(), "cut at {cut}");
        let expected = if cut == 0 {
            Closed::Peer
        } else {
            Closed::Stream(ErrorKind::UnexpectedEof)
        };
        assert_eq!(closed, expected, "cut at {cut}");
    }
    let (frames, closed) = read_all(&whole[..]);
    assert_eq!(frames, [frame(1, vec![1, 2, 3])]);
    assert_eq!(closed, Closed::Peer);
}

/// Fails with `error` on every call, after `interrupted` interruptions.
struct Failing {
    interrupted: usize,
    error: ErrorKind,
    bytes: &'static [u8],
}

impl Read for Failing {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.interrupted > 0 {
            self.interrupted -= 1;
            return Err(ErrorKind::Interrupted.into());
        }
        if self.bytes.is_empty() {
            return Err(self.error.into());
        }
        let length = buffer.len().min(self.bytes.len());
        buffer[..length].copy_from_slice(&self.bytes[..length]);
        self.bytes = &self.bytes[length..];
        Ok(length)
    }
}

impl Write for Failing {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        if self.interrupted > 0 {
            self.interrupted -= 1;
            return Err(ErrorKind::Interrupted.into());
        }
        Err(self.error.into())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_failing_stream_ends_the_session_and_an_interrupted_one_does_not() {
    let reader = Failing {
        interrupted: 3,
        error: ErrorKind::ConnectionReset,
        bytes: &[2, 0, 0, 0, 1, 0xa0],
    };
    let (frames, closed) = read_all(reader);
    assert_eq!(frames, [frame(1, vec![0xa0])], "interruptions are retried");
    assert_eq!(closed, Closed::Stream(ErrorKind::ConnectionReset));

    let writer = Failing {
        interrupted: 3,
        error: ErrorKind::BrokenPipe,
        bytes: &[],
    };
    let end = byte_stream(io::empty(), writer);
    let sent = frame(1, vec![0xa0]);
    let broken = Closed::Stream(ErrorKind::BrokenPipe);
    for _ in 0..2 {
        match end.send(sent.clone(), Lane::Call) {
            Err(SendError::Closed(returned, reported)) => {
                assert_eq!(returned, sent);
                assert_eq!(reported, broken);
            }
            result => panic!("a broken stream ends the session, got {result:?}"),
        }
    }
    assert_eq!(end.recv(), Err(broken));
}

/// Serves the first two bytes of a length prefix, then panics; as a writer,
/// panics at once.
struct PanicsMidFrame {
    served: bool,
}

impl Read for PanicsMidFrame {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        assert!(!self.served, "the stream fails in the middle of a frame");
        self.served = true;
        buffer[..2].copy_from_slice(&[2, 0]);
        Ok(2)
    }
}

impl Write for PanicsMidFrame {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        panic!("the stream fails in the middle of a frame");
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_stream_that_panics_mid_frame_ends_the_session_instead_of_losing_its_framing() {
    let lost = Closed::Stream(ErrorKind::Other);
    let reading = byte_stream(PanicsMidFrame { served: false }, io::sink());
    let panicked = panic::catch_unwind(AssertUnwindSafe(|| reading.recv()));
    assert!(panicked.is_err(), "the stream's panic reaches the caller");
    assert_eq!(
        reading.recv(),
        Err(lost),
        "the half-read prefix is never resumed"
    );

    let writing = byte_stream(io::empty(), PanicsMidFrame { served: false });
    let sent = frame(1, vec![0xa0]);
    let panicked = panic::catch_unwind(AssertUnwindSafe(|| writing.send(sent.clone(), Lane::Call)));
    assert!(panicked.is_err(), "the stream's panic reaches the caller");
    match writing.send(sent.clone(), Lane::Call) {
        Err(SendError::Closed(returned, reported)) => {
            assert_eq!(returned, sent);
            assert_eq!(reported, lost, "the half-written frame is never continued");
        }
        result => panic!("the session has ended, got {result:?}"),
    }
}

#[test]
fn closing_an_end_ends_the_peers_stream_after_what_it_sent() {
    for drop_instead in [false, true] {
        let (mut peer_reads, end_writes) = io::pipe().expect("an OS pipe");
        let (end_reads, _peer_writes) = io::pipe().expect("an OS pipe");
        let end = byte_stream(end_reads, end_writes);
        let sent = frame(2, vec![0xa0]);
        end.send(sent.clone(), Lane::Call)
            .expect("the pipe takes a frame");
        if drop_instead {
            drop(end);
        } else {
            end.close();
        }
        // The peer's raw stream holds the frame, then ends.
        let mut bytes = Vec::new();
        peer_reads.read_to_end(&mut bytes).expect("the stream ends");
        assert_eq!(bytes, write_all(&[sent]));
    }
}

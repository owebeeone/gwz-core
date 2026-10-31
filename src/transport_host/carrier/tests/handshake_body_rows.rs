//! The handshake's bodies: one encoding per value, and every other encoding refused (step 2.3d-1).
use super::super::handshake::{
    Action, BodyFault, Control, Hello, OpenRequest, Opened, SERVER_UNAVAILABLE,
    SERVER_VERSION_MISMATCH, SHOWN_BYTES, State, escape,
};
use super::handshake_support::{hello, state};
use crate::protocol::generated::GwzErrorCode;

fn opened() -> Opened {
    Opened {
        protocol: 1,
        server: "g".into(),
        core: "c".into(),
        server_pid: 5,
        open_sessions: 0,
    }
}

#[test]
fn bodies_encode_to_their_golden_bytes() {
    let small = Hello {
        protocol_min: 1,
        protocol_max: 2,
        server: "g".into(),
        core: "c".into(),
        server_pid: 5,
    };
    assert_eq!(
        small.encode(),
        [0xa5, 1, 1, 2, 2, 3, 0x61, b'g', 4, 0x61, b'c', 5, 5]
    );
    assert_eq!(
        opened().encode(),
        [0xa5, 1, 1, 2, 0x61, b'g', 3, 0x61, b'c', 4, 5, 5, 0]
    );
    let stop = Control {
        action: Action::Stop,
        force: true,
    };
    assert_eq!(stop.encode(), [0xa2, 1, 1, 2, 0xf5]);
    let status = Control {
        action: Action::Status,
        force: false,
    };
    assert_eq!(status.encode(), [0xa1, 1, 0]);
    let bare = State {
        server_pid: 5,
        open_sessions: 0,
        max_sessions: 64,
        idle_exit: None,
        log: None,
        stopping: false,
    };
    assert_eq!(bare.encode(), [0xa4, 1, 5, 2, 0, 3, 0x18, 0x40, 6, 0xf4]);
    let full = State {
        idle_exit: Some(300),
        log: Some("l".into()),
        stopping: true,
        ..bare
    };
    assert_eq!(
        full.encode(),
        [
            0xa6, 1, 5, 2, 0, 3, 0x18, 0x40, 4, 0x19, 0x01, 0x2c, 5, 0x61, b'l', 6, 0xf5
        ]
    );
    let plain = OpenRequest {
        protocol: 1,
        client: "c".into(),
        limits: None,
    };
    assert_eq!(
        plain.encode(),
        [0xa4, 1, 1, 2, 0x61, b'c', 5, 0xa0, 6, 0xf5]
    );
    let limited = OpenRequest {
        limits: Some(vec![0xa1, 1, 5]),
        ..plain
    };
    assert_eq!(
        limited.encode(),
        [0xa5, 1, 1, 2, 0x61, b'c', 4, 0xa1, 1, 5, 5, 0xa0, 6, 0xf5]
    );
}

#[test]
fn bodies_round_trip_and_reencode_identically() {
    let bodies = [
        hello().encode(),
        opened().encode(),
        state().encode(),
        Control {
            action: Action::Stop,
            force: true,
        }
        .encode(),
        OpenRequest {
            protocol: 2,
            client: "gwz 1.1.0".into(),
            limits: Some(vec![0xa2, 1, 0x18, 0x40, 2, 0x19, 0x04, 0x00]),
        }
        .encode(),
    ];
    let hello_back = Hello::decode(&bodies[0]).unwrap();
    assert_eq!(hello_back, hello());
    assert_eq!(hello_back.encode(), bodies[0]);
    let opened_back = Opened::decode(&bodies[1]).unwrap();
    assert_eq!(opened_back, opened());
    assert_eq!(opened_back.encode(), bodies[1]);
    let state_back = State::decode(&bodies[2]).unwrap();
    assert_eq!(state_back, state());
    assert_eq!(state_back.encode(), bodies[2]);
    let control_back = Control::decode(&bodies[3]).unwrap();
    assert_eq!(control_back.encode(), bodies[3]);
    let open_back = OpenRequest::decode(&bodies[4]).unwrap();
    assert_eq!(open_back.protocol, 2);
    assert_eq!(open_back.encode(), bodies[4]);
}

/// `Hello` bytes for min 1, max 2, server "g", core "c", pid 5, spelled as the row needs.
const HELLO: [u8; 13] = [0xa5, 1, 1, 2, 2, 3, 0x61, b'g', 4, 0x61, b'c', 5, 5];

fn hello_with(edit: impl FnOnce(&mut Vec<u8>)) -> Result<Hello, BodyFault> {
    let mut bytes = HELLO.to_vec();
    edit(&mut bytes);
    Hello::decode(&bytes)
}

#[test]
fn non_shortest_heads_are_rejected() {
    // Key 1 as 0x18 0x01, the value 1 as 0x18 0x01, the map's count as 0xb8 0x05, a text length as 0x78 0x01.
    assert_eq!(
        hello_with(|b| {
            b.splice(1..2, [0x18, 1]);
        }),
        Err(BodyFault::NonShortest)
    );
    assert_eq!(
        hello_with(|b| {
            b.splice(2..3, [0x18, 1]);
        }),
        Err(BodyFault::NonShortest)
    );
    assert_eq!(
        hello_with(|b| {
            b.splice(0..1, [0xb8, 5]);
        }),
        Err(BodyFault::NonShortest)
    );
    assert_eq!(
        hello_with(|b| {
            b.splice(6..7, [0x78, 1]);
        }),
        Err(BodyFault::NonShortest)
    );
    // A 16-bit head for a value that fits one byte.
    assert_eq!(
        hello_with(|b| {
            b.splice(12..13, [0x19, 0, 5]);
        }),
        Err(BodyFault::NonShortest)
    );
}

#[test]
fn unsorted_and_duplicate_keys_are_rejected() {
    assert_eq!(
        hello_with(|b| b.swap(1, 3)),
        Err(BodyFault::Unsorted(1)),
        "keys 2 then 1"
    );
    assert_eq!(
        hello_with(|b| b[3] = 1),
        Err(BodyFault::Duplicate(1)),
        "key 1 twice"
    );
}

#[test]
fn bytes_after_the_item_are_rejected() {
    assert_eq!(hello_with(|b| b.push(0)), Err(BodyFault::Trailing));
    assert_eq!(
        hello_with(|b| {
            b.pop();
        }),
        Err(BodyFault::Malformed)
    );
}

#[test]
fn indefinite_lengths_tags_floats_and_other_shapes_are_rejected() {
    // An indefinite map, a tagged value, a float, a byte string where text goes, text that is not UTF-8, an array.
    assert_eq!(
        hello_with(|b| {
            b[0] = 0xbf;
            b.push(0xff);
        }),
        Err(BodyFault::Malformed)
    );
    assert_eq!(
        hello_with(|b| {
            b.splice(12..13, [0xc1, 5]);
        }),
        Err(BodyFault::Malformed)
    );
    assert_eq!(
        hello_with(|b| {
            b.splice(12..13, [0xf9, 0, 0]);
        }),
        Err(BodyFault::Malformed)
    );
    assert_eq!(
        hello_with(|b| b[6] = 0x41),
        Err(BodyFault::WrongType(3)),
        "a byte string is not text"
    );
    assert_eq!(
        hello_with(|b| b[7] = 0xff),
        Err(BodyFault::Malformed),
        "text must be UTF-8"
    );
    assert_eq!(Hello::decode(&[0x80]), Err(BodyFault::Malformed));
}

#[test]
fn nested_items_are_held_to_the_same_rule() {
    let with_limits = |limits: &[u8]| {
        let mut body = vec![0xa5, 1, 1, 2, 0x61, b'c', 4];
        body.extend_from_slice(limits);
        body.extend_from_slice(&[5, 0xa0, 6, 0xf5]);
        OpenRequest::decode(&body)
    };
    assert!(with_limits(&[0xa1, 1, 5]).is_ok());
    assert_eq!(
        with_limits(&[0xa1, 0x18, 1, 5]),
        Err(BodyFault::NonShortest)
    );
    assert_eq!(
        with_limits(&[0xa2, 2, 0, 1, 0]),
        Err(BodyFault::Unsorted(1))
    );
    assert_eq!(
        with_limits(&[0xa2, 1, 0, 1, 0]),
        Err(BodyFault::Duplicate(1))
    );
    assert_eq!(with_limits(&[0xbf, 0xff]), Err(BodyFault::Malformed));
}

#[test]
fn unknown_missing_mistyped_and_out_of_range_keys_are_refused() {
    assert_eq!(
        hello_with(|b| {
            b[0] = 0xa6;
            b.extend_from_slice(&[7, 0]);
        }),
        Err(BodyFault::Unknown(7))
    );
    assert_eq!(
        hello_with(|b| {
            b[0] = 0xa4;
            b.truncate(8);
            b.extend_from_slice(&[5, 5]);
        }),
        Err(BodyFault::Missing(4))
    );
    assert_eq!(
        hello_with(|b| {
            b.splice(2..3, [0x61, b'x']);
        }),
        Err(BodyFault::WrongType(1)),
        "text where a number goes"
    );
    // A range that starts at 0, one that is empty, a number past u32.
    assert_eq!(hello_with(|b| b[2] = 0), Err(BodyFault::Range(1)));
    assert_eq!(hello_with(|b| b[2] = 3), Err(BodyFault::Range(2)));
    assert_eq!(
        hello_with(|b| {
            b.splice(12..13, [0x1b, 0, 0, 0, 1, 0, 0, 0, 0]);
        }),
        Err(BodyFault::Range(5))
    );
    // `force: false` is an absent key, and `action` is 0 or 1.
    assert_eq!(
        Control::decode(&[0xa2, 1, 0, 2, 0xf4]),
        Err(BodyFault::Range(2))
    );
    assert_eq!(Control::decode(&[0xa1, 1, 2]), Err(BodyFault::Range(1)));
}

#[test]
fn escape_shows_printable_text_and_escapes_the_rest() {
    assert_eq!(escape(b"hello", SHOWN_BYTES), "hello");
    assert_eq!(escape(b"a\\b", SHOWN_BYTES), "a\\\\b");
    assert_eq!(escape(b"\x7f\x1f ~", SHOWN_BYTES), "\\x7f\\x1f ~");
    assert_eq!(
        escape(b"\x1b[31m\r\n\0\xff", SHOWN_BYTES),
        "\\x1b[31m\\x0d\\x0a\\x00\\xff"
    );
    assert_eq!(escape(&[b'x'; 40], 32), format!("{}...", "x".repeat(32)));
    assert_eq!(escape(&[b'x'; 32], 32), "x".repeat(32));
}

#[test]
fn the_error_codes_are_the_generated_schemas() {
    assert_eq!(
        SERVER_UNAVAILABLE as i64,
        GwzErrorCode::ServerUnavailable.wire()
    );
    assert_eq!(
        SERVER_VERSION_MISMATCH as i64,
        GwzErrorCode::ServerVersionMismatch.wire()
    );
}

//! The wire adapter: a message's frame, the close announcement and the shape of a `SessionOpen` the client placement
//! carries.
use super::super::{
    CloseCode, ProtocolFault, SessionOpenRefusal, validate_session_open,
    wire::{
        CONTROL_TAG, DATA_TAG, MAX_TRANSPORT_FRAME, close_frame, decode_attachment,
        encode_attachment, is_close, parse_close,
    },
};
use super::support::*;
use gwz_session_contract::Frame;
use gwz_transport::protocol::MessageKind;

#[test]
fn an_attachment_round_trips_through_its_frame() {
    for message in [
        data(3, 0, vec![9; 1000]),
        window(3, 77),
        cancel(3),
        close(3, 5),
    ] {
        let frame = encode_attachment(&message).unwrap();
        // Data and opens travel under tag 16, everything else under 17 (D-N).
        let expected = if message.1.kind == MessageKind::Data {
            DATA_TAG
        } else {
            CONTROL_TAG
        };
        assert_eq!(frame.tag, expected);
        assert!(frame.size() <= MAX_TRANSPORT_FRAME);
        assert_eq!(decode_attachment(&frame).unwrap(), message);
    }
}

#[test]
fn a_frame_that_does_not_decode_is_a_message_fault() {
    let good = encode_attachment(&window(1, 1)).unwrap();
    for body in [
        Vec::new(),
        vec![200],
        vec![3, b'a', b'b'],
        [good.body.clone(), vec![0]].concat(),
        good.body[..good.body.len() - 1].to_vec(),
    ] {
        let frame = Frame {
            tag: good.tag,
            body,
        };
        assert_eq!(decode_attachment(&frame), Err(ProtocolFault::Message));
    }
    // A message under the other class's tag is refused: it would be counted and ordered in the wrong lane.
    let data_frame = encode_attachment(&data(1, 0, vec![1])).unwrap();
    for (message_frame, wrong) in [(&good, DATA_TAG), (&data_frame, CONTROL_TAG)] {
        let frame = Frame {
            tag: wrong,
            body: message_frame.body.clone(),
        };
        assert_eq!(decode_attachment(&frame), Err(ProtocolFault::Message));
    }
    // A request name the mux would refuse is refused here too.
    let mut bad = window(1, 1);
    bad.0 = String::new();
    assert_eq!(encode_attachment(&bad), Err(ProtocolFault::Message));
    bad.0 = "x".repeat(129);
    assert_eq!(encode_attachment(&bad), Err(ProtocolFault::Message));
}

#[test]
fn the_close_announcement_round_trips_and_an_unknown_code_is_a_fault() {
    for code in [
        CloseCode::Normal,
        CloseCode::Dropped,
        CloseCode::Cancelled,
        CloseCode::ProtocolFault,
    ] {
        let frame = close_frame(code);
        // A control frame whose request-name length is zero: no message has one.
        assert_eq!(frame.tag, CONTROL_TAG);
        assert!(is_close(&frame));
        assert_eq!(parse_close(&frame), Ok(code));
    }
    assert!(!is_close(&encode_attachment(&window(1, 1)).unwrap()));
    for body in [vec![], vec![0], vec![0, 9], vec![0, 0, 0], vec![7, 0]] {
        let frame = Frame {
            tag: CONTROL_TAG,
            body,
        };
        assert_eq!(parse_close(&frame), Err(ProtocolFault::Close));
    }
    assert_eq!(
        parse_close(&Frame {
            tag: DATA_TAG,
            body: vec![0, 0]
        }),
        Err(ProtocolFault::Close)
    );
}

/// Deterministic CBOR for a map of unsigned keys, enough to build a `SessionOpen`.
mod cbor {
    pub fn uint(n: u64) -> Vec<u8> {
        head(0, n)
    }
    pub fn text(s: &str) -> Vec<u8> {
        [head(3, s.len() as u64), s.as_bytes().to_vec()].concat()
    }
    pub fn bytes(b: &[u8]) -> Vec<u8> {
        [head(2, b.len() as u64), b.to_vec()].concat()
    }
    pub fn array(items: &[Vec<u8>]) -> Vec<u8> {
        [head(4, items.len() as u64), items.concat()].concat()
    }
    pub fn map(entries: &[(u64, Vec<u8>)]) -> Vec<u8> {
        let mut out = head(5, entries.len() as u64);
        for (key, value) in entries {
            out.extend(uint(*key));
            out.extend(value);
        }
        out
    }
    fn head(major: u8, n: u64) -> Vec<u8> {
        let m = major << 5;
        match n {
            0..=23 => vec![m | n as u8],
            24..=255 => vec![m | 24, n as u8],
            256..=65535 => [vec![m | 25], (n as u16).to_be_bytes().to_vec()].concat(),
            _ => [vec![m | 27], n.to_be_bytes().to_vec()].concat(),
        }
    }
}

pub(super) const CANARY: &[u8] = b"CANARY-GH_TOKEN-7f3a9c";

/// The environment entry list of the parked schema, carrying the canary.
pub(super) fn environment_with_canary() -> Vec<u8> {
    cbor::array(&[cbor::map(&[
        (1, cbor::bytes(b"GH_TOKEN")),
        (2, cbor::bytes(CANARY)),
    ])])
}

/// A `SessionOpen` of the client placement: protocol and client name, no environment.
pub(super) fn session_open_without_environment() -> Vec<u8> {
    cbor::map(&[(1, cbor::uint(1)), (2, cbor::text("gwz 1.1.0"))])
}

pub(super) fn session_open_with_environment() -> Vec<u8> {
    cbor::map(&[
        (1, cbor::uint(1)),
        (2, cbor::text("gwz 1.1.0")),
        (3, environment_with_canary()),
    ])
}

#[test]
fn a_session_open_with_no_environment_is_accepted() {
    assert_eq!(
        validate_session_open(&session_open_without_environment()),
        Ok(())
    );
    // The optional session limits (key 4) are not the environment.
    let with_limits = cbor::map(&[
        (1, cbor::uint(1)),
        (2, cbor::text("gwz-py 1.1.0")),
        (4, cbor::map(&[(1, cbor::uint(8))])),
    ]);
    assert_eq!(validate_session_open(&with_limits), Ok(()));
}

#[test]
fn a_session_open_carrying_any_environment_field_is_refused() {
    let base = [(1, cbor::uint(1)), (2, cbor::text("gwz"))];
    let with = |extra: (u64, Vec<u8>)| cbor::map(&[base[0].clone(), base[1].clone(), extra]);
    // 3 is the environment; 5 the process attributes (umask, OpenSSL, off switch), the client's process facts; 6 the
    // host-environment marker.
    for key in [3u64, 5, 6] {
        assert_eq!(
            validate_session_open(&with((key, cbor::uint(1)))),
            Err(SessionOpenRefusal::ForbiddenField(key)),
            "key {key}"
        );
    }
    // Any key the shape does not name is refused, so a field added later is not carried by accident.
    for key in [0u64, 7, 99, 1 << 40] {
        assert_eq!(
            validate_session_open(&with((key, cbor::uint(1)))),
            Err(SessionOpenRefusal::ForbiddenField(key)),
            "key {key}"
        );
    }
    assert_eq!(
        validate_session_open(&session_open_with_environment()),
        Err(SessionOpenRefusal::ForbiddenField(3))
    );
}

#[test]
fn a_session_open_of_the_wrong_shape_is_refused() {
    use SessionOpenRefusal::*;
    assert_eq!(validate_session_open(&cbor::uint(1)), Err(NotAMap));
    assert_eq!(validate_session_open(&[]), Err(Malformed));
    assert_eq!(
        validate_session_open(&cbor::map(&[(2, cbor::text("gwz"))])),
        Err(MissingField(1))
    );
    assert_eq!(
        validate_session_open(&cbor::map(&[(1, cbor::uint(1))])),
        Err(MissingField(2))
    );
    let duplicate = [
        vec![0xa3],
        cbor::uint(1),
        cbor::uint(1),
        cbor::uint(1),
        cbor::uint(1),
        cbor::uint(2),
        cbor::text("gwz"),
    ]
    .concat();
    assert_eq!(validate_session_open(&duplicate), Err(DuplicateField(1)));
    // Truncated, trailing bytes, an indefinite-length map and a length that claims more than the body holds.
    let good = session_open_without_environment();
    assert_eq!(
        validate_session_open(&good[..good.len() - 1]),
        Err(Malformed)
    );
    assert_eq!(
        validate_session_open(&[good.clone(), vec![0]].concat()),
        Err(Malformed)
    );
    assert_eq!(
        validate_session_open(&[0xbf, 0x01, 0x01, 0xff]),
        Err(Malformed)
    );
    assert_eq!(
        validate_session_open(&[
            0xa1, 0x01, 0x5b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff
        ]),
        Err(Malformed)
    );
    // A key that is not an unsigned integer.
    assert_eq!(
        validate_session_open(&[0xa1, 0x61, b'a', 0x01]),
        Err(Malformed)
    );
}

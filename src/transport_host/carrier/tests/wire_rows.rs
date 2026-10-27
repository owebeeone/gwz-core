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
    pub fn boolean(value: bool) -> Vec<u8> {
        vec![if value { 0xf5 } else { 0xf4 }]
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

/// A `SessionOpen` of the client placement, in the shape `GwzSplitCommandChannel.md` §3.1 freezes: protocol and client
/// name, no environment, `attributes` with `umask` and `openssl` absent, `host_environment` true.
pub(super) fn session_open_without_environment() -> Vec<u8> {
    frozen_with(&[])
}

/// The frozen shape with `extra` entries added (keys not already in it).
fn frozen_with(extra: &[(u64, Vec<u8>)]) -> Vec<u8> {
    let mut entries = vec![
        (1, cbor::uint(1)),
        (2, cbor::text("gwz 1.1.0")),
        (5, cbor::map(&[])),
        (6, cbor::boolean(true)),
    ];
    entries.extend(extra.iter().cloned());
    entries.sort_by_key(|(key, _)| *key);
    cbor::map(&entries)
}

/// The frozen shape with the entry at `key` replaced (or removed, for `None`).
fn frozen_changed(key: u64, value: Option<Vec<u8>>) -> Vec<u8> {
    let mut entries = vec![
        (1, cbor::uint(1)),
        (2, cbor::text("gwz 1.1.0")),
        (5, cbor::map(&[])),
        (6, cbor::boolean(true)),
    ];
    entries.retain(|(k, _)| *k != key);
    entries.extend(value.map(|value| (key, value)));
    entries.sort_by_key(|(key, _)| *key);
    cbor::map(&entries)
}

/// Every shape of `SessionOpen` the frozen one is not, named: each must be refused wherever the shape is checked.
pub(super) fn refused_session_opens() -> Vec<(&'static str, Vec<u8>)> {
    let umask = cbor::map(&[(1, cbor::uint(0o22))]);
    let openssl = cbor::map(&[(3, cbor::text("OpenSSL 3.0"))]);
    let off_true = cbor::map(&[(2, cbor::boolean(true))]);
    vec![
        (
            "the old shape",
            cbor::map(&[(1, cbor::uint(1)), (2, cbor::text("gwz"))]),
        ),
        ("no attributes", frozen_changed(5, None)),
        ("no marker", frozen_changed(6, None)),
        (
            "marker false",
            frozen_changed(6, Some(cbor::boolean(false))),
        ),
        ("umask", frozen_changed(5, Some(umask))),
        ("openssl", frozen_changed(5, Some(openssl))),
        ("transport_off true", frozen_changed(5, Some(off_true))),
        ("an environment", session_open_with_environment()),
        ("an unknown key", frozen_with(&[(7, cbor::uint(1))])),
    ]
}

/// The frames on a raw wire, as `(tag, body)`.
pub(super) fn frames_of(mut bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
    let mut frames = Vec::new();
    while bytes.len() >= 5 {
        let size = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
        frames.push((bytes[4], bytes[5..4 + size].to_vec()));
        bytes = &bytes[4 + size..];
    }
    assert!(bytes.is_empty(), "a whole number of frames");
    frames
}

pub(super) fn session_open_with_environment() -> Vec<u8> {
    frozen_with(&[(3, environment_with_canary())])
}

#[test]
fn the_frozen_session_open_is_accepted() {
    assert_eq!(
        validate_session_open(&session_open_without_environment()),
        Ok(())
    );
    // The optional session limits (key 4) are not the environment.
    let with_limits = frozen_with(&[(4, cbor::map(&[(1, cbor::uint(8))]))]);
    assert_eq!(validate_session_open(&with_limits), Ok(()));
    // `transport_off` may be present when it is false.
    let off_false = frozen_changed(5, Some(cbor::map(&[(2, cbor::boolean(false))])));
    assert_eq!(validate_session_open(&off_false), Ok(()));
}

#[test]
fn every_other_combination_of_the_session_open_is_refused() {
    use SessionOpenRefusal::*;
    // The old shape, without `attributes` and the marker, is not the frozen one.
    assert_eq!(
        validate_session_open(&cbor::map(&[(1, cbor::uint(1)), (2, cbor::text("gwz"))])),
        Err(MissingField(5))
    );
    assert_eq!(
        validate_session_open(&frozen_changed(5, None)),
        Err(MissingField(5))
    );
    assert_eq!(
        validate_session_open(&frozen_changed(6, None)),
        Err(MissingField(6))
    );
    // The marker must be `true`, and a boolean.
    for marker in [cbor::boolean(false), cbor::uint(1), cbor::text("true")] {
        assert_eq!(
            validate_session_open(&frozen_changed(6, Some(marker))),
            Err(ForbiddenValue(6))
        );
    }
    // `attributes` is a map that names nothing but a false `transport_off`.
    let umask = cbor::map(&[(1, cbor::uint(0o22))]);
    let openssl = cbor::map(&[(3, cbor::text("OpenSSL 3.0"))]);
    let off_true = cbor::map(&[(2, cbor::boolean(true))]);
    let unknown = cbor::map(&[(9, cbor::boolean(false))]);
    let off_not_bool = cbor::map(&[(2, cbor::uint(0))]);
    let both = cbor::map(&[(1, cbor::uint(0o22)), (2, cbor::boolean(false))]);
    for attributes in [
        umask,
        openssl,
        off_true,
        unknown,
        off_not_bool,
        both,
        cbor::uint(0),
        cbor::array(&[]),
    ] {
        assert_eq!(
            validate_session_open(&frozen_changed(5, Some(attributes))),
            Err(ForbiddenValue(5))
        );
    }
    // The environment (3) and any key the shape does not name are refused.
    for key in [3u64, 0, 7, 99, 1 << 40] {
        assert_eq!(
            validate_session_open(&frozen_with(&[(key, cbor::uint(1))])),
            Err(ForbiddenField(key)),
            "key {key}"
        );
    }
    assert_eq!(
        validate_session_open(&session_open_with_environment()),
        Err(ForbiddenField(3))
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

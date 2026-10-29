//! The frozen lane body with the transport's own envelopes (client
//! placement projection): tag by kind and the codec's round trip
//! (`GwzSplitCommandChannel.md` §3.3).

use gwz_transport::protocol::{Envelope, MessageKind, Open, Opened};

use super::{TRANSPORT_CONTROL, TRANSPORT_DATA, decode_lane, encode_lane};

fn envelope(kind: MessageKind) -> Envelope {
    Envelope {
        version: 1,
        session_id: "binding-1".to_owned(),
        stream_id: 1,
        kind,
        open: (kind == MessageKind::Open).then(Open::default),
        opened: (kind == MessageKind::Opened).then(Opened::default),
        ..Envelope::default()
    }
}

#[test]
fn an_open_takes_tag_16_its_answer_tag_17_and_both_round_trip() {
    let open = encode_lane("request-1", envelope(MessageKind::Open));
    assert_eq!(open.tag, TRANSPORT_DATA);
    assert_eq!(
        decode_lane(&open.body).unwrap(),
        ("request-1".to_owned(), envelope(MessageKind::Open))
    );
    let opened = encode_lane("request-1", envelope(MessageKind::Opened));
    assert_eq!(opened.tag, TRANSPORT_CONTROL);
    for kind in [MessageKind::Data, MessageKind::CheckIdentity] {
        assert_eq!(
            encode_lane("r", envelope(kind)).tag,
            TRANSPORT_DATA,
            "{kind:?}"
        );
    }
    for kind in [
        MessageKind::Bind,
        MessageKind::Window,
        MessageKind::Closed,
        MessageKind::Cancel,
    ] {
        assert_eq!(
            encode_lane("r", envelope(kind)).tag,
            TRANSPORT_CONTROL,
            "{kind:?}"
        );
    }
}

#[test]
fn a_body_that_does_not_decode_is_refused() {
    assert!(decode_lane(&[0xff]).is_err());
}

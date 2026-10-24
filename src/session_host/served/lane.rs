//! The transport lane's body, `TransportLaneFrame { request_id, envelope }`,
//! in the client placement's projection (gwz-dev
//! `dev-docs/GwzSplitCommandChannel.md` §3.3; step 2.2b).
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | Tag 17 for every kind the transport charges to its control reserve, tag 16 for `data`, `open` and `check_identity` | §3.3 "Tag by kind" (`gwz-transport/src/mux/mod.rs` `enqueue`) |
//! | A body that does not decode is a protocol error | session design §3 |
//! | The target is `(request_id, session_id, stream_id)` | §3.3 "Writer interface" |

use gwz_session_contract::Frame;
use gwz_session_host::{LaneFrame, LaneKind, ProtocolError, StreamTarget, lane_tag};
use gwz_transport::protocol::{Envelope, MessageKind};

/// Whether the transport charges `kind` to its control reserve: every kind
/// but `data`, `open` and `check_identity`.
pub(crate) fn control(kind: MessageKind) -> bool {
    !matches!(
        kind,
        MessageKind::Data | MessageKind::Open | MessageKind::CheckIdentity
    )
}

fn target(request_id: &str, envelope: &Envelope) -> StreamTarget {
    StreamTarget {
        request_id: request_id.to_owned(),
        session_id: envelope.session_id.clone(),
        stream_id: envelope.stream_id.max(0) as u64,
    }
}

/// Encodes `envelope` for the call `request_id` as a lane frame, under the
/// tag its kind takes, with its target.
pub(crate) fn encode_lane(request_id: &str, envelope: Envelope) -> (Frame, StreamTarget) {
    let tag = lane_tag(control(envelope.kind));
    let target = target(request_id, &envelope);
    let body = crate::TransportLaneFrame {
        request_id: request_id.to_owned(),
        envelope,
    };
    (
        Frame {
            tag,
            body: crate::encode(&body.to_cbor()),
        },
        target,
    )
}

/// Decodes a lane frame's body into the header the lane routes by.
///
/// # Errors
///
/// A body that does not decode.
pub(crate) fn decode_lane(frame: &Frame) -> Result<LaneFrame, ProtocolError> {
    let body = crate::TransportLaneFrame::decode(&frame.body)
        .map_err(|_| ProtocolError("a TransportLaneFrame body that does not decode".to_owned()))?;
    let envelope = &body.envelope;
    let kind = match envelope.kind {
        MessageKind::Data => {
            LaneKind::Data(envelope.data.as_ref().map_or(0, |data| data.payload.len()))
        }
        MessageKind::Window => LaneKind::Window(
            envelope
                .window
                .as_ref()
                .map_or(0, |window| window.max_offset.max(0) as u64),
        ),
        kind => LaneKind::Other {
            kind: kind.wire(),
            control: control(kind),
        },
    };
    Ok(LaneFrame {
        tag: frame.tag,
        target: target(&body.request_id, envelope),
        kind,
        body: frame.body.clone(),
    })
}

/// The envelope in a lane frame's body, for the call's consumer.
///
/// # Errors
///
/// A body that does not decode.
#[allow(dead_code, reason = "the carrier-backed handle (2.2c) consumes it")]
pub(crate) fn envelope(body: &[u8]) -> Result<Envelope, ProtocolError> {
    crate::TransportLaneFrame::decode(body)
        .map(|frame| frame.envelope)
        .map_err(|_| ProtocolError("a TransportLaneFrame body that does not decode".to_owned()))
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests;
    }
}

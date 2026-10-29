//! The transport lane's frozen body, `TransportLaneFrame { request_id,
//! envelope }`, in the client placement's projection (gwz-dev
//! `dev-docs/GwzSplitCommandChannel.md` §3.3; step 2.2b): its codec only.
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | Tag 17 for every kind the transport charges to its control reserve, tag 16 for `data`, `open` and `check_identity` | §3.3 "Tag by kind" (`gwz-transport/src/mux/mod.rs` `enqueue`) |
//! | The body is the Taut `TransportLaneFrame`, which carries the owner's `Envelope` unchanged | §3.3 "Body" |
//!
//! Routing these frames, and every per-binding state (scope, the per-stream
//! bounds, binding-only faults), is step 2.1's carrier's router, which 2.2c
//! moves onto this body form (the remediation plan's decision 1).

use gwz_session_contract::Frame;
use gwz_transport::protocol::{Envelope, MessageKind};

/// The tag of transport data: `data`, `open` and `check_identity`.
pub(crate) const TRANSPORT_DATA: u8 = 16;
/// The tag of transport control: every other kind.
pub(crate) const TRANSPORT_CONTROL: u8 = 17;

/// Whether the transport charges `kind` to its control reserve: every kind
/// but `data`, `open` and `check_identity`.
pub(crate) fn control(kind: MessageKind) -> bool {
    !matches!(
        kind,
        MessageKind::Data | MessageKind::Open | MessageKind::CheckIdentity
    )
}

/// Encodes `envelope` for the call `request_id` as a lane frame, under the
/// tag its kind takes.
#[allow(dead_code, reason = "the carrier-backed handle (2.2c) sends with it")]
pub(crate) fn encode_lane(request_id: &str, envelope: Envelope) -> Frame {
    let tag = if control(envelope.kind) {
        TRANSPORT_CONTROL
    } else {
        TRANSPORT_DATA
    };
    let body = crate::TransportLaneFrame {
        request_id: request_id.to_owned(),
        envelope,
    };
    Frame {
        tag,
        body: crate::encode(&body.to_cbor()),
    }
}

/// Decodes a lane frame's body: the call it names and its envelope.
///
/// # Errors
///
/// A body that does not decode.
#[allow(
    dead_code,
    reason = "the carrier-backed handle (2.2c) receives with it"
)]
pub(crate) fn decode_lane(body: &[u8]) -> Result<(String, Envelope), crate::cbor::DecodeError> {
    crate::TransportLaneFrame::decode(body).map(|frame| (frame.request_id, frame.envelope))
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests;
    }
}

//! The pre-open gate of a host's end (plan step 2.3g1, lane owner Q4; `GwzSplitCommandChannel.md` §3.3 "When": a lane
//! frame before `SessionOpened` is a protocol error ending the carrier). With `CarrierLimits::awaiting_open`, the end
//! takes the handshake's one registry frame; any other frame read before it queues `SessionOpened` ends it as
//! `Protocol(BeforeOpened(class))`. The rows write raw bytes, so what the end reads, and when, is the row's.
use super::super::{
    CarrierClosed, CarrierLimits, CloseCode, CloseReason, FrameClass, Lane, ProtocolFault, wire,
};
use super::handshake_support::open_body;
use super::support::*;
use gwz_session_contract::{Frame, FrameKey, Tag, Target};
use std::time::Duration;

const SHORT: Duration = Duration::from_millis(50);

fn gated() -> CarrierLimits {
    limits(8, 4).awaiting_open()
}

fn session_open() -> Vec<u8> {
    wire_frame(Tag::SessionOpen.byte(), &open_body(1))
}

fn call(body: &[u8]) -> Vec<u8> {
    wire_frame(Tag::SessionCall.byte(), body)
}

fn opened() -> Frame {
    Frame::new(Tag::SessionOpened, b"opened".to_vec())
}

fn before_opened(class: FrameClass) -> CarrierClosed {
    CarrierClosed {
        reason: CloseReason::Protocol(ProtocolFault::BeforeOpened(class)),
    }
}

fn ended(carrier: &super::super::StreamCarrier) -> CarrierClosed {
    carrier.wait_closed(WAIT).expect("the carrier ends")
}

/// Row 2 (carrier): a `SessionCall` read after the handshake's frame and before `SessionOpened` is queued ends the
/// carrier as a protocol error, and the call is never delivered.
#[test]
fn a_call_before_session_opened_ends_the_carrier_as_a_protocol_error() {
    let (carrier, _from_carrier, mut to_carrier) = against_raw(gated());
    write_all(&mut to_carrier, &[session_open(), call(b"early")].concat());
    assert_eq!(ended(&carrier), before_opened(FrameClass::Registry));
    assert_eq!(
        carrier.recv_registry_timeout(SHORT).unwrap_err(),
        before_opened(FrameClass::Registry),
        "nothing is delivered after the fault"
    );
}

/// Row 2 (carrier), the other side: once `SessionOpened` is queued, the session's frames are delivered.
#[test]
fn once_session_opened_is_queued_a_call_is_delivered() {
    let (carrier, _from_carrier, mut to_carrier) = against_raw(gated());
    write_all(&mut to_carrier, &session_open());
    let open = eventually("the open", || carrier.recv_registry_timeout(SHORT).unwrap());
    assert_eq!(open.tag, Tag::SessionOpen.byte());
    carrier
        .send_frame(opened(), Lane::Control, FrameKey::ordinary(Target::new(0)))
        .unwrap();
    write_all(&mut to_carrier, &call(b"first"));
    let first = eventually("the call", || carrier.recv_registry_timeout(SHORT).unwrap());
    assert_eq!(first.body, b"first");
    assert_eq!(carrier.wait_closed(SHORT), None);
}

/// A second handshake frame is no handshake frame: the gate takes one.
#[test]
fn a_second_open_before_session_opened_is_a_protocol_error() {
    let (carrier, _from_carrier, mut to_carrier) = against_raw(gated());
    write_all(&mut to_carrier, &[session_open(), session_open()].concat());
    assert_eq!(ended(&carrier), before_opened(FrameClass::Registry));
}

/// Row 3 (carry-in from 2.3d): a tag-17 frame for a binding that is not open, read after the handshake's frame and
/// before `SessionOpened`, ends the carrier as a protocol error; it is not discarded as an unopened binding's.
#[test]
fn a_lane_frame_for_an_unopened_binding_before_session_opened_ends_the_carrier() {
    let (carrier, _from_carrier, mut to_carrier) = against_raw(gated());
    write_all(
        &mut to_carrier,
        &[session_open(), wire_for("unopened", window(1, 64))].concat(),
    );
    assert_eq!(ended(&carrier), before_opened(FrameClass::TransportControl));
    assert_eq!(carrier.discarded_frames(), 0);
}

/// Row 3: the same for tag 16, and for a lane frame that comes before the handshake's frame.
#[test]
fn a_data_frame_first_ends_the_carrier_before_any_handshake_frame() {
    let (carrier, _from_carrier, mut to_carrier) = against_raw(gated());
    write_all(&mut to_carrier, &wire_for("unopened", data(1, 0, vec![1])));
    assert_eq!(ended(&carrier), before_opened(FrameClass::TransportData));
}

/// The peer's close announcement before `SessionOpened` is its close, not a protocol error: the handshake's frame
/// is still delivered, then the closure.
#[test]
fn a_close_announcement_before_session_opened_is_the_peers_close() {
    let (carrier, _from_carrier, mut to_carrier) = against_raw(gated());
    let close = wire::close_frame(CloseCode::Normal);
    write_all(
        &mut to_carrier,
        &[session_open(), wire_frame(close.tag, &close.body)].concat(),
    );
    drop(to_carrier);
    let open = eventually("the open", || carrier.recv_registry_timeout(SHORT).unwrap());
    assert_eq!(open.tag, Tag::SessionOpen.byte());
    let closure = eventually("the closure", || carrier.recv_registry_timeout(SHORT).err());
    assert_eq!(closure.reason, CloseReason::Peer(CloseCode::Normal));
}

/// The gate is the host's alone: an end built without it delivers a call that arrives before any open, as every
/// client end and the 2.1 rows expect.
#[test]
fn an_end_without_the_gate_delivers_frames_as_before() {
    let (carrier, _from_carrier, mut to_carrier) = against_raw(limits(8, 4));
    write_all(&mut to_carrier, &[call(b"one"), call(b"two")].concat());
    let one = eventually("one", || carrier.recv_registry_timeout(SHORT).unwrap());
    let two = eventually("two", || carrier.recv_registry_timeout(SHORT).unwrap());
    assert_eq!((one.body, two.body), (b"one".to_vec(), b"two".to_vec()));
}

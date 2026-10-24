//! Messages, pairs and waits the rows share.
use super::super::{
    BindingLimits, Carrier, CarrierBinding, CarrierId, CarrierLimits, Peer, Side, StreamCarrier,
};
use crate::transport_host::Attachment;
use gwz_transport::protocol::{
    Cancel, Close, Data, Destination, Envelope, ErrorCode, MessageKind, Open, Window,
};
use std::{
    io::{self, Read, Write},
    task::Waker,
    time::{Duration, Instant},
};

pub(super) const WAIT: Duration = Duration::from_secs(20);

fn envelope(kind: MessageKind, stream_id: i64) -> Envelope {
    Envelope {
        version: 1,
        session_id: "session-1".into(),
        stream_id,
        kind,
        ..Default::default()
    }
}

pub(super) fn data(stream_id: i64, offset: i64, payload: Vec<u8>) -> Attachment {
    let mut message = envelope(MessageKind::Data, stream_id);
    message.data = Some(Data { offset, payload });
    (format!("request-{stream_id}"), message)
}

/// An `open` of the stream: a data-class message that starts work.
pub(super) fn open_stream(stream_id: i64) -> Attachment {
    let mut message = envelope(MessageKind::Open, stream_id);
    message.version = 2;
    message.open = Some(Open {
        endpoint_id: "endpoint-1".into(),
        operation_id: "operation-1".into(),
        destination: Destination {
            host: "example.test".into(),
            port: 22,
            path: "/repo.git".into(),
            ssh_username: Some("git".into()),
            ..Default::default()
        },
        deadlines: gwz_transport::protocol::Deadlines {
            allocation_ms: 1_000,
            connect_ms: 1_000,
            io_ms: 1_000,
            interaction_ms: 1_000,
            cleanup_ms: 1_000,
        },
        receive_limits: gwz_transport::binding::default_limits(),
        ..Default::default()
    });
    (format!("request-{stream_id}"), message)
}

pub(super) fn window(stream_id: i64, max_offset: i64) -> Attachment {
    let mut message = envelope(MessageKind::Window, stream_id);
    message.window = Some(Window { max_offset });
    (format!("request-{stream_id}"), message)
}

pub(super) fn cancel(stream_id: i64) -> Attachment {
    let mut message = envelope(MessageKind::Cancel, stream_id);
    message.cancel = Some(Cancel {
        reason: ErrorCode::Cancelled,
    });
    (format!("request-{stream_id}"), message)
}

pub(super) fn close(stream_id: i64, final_offset: i64) -> Attachment {
    let mut message = envelope(MessageKind::Close, stream_id);
    message.close = Some(Close { final_offset });
    (format!("request-{stream_id}"), message)
}

/// The kind and stream a message carries: what the rows compare, since an envelope is large.
pub(super) fn tag(message: &Attachment) -> (MessageKind, i64, String) {
    (message.1.kind, message.1.stream_id, message.0.clone())
}

pub(super) fn limits(data: usize, control: usize) -> CarrierLimits {
    CarrierLimits::new(data, control).expect("limits with room")
}

/// Two stream carriers joined by two OS pipes, one per direction, both sides of one connection.
pub(super) fn over_pipes(limits: CarrierLimits) -> (StreamCarrier, StreamCarrier) {
    let (core_read, client_write) = io::pipe().expect("pipe");
    let (client_read, core_write) = io::pipe().expect("pipe");
    let id = CarrierId::random().expect("random");
    let core = StreamCarrier::new(
        Box::new(core_read),
        Box::new(core_write),
        Box::new(|| {}),
        CarrierBinding::new(id, Side::Core, Peer::InProcess),
        limits,
    )
    .expect("core carrier");
    let client = StreamCarrier::new(
        Box::new(client_read),
        Box::new(client_write),
        Box::new(|| {}),
        CarrierBinding::new(id, Side::Client, Peer::InProcess),
        limits,
    )
    .expect("client carrier");
    for carrier in [&core, &client] {
        carrier
            .open_binding("session-1", BindingLimits::default())
            .expect("the rows' binding");
    }
    (core, client)
}

/// A carrier whose far end is raw bytes the row holds: (carrier, what the carrier wrote, where the row writes).
pub(super) fn against_raw(
    limits: CarrierLimits,
) -> (StreamCarrier, io::PipeReader, io::PipeWriter) {
    let (from_carrier, carrier_write) = io::pipe().expect("pipe");
    let (carrier_read, to_carrier) = io::pipe().expect("pipe");
    let carrier = StreamCarrier::new(
        Box::new(carrier_read),
        Box::new(carrier_write),
        Box::new(|| {}),
        CarrierBinding::new(
            CarrierId::random().expect("random"),
            Side::Core,
            Peer::InProcess,
        ),
        limits,
    )
    .expect("carrier");
    carrier
        .open_binding("session-1", BindingLimits::default())
        .expect("the rows' binding");
    (carrier, from_carrier, to_carrier)
}

/// Polls `check` until it returns `Some` or the wait ends.
pub(super) fn eventually<T>(what: &str, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(found) = check() {
            return found;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// The next message of `end`, waiting for it.
pub(super) fn next(end: &dyn Carrier) -> Attachment {
    eventually("a message", || match end.try_recv() {
        Ok(Some(arrival)) => Some(arrival.message),
        Ok(None) => None,
        Err(closed) => panic!("the carrier closed while a message was awaited: {closed}"),
    })
}

/// Every byte a raw reader gets until its stream ends.
pub(super) fn read_to_end(mut reader: impl Read) -> Vec<u8> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).expect("read");
    bytes
}

/// One wire frame, as `session-channel` frames it.
pub(super) fn wire_frame(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut bytes = u32::try_from(body.len() + 1)
        .expect("small")
        .to_le_bytes()
        .to_vec();
    bytes.push(tag);
    bytes.extend_from_slice(body);
    bytes
}

pub(super) fn write_all(mut writer: impl Write, bytes: &[u8]) {
    writer.write_all(bytes).expect("write");
    writer.flush().expect("flush");
}

/// A waker that counts its wakes.
pub(super) struct Wakes(std::sync::atomic::AtomicUsize);

impl Wakes {
    pub(super) fn count(&self) -> usize {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl std::task::Wake for Wakes {
    fn wake(self: std::sync::Arc<Self>) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

pub(super) fn counting_waker() -> (Waker, std::sync::Arc<Wakes>) {
    let wakes = std::sync::Arc::new(Wakes(std::sync::atomic::AtomicUsize::new(0)));
    (Waker::from(wakes.clone()), wakes)
}

/// One frame off a raw wire: its tag and body.
pub(super) fn read_wire_frame(reader: &mut impl Read) -> (u8, Vec<u8>) {
    let mut prefix = [0u8; 4];
    reader.read_exact(&mut prefix).expect("a length prefix");
    let size = u32::from_le_bytes(prefix) as usize;
    let mut rest = vec![0u8; size];
    reader.read_exact(&mut rest).expect("a frame");
    (rest[0], rest[1..].to_vec())
}

/// What the carrier wrote, decoded: the message each transport frame carries.
pub(super) fn read_message(reader: &mut impl Read) -> Attachment {
    let (tag, body) = read_wire_frame(reader);
    super::super::wire::decode_attachment(&gwz_session_contract::Frame { tag, body })
        .expect("a message")
}

/// A message as the bytes a peer writes to the wire.
pub(super) fn wire_message(message: &Attachment) -> Vec<u8> {
    let frame = super::super::wire::encode_attachment(message).expect("encodes");
    wire_frame(frame.tag, &frame.body)
}

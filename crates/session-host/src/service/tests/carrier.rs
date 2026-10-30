use super::*;

/// The test's carrier, standing in for step 2.1's: the frames a handler sends
/// toward the client on the transport lane, and those the test router hands
/// back to a live call. It is the session's data, so a handler reaches it
/// through its gate.
#[derive(Clone, Default)]
pub(super) struct Carrier {
    pub(super) shared: Arc<CarrierShared>,
}

#[derive(Default)]
pub(super) struct CarrierShared {
    sent: Mutex<Vec<Frame>>,
    sent_changed: Condvar,
    inbound: Mutex<HashMap<String, Vec<Vec<u8>>>>,
    inbound_changed: Condvar,
}

impl Carrier {
    pub(super) fn send(&self, frame: Frame) {
        crate::lock(&self.shared.sent).push(frame);
        self.shared.sent_changed.notify_all();
    }

    pub(super) fn wait_sent(&self) -> Frame {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut sent = crate::lock(&self.shared.sent);
        loop {
            if !sent.is_empty() {
                return sent.remove(0);
            }
            assert!(Instant::now() < deadline, "no lane frame was sent");
            sent = self
                .shared
                .sent_changed
                .wait_timeout(sent, Duration::from_millis(20))
                .unwrap()
                .0;
        }
    }

    fn deliver(&self, request_id: &str, body: Vec<u8>) {
        crate::lock(&self.shared.inbound)
            .entry(request_id.to_owned())
            .or_default()
            .push(body);
        self.shared.inbound_changed.notify_all();
    }

    pub(super) fn wait_inbound(&self, request_id: &str, timeout: Duration) -> Option<Vec<u8>> {
        let deadline = Instant::now() + timeout;
        let mut inbound = crate::lock(&self.shared.inbound);
        loop {
            if let Some(body) = inbound.get_mut(request_id).and_then(|queue| queue.pop()) {
                return Some(body);
            }
            if Instant::now() >= deadline {
                return None;
            }
            inbound = self
                .shared
                .inbound_changed
                .wait_timeout(inbound, Duration::from_millis(20))
                .unwrap()
                .0;
        }
    }
}

/// The test router, the brief's fake of step 2.1's `FrameRouter`: a
/// transport-lane frame (tag 16 or 17, body `request_id|kind`) goes to its
/// live call, through the service's live-call registry, and every other frame
/// to the service's session-class seam. Before the session opens a lane frame
/// is a protocol error, as every frame is.
pub(super) fn test_route(
    service: &Service<Ports>,
    carrier: &Carrier,
    frame: &Frame,
) -> Result<&'static str, ProtocolError> {
    if frame.tag != 16 && frame.tag != 17 {
        return service.route(frame).map(|()| "call");
    }
    if !service.is_open() {
        return Err(ProtocolError(format!(
            "a frame of tag {} before the session opened",
            frame.tag
        )));
    }
    let body = text(&frame.body);
    let (request_id, _) = body.split_once('|').unwrap();
    if !service.is_live(request_id) {
        return Ok("discarded");
    }
    carrier.deliver(request_id, frame.body.clone());
    Ok("delivered")
}

pub(super) fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// A call's body: `id|method|mode|request`.
pub(super) fn body(call_id: u64, method: &str, submit: bool, request: &str) -> Vec<u8> {
    format!("{call_id}|{method}|{}|{request}", u8::from(submit)).into_bytes()
}

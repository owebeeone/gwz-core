//! What the handshake rows share: scripted wires with a clock the rows set, and the bodies they send.
use super::super::handshake::{
    ByteWire, Control, FrameWire, Heard, Hello, Hooks, OpenRequest, OpenedInfo, Refusal, State,
    WireEnd,
};
use super::super::wire::session_error_frame;
use super::support::wire_frame;
use gwz_session_contract::Frame;
use std::{cell::RefCell, collections::VecDeque, rc::Rc, time::Duration, time::Instant};

pub(super) type Log = Rc<RefCell<Vec<String>>>;

pub(super) fn hello() -> Hello {
    Hello {
        protocol_min: 1,
        protocol_max: 2,
        server: "gwz 1.1.0".into(),
        core: "gwz-core 1.1.0 transport".into(),
        server_pid: 4242,
    }
}

pub(super) fn open_body(protocol: u32) -> Vec<u8> {
    OpenRequest {
        protocol,
        client: "gwz 1.1.0".into(),
        limits: None,
    }
    .encode()
}

/// A raw definite map of `(key, encoded value)` entries, keys under 24 and at most 23 entries.
pub(super) fn raw_map(entries: &[(u8, &[u8])]) -> Vec<u8> {
    let mut out = vec![0xa0 | entries.len() as u8];
    for (key, value) in entries {
        out.push(*key);
        out.extend_from_slice(value);
    }
    out
}

/// A `SessionOpen` body of the client placement's shape with `extra` entries added: `(key, encoded value)`.
pub(super) fn open_with(extra: &[(u8, &[u8])]) -> Vec<u8> {
    let mut entries: Vec<(u8, &[u8])> =
        vec![(1, &[0x01]), (2, &[0x61, b'c']), (5, &[0xa0]), (6, &[0xf5])];
    for &(key, value) in extra {
        entries.retain(|(have, _)| *have != key);
        entries.push((key, value));
    }
    entries.sort_by_key(|(key, _)| *key);
    raw_map(&entries)
}

/// The host's frames as the bytes a client reads.
pub(super) fn wire_bytes(frames: &[Frame]) -> Vec<u8> {
    frames
        .iter()
        .flat_map(|frame| wire_frame(frame.tag, &frame.body))
        .collect()
}

pub(super) fn refusal_frame(code: u64, message: &str) -> Frame {
    session_error_frame(code, message)
}

pub(super) fn state() -> State {
    State {
        server_pid: 4242,
        open_sessions: 2,
        max_sessions: 64,
        idle_exit: Some(300),
        log: Some("/var/log/gwz.log".into()),
        stopping: false,
    }
}

/// How a scripted wire ends once its script is read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum After {
    /// Nothing more arrives: a read waits out its bound.
    Silent,
    /// The stream ends.
    Ended,
}

/// The client's wire: the host's bytes, a clock that moves only when a read waits, and what the client wrote.
pub(super) struct ScriptedBytes {
    pub(super) clock: Instant,
    script: Vec<u8>,
    pub(super) read: usize,
    pub(super) written: Vec<u8>,
    pub(super) aborted: bool,
    pub(super) reads: usize,
    after: After,
}

impl ScriptedBytes {
    pub(super) fn new(script: Vec<u8>, after: After) -> Self {
        Self {
            clock: Instant::now(),
            script,
            read: 0,
            written: Vec::new(),
            aborted: false,
            reads: 0,
            after,
        }
    }
}

impl ByteWire for ScriptedBytes {
    fn now(&self) -> Instant {
        self.clock
    }

    fn send(&mut self, bytes: &[u8]) -> Result<(), WireEnd> {
        self.written.extend_from_slice(bytes);
        Ok(())
    }

    fn recv(&mut self, count: usize, wait: Duration) -> Result<Heard, WireEnd> {
        self.reads += 1;
        let left = self.script.len() - self.read;
        if left >= count {
            let bytes = self.script[self.read..self.read + count].to_vec();
            self.read += count;
            return Ok(Heard::Bytes(bytes));
        }
        let partial = self.script[self.read..].to_vec();
        self.read = self.script.len();
        match self.after {
            After::Silent => {
                self.clock += wait;
                Ok(Heard::Silent(partial))
            }
            After::Ended => Ok(Heard::Ended(partial)),
        }
    }

    fn abort(&mut self) {
        self.aborted = true;
    }
}

/// The host's wire: the frames the client sends, a clock that moves when a receive waits or a send costs, and a log
/// shared with the hooks so a row sees the order of events.
pub(super) struct ScriptedFrames {
    pub(super) clock: Instant,
    pub(super) inbox: VecDeque<Frame>,
    pub(super) sent: Vec<Frame>,
    pub(super) closed: bool,
    /// What each send adds to the clock.
    pub(super) send_cost: Duration,
    /// A tag whose send fails (the stream has gone): the frame is not recorded as sent.
    pub(super) fail_send: Option<u8>,
    pub(super) log: Log,
}

impl ScriptedFrames {
    pub(super) fn new(inbox: Vec<Frame>, log: Log) -> Self {
        Self {
            clock: Instant::now(),
            inbox: inbox.into(),
            sent: Vec::new(),
            closed: false,
            send_cost: Duration::ZERO,
            fail_send: None,
            log,
        }
    }
}

impl FrameWire for ScriptedFrames {
    fn now(&self) -> Instant {
        self.clock
    }

    fn send(&mut self, frame: Frame) -> Result<(), WireEnd> {
        self.log.borrow_mut().push(format!("send {}", frame.tag));
        self.clock += self.send_cost;
        if self.fail_send == Some(frame.tag) {
            return Err(WireEnd::Io(std::io::ErrorKind::BrokenPipe));
        }
        self.sent.push(frame);
        Ok(())
    }

    fn recv(&mut self, wait: Duration) -> Result<Option<Frame>, WireEnd> {
        self.log.borrow_mut().push("recv".into());
        match self.inbox.pop_front() {
            Some(frame) => Ok(Some(frame)),
            None => {
                self.clock += wait;
                Ok(None)
            }
        }
    }

    fn close(&mut self) {
        self.closed = true;
    }
}

/// Hooks that record what they were asked and answer as a row sets.
pub(super) struct RecordingHooks {
    pub(super) log: Log,
    pub(super) opened: Vec<OpenRequest>,
    pub(super) controls: Vec<Control>,
    pub(super) open_answer: Result<OpenedInfo, Refusal>,
    pub(super) control_answer: Result<State, Refusal>,
}

impl RecordingHooks {
    pub(super) fn new(log: Log) -> Self {
        Self {
            log,
            opened: Vec::new(),
            controls: Vec::new(),
            open_answer: Ok(OpenedInfo { open_sessions: 3 }),
            control_answer: Ok(state()),
        }
    }
}

impl Hooks for RecordingHooks {
    fn open(&mut self, open: &OpenRequest) -> Result<OpenedInfo, Refusal> {
        self.log.borrow_mut().push("hook open".into());
        self.opened.push(open.clone());
        self.open_answer.clone()
    }

    fn control(&mut self, control: &Control) -> Result<State, Refusal> {
        self.log.borrow_mut().push("hook control".into());
        self.controls.push(*control);
        self.control_answer.clone()
    }
}

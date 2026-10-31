//! The client's side: read the host's hello, check the version range, send one frame, read the answer, all within
//! the bound counted from the start of the connect. Control flow is the state machine: the client cannot send before
//! it holds a well-formed hello, and a frame out of order ends the exchange. The one frame the host may send in place
//! of its hello is the refusal of this peer (see the module doc of `handshake`).
use super::super::{validate_session_open, wire::INVALID_REQUEST};
use super::{
    Bounds, ByteWire, Control, FIRST_FRAME_MAX, FRAME_HEAD, Heard, Hello, OpenRequest, Opened,
    REPLY_FRAME_MAX, SERVER_UNAVAILABLE, SERVER_VERSION_MISMATCH, SHOWN_BYTES, SHOWN_MESSAGE,
    State, WireEnd,
    body::{BodyFault, decode_refusal},
    escape,
};
use gwz_session_channel::write_frame;
use gwz_session_contract::{Frame, Tag};
use std::{
    fmt,
    ops::RangeInclusive,
    time::{Duration, Instant},
};

/// What the client asks the host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Intent {
    /// A session: `SessionOpen`, answered by `SessionOpened`.
    Open,
    /// The `server` subcommands: `ServerControl`, answered by `ServerState`.
    Control(Control),
}

/// What the client brings.
#[derive(Clone, Debug)]
pub(crate) struct ClientConfig {
    /// The session protocol versions this client speaks.
    pub(crate) supported: RangeInclusive<u32>,
    /// The `client` field of the `SessionOpen`: who is asking.
    pub(crate) client: String,
    /// The encoded `SessionLimits` the client states, if any.
    pub(crate) limits: Option<Vec<u8>>,
    /// The listener's process, when the connect learned it, for the messages.
    pub(crate) peer_pid: Option<u32>,
    pub(crate) bounds: Bounds,
}

/// What the client holds when the handshake is done.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ClientOutcome {
    Opened { hello: Hello, opened: Opened },
    State { hello: Hello, state: State },
}

/// Where the exchange was.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Stage {
    /// Waiting for the host's `SessionHello`.
    Hello,
    /// Waiting for `SessionOpened` or `ServerState`.
    Reply,
}

/// Why the host is not a server that answers (`server_unavailable`).
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Cause {
    /// The bound passed with nothing more to read; `first` is what had arrived by then, escaped (empty when nothing
    /// had).
    Silent { waited: Duration, first: String },
    /// The stream ended; `first` is what had arrived, escaped.
    Ended { first: String },
    /// The first frame is not a well-formed `SessionHello`; `first` is its first bytes, escaped.
    NotAHello { first: String },
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct Unavailable {
    pub(crate) stage: Stage,
    pub(crate) cause: Cause,
    pub(crate) peer_pid: Option<u32>,
}

/// Why the handshake failed, in the client.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ClientFault {
    /// `server_unavailable`.
    Unavailable(Unavailable),
    /// `server_version_mismatch`: the host serves `host` and the client `client`; nothing was sent.
    VersionMismatch {
        host: (u32, u32),
        client: (u32, u32),
    },
    /// The host refused with its one `SessionError`; `message` is escaped.
    Refused {
        code: u64,
        message: String,
    },
    /// A frame out of order, a body that does not decode, or an answer that differs from the hello.
    Protocol(&'static str),
    /// The request the client would send is longer than a first frame ([`FIRST_FRAME_MAX`], counting the tag): it is
    /// not sent, so that the write cannot outlast the handshake's bound. `invalid_request`.
    TooLarge {
        size: usize,
        cap: usize,
    },
    Wire(WireEnd),
}

impl ClientFault {
    /// The `GwzErrorCode` the failure reports, when it has one of its own.
    pub(crate) fn code(&self) -> Option<u64> {
        match self {
            ClientFault::Unavailable(_) => Some(SERVER_UNAVAILABLE),
            ClientFault::VersionMismatch { .. } => Some(SERVER_VERSION_MISMATCH),
            ClientFault::Refused { code, .. } => Some(*code),
            ClientFault::TooLarge { .. } => Some(INVALID_REQUEST),
            ClientFault::Protocol(_) | ClientFault::Wire(_) => None,
        }
    }
}

impl fmt::Display for ClientFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientFault::Unavailable(Unavailable {
                stage,
                cause,
                peer_pid,
            }) => {
                let waiting = match stage {
                    Stage::Hello => "the host's SessionHello",
                    Stage::Reply => "the host's reply",
                };
                match cause {
                    Cause::Silent { waited, first } if first.is_empty() => write!(
                        f,
                        "server unavailable: no answer in {}s while waiting for {waiting}",
                        waited.as_secs()
                    )?,
                    Cause::Silent { waited, first } => write!(
                        f,
                        "server unavailable: no answer in {}s while waiting for {waiting}; it had sent \"{first}\"",
                        waited.as_secs()
                    )?,
                    Cause::Ended { first } => write!(
                        f,
                        "server unavailable: the stream ended before {waiting}; it had sent \"{first}\""
                    )?,
                    Cause::NotAHello { first } => write!(
                        f,
                        "server unavailable: the first frame is not a SessionHello; it began \"{first}\""
                    )?,
                }
                match peer_pid {
                    Some(pid) => write!(f, " (process {pid})"),
                    None => Ok(()),
                }
            }
            ClientFault::VersionMismatch { host, client } => write!(
                f,
                "server version mismatch: the host serves protocols {}..={} and this client {}..={}",
                host.0, host.1, client.0, client.1
            ),
            ClientFault::Refused { code, message } => {
                write!(f, "the host refused (code {code}): {message}")
            }
            ClientFault::Protocol(why) => write!(f, "protocol error: {why}"),
            ClientFault::TooLarge { size, cap } => write!(
                f,
                "invalid request: the request is {size} bytes and a handshake frame holds at most {cap}"
            ),
            ClientFault::Wire(end) => write!(f, "the stream failed: {end:?}"),
        }
    }
}

impl std::error::Error for ClientFault {}

/// Runs the handshake over `wire`; `started` is when the connect began. On every failure the stream is aborted.
pub(crate) fn run(
    wire: &mut dyn ByteWire,
    started: Instant,
    config: &ClientConfig,
    intent: &Intent,
) -> Result<ClientOutcome, ClientFault> {
    let result = exchange(wire, started, config, intent);
    if result.is_err() {
        wire.abort();
    }
    result
}

struct Hearing<'a> {
    wire: &'a mut dyn ByteWire,
    started: Instant,
    deadline: Instant,
    peer_pid: Option<u32>,
}

impl Hearing<'_> {
    fn unavailable(&self, stage: Stage, cause: Cause) -> ClientFault {
        ClientFault::Unavailable(Unavailable {
            stage,
            cause,
            peer_pid: self.peer_pid,
        })
    }

    /// Exactly `count` bytes within what is left of the bound.
    fn bytes(&mut self, count: usize, stage: Stage) -> Result<Vec<u8>, ClientFault> {
        let now = self.wire.now();
        let silent = |this: &Self, now: Instant, partial: &[u8]| {
            let waited = now.saturating_duration_since(this.started);
            let first = escape(partial, SHOWN_BYTES);
            this.unavailable(stage, Cause::Silent { waited, first })
        };
        let left = self
            .deadline
            .checked_duration_since(now)
            .filter(|left| !left.is_zero())
            .ok_or_else(|| silent(self, now, &[]))?;
        match self.wire.recv(count, left).map_err(ClientFault::Wire)? {
            Heard::Bytes(bytes) => Ok(bytes),
            Heard::Silent(partial) => Err(silent(self, self.wire.now(), &partial)),
            Heard::Ended(partial) => {
                let first = escape(&partial, SHOWN_BYTES);
                Err(self.unavailable(stage, Cause::Ended { first }))
            }
        }
    }

    /// One frame: its length is judged before its body is read, so an oversize or empty frame costs five bytes.
    fn frame(
        &mut self,
        stage: Stage,
        cap: usize,
    ) -> Result<(Frame, [u8; FRAME_HEAD]), ClientFault> {
        let head: [u8; FRAME_HEAD] = self
            .bytes(FRAME_HEAD, stage)?
            .try_into()
            .expect("five bytes");
        let size = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as usize;
        if size == 0 || size > cap {
            return Err(match stage {
                Stage::Hello => self.not_a_hello(&head),
                Stage::Reply => ClientFault::Protocol("a reply frame with a bad length"),
            });
        }
        let body = if size > 1 {
            self.bytes(size - 1, stage)?
        } else {
            Vec::new()
        };
        Ok((Frame { tag: head[4], body }, head))
    }

    fn not_a_hello(&self, shown: &[u8]) -> ClientFault {
        let first = escape(shown, SHOWN_BYTES);
        self.unavailable(Stage::Hello, Cause::NotAHello { first })
    }
}

fn exchange(
    wire: &mut dyn ByteWire,
    started: Instant,
    config: &ClientConfig,
    intent: &Intent,
) -> Result<ClientOutcome, ClientFault> {
    let wait = match intent {
        Intent::Open => config.bounds.open_wait,
        Intent::Control(_) => config.bounds.state_wait,
    };
    let mut hearing = Hearing {
        wire,
        started,
        deadline: started + wait,
        peer_pid: config.peer_pid,
    };
    let (first, head) = hearing.frame(Stage::Hello, FIRST_FRAME_MAX)?;
    if first.tag == Tag::SessionError.byte() {
        // The listener's refusal of this peer, in place of the hello. Anything else under this tag is not one.
        if let Ok(refused) = refusal(&first.body) {
            return Err(refused);
        }
    }
    if first.tag != Tag::SessionHello.byte() {
        return Err(hearing.not_a_hello(&head));
    }
    let hello = Hello::decode(&first.body).map_err(|_| hearing.not_a_hello(&first.body))?;
    let (low, high) = (*config.supported.start(), *config.supported.end());
    let protocol = hello.protocol_max.min(high);
    if protocol < hello.protocol_min.max(low) {
        return Err(ClientFault::VersionMismatch {
            host: (hello.protocol_min, hello.protocol_max),
            client: (low, high),
        });
    }
    let bytes = request_bytes(config, intent, protocol)?;
    hearing.wire.send(&bytes).map_err(ClientFault::Wire)?;
    let (reply, _) = hearing.frame(Stage::Reply, REPLY_FRAME_MAX)?;
    let malformed = |_| ClientFault::Protocol("a reply body that does not decode");
    match (reply.tag, intent) {
        (3, _) => Err(refusal(&reply.body).map_err(malformed)?),
        (5, Intent::Open) => {
            let opened = Opened::decode(&reply.body).map_err(malformed)?;
            let same = opened.protocol == protocol
                && opened.server == hello.server
                && opened.core == hello.core
                && opened.server_pid == hello.server_pid;
            if !same {
                return Err(ClientFault::Protocol(
                    "SessionOpened differs from the SessionHello",
                ));
            }
            Ok(ClientOutcome::Opened { hello, opened })
        }
        (8, Intent::Control(_)) => {
            let state = State::decode(&reply.body).map_err(malformed)?;
            Ok(ClientOutcome::State { hello, state })
        }
        _ => Err(ClientFault::Protocol("a frame out of order")),
    }
}

/// The host's refusal in `body`, a `SessionError` for call ID 0, as the fault to report: its message is escaped.
fn refusal(body: &[u8]) -> Result<ClientFault, BodyFault> {
    let (code, message) = decode_refusal(body)?;
    let message = escape(message.as_bytes(), SHOWN_MESSAGE);
    Ok(ClientFault::Refused { code, message })
}

/// The one frame the client sends for `intent`, as the bytes to write, with `protocol` in a `SessionOpen`. It is
/// refused when it is not of the client placement's shape, and when it is longer than a first frame
/// ([`FIRST_FRAME_MAX`], counting the tag): the write is made on the caller's thread, and so is bounded by its size
/// alone. [`dial`](super::dial) asks the same question before it connects.
pub(crate) fn request_bytes(
    config: &ClientConfig,
    intent: &Intent,
    protocol: u32,
) -> Result<Vec<u8>, ClientFault> {
    let request = match intent {
        Intent::Open => {
            let body = OpenRequest {
                protocol,
                client: config.client.clone(),
                limits: config.limits.clone(),
            }
            .encode();
            let size = body.len() + 1;
            if size > FIRST_FRAME_MAX {
                return Err(ClientFault::TooLarge {
                    size,
                    cap: FIRST_FRAME_MAX,
                });
            }
            // The sender's check, before a byte is written (`wire.rs`).
            validate_session_open(&body).map_err(|_| {
                ClientFault::Protocol("the client's SessionOpen is not of its shape")
            })?;
            Frame::new(Tag::SessionOpen, body)
        }
        Intent::Control(control) => Frame::new(Tag::ServerControl, control.encode()),
    };
    let mut bytes = Vec::new();
    write_frame(&mut bytes, &request)
        .map_err(|_| ClientFault::Protocol("the request is too large"))?;
    Ok(bytes)
}

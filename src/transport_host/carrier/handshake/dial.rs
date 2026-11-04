//! The client's whole open of a local server: connect, handshake, carrier (plan step 2.3d-2).
//!
//! [`dial`] starts the clock, connects to the address with the deadline the handshake's bound gives (a full accept
//! queue or a busy pipe is waited for until then), runs the client's handshake over the raw stream, and only then
//! builds the carrier. Until the handshake is done the stream is read by a helper thread, so that a host that goes
//! quiet is given up on at the bound by aborting the stream, on every platform: a blocked read cannot be timed out
//! otherwise (a pipe's read has no timeout).
use super::super::{
    CarrierLimits, StreamCarrier,
    local::{
        LocalRefusal, PeerIdentity, PeerPolicy, RawConnection, Target,
        address::SERVER_ADDRESS_REFUSED, connect_stream,
    },
    wire::INVALID_REQUEST,
};
use super::{
    ByteWire, ClientConfig, ClientFault, ClientOutcome, Heard, Intent, SERVER_LISTENER_REFUSED,
    SERVER_UNAVAILABLE, WireEnd, client::request_bytes, client_handshake,
};
use std::{
    io::{self, Read, Write},
    sync::{
        Arc, Mutex, PoisonError,
        mpsc::{Receiver, RecvTimeoutError, Sender, channel},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// How long the helper thread is given to hand the stream back once a handshake has succeeded: it is idle then, so
/// this only bounds a thread that has died.
const RETURN_WAIT: Duration = Duration::from_secs(5);

enum Ask {
    /// Read exactly this many bytes.
    Read(usize),
    /// The handshake is over: give the read half back.
    Return,
}

/// The bytes the helper thread has read towards the answer it is giving, so that a read that runs out of time can say
/// what had arrived. [`StreamWire::recv`] clears it before it asks, so that nothing of an earlier answer is in it.
type Progress = Arc<Mutex<Vec<u8>>>;

fn progress_of(progress: &Progress) -> std::sync::MutexGuard<'_, Vec<u8>> {
    progress.lock().unwrap_or_else(PoisonError::into_inner)
}

enum Reply {
    Bytes(Vec<u8>),
    /// The stream ended, after these bytes.
    Ended(Vec<u8>),
    Failed(io::ErrorKind),
    Returned(Box<dyn Read + Send>),
}

/// Reads for the wire, one request at a time, and returns the half when asked. The thread ends when it is told to
/// return the half, when its wire is dropped, or when its stream fails or ends: aborting the stream is what frees a
/// read that is waiting.
fn pump(
    mut read: Box<dyn Read + Send>,
    asks: Receiver<Ask>,
    replies: Sender<Reply>,
    progress: Progress,
) {
    while let Ok(ask) = asks.recv() {
        let count = match ask {
            Ask::Return => {
                let _ = replies.send(Reply::Returned(read));
                return;
            }
            Ask::Read(count) => count,
        };
        let mut bytes = vec![0u8; count];
        let mut filled = 0;
        let reply = loop {
            if filled == count {
                break Reply::Bytes(bytes);
            }
            match read.read(&mut bytes[filled..]) {
                Ok(0) => {
                    bytes.truncate(filled);
                    break Reply::Ended(bytes);
                }
                Ok(read) => {
                    progress_of(&progress).extend_from_slice(&bytes[filled..filled + read]);
                    filled += read;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => break Reply::Failed(error.kind()),
            }
        };
        let finished = !matches!(reply, Reply::Bytes(_));
        if replies.send(reply).is_err() || finished {
            return;
        }
    }
}

/// A connected, peer-checked stream as the client's [`ByteWire`]. Dropping it before [`StreamWire::into_carrier`]
/// shuts the stream.
pub(crate) struct StreamWire {
    write: Box<dyn Write + Send>,
    abort: Box<dyn Fn() + Send + Sync>,
    peer: PeerIdentity,
    side: super::super::Side,
    asks: Sender<Ask>,
    replies: Receiver<Reply>,
    progress: Progress,
    reader: Option<JoinHandle<()>>,
}

impl StreamWire {
    pub(crate) fn new(raw: RawConnection) -> io::Result<Self> {
        let (asks, asked) = channel();
        let (replied, replies) = channel();
        let read = raw.read;
        let progress = Progress::default();
        let seen = Arc::clone(&progress);
        let reader = thread::Builder::new()
            .name("gwz-handshake-read".into())
            .spawn(move || pump(read, asked, replied, seen))?;
        Ok(Self {
            write: raw.write,
            abort: raw.abort,
            peer: raw.peer,
            side: raw.side,
            asks,
            replies,
            progress,
            reader: Some(reader),
        })
    }

    /// The peer's process, as the connect read it.
    pub(crate) fn peer_pid(&self) -> Option<u32> {
        self.peer.pid
    }

    /// The handshake succeeded: the read half comes back from the helper thread and the carrier is built over the
    /// stream.
    pub(crate) fn into_carrier(
        mut self,
        limits: CarrierLimits,
    ) -> Result<StreamCarrier, LocalRefusal> {
        let gone = || LocalRefusal::os("handshake reader", &io::ErrorKind::BrokenPipe.into());
        self.asks.send(Ask::Return).map_err(|_| gone())?;
        let Reply::Returned(read) = self.replies.recv_timeout(RETURN_WAIT).map_err(|_| gone())?
        else {
            return Err(gone());
        };
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        let write = std::mem::replace(&mut self.write, Box::new(io::sink()));
        let abort = std::mem::replace(&mut self.abort, Box::new(|| {}));
        RawConnection {
            read,
            write,
            abort,
            peer: self.peer.clone(),
            side: self.side,
        }
        .into_carrier(limits)
    }
}

impl ByteWire for StreamWire {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn send(&mut self, bytes: &[u8]) -> Result<(), WireEnd> {
        self.write
            .write_all(bytes)
            .and_then(|()| self.write.flush())
            .map_err(|error| WireEnd::Io(error.kind()))
    }

    fn recv(&mut self, count: usize, wait: Duration) -> Result<Heard, WireEnd> {
        progress_of(&self.progress).clear();
        self.asks
            .send(Ask::Read(count))
            .map_err(|_| WireEnd::Io(io::ErrorKind::BrokenPipe))?;
        match self.replies.recv_timeout(wait) {
            Ok(Reply::Bytes(bytes)) => Ok(Heard::Bytes(bytes)),
            Ok(Reply::Ended(partial)) => Ok(Heard::Ended(partial)),
            Ok(Reply::Failed(kind)) => Err(WireEnd::Io(kind)),
            Ok(Reply::Returned(_)) | Err(RecvTimeoutError::Disconnected) => {
                Err(WireEnd::Io(io::ErrorKind::BrokenPipe))
            }
            Err(RecvTimeoutError::Timeout) => {
                Ok(Heard::Silent(progress_of(&self.progress).clone()))
            }
        }
    }

    fn abort(&mut self) {
        (self.abort)();
    }
}

impl Drop for StreamWire {
    fn drop(&mut self) {
        // Frees a helper thread that is still waiting in a read; a stream already handed on has a no-op abort.
        (self.abort)();
    }
}

/// Why a [`dial`] did not give a carrier.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum DialFault {
    /// The request cannot be sent (it is not of its shape, or it is longer than a first frame): nothing was connected.
    Request(ClientFault),
    /// The connect, the peer check or the carrier's start failed or ran out of time.
    Connect(LocalRefusal),
    /// The handshake failed; the stream was aborted.
    Handshake(ClientFault),
}

impl DialFault {
    /// The `GwzErrorCode` the failure reports (server design §17), when it has one:
    /// - a request that cannot be sent is `invalid_request`;
    /// - a connect that never completes, a socket that refuses or is missing, and a host that did not act as a gwz
    ///   server (a reply out of order, a stream that failed) are `server_unavailable`;
    /// - an address the walk (or the pipe's checks) did not give a connect is `server_address_refused`, unless nothing
    ///   answers there (a missing name), which is `server_unavailable`;
    /// - a listener this process refused is `server_listener_refused`;
    /// - any other handshake failure carries the code of its [`ClientFault`].
    pub(crate) fn code(&self) -> Option<u64> {
        match self {
            DialFault::Request(_) => Some(INVALID_REQUEST),
            DialFault::Connect(LocalRefusal::Timeout) => Some(SERVER_UNAVAILABLE),
            DialFault::Connect(LocalRefusal::Os {
                kind: io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound,
                ..
            }) => Some(SERVER_UNAVAILABLE),
            DialFault::Connect(LocalRefusal::Peer(_)) => Some(SERVER_LISTENER_REFUSED),
            DialFault::Connect(LocalRefusal::Address { unavailable, .. }) => {
                Some(if *unavailable {
                    SERVER_UNAVAILABLE
                } else {
                    SERVER_ADDRESS_REFUSED
                })
            }
            DialFault::Connect(_) => None,
            DialFault::Handshake(ClientFault::Protocol(_) | ClientFault::Wire(_)) => {
                Some(SERVER_UNAVAILABLE)
            }
            DialFault::Handshake(fault) => fault.code(),
        }
    }
}

/// Connects to `target` and handshakes, all within the bound `intent` gives from the moment of the call. The
/// carrier is returned with the handshake's outcome; for a control exchange the host closes it next.
///
/// `target` is the walk of the address (Linux and macOS: `address::walk`, under the process's `NoTriggerGate`, whose
/// hold ends when the connect returns) or the pipe name the pipe checks passed (Windows: `address::check_pipe`); only
/// those make one, so a path the walk did not produce does not compile here: `dial(&LocalAddress::from_path(..), ..)`,
/// the call before plan 2.3e2, is a type error (E0308), which closes 2.3e round 3 Code N3-1.
pub(crate) fn dial(
    target: Target<'_>,
    policy: &PeerPolicy,
    limits: CarrierLimits,
    config: &ClientConfig,
    intent: &Intent,
) -> Result<(StreamCarrier, ClientOutcome), DialFault> {
    let started = Instant::now();
    // Before anything is connected: the request is written on this thread, so only its size bounds the write. It is
    // checked at the highest protocol this client speaks, the one it sends to a host that serves it.
    request_bytes(config, intent, *config.supported.end()).map_err(DialFault::Request)?;
    let wait = match intent {
        Intent::Open => config.bounds.open_wait,
        Intent::Control(_) => config.bounds.state_wait,
    };
    let raw = connect_stream(target, policy, started + wait).map_err(DialFault::Connect)?;
    let mut wire = StreamWire::new(raw)
        .map_err(|error| DialFault::Connect(LocalRefusal::os("spawn", &error)))?;
    let mut config = config.clone();
    config.peer_pid = config.peer_pid.or(wire.peer_pid());
    let outcome =
        client_handshake(&mut wire, started, &config, intent).map_err(DialFault::Handshake)?;
    let carrier = wire.into_carrier(limits).map_err(DialFault::Connect)?;
    Ok((carrier, outcome))
}

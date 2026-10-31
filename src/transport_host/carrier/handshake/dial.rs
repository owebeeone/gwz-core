//! The client's whole open of a local server: connect, handshake, carrier (plan step 2.3d-2).
//!
//! [`dial`] starts the clock, connects to the address with the deadline the handshake's bound gives (a full accept
//! queue or a busy pipe is waited for until then), runs the client's handshake over the raw stream, and only then
//! builds the carrier. Until the handshake is done the stream is read by a helper thread, so that a host that goes
//! quiet is given up on at the bound by aborting the stream, on every platform: a blocked read cannot be timed out
//! otherwise (a pipe's read has no timeout).
use super::super::{
    CarrierLimits, StreamCarrier,
    local::{LocalAddress, LocalRefusal, PeerIdentity, PeerPolicy, RawConnection, connect_stream},
};
use super::{
    ByteWire, ClientConfig, ClientFault, ClientOutcome, Heard, Intent, WireEnd, client_handshake,
};
use std::{
    io::{self, Read, Write},
    sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel},
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
fn pump(mut read: Box<dyn Read + Send>, asks: Receiver<Ask>, replies: Sender<Reply>) {
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
                Ok(read) => filled += read,
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
    reader: Option<JoinHandle<()>>,
}

impl StreamWire {
    pub(crate) fn new(raw: RawConnection) -> io::Result<Self> {
        let (asks, asked) = channel();
        let (replied, replies) = channel();
        let read = raw.read;
        let reader = thread::Builder::new()
            .name("gwz-handshake-read".into())
            .spawn(move || pump(read, asked, replied))?;
        Ok(Self {
            write: raw.write,
            abort: raw.abort,
            peer: raw.peer,
            side: raw.side,
            asks,
            replies,
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
            Err(RecvTimeoutError::Timeout) => Ok(Heard::Silent),
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
    /// The connect, the peer check or the carrier's start failed or ran out of time.
    Connect(LocalRefusal),
    /// The handshake failed; the stream was aborted.
    Handshake(ClientFault),
}

/// Connects to `address` and handshakes, all within the bound `intent` gives from the moment of the call. The
/// carrier is returned with the handshake's outcome; for a control exchange the host closes it next.
pub(crate) fn dial(
    address: &LocalAddress,
    policy: &PeerPolicy,
    limits: CarrierLimits,
    config: &ClientConfig,
    intent: &Intent,
) -> Result<(StreamCarrier, ClientOutcome), DialFault> {
    let started = Instant::now();
    let wait = match intent {
        Intent::Open => config.bounds.open_wait,
        Intent::Control(_) => config.bounds.state_wait,
    };
    let raw = connect_stream(address, policy, started + wait).map_err(DialFault::Connect)?;
    let mut wire = StreamWire::new(raw)
        .map_err(|error| DialFault::Connect(LocalRefusal::os("spawn", &error)))?;
    let mut config = config.clone();
    config.peer_pid = config.peer_pid.or(wire.peer_pid());
    let outcome =
        client_handshake(&mut wire, started, &config, intent).map_err(DialFault::Handshake)?;
    let carrier = wire.into_carrier(limits).map_err(DialFault::Connect)?;
    Ok((carrier, outcome))
}

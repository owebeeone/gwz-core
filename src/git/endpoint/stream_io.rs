//! Blocking Git-facing access to a message stream. The host pumps messages and
//! timers independently; this adapter owns no socket, timer or delivery queue.
use gwz_transport::stream::{CloseResult, Error, Stream};
use std::{
    future::Future,
    io::{self, Read, Write},
    pin::pin,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU32, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};

/// How long a push's producer may stay outside every call on its stream, with
/// no byte moving, before the pump charges the silence to the network again:
/// ten minutes (operator decision D45).
pub(crate) const PRODUCER_CEILING_MS: u64 = 600_000;

/// A call has returned at least once.
const ARMED: u32 = 1 << 31;
/// The calls in flight.
const CALLS: u32 = ARMED - 1;

/// Tells the pump of a push whether the thread that owns the stream is
/// producing: outside every call on it. libgit2 reads a whole object into
/// memory before it writes the object's first byte, and that silence is the
/// producer's, not the network's (GwzPushProducerBusyDesign). One word holds
/// the armed bit and the number of calls in flight, so that [`busy`] is one load
/// and a thread about to enter a call can never read busy.
///
/// It lives in the process of the stream and its pump (the local placement); it
/// crosses no carrier. A stream with no gate, or one no call has returned from,
/// is never busy, which is the pump's behaviour without a gate.
///
/// [`busy`]: ProducerGate::busy
#[derive(Clone)]
pub(crate) struct ProducerGate(Arc<GateState>);
struct GateState {
    calls: AtomicU32,
    ceiling_ms: u64,
}
impl ProducerGate {
    pub(crate) fn new(ceiling_ms: u64) -> Self {
        Self(Arc::new(GateState {
            calls: AtomicU32::new(0),
            ceiling_ms,
        }))
    }

    /// The most the pump lets the producer stay busy with no byte moving.
    pub(crate) fn ceiling_ms(&self) -> u64 {
        self.0.ceiling_ms
    }

    /// The owner of the stream is outside every call on it, and has made one.
    pub(crate) fn busy(&self) -> bool {
        let calls = self.0.calls.load(Ordering::Acquire);
        calls & ARMED != 0 && calls & CALLS == 0
    }

    pub(crate) fn enter(&self) -> CallGuard {
        self.0.calls.fetch_add(1, Ordering::AcqRel);
        CallGuard(self.clone())
    }
}
/// One call in flight on a gated stream; the call ends when it drops, on every
/// path out of it.
pub(crate) struct CallGuard(ProducerGate);
impl Drop for CallGuard {
    fn drop(&mut self) {
        let _ = self
            .0
            .0
            .calls
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |calls| {
                Some((calls | ARMED) - 1)
            });
    }
}

/// A file-like view of one existing exchange. Clones share byte positions and
/// cancellation. Never call from the worker responsible for delivery/timers.
#[derive(Clone)]
pub struct BlockingStream {
    stream: Stream,
    /// The stall bound this stream's exchange runs under, in milliseconds;
    /// 0 when the stall clock is off or the opener did not say. A timeout
    /// names the bound only when it is not 0.
    stall_ms: u64,
    /// Whether the thread that owns this stream is inside a call on it, when
    /// the pump of a push asked to know.
    producer: Option<ProducerGate>,
}
pub(crate) const REPOSITORY_REFUSED: &str = "GWZ SSH repository access refused";
impl BlockingStream {
    pub fn new(stream: Stream) -> Self {
        Self {
            stream,
            stall_ms: 0,
            producer: None,
        }
    }

    /// This stream with a gate that says whether its owner is inside a call.
    pub(crate) fn with_producer(mut self, producer: Option<ProducerGate>) -> Self {
        self.producer = producer;
        self
    }

    /// Marks a call on this stream, for the gate's sake; ends when dropped.
    fn call(&self) -> Option<CallGuard> {
        self.producer.as_ref().map(ProducerGate::enter)
    }

    /// This stream with the stall bound (`--ssh-timeout`, in milliseconds)
    /// its exchange was opened under, so that a stalled read says so.
    pub(crate) fn with_stall_ms(mut self, stall_ms: u64) -> Self {
        self.stall_ms = stall_ms;
        self
    }

    pub(crate) fn retained_failure(&self) -> Option<gwz_transport::protocol::Failure> {
        self.stream.retained_failure()
    }

    pub(crate) fn retained_failure_facts(&self) -> Option<gwz_transport::protocol::Facts> {
        self.stream.retained_failure_facts()
    }

    /// Half-close outgoing bytes; incoming bytes remain readable.
    pub fn end_write(&self) -> io::Result<()> {
        let _call = self.call();
        wait(self.stream.end_write()).map_err(|error| io_error(error, self.stall_ms))
    }

    /// Wait for endpoint cleanup and return its facts; does not imply Git success.
    pub fn close(&self) -> io::Result<CloseResult> {
        let _call = self.call();
        wait(self.stream.close()).map_err(|error| io_error(error, self.stall_ms))
    }

    /// Wake pending calls and request cancellation, without asserting reuse.
    pub fn cancel(&self) {
        self.stream.cancel();
    }
}
impl Read for BlockingStream {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let _call = self.call();
        wait(self.stream.read(output)).map_err(|error| io_error(error, self.stall_ms))
    }
}
impl Write for BlockingStream {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        let _call = self.call();
        wait(self.stream.write(input)).map_err(|error| io_error(error, self.stall_ms))
    }

    fn flush(&mut self) -> io::Result<()> {
        let _call = self.call();
        wait(self.stream.flush()).map_err(|error| io_error(error, self.stall_ms))
    }
}

/// The stall bound, in seconds as `--ssh-timeout` takes it, for a message
/// that names the flag and its value (T7): `9` for 9000 ms, and `0.3` for the
/// 300 ms only a test fixture sets. `None` when the bound is 0: `--ssh-timeout
/// 0` turns the stall clock off, so no stall can be its doing and no message
/// may name it. It lives here, not in `setup_retry`, because the consumer
/// proof loads this file alone (tests/transport_consumer/README.md).
pub(crate) fn stall_seconds(stall_ms: u64) -> Option<String> {
    if stall_ms == 0 {
        return None;
    }
    let (whole, rest) = (stall_ms / 1000, stall_ms % 1000);
    Some(if rest == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{rest:03}")
            .trim_end_matches('0')
            .to_owned()
    })
}

/// The unit after a [`stall_seconds`] value: `1 second`, and `seconds` for
/// every other value, `0.3 seconds` included.
pub(crate) fn seconds_unit(seconds: &str) -> &'static str {
    if seconds == "1" { "second" } else { "seconds" }
}

/// A stalled read or write, worded for the flag that sets the bound it ran
/// out of (T7). The stream's own error stays as its source.
#[derive(Debug)]
pub(crate) struct StreamStall {
    error: Error,
    seconds: String,
}
impl std::fmt::Display for StreamStall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let seconds = &self.seconds;
        let unit = seconds_unit(seconds);
        write!(
            f,
            "no progress for {seconds} {unit} (--ssh-timeout {seconds})"
        )
    }
}
impl std::error::Error for StreamStall {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

pub(crate) fn io_error(error: Error, stall_ms: u64) -> io::Error {
    let stalled = matches!(
        error,
        Error::Timeout
            | Error::PeerFailed {
                code: gwz_transport::protocol::ErrorCode::Timeout,
                ..
            }
    );
    // With the stall clock off (`--ssh-timeout 0`, or an opener that did not
    // say) there is no bound to name, and the message stays the stream's own.
    if stalled && let Some(seconds) = stall_seconds(stall_ms) {
        return io::Error::new(io::ErrorKind::TimedOut, StreamStall { error, seconds });
    }
    let kind = match error {
        // Interrupted would make std::io::write_all retry a terminal error.
        Error::Cancelled => io::ErrorKind::ConnectionAborted,
        Error::CarrierLost | Error::WriteClosed | Error::Closed => io::ErrorKind::BrokenPipe,
        Error::Timeout => io::ErrorKind::TimedOut,
        Error::Protocol => io::ErrorKind::InvalidData,
        Error::InvalidConfig | Error::WrongState | Error::WrongSide => io::ErrorKind::InvalidInput,
        Error::PeerFailed { code, .. } => {
            if code == gwz_transport::protocol::ErrorCode::RepositoryRefused {
                return io::Error::new(io::ErrorKind::PermissionDenied, REPOSITORY_REFUSED);
            }
            match code {
                gwz_transport::protocol::ErrorCode::Timeout => io::ErrorKind::TimedOut,
                gwz_transport::protocol::ErrorCode::Cancelled => io::ErrorKind::ConnectionAborted,
                _ => io::ErrorKind::Other,
            }
        }
        Error::WouldBlock | Error::WaiterCapacity => io::ErrorKind::Other,
    };
    io::Error::new(kind, error)
}

#[derive(Default)]
struct Signal {
    ready: Mutex<bool>,
    changed: Condvar,
}
impl Wake for Signal {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        *self.ready.lock().unwrap_or_else(|error| error.into_inner()) = true;
        self.changed.notify_one();
    }
}

fn wait<F: Future>(future: F) -> F::Output {
    let signal = Arc::new(Signal::default());
    let waker = Waker::from(signal.clone());
    let mut context = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        // No wake mutex held during poll: wake may run synchronously inside it.
        if let Poll::Ready(result) = future.as_mut().poll(&mut context) {
            return result;
        }
        let mut ready = signal
            .ready
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        while !*ready {
            ready = signal
                .changed
                .wait(ready)
                .unwrap_or_else(|error| error.into_inner());
        }
        *ready = false;
    }
}

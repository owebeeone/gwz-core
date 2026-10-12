//! Blocking Git-facing access to a message stream. The host pumps messages and
//! timers independently; this adapter owns no socket, timer or delivery queue.
use gwz_transport::stream::{CloseResult, Error, Stream};
use std::{
    future::Future,
    io::{self, Read, Write},
    pin::pin,
    sync::{Arc, Condvar, Mutex},
    task::{Context, Poll, Wake, Waker},
};

/// A file-like view of one existing exchange. Clones share byte positions and
/// cancellation. Never call from the worker responsible for delivery/timers.
#[derive(Clone)]
pub struct BlockingStream {
    stream: Stream,
    /// The stall bound this stream's exchange runs under, in milliseconds;
    /// 0 when the stall clock is off or the opener did not say. A timeout
    /// names the bound only when it is not 0.
    stall_ms: u64,
}
pub(crate) const REPOSITORY_REFUSED: &str = "GWZ SSH repository access refused";
impl BlockingStream {
    pub fn new(stream: Stream) -> Self {
        Self {
            stream,
            stall_ms: 0,
        }
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
        wait(self.stream.end_write()).map_err(|error| io_error(error, self.stall_ms))
    }

    /// Wait for endpoint cleanup and return its facts; does not imply Git success.
    pub fn close(&self) -> io::Result<CloseResult> {
        wait(self.stream.close()).map_err(|error| io_error(error, self.stall_ms))
    }

    /// Wake pending calls and request cancellation, without asserting reuse.
    pub fn cancel(&self) {
        self.stream.cancel();
    }
}
impl Read for BlockingStream {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        wait(self.stream.read(output)).map_err(|error| io_error(error, self.stall_ms))
    }
}
impl Write for BlockingStream {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        wait(self.stream.write(input)).map_err(|error| io_error(error, self.stall_ms))
    }

    fn flush(&mut self) -> io::Result<()> {
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

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
        write!(
            f,
            "no progress for {seconds} seconds (--ssh-timeout {seconds})"
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
    if stalled && let Some(seconds) = super::setup_retry::stall_seconds(stall_ms) {
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

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests {
            use super::*;
            use gwz_transport::protocol::{Effect, ErrorCode};

            fn peer_timeout() -> Error {
                Error::PeerFailed {
                    code: ErrorCode::Timeout,
                    effect: Effect::Possible,
                }
            }

            #[test]
            fn a_stalled_read_names_the_flag_and_its_value() {
                for error in [Error::Timeout, peer_timeout()] {
                    let reported = io_error(error, 9_000);
                    assert_eq!(reported.kind(), io::ErrorKind::TimedOut);
                    assert_eq!(
                        reported.to_string(),
                        "no progress for 9 seconds (--ssh-timeout 9)"
                    );
                    // The stream's own error stays reachable as the source.
                    let stall = reported.get_ref().unwrap();
                    assert!(
                        stall
                            .source()
                            .is_some_and(|source| source.downcast_ref::<Error>() == Some(&error))
                    );
                }
            }

            #[test]
            fn a_stalled_read_names_a_value_other_than_the_default() {
                let reported = io_error(Error::Timeout, 45_000);
                assert_eq!(
                    reported.to_string(),
                    "no progress for 45 seconds (--ssh-timeout 45)"
                );
            }

            #[test]
            fn no_stall_message_names_a_zero_bound() {
                // `--ssh-timeout 0` turns the stall clock off, so a timeout that reaches
                // a stream opened under it, or opened with no bound recorded, is not
                // worded as a stall.
                for error in [Error::Timeout, peer_timeout()] {
                    let reported = io_error(error, 0);
                    assert_eq!(reported.kind(), io::ErrorKind::TimedOut);
                    assert!(!reported.to_string().contains("--ssh-timeout"));
                    assert!(!reported.to_string().contains("no progress"));
                }
            }

            #[test]
            fn only_a_timeout_is_worded_as_a_stall() {
                for (error, kind) in [
                    (Error::Cancelled, io::ErrorKind::ConnectionAborted),
                    (Error::CarrierLost, io::ErrorKind::BrokenPipe),
                    (Error::Protocol, io::ErrorKind::InvalidData),
                ] {
                    let reported = io_error(error, 9_000);
                    assert_eq!(reported.kind(), kind);
                    assert!(!reported.to_string().contains("--ssh-timeout"));
                }
                let other = Error::PeerFailed {
                    code: ErrorCode::Protocol,
                    effect: Effect::None,
                };
                assert!(
                    !io_error(other, 9_000)
                        .to_string()
                        .contains("--ssh-timeout")
                );
            }
        }
    }
}

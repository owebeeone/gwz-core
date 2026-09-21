//! Blocking Git-facing access to a message stream. The host pumps messages and
//! timers independently; this adapter owns no socket, timer or delivery queue.
use gwz_transport::stream::{CloseResult, Error, Stream};
use std::{
    future::Future,
    io::{self, Read, Write},
    pin::pin,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};

/// A file-like view of one existing exchange. Clones share byte positions and
/// cancellation. Never call from the worker responsible for delivery/timers.
#[derive(Clone)]
pub struct BlockingStream {
    stream: Stream,
    repository_refused: Option<Arc<AtomicBool>>,
}
pub(crate) const REPOSITORY_REFUSED: &str = "GWZ SSH repository access refused";
impl BlockingStream {
    pub fn new(stream: Stream) -> Self {
        Self {
            stream,
            repository_refused: None,
        }
    }
    pub(crate) fn with_repository_receipt(stream: Stream, receipt: Arc<AtomicBool>) -> Self {
        Self {
            stream,
            repository_refused: Some(receipt),
        }
    }

    /// Half-close outgoing bytes; incoming bytes remain readable.
    pub fn end_write(&self) -> io::Result<()> {
        wait(self.stream.end_write()).map_err(io_error)
    }

    /// Wait for endpoint cleanup and return its facts; does not imply Git success.
    pub fn close(&self) -> io::Result<CloseResult> {
        wait(self.stream.close()).map_err(io_error)
    }

    /// Wake pending calls and request cancellation, without asserting reuse.
    pub fn cancel(&self) {
        self.stream.cancel();
    }
}
impl Read for BlockingStream {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let result = wait(self.stream.read(output)).map_err(io_error);
        if !output.is_empty()
            && self
                .repository_refused
                .as_ref()
                .is_some_and(|r| r.load(Ordering::Acquire))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                REPOSITORY_REFUSED,
            ));
        }
        result
    }
}
impl Write for BlockingStream {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        wait(self.stream.write(input)).map_err(io_error)
    }

    fn flush(&mut self) -> io::Result<()> {
        wait(self.stream.flush()).map_err(io_error)
    }
}

fn io_error(error: Error) -> io::Error {
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

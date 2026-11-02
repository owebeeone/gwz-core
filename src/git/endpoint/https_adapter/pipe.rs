//! One exchange's two halves: the client's `ExchangeIo` (the host's `MessageStream` polls it) and the state it shares
//! with the exchange's task on the worker's runtime: the request body (a POST's), the response body the task has read
//! and the client has not, how the client ended it, and the waker of the client's pending read.
//!
//! The client's polls are woken, never re-polled (contract section 9; Phase 1 Code P2-4, step R2b). A pending read leaves
//! its waker in the shared state, which the task's `deliver` and `end` and the client's `abort` wake. A pending write
//! waits for room in the request body's channel (`PollSender`), so a write that is `Pending` has accepted nothing.
//! `poll_shutdown_write` ends a POST's request body, and Hyper sends its end; an advertisement has no body, so there it
//! is `Ready(Ok)` with nothing to end. Smart HTTP is half duplex (a client reads the response only after it has written
//! its whole request), so the client's first read ends the body too, as its finish does.
use super::{Binding, exchange::Work, stream_error};
use crate::git::endpoint::ssh_adapter::Settle;
use bytes::Bytes;
use gwz_connection::{AuthContext, ExchangeIo, ExchangeSpec};
use gwz_transport::protocol::{Disposition, ErrorCode};
use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Mutex, MutexGuard},
    task::{Context, Poll, Waker, ready},
};
use tokio::sync::{Notify, mpsc};
use tokio_util::sync::PollSender;

/// Response bytes the task holds for the client before it reads more from the connection.
const RESPONSE_BUFFER: usize = 64 * 1024;
/// Request body chunks queued for Hyper.
const BODY_CHUNKS: usize = 8;
/// The most one `write` accepts.
const CHUNK: usize = 64 * 1024;

#[derive(Default)]
struct State {
    response: VecDeque<u8>,
    /// Set by the task: `Ok` at the response body's validated end, the failure otherwise.
    end: Option<Result<(), ErrorCode>>,
    /// The client asked for the orderly end.
    finishing: bool,
    /// The client abandoned the exchange.
    aborted: bool,
    /// The waker of the client's pending read: the response's next bytes, its end or the abandonment wake it.
    reader: Option<Waker>,
}
/// Wakes the client's pending read, if any, once `state`'s lock is released.
fn wake_reader(mut state: MutexGuard<'_, State>) {
    let reader = state.reader.take();
    drop(state);
    if let Some(reader) = reader {
        reader.wake();
    }
}
/// What the task and the client share. The task waits on `changed`, which every client call that can unblock it
/// notifies.
#[derive(Default)]
pub(super) struct Pipe {
    state: Mutex<State>,
    changed: Notify,
}
/// Why the task stops reading: the client ended or abandoned the exchange before the response's end.
pub(super) struct Interrupted;
impl Pipe {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// Waits until `ready` holds for the state.
    async fn until(&self, ready: impl Fn(&State) -> bool) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if ready(&self.lock()) {
                return;
            }
            changed.await;
        }
    }
    /// Resolves once the client has ended or abandoned the exchange.
    pub(super) async fn interrupted(&self) -> Interrupted {
        self.until(|state| state.finishing || state.aborted).await;
        Interrupted
    }
    /// Waits for room in the response buffer; `Err` when the client ended it first (an unread body).
    pub(super) async fn room(&self) -> Result<(), Interrupted> {
        self.until(|state| {
            state.response.len() < RESPONSE_BUFFER || state.finishing || state.aborted
        })
        .await;
        let state = self.lock();
        if state.finishing || state.aborted {
            Err(Interrupted)
        } else {
            Ok(())
        }
    }
    pub(super) fn deliver(&self, bytes: &[u8]) {
        let mut state = self.lock();
        state.response.extend(bytes);
        wake_reader(state);
    }
    /// The response's end, as the client's reads will see it once the buffer is drained. The first word stands.
    pub(super) fn end(&self, result: Result<(), ErrorCode>) {
        let mut state = self.lock();
        if state.end.is_none() {
            state.end = Some(result);
            wake_reader(state);
        }
    }
    /// After the response's validated end: whether the client read all of it and asked for the end (`true`), or
    /// abandoned it or ended it with bytes unread (`false`).
    pub(super) async fn settled(&self) -> bool {
        self.until(|state| state.finishing || state.aborted).await;
        let state = self.lock();
        !state.aborted && state.response.is_empty()
    }
}

/// The client's half. See the module doc.
pub(crate) struct HttpsExchange {
    /// A POST's request body until it ends; `None` for an advertisement and once ended. A pending write's reservation
    /// stays in it, so the room that frees wakes that write.
    body: Option<PollSender<io::Result<Bytes>>>,
    advertisement: bool,
    pipe: Arc<Pipe>,
    settle: Arc<Settle>,
}
/// The two halves of an exchange for `spec` of the operation whose context is `auth`, which `settle` ends.
pub(super) fn exchange(
    spec: &ExchangeSpec,
    auth: &AuthContext,
    settle: Arc<Settle>,
) -> (HttpsExchange, Work) {
    let pipe = Arc::new(Pipe::default());
    let advertisement = spec.service().is_advertisement();
    let (sender, receiver) = if advertisement {
        (None, None)
    } else {
        let (sender, receiver) = mpsc::channel(BODY_CHUNKS);
        (Some(sender), Some(receiver))
    };
    let work = Work {
        key: spec.key().clone(),
        service: spec.service(),
        path: spec.path().into(),
        body: receiver,
        auth: auth.clone(),
        prior: Binding::Anonymous,
        authenticated: None,
        pipe: pipe.clone(),
        settle: settle.clone(),
    };
    (
        HttpsExchange {
            body: sender.map(PollSender::new),
            advertisement,
            pipe,
            settle,
        },
        work,
    )
}
impl HttpsExchange {
    /// The request body is complete: Hyper sends its end.
    fn end_body(&mut self) {
        self.body = None;
    }
    /// Reads response bytes into `out`, or the response's end; `None` when neither is there yet, having left `waker` to
    /// be woken by what comes.
    fn take(&mut self, out: &mut [u8], waker: &Waker) -> Option<io::Result<usize>> {
        self.end_body();
        if out.is_empty() {
            return Some(Ok(0));
        }
        let mut state = self.pipe.lock();
        if !state.response.is_empty() {
            let n = out.len().min(state.response.len());
            for (slot, byte) in out.iter_mut().zip(state.response.drain(..n)) {
                *slot = byte;
            }
            drop(state);
            // A read makes room: the task may read more of the response.
            self.pipe.changed.notify_one();
            return Some(Ok(n));
        }
        match state.end {
            Some(Ok(())) => Some(Ok(0)),
            Some(Err(code)) => Some(Err(stream_error(code))),
            // The client abandoned the exchange: no end will come.
            None if state.aborted => Some(Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "the exchange was abandoned",
            ))),
            None => {
                state.reader = Some(waker.clone());
                None
            }
        }
    }
}
fn body_closed() -> io::Error {
    io::Error::new(
        io::ErrorKind::BrokenPipe,
        "the exchange no longer takes its request body",
    )
}
impl ExchangeIo for HttpsExchange {
    fn poll_read(&mut self, cx: &mut Context<'_>, out: &mut [u8]) -> Poll<io::Result<usize>> {
        self.take(out, cx.waker())
            .map_or(Poll::Pending, Poll::Ready)
    }
    fn poll_write(&mut self, cx: &mut Context<'_>, data: &[u8]) -> Poll<io::Result<usize>> {
        if data.is_empty() {
            return Poll::Ready(Ok(0));
        }
        if self.advertisement {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "an advertisement request has no body",
            )));
        }
        let Some(body) = &mut self.body else {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the request body has ended",
            )));
        };
        // A slot first: `Pending` leaves the reservation (and `cx`'s waker) with the channel and accepts nothing.
        if ready!(body.poll_reserve(cx)).is_err() {
            return Poll::Ready(Err(body_closed()));
        }
        let accepted = data.len().min(CHUNK);
        match body.send_item(Ok(Bytes::copy_from_slice(&data[..accepted]))) {
            Ok(()) => Poll::Ready(Ok(accepted)),
            Err(_) => Poll::Ready(Err(body_closed())),
        }
    }
    /// Ends a POST's request body: Hyper sends its end, and later writes fail. An advertisement has no body to end, so
    /// it is `Ready(Ok)` there too.
    fn poll_shutdown_write(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let _ = cx;
        self.end_body();
        Poll::Ready(Ok(()))
    }
    fn poll_finish(&mut self, cx: &mut Context<'_>) -> Poll<Disposition> {
        self.end_body();
        let first = {
            let mut state = self.pipe.lock();
            !std::mem::replace(&mut state.finishing, true)
        };
        if first {
            self.pipe.changed.notify_one();
        }
        self.settle.poll(cx)
    }
    fn abort(&mut self) {
        self.end_body();
        let mut state = self.pipe.lock();
        state.aborted = true;
        wake_reader(state);
        self.pipe.changed.notify_one();
        self.settle.end(Disposition::Discarded);
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod tests;
    }
}

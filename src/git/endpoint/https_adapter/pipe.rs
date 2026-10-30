//! One exchange's two halves: the client's `ExchangeIo` (the host's `MessageStream` reads and writes it without
//! blocking) and the state it shares with the exchange's task on the worker's runtime: the request body (a POST's),
//! the response body the task has read and the client has not, and how the client ended it.
//!
//! `ExchangeIo` has no `shutdown_write` yet (step 1.3b's finding; contract 9 names one), so a POST's request body ends
//! at the client's first `read` (or at its finish): smart HTTP is half duplex, a client reads the response only after
//! it has written its whole request.
use super::{Binding, exchange::Work, stream_error};
use crate::git::endpoint::ssh_adapter::Settle;
use bytes::Bytes;
use gwz_connection::{AuthContext, ExchangeIo, ExchangeSpec};
use gwz_transport::protocol::{Disposition, ErrorCode};
use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};
use tokio::sync::{
    Notify,
    mpsc::{self, error::TrySendError},
};

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
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
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
        self.lock().response.extend(bytes);
    }
    /// The response's end, as the client's reads will see it once the buffer is drained. The first word stands.
    pub(super) fn end(&self, result: Result<(), ErrorCode>) {
        let mut state = self.lock();
        if state.end.is_none() {
            state.end = Some(result);
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
    /// A POST's request body until it ends; `None` for an advertisement and once ended.
    body: Option<mpsc::Sender<io::Result<Bytes>>>,
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
            body: sender,
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
}
impl ExchangeIo for HttpsExchange {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }
        if self.advertisement {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "an advertisement request has no body",
            ));
        }
        let Some(body) = &self.body else {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the request body has ended",
            ));
        };
        let accepted = data.len().min(CHUNK);
        match body.try_send(Ok(Bytes::copy_from_slice(&data[..accepted]))) {
            Ok(()) => Ok(accepted),
            Err(TrySendError::Full(_)) => Err(io::ErrorKind::WouldBlock.into()),
            Err(TrySendError::Closed(_)) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the exchange no longer takes its request body",
            )),
        }
    }
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.end_body();
        if out.is_empty() {
            return Ok(0);
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
            return Ok(n);
        }
        match state.end {
            Some(Ok(())) => Ok(0),
            Some(Err(code)) => Err(stream_error(code)),
            None => Err(io::ErrorKind::WouldBlock.into()),
        }
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
        self.pipe.lock().aborted = true;
        self.pipe.changed.notify_one();
        self.settle.end(Disposition::Discarded);
    }
}

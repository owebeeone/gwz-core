//! The SSH endpoint adapter of the connection host (plan step 1.3b; contract sections 4.1, 6, 8 and 9 of
//! dev-docs/GwzConnectionApiContract.md): a `gwz_connection::EndpointAdapter` at the Connector/Resource level, over
//! today's SSH connector (`ssh_local::native_connector`: trust, password, selected key, helpers, agent), its
//! `NativeResource`s and their channel pump.
//!
//! - `connect` starts one physical SSH connection (one setup job) for the connection the host's pool named;
//!   `cancel_connect` and `close` dispose of it, and each ends with exactly one completion.
//! - `start_exchange` runs one exchange on a channel of a leased, idle connection: the Git service and path of the
//!   `ExchangeSpec`, a fresh pair of in-process streams, and the channel pump. The connection is idle again only after
//!   the pump's clean close and the reclaim of its session; the exchange's end (`ExchangeIo::poll_finish`) says
//!   `Reusable` only then.
//! - Idle liveness: an idle session whose peer closes it, resets it or sends unasked bytes is disposed and reported
//!   `Completion::IdleClosed` (the resource's idle watch).
//! - Fault scope: a stream or channel failure ends that exchange only (`Discarded`; the host's pool closes that one
//!   connection); a connection's physical failure retires that connection only; neither touches another connection or
//!   the host.
//!
//! It owns no pool and no reuse policy: what to connect, which idle connection to lease and when to close are the
//! host's. Its I/O runs on the adapter's own worker thread (`worker.rs`), started by the first `connect`, outside every
//! connection-host lock; the methods here only queue commands and read completions. Nothing here is a global.
//!
//! HTTPS is served by `https_adapter` (step 1.3c): the production factory's router (`transport_host/connection_factory.rs`)
//! sends each connect to the adapter its key's scheme names, so a connect for another scheme that reaches this adapter
//! still completes at once with `UnsupportedOperation`. Its outbox and exchange settlement (`Shared`, `Settle`) are the
//! HTTPS adapter's too.
use super::{
    ssh_channel::GitService as ChannelService,
    ssh_pool::Connector,
    ssh_worker::{BridgeContext, ChannelResource, stream_config},
};
use gwz_connection::{
    Completion, ConnectCommand, EndpointAdapter, ExchangeIo, ExchangeSpec, GitService,
};
use gwz_transport::{
    pool::ConnectionId,
    protocol::{Deadlines, Disposition},
    stream::{MessageEndpoint, Side, Stream},
};
use std::{
    collections::VecDeque,
    future::Future,
    io,
    pin::{Pin, pin},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender},
    },
    task::{Context, Poll, Waker},
    thread::{self, JoinHandle, Thread},
    time::Instant,
};

mod worker;

/// The budgets of the adapter's connections and exchanges, from the endpoint's pool policy.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AdapterSettings {
    /// The setup's bound (0: none), from the connect.
    pub(crate) connect_timeout_ms: u64,
    /// An exchange's network stall bound (0: none).
    pub(crate) io_timeout_ms: u64,
    pub(crate) interaction_timeout_ms: u64,
    /// The bound on a channel's or a connection's orderly close, after which it is forced.
    pub(crate) cleanup_timeout_ms: u64,
}
impl AdapterSettings {
    fn deadlines(&self) -> Deadlines {
        Deadlines {
            allocation_ms: 0,
            connect_ms: self.connect_timeout_ms as i64,
            io_ms: self.io_timeout_ms as i64,
            interaction_ms: self.interaction_timeout_ms as i64,
            cleanup_ms: self.cleanup_timeout_ms as i64,
        }
    }
}

/// What the worker reports, and the wakers of the host's driver and of a disposal.
#[derive(Default)]
struct Outbox {
    completions: VecDeque<Completion>,
    waker: Option<Waker>,
    disposed: bool,
    dispose_waker: Option<Waker>,
}
#[derive(Default)]
pub(super) struct Shared {
    outbox: Mutex<Outbox>,
}
impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, Outbox> {
        self.outbox.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub(super) fn push(&self, completion: Completion) {
        let waker = {
            let mut outbox = self.lock();
            outbox.completions.push_back(completion);
            outbox.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
    pub(super) fn disposed(&self) {
        let wakers = {
            let mut outbox = self.lock();
            outbox.disposed = true;
            [outbox.waker.take(), outbox.dispose_waker.take()]
        };
        for waker in wakers.into_iter().flatten() {
            waker.wake();
        }
    }
    /// The next completion, or `Pending` with the driver's waker registered (the HTTPS adapter shares this outbox).
    pub(super) fn poll_completion(&self, cx: &mut Context<'_>) -> Poll<Completion> {
        let mut outbox = self.lock();
        match outbox.completions.pop_front() {
            Some(completion) => Poll::Ready(completion),
            None => {
                outbox.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
    /// `Ready` once the worker has confirmed disposal, or `Pending` with the disposal's waker registered.
    pub(super) fn poll_disposed(&self, cx: &mut Context<'_>) -> Poll<()> {
        let mut outbox = self.lock();
        if outbox.disposed {
            return Poll::Ready(());
        }
        outbox.dispose_waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

/// How an exchange ended, as the worker found it: set once, read by the exchange's `poll_finish`.
#[derive(Default)]
pub(super) struct Settle {
    state: Mutex<SettleState>,
}
#[derive(Default)]
struct SettleState {
    ended: Option<Disposition>,
    waker: Option<Waker>,
    /// The member asked for the orderly end (its close): the worker's cleanup bound runs from here.
    requested: bool,
}
impl Settle {
    fn lock(&self) -> std::sync::MutexGuard<'_, SettleState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// The first word stands.
    pub(super) fn end(&self, disposition: Disposition) {
        let waker = {
            let mut state = self.lock();
            if state.ended.is_some() {
                return;
            }
            state.ended = Some(disposition);
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
    pub(super) fn requested(&self) -> bool {
        self.lock().requested
    }
    pub(super) fn poll(&self, cx: &mut Context<'_>) -> Poll<Disposition> {
        let mut state = self.lock();
        state.requested = true;
        match state.ended {
            Some(disposition) => Poll::Ready(disposition),
            None => {
                state.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

/// The commands the adapter's methods queue for its worker.
pub(super) enum Command {
    Connect(ConnectCommand),
    CancelConnect(ConnectionId),
    Close(ConnectionId),
    Start {
        connection: ConnectionId,
        exchange: Box<Exchange>,
    },
    Dispose,
}
/// One exchange's worker half: the endpoint side of its streams (for the channel pump), the client's message endpoint
/// and how it ends.
pub(super) struct Exchange {
    pub(super) channel: Option<(Stream, MessageEndpoint)>,
    pub(super) peer: MessageEndpoint,
    pub(super) service: ChannelService,
    pub(super) path: String,
    pub(super) settle: Arc<Settle>,
}

type Start = Box<dyn FnOnce(Receiver<Command>, Arc<Shared>) -> io::Result<JoinHandle<()>> + Send>;
/// The SSH adapter of one endpoint instance. See the module doc.
pub(crate) struct SshAdapter {
    /// Starts the worker on the first connect.
    start: Option<(Start, Receiver<Command>)>,
    commands: Sender<Command>,
    worker: Option<Thread>,
    shared: Arc<Shared>,
    settings: AdapterSettings,
    dispose_sent: bool,
}
impl SshAdapter {
    /// An adapter over the connector `make` builds on its worker, on the worker's clock origin. Nothing starts here:
    /// no thread, no network, no key or agent access (contract section 5).
    pub(crate) fn new<C>(
        make: impl FnOnce(Instant) -> C + Send + 'static,
        settings: AdapterSettings,
    ) -> Self
    where
        C: Connector + Send + 'static,
        C::Resource: ChannelResource,
    {
        let (commands, receiver) = mpsc::channel();
        let start: Start = Box::new(move |receiver, shared| {
            thread::Builder::new()
                .name("gwz-ssh-adapter".into())
                .spawn(move || {
                    let origin = Instant::now();
                    worker::run(make(origin), origin, receiver, shared, settings);
                })
        });
        Self {
            start: Some((start, receiver)),
            commands,
            worker: None,
            shared: Arc::default(),
            settings,
            dispose_sent: false,
        }
    }
    fn send(&mut self, command: Command) {
        if let Some((start, receiver)) = self.start.take() {
            match start(receiver, self.shared.clone()) {
                Ok(join) => self.worker = Some(join.thread().clone()),
                Err(_) => {
                    // No worker: every connect fails at once and nothing physical exists.
                    self.shared.disposed();
                }
            }
        }
        let Some(worker) = &self.worker else {
            if let Command::Connect(command) = command {
                self.shared.push(Completion::Connected {
                    connection: command.connection,
                    result: Err(worker::failure(
                        gwz_transport::protocol::ErrorCode::Unavailable,
                    )),
                });
            }
            return;
        };
        if self.commands.send(command).is_ok() {
            worker.unpark();
        }
    }
}
impl EndpointAdapter for SshAdapter {
    fn connect(&mut self, command: ConnectCommand) {
        self.send(Command::Connect(command));
    }
    fn cancel_connect(&mut self, connection: ConnectionId) {
        self.send(Command::CancelConnect(connection));
    }
    fn close(&mut self, connection: ConnectionId) {
        self.send(Command::Close(connection));
    }
    fn start_exchange(
        &mut self,
        connection: ConnectionId,
        spec: &ExchangeSpec,
    ) -> io::Result<Box<dyn ExchangeIo>> {
        let worker = self.worker.clone().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotConnected, "SSH adapter has no worker")
        })?;
        let context = BridgeContext {
            session_id: "gwz-ssh-adapter".into(),
            stream_id: 1,
            version: 2,
            limits: gwz_transport::binding::default_limits(),
            deadlines: self.settings.deadlines(),
            waker: None,
        };
        let (client, peer) =
            Stream::new(stream_config(&context, Side::Initiator)).map_err(io::Error::other)?;
        let (stream, endpoint) =
            Stream::new(stream_config(&context, Side::Endpoint)).map_err(io::Error::other)?;
        let settle = Arc::new(Settle::default());
        let service = match spec.service() {
            GitService::UploadPackAdvertisement | GitService::UploadPackExchange => {
                ChannelService::UploadPack
            }
            GitService::ReceivePackAdvertisement | GitService::ReceivePackExchange => {
                ChannelService::ReceivePack
            }
        };
        self.send(Command::Start {
            connection,
            exchange: Box::new(Exchange {
                channel: Some((stream, endpoint)),
                peer,
                service,
                path: spec.path().into(),
                settle: settle.clone(),
            }),
        });
        Ok(Box::new(SshExchange {
            client,
            settle,
            worker,
            closing: None,
            closed: false,
        }))
    }
    fn poll_completion(&mut self, cx: &mut Context<'_>) -> Poll<Completion> {
        self.shared.poll_completion(cx)
    }
    fn poll_dispose(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        if self.worker.is_none() {
            // Never started: nothing physical was made.
            return Poll::Ready(());
        }
        if !self.dispose_sent {
            self.dispose_sent = true;
            self.send(Command::Dispose);
        }
        self.shared.poll_disposed(cx)
    }
}
impl Drop for SshAdapter {
    /// The last resort: the worker sees its command channel close, forces every connection's disposal and ends.
    fn drop(&mut self) {
        if let Some(worker) = &self.worker {
            worker.unpark();
        }
    }
}

/// One exchange's client half: the initiator stream the host's `MessageStream` reads and writes, without blocking.
struct SshExchange {
    client: Stream,
    settle: Arc<Settle>,
    worker: Thread,
    /// The member's orderly end (its EOF, then its close): `None` until asked for, and once it has run.
    closing: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
    closed: bool,
}
/// Polls a stream call once: its result now, or `WouldBlock`.
fn once<T>(future: impl Future<Output = Result<T, gwz_transport::stream::Error>>) -> io::Result<T> {
    match pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(Ok(value)) => Ok(value),
        Poll::Ready(Err(error)) => Err(io::Error::other(error)),
        Poll::Pending => Err(io::ErrorKind::WouldBlock.into()),
    }
}
impl ExchangeIo for SshExchange {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let written = once(self.client.write(data));
        self.worker.unpark();
        written
    }
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let read = once(self.client.read(out));
        // A read returns credit: the worker may now move more of the server's bytes.
        self.worker.unpark();
        read
    }
    fn poll_finish(&mut self, cx: &mut Context<'_>) -> Poll<Disposition> {
        if !self.closed {
            let closing = self.closing.get_or_insert_with(|| {
                let client = self.client.clone();
                Box::pin(async move {
                    let _ = client.end_write().await;
                    let _ = client.close().await;
                })
            });
            if closing.as_mut().poll(cx).is_ready() {
                self.closed = true;
                self.closing = None;
            }
        }
        self.worker.unpark();
        self.settle.poll(cx)
    }
    fn abort(&mut self) {
        self.client.cancel();
        self.settle.end(Disposition::Discarded);
        self.worker.unpark();
    }
}

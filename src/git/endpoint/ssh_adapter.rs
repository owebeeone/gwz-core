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
//!   `Completion::IdleClosed` (the resource's idle watch), once it has carried its first exchange. Before that only a
//!   hang-up retires it (the peer's FIN or reset, `Resource::poll_hung_up`): the server may write to a new session after
//!   authentication (OpenSSH's host-key notice), which the first exchange's channel reads, and those bytes must not
//!   retire it (accepted idle-loss design, `dev-docs/history/GwzTransportIdleLossDesign-ReviewState.md`).
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
    AuthContext, Completion, ConnectCommand, EndpointAdapter, ExchangeIo, ExchangeSpec, GitService,
    ProofId, ReuseCommand,
};
use gwz_transport::{
    pool::ConnectionId,
    protocol::{Deadlines, Disposition},
    stream::{Error as StreamError, MessageEndpoint, Side, Stream, StreamWaiter},
};
use std::{
    collections::VecDeque,
    future::Future,
    io,
    pin::Pin,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender},
    },
    task::{Context, Poll, Waker},
    thread::{self, JoinHandle, Thread},
    time::Instant,
};

mod claims;
mod worker;

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod lease_tests;

        /// The reports an adapter's worker has made that the host has not yet taken (the rows read them).
        #[derive(Clone)]
        pub(crate) struct Untaken(Arc<Shared>);
        impl Untaken {
            /// How many `IdleClosed` reports wait in the outbox.
            pub(crate) fn idle_closed(&self) -> usize {
                self.0
                    .lock()
                    .completions
                    .iter()
                    .filter(|completion| matches!(completion, Completion::IdleClosed { .. }))
                    .count()
            }
            /// How many reuse verdicts `Authorized` wait in the outbox.
            pub(crate) fn authorized(&self) -> usize {
                self.0
                    .lock()
                    .completions
                    .iter()
                    .filter(|completion| {
                        matches!(
                            completion,
                            Completion::Reuse {
                                verdict: gwz_connection::ReuseVerdict::Authorized,
                                ..
                            }
                        )
                    })
                    .count()
            }
            /// Whether the adapter's claims record nothing.
            pub(crate) fn claims_empty(&self) -> bool {
                self.0.claims.is_empty()
            }
            /// How many retirements stand in the adapter's claims.
            pub(crate) fn retired(&self) -> usize {
                self.0.claims.retired()
            }
        }
        impl Shared {
            pub(super) fn untaken(self: &Arc<Self>) -> Untaken {
                Untaken(self.clone())
            }
            /// Whether the host's lease of `connection` stands in the claims.
            pub(super) fn leased(&self, connection: ConnectionId) -> bool {
                self.claims.leased(connection)
            }
        }
        impl SshAdapter {
            pub(crate) fn untaken(&self) -> Untaken {
                self.shared.untaken()
            }
        }
    }
}

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
/// The deadline of setup work started at `now` (the adapter's own clock, milliseconds): the earlier of the adapter's own
/// bound `bound_ms` (none when 0) and the span the host gave the work, `remaining_ms` (none when `None`): a reuse proof's
/// opening (`ReuseCommand::remaining_ms`) or a connect's network deadline (`ConnectCommand::network_remaining_ms`,
/// Phase 1 State P3-2(e)). The work never outlasts the host's deadline; `None` only when neither bounds it. The SSH and
/// HTTPS adapters share it.
pub(super) fn bounded_deadline(now: u64, bound_ms: u64, remaining_ms: Option<u64>) -> Option<u64> {
    let own = (bound_ms != 0).then(|| now.saturating_add(bound_ms));
    let host = remaining_ms.map(|remaining| now.saturating_add(remaining));
    [own, host].into_iter().flatten().min()
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
    /// Which connections the host has leased and which the worker has retired (each adapter's own, as is its outbox).
    claims: claims::Claims,
}
/// The host's lease of an idle connection, from `start_exchange`'s claim until the exchange's start is queued for the
/// worker, whose `begun` then takes it up. Dropped unqueued (the start failed, or no worker took it), it is released, so
/// that a failed start leaves nothing standing between the connection and its retirement.
pub(super) struct Lease {
    shared: Arc<Shared>,
    connection: ConnectionId,
    queued: bool,
}
impl Lease {
    /// The start is queued: the worker's `begun` ends the lease.
    pub(super) fn queued(mut self) {
        self.queued = true;
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if !self.queued {
            self.shared.claims.release(self.connection);
        }
    }
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
    /// The host leases `connection` for an exchange whose start it is about to queue; `None`: the worker retired it.
    pub(super) fn lease(self: &Arc<Self>, connection: ConnectionId) -> Option<Lease> {
        self.claims.lease(connection).then(|| Lease {
            shared: self.clone(),
            connection,
            queued: false,
        })
    }
    /// The worker retires `connection`, an idle connection; false: the host leased it first, so it stays.
    pub(super) fn retire(&self, connection: ConnectionId) -> bool {
        self.claims.retire(connection)
    }
    /// The worker has taken up the exchange's start for `connection`.
    pub(super) fn begun(&self, connection: ConnectionId) {
        self.claims.begun(connection);
    }
    /// The next completion, or `Pending` with the driver's waker registered (each adapter has its own outbox). A
    /// completion that ends a connection is read by the host here, and its claims are forgotten here, not at the
    /// disposal: the host may lease a connection it still counts idle until it has read this. After this read the
    /// claims no longer refuse a start of it. The host can still make one: a pool lease it held while a reuse proof
    /// ran, whose `Authorized` verdict it serves after reading this report in the same pass. The production adapter
    /// shape refuses that start in `SchemeRouter`, whose route of the connection goes with this report
    /// (`NotConnected`, and the opening asks again); an adapter given to the host without the router has no such guard
    /// (fix round, CODE-P2-1).
    pub(super) fn poll_completion(&self, cx: &mut Context<'_>) -> Poll<Completion> {
        let completion = {
            let mut outbox = self.lock();
            match outbox.completions.pop_front() {
                Some(completion) => completion,
                None => {
                    outbox.waker = Some(cx.waker().clone());
                    return Poll::Pending;
                }
            }
        };
        match &completion {
            Completion::Connected {
                connection,
                result: Err(_),
            }
            | Completion::Closed { connection }
            | Completion::IdleClosed { connection } => self.claims.forget(*connection),
            _ => {}
        }
        Poll::Ready(completion)
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
    /// Prove an idle connection another binding last used for the binding (plan step 1.6).
    Reuse(ReuseCommand),
    CancelReuse(ProofId),
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
    /// Queues `command` for the worker, starting it on the first command. False: no worker took it.
    fn send(&mut self, command: Command) -> bool {
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
            return false;
        };
        let sent = self.commands.send(command).is_ok();
        if sent {
            worker.unpark();
        }
        sent
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
        // An SSH exchange runs on the connection its setup authenticated, and the setup had the connecting operation's
        // context (`connect`); the exchange's own context is not read here, and nothing of it is kept.
        _auth: &AuthContext,
    ) -> io::Result<Box<dyn ExchangeIo>> {
        let worker = self.worker.clone().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotConnected, "SSH adapter has no worker")
        })?;
        // A connection the worker retired (a failed reuse proof, an idle loss) is refused here, before anything is
        // queued: the host takes `NotConnected` as a lost lease and asks again for a new connection. One this leases is
        // not retired; every return below that queues no start releases the lease (`Lease`'s drop).
        let Some(lease) = self.shared.lease(connection) else {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "the connection was retired",
            ));
        };
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
        let queued = self.send(Command::Start {
            connection,
            exchange: Box::new(Exchange {
                channel: Some((stream, endpoint)),
                peer,
                service,
                path: spec.path().into(),
                settle: settle.clone(),
            }),
        });
        if !queued {
            // The worker has ended (it was disposed of): nothing will run the exchange, and nothing was sent.
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the SSH adapter's worker has ended",
            ));
        }
        lease.queued();
        Ok(Box::new(SshExchange {
            client,
            settle,
            worker,
            reading: StreamWaiter::new(),
            writing: StreamWaiter::new(),
            flushing: None,
            ending: None,
            write_ended: false,
            closing: None,
            closed: false,
        }))
    }
    fn authorize_reuse(&mut self, command: ReuseCommand) -> bool {
        // No worker holds no connection: nothing to prove. Otherwise the worker answers with `Completion::Reuse`.
        if self.worker.is_none() {
            return false;
        }
        self.send(Command::Reuse(command));
        true
    }
    fn cancel_reuse(&mut self, proof: ProofId) {
        if self.worker.is_some() {
            self.send(Command::CancelReuse(proof));
        }
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

/// A stream call persisted across polls: it owns a clone of the client stream, so the caller's dropped future does not
/// drop the work it started.
type StreamCall = Pin<Box<dyn Future<Output = Result<(), StreamError>> + Send>>;
/// One exchange's client half: the initiator stream the host's `MessageStream` reads and writes. Its `poll_*` methods
/// wait on the stream itself (contract section 9): a pending read or write keeps its slot registered between polls, so
/// the worker's delivery, the credit it returns or the stream's end wakes the caller, and no caller sleep-polls.
struct SshExchange {
    client: Stream,
    settle: Arc<Settle>,
    worker: Thread,
    /// One wait registration per direction (`StreamWaiter`'s rule).
    reading: StreamWaiter,
    writing: StreamWaiter,
    /// A flush the caller started and has not yet seen end. A write the stream accepts drops it, so a later flush starts
    /// a new one that covers those bytes instead of resolving on the earlier one's ticket.
    flushing: Option<StreamCall>,
    /// The request's half-close (`poll_shutdown_write`) asked for and not yet sent, and whether it was sent: once it
    /// was, later polls are `Ready(Ok)` and later writes fail (the stream refuses them).
    ending: Option<StreamCall>,
    write_ended: bool,
    /// The member's orderly end (its EOF, then its close): `None` until asked for, and once it has run.
    closing: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
    closed: bool,
}
impl ExchangeIo for SshExchange {
    fn poll_read(&mut self, cx: &mut Context<'_>, out: &mut [u8]) -> Poll<io::Result<usize>> {
        let read = self.client.poll_read(cx, &mut self.reading, out);
        // A read returns credit: the worker may now move more of the server's bytes.
        self.worker.unpark();
        read.map_err(io::Error::other)
    }
    fn poll_write(&mut self, cx: &mut Context<'_>, data: &[u8]) -> Poll<io::Result<usize>> {
        let written = self.client.poll_write(cx, &mut self.writing, data);
        if matches!(written, Poll::Ready(Ok(count)) if count > 0) {
            self.flushing = None;
        }
        self.worker.unpark();
        written.map_err(io::Error::other)
    }
    fn poll_flush(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let flushing = self.flushing.get_or_insert_with(|| {
            let client = self.client.clone();
            Box::pin(async move { client.flush().await })
        });
        let flushed = flushing.as_mut().poll(cx);
        if flushed.is_ready() {
            self.flushing = None;
        }
        self.worker.unpark();
        flushed.map_err(io::Error::other)
    }
    fn poll_shutdown_write(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.write_ended {
            return Poll::Ready(Ok(()));
        }
        let ending = self.ending.get_or_insert_with(|| {
            let client = self.client.clone();
            Box::pin(async move { client.end_write().await })
        });
        let ended = ending.as_mut().poll(cx);
        if ended.is_ready() {
            // A failed stream fails every later call too; asking again polls a new end.
            self.ending = None;
            self.write_ended = matches!(ended, Poll::Ready(Ok(())));
        }
        self.worker.unpark();
        ended.map_err(io::Error::other)
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

//! Shared endpoint worker. Native setup is an injected nonblocking owner;
//! blocking Git callers never drive the worker that services their streams.
use super::{
    agent_job::Cleanup,
    ssh_admission::{Admissions, Reader},
    ssh_channel::{GitService, SshChannel},
    ssh_key_snapshot::{Entry, Registry},
    ssh_pool::{Connector, PoolHost, Progress, Resource},
    ssh_pump::SshPump,
    ssh_shutdown::{self, Status},
    stream_io::BlockingStream,
};
use gwz_transport::{
    pool::{Checkout, Config as PoolConfig, Identity, Key, Lease, Owner, Pool},
    protocol::{Disposition, Facts, Opened},
    stream::{Config as StreamConfig, MessageEndpoint, Side, Stream},
};
use std::{
    future::Future,
    io,
    path::PathBuf,
    pin::pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
    },
    task::{Context, Poll, Wake, Waker},
    thread::{self, JoinHandle, Thread},
    time::{Duration, Instant},
};

pub(crate) use super::ssh_shutdown::ShutdownStatus;
pub(crate) trait ChannelResource: Resource + Send + 'static {
    fn observation(&self) -> (bool, Facts) {
        (false, Facts::default())
    }
    fn start_exchange(
        &mut self,
        stream: Stream,
        endpoint: MessageEndpoint,
        service: GitService,
        path: &str,
    ) -> io::Result<()>;
    fn pump(&mut self) -> Option<&mut SshPump<SshChannel>>;
    /// Restore an idle owner only after the pump's acknowledged complete close.
    fn reclaim(&mut self) -> bool;
}
struct ThreadWake(Thread);
impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}
struct Shared {
    sender: SyncSender<OpenRequest>,
    worker: Thread,
    outstanding: Arc<AtomicUsize>,
    capacity: usize,
    timeout: Option<Duration>,
    stop: Arc<AtomicBool>,
    origin: Instant,
    join: Mutex<Option<JoinHandle<()>>>,
    status: Status,
}
impl Drop for Shared {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.unpark();
        if let Some(join) = self
            .join
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = join.join();
        }
    }
}
pub(super) struct Permit(Arc<AtomicUsize>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
pub(super) struct OpenRequest {
    pub(super) key: Key,
    pub(super) identity: Identity,
    pub(super) service: GitService,
    pub(super) path: String,
    pub(super) deadline: Option<u64>,
    pub(super) cancelled: Arc<AtomicBool>,
    pub(super) reply: Option<SyncSender<io::Result<(BlockingStream, Opened)>>>,
    pub(super) selected: Option<PathBuf>,
    pub(super) authority: Option<Arc<Entry>>,
    pub(super) progress: Progress,
    pub(super) permit: Permit,
}
impl OpenRequest {
    pub(super) fn reject(&mut self, kind: io::ErrorKind) {
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(Err(kind.into()));
        }
    }

    pub(super) fn expired(&self, now: u64) -> bool {
        self.cancelled.load(Ordering::Acquire) || self.deadline.is_some_and(|at| now >= at)
    }
    pub(super) fn complete(self, result: io::Result<(BlockingStream, Opened)>) {
        // Release admission before publishing the reply. The physical pool now
        // bounds an active stream; pending admission remains independently bounded.
        let Self { reply, permit, .. } = self;
        drop(permit);
        if let Some(reply) = reply {
            let _ = reply.send(result);
        }
    }
}
#[derive(Clone)]
pub(crate) struct Endpoint {
    shared: Arc<Shared>,
}
impl Endpoint {
    pub(crate) fn new<C>(config: PoolConfig, connector: C, io_timeout_ms: u64) -> io::Result<Self>
    where
        C: Connector + Send + 'static,
        C::Resource: ChannelResource,
    {
        Self::with_connector(config, |_| connector, io_timeout_ms)
    }
    pub(crate) fn with_connector<C>(
        config: PoolConfig,
        factory: impl FnOnce(Instant) -> C,
        io_timeout_ms: u64,
    ) -> io::Result<Self>
    where
        C: Connector + Send + 'static,
        C::Resource: ChannelResource,
    {
        Self::with_registry(
            config,
            Registry::new(),
            |origin, _| factory(origin),
            io_timeout_ms,
        )
    }
    pub(crate) fn with_registry<C>(
        config: PoolConfig,
        registry: Registry,
        factory: impl FnOnce(Instant, Registry) -> C,
        io_timeout_ms: u64,
    ) -> io::Result<Self>
    where
        C: Connector + Send + 'static,
        C::Resource: ChannelResource,
    {
        Self::with_reader(
            config,
            registry,
            Arc::new(Registry::start),
            factory,
            io_timeout_ms,
        )
    }
    pub(crate) fn with_reader<C>(
        config: PoolConfig,
        registry: Registry,
        reader: Reader,
        factory: impl FnOnce(Instant, Registry) -> C,
        io_timeout_ms: u64,
    ) -> io::Result<Self>
    where
        C: Connector + Send + 'static,
        C::Resource: ChannelResource,
    {
        let retention = Cleanup::reserve()?;
        let origin = Instant::now();
        let connector = factory(origin, registry.clone());
        if io_timeout_ms > i32::MAX as u64 {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let capacity = config.max_requests;
        let cleanup = config.cleanup_timeout_ms;
        let timeout = (config.connect_timeout_ms != 0).then(|| {
            Duration::from_millis(
                config
                    .allocation_timeout_ms
                    .saturating_add(config.connect_timeout_ms)
                    .saturating_add(config.interaction_timeout_ms),
            )
        });
        let (pool, host) = PoolHost::new(config, connector, 0)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        static NEXT_WORKER: AtomicU64 = AtomicU64::new(1);
        let id = NEXT_WORKER
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| io::Error::other("endpoint IDs exhausted"))?;
        let (sender, receiver) = mpsc::sync_channel(capacity);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let status = Status::default();
        let worker_status = status.clone();
        let join = thread::Builder::new()
            .name("gwz-ssh-endpoint".into())
            .spawn(move || {
                let run_pool = pool.clone();
                let report = worker_status.clone();
                ssh_shutdown::manage(
                    host,
                    Admissions::new(registry, reader, origin, cleanup),
                    pool,
                    worker_status,
                    retention,
                    origin,
                    |host, admissions| {
                        run(
                            receiver,
                            run_pool,
                            host,
                            admissions,
                            io_timeout_ms,
                            cleanup,
                            worker_stop,
                            move || elapsed(origin),
                            id,
                            &report,
                        );
                    },
                );
            })?;
        Ok(Self {
            shared: Arc::new(Shared {
                sender,
                worker: join.thread().clone(),
                outstanding: Arc::new(AtomicUsize::new(0)),
                capacity,
                timeout,
                stop,
                origin,
                join: Mutex::new(Some(join)),
                status,
            }),
        })
    }
    pub(crate) fn open(
        &self,
        key: Key,
        identity: Identity,
        service: GitService,
        path: &str,
    ) -> io::Result<BlockingStream> {
        self.open_observed(key, identity, service, path)
            .map(|(stream, _)| stream)
    }
    pub(crate) fn open_observed(
        &self,
        key: Key,
        identity: Identity,
        service: GitService,
        path: &str,
    ) -> io::Result<(BlockingStream, Opened)> {
        self.enqueue(key, identity, None, service, path, Progress::default())
    }
    pub(crate) fn open_selected(
        &self,
        key: Key,
        selected: PathBuf,
        service: GitService,
        path: &str,
    ) -> io::Result<(BlockingStream, Opened)> {
        self.enqueue(
            key,
            Identity::Ambient,
            Some(selected),
            service,
            path,
            Progress::default(),
        )
    }
    pub(crate) fn open_reported(
        &self,
        key: Key,
        selected: Option<PathBuf>,
        service: GitService,
        path: &str,
        progress: Progress,
    ) -> io::Result<(BlockingStream, Opened)> {
        self.enqueue(key, Identity::Ambient, selected, service, path, progress)
    }
    pub(crate) fn pending_requests(&self) -> usize {
        self.shared.outstanding.load(Ordering::Acquire)
    }
    fn enqueue(
        &self,
        key: Key,
        identity: Identity,
        selected: Option<PathBuf>,
        service: GitService,
        path: &str,
        progress: Progress,
    ) -> io::Result<(BlockingStream, Opened)> {
        if self.shared.stop.load(Ordering::Acquire) {
            return Err(stopped());
        }
        if path.is_empty() || path.len() > 16_384 || path.contains('\0') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid repository operand",
            ));
        }
        self.shared
            .outstanding
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < self.shared.capacity).then_some(n + 1)
            })
            .map_err(|_| io::Error::new(io::ErrorKind::WouldBlock, "endpoint admission full"))?;
        let permit = Permit(self.shared.outstanding.clone());
        let (reply, result) = mpsc::sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let absolute = self.shared.timeout.map(|d| Instant::now() + d);
        let request = OpenRequest {
            key,
            identity,
            service,
            path: path.to_owned(),
            permit,
            reply: Some(reply),
            selected,
            authority: None,
            progress,
            cancelled: cancelled.clone(),
            deadline: absolute.map(|at| at.duration_since(self.shared.origin).as_millis() as u64),
        };
        match self.shared.sender.try_send(request) {
            Ok(()) => self.shared.worker.unpark(),
            Err(TrySendError::Full(_)) => {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "endpoint queue full",
                ));
            }
            Err(TrySendError::Disconnected(_)) => return Err(stopped()),
        }
        let received = match absolute {
            Some(at) => result.recv_timeout(at.saturating_duration_since(Instant::now())),
            None => result.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match received {
            Ok(result) => result,
            Err(error) => {
                cancelled.store(true, Ordering::Release);
                self.shared.worker.unpark();
                Err(match error {
                    RecvTimeoutError::Timeout => io::ErrorKind::TimedOut.into(),
                    RecvTimeoutError::Disconnected => stopped(),
                })
            }
        }
    }
    pub(crate) fn shutdown_watch(&self) -> ssh_shutdown::ShutdownWatch {
        ssh_shutdown::ShutdownWatch(self.shared.status.clone())
    }
    pub(crate) fn shutdown_status(&self) -> ShutdownStatus {
        *self.shared.status.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// Cancels pending/active work through every clone, even with timeouts disabled.
    pub(crate) fn shutdown(&self) {
        self.shared.stop.store(true, Ordering::Release);
        self.shared.worker.unpark();
    }
}
struct Pending {
    checkout: Checkout,
    request: OpenRequest,
}
struct Active {
    lease: Option<Lease>,
    peer: MessageEndpoint,
}
impl Drop for Active {
    fn drop(&mut self) {
        self.peer.disconnect();
    }
}
struct StopOnExit(Arc<AtomicBool>);
impl Drop for StopOnExit {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
fn run<C>(
    receiver: Receiver<OpenRequest>,
    pool: Pool,
    host: &mut PoolHost<C>,
    admissions: &mut Admissions,
    io_timeout_ms: u64,
    cleanup: u64,
    stop: Arc<AtomicBool>,
    clock: impl Fn() -> u64,
    worker_id: u64,
    status: &Status,
) where
    C: Connector,
    C::Resource: ChannelResource,
{
    let _stop_on_exit = StopOnExit(stop.clone());
    let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut pending = Vec::<Pending>::new();
    let mut active = Vec::<Active>::new();
    let mut serial = 0_i64;
    let mut stopping_at = None;
    let session = format!("ssh-worker-{worker_id}");
    loop {
        let now = clock();
        if stop.load(Ordering::Acquire) && stopping_at.is_none() {
            stopping_at = Some(now.saturating_add(cleanup));
            for item in pending.drain(..) {
                item.request.complete(Err(stopped()));
            }
            active.clear(); // disconnect Git callers before waiting for physical disposal
            pool.shutdown();
        }
        pending.retain_mut(|item| {
            if item.request.expired(now) {
                item.request.reject(io::ErrorKind::TimedOut);
                false
            } else {
                true
            }
        });
        let ready = admissions.poll(&mut cx, now, stopping_at.is_some());
        if let Some(error) = admissions.take_failure() {
            ssh_shutdown::fail(status, error);
            stop.store(true, Ordering::Release);
        }
        if host
            .step_reported(&mut cx, now, |connection| {
                pending
                    .iter()
                    .find(|p| p.checkout.opening_connection() == Some(connection))
                    .map(|p| p.request.progress.clone())
                    .unwrap_or_default()
            })
            .is_err()
        {
            ssh_shutdown::fail(status, io::ErrorKind::Other);
            break;
        }
        ssh_shutdown::sample(status, host, admissions);
        if let Some(error) = host.take_disposal_error() {
            ssh_shutdown::fail(status, error.kind());
            stop.store(true, Ordering::Release);
            if stopping_at.is_none() {
                continue;
            }
        }
        if stopping_at.is_some_and(|at| now >= at)
            || (stopping_at.is_some() && host.shutdown_complete() && admissions.is_empty())
        {
            break; // The owner transfers unfinished cleanup to the reserved supervisor slot.
        }
        let incoming = receiver.try_iter().take(32);
        for request in ready.into_iter().chain(incoming) {
            if stopping_at.is_some() {
                request.complete(Err(stopped()));
                continue;
            }
            if request.expired(now) {
                request.complete(Err(io::ErrorKind::TimedOut.into()));
                continue;
            }
            if request.selected.is_some() {
                admissions.start(request);
                continue;
            }
            let Some(next) = serial.checked_add(1) else {
                request.complete(Err(io::Error::other("stream IDs exhausted")));
                continue;
            };
            serial = next;
            match pool.checkout_until(
                gwz_transport::pool::Request::new(
                    request.key.clone(),
                    request.identity.clone(),
                    Owner::new(&session, serial.to_string()),
                ),
                request.deadline,
            ) {
                Ok(checkout) => pending.push(Pending { checkout, request }),
                Err(error) => request.complete(Err(io::Error::other(error))),
            }
        }
        let mut index = 0;
        while index < pending.len() {
            if pending[index].request.expired(now) {
                let item = pending.swap_remove(index);
                item.request.complete(Err(io::ErrorKind::TimedOut.into()));
                continue;
            }
            let ready = pin!(&mut pending[index].checkout).poll(&mut cx);
            match ready {
                Poll::Pending => index += 1,
                Poll::Ready(result) => {
                    let item = pending.swap_remove(index);
                    let result = result.map_err(io::Error::other).and_then(|lease| {
                        let Some(next) = serial.checked_add(1) else {
                            return Err(io::Error::other("stream IDs exhausted"));
                        };
                        serial = next;
                        attach(
                            host,
                            lease,
                            &item.request,
                            &session,
                            serial,
                            now,
                            io_timeout_ms,
                            &mut active,
                        )
                    });
                    item.request.complete(result);
                }
            }
        }
        let mut index = 0;
        while index < active.len() {
            let exchange = &mut active[index];
            exchange.peer.advance(now);
            let disposition = match host.resource(exchange.lease.as_ref().expect("active lease")) {
                Ok(resource) => {
                    let result = match resource.pump() {
                        Some(pump) => transfer(pump, &exchange.peer, &mut cx, now),
                        None => Err(()),
                    };
                    match result {
                        Ok(false) => None,
                        Ok(true) if resource.reclaim() => Some(Disposition::Reusable),
                        _ => Some(Disposition::Discarded),
                    }
                }
                Err(_) => Some(Disposition::Discarded),
            };
            if let Some(disposition) = disposition {
                let exchange = active.swap_remove(index);
                // Active owns a disconnect guard, so consume its lease through
                // a separate release helper after detaching that guard below.
                release(host, exchange, disposition);
            } else {
                index += 1;
            }
        }
        // The host has no socket readiness API yet. Park between bounded polls;
        // callers wake immediately, timers run independently, and idle pools sleep.
        let wait = if active.is_empty()
            && pending.is_empty()
            && admissions.is_empty()
            && stopping_at.is_none()
        {
            host.next_deadline()
                .map_or(1000, |at| at.saturating_sub(now).min(1000))
        } else {
            1
        };
        thread::park_timeout(Duration::from_millis(wait.max(1)));
    }
    for item in pending {
        item.request.complete(Err(stopped()));
    }
    drop(active);
    // Dropping receiver releases queued permits and wakes waiting opens.
}
fn attach<C: Connector>(
    host: &mut PoolHost<C>,
    lease: Lease,
    request: &OpenRequest,
    session: &str,
    serial: i64,
    now: u64,
    io_timeout: u64,
    active: &mut Vec<Active>,
) -> io::Result<(BlockingStream, Opened)>
where
    C::Resource: ChannelResource,
{
    if request.expired(now) {
        return Err(io::ErrorKind::TimedOut.into());
    }
    let config = |side| {
        let mut config = StreamConfig::new(session, serial, side);
        config.io_timeout_ms = io_timeout;
        config
    };
    let (client, peer) = Stream::new(config(Side::Initiator)).map_err(io::Error::other)?;
    let (stream, endpoint) = Stream::new(config(Side::Endpoint)).map_err(io::Error::other)?;
    peer.advance(now);
    endpoint.advance(now);
    let connection_id = format!(
        "{session}-{}",
        lease.connection().map_err(io::Error::other)?.sequence()
    );
    let reused = host.allocation_reused(&lease).map_err(io::Error::other)?;
    let resource = host.resource(&lease).map_err(io::Error::other)?;
    let (_, mut facts) = resource.observation();
    if reused {
        facts.credential_offered = false;
    }
    *request.progress.lock().unwrap_or_else(|e| e.into_inner()) = facts.clone();
    resource.start_exchange(stream, endpoint, request.service, &request.path)?;
    let receipt = Arc::new(AtomicBool::new(false));
    resource
        .pump()
        .ok_or_else(|| io::Error::other("resource did not install a channel pump"))?
        .track_repository_refusal(receipt.clone());
    active.push(Active {
        lease: Some(lease),
        peer,
    });
    Ok((
        BlockingStream::with_repository_receipt(client, receipt),
        Opened {
            connection_id,
            reused,
            endpoint_id: session.into(),
            trust_owner: session.into(),
            facts,
            receive_limits: config(Side::Endpoint).receive_limits,
        },
    ))
}
fn transfer(
    pump: &mut SshPump<SshChannel>,
    peer: &MessageEndpoint,
    cx: &mut Context<'_>,
    now: u64,
) -> Result<bool, ()> {
    // Time must precede incoming Close, which switches away from the I/O clock.
    pump.advance(now);
    let result = (|| {
        for _ in 0..8 {
            match pin!(peer.next_message()).poll(cx) {
                Poll::Ready(Ok(Some(message))) => pump.deliver(message).map_err(|_| ())?,
                Poll::Ready(Err(_)) => return Err(()),
                _ => break,
            }
        }
        pump.tick(cx, now).map_err(|_| ())?;
        for _ in 0..8 {
            match pump.poll_next_message(cx) {
                Poll::Ready(Ok(Some(message))) => peer.deliver(message).map_err(|_| ())?,
                Poll::Ready(Err(_)) => return Err(()),
                _ => break,
            }
        }
        Ok(pump.stream_stats().terminal)
    })();
    if result.is_err() {
        pump.cancel();
        peer.disconnect();
    }
    result
}
fn release<C: Connector>(host: &mut PoolHost<C>, mut exchange: Active, disposition: Disposition) {
    let lease = exchange.lease.take().expect("active lease");
    let _ = host.release(lease, disposition);
}
fn elapsed(origin: Instant) -> u64 {
    origin.elapsed().as_millis() as u64
}
fn stopped() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "SSH endpoint stopped")
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        #[path = "../../../tests/transport_ssh/support/worker_queue.rs"]
        mod queue_tests;
    }
}

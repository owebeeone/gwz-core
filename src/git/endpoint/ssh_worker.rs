//! Shared endpoint worker. Native setup is an injected nonblocking owner; an
//! open is submitted without waiting, and its reply is an
//! [`EndpointAttachment`] that the placement endpoint drives.
use super::{
    agent_job::{Cleanup, Job},
    setup_retry::{self, Phase},
    ssh_admission::{Admissions, Reader},
    ssh_channel::{GitService, SshChannel},
    ssh_key_snapshot::{Entry, Registry},
    ssh_pool::{Connector, PoolHost, Progress, Resource},
    ssh_pump::SshPump,
    ssh_shutdown::{self, Status},
};
use gwz_transport::{
    pool::{Checkout, Config as PoolConfig, Identity, Key, Lease, Owner, Pool},
    protocol::{
        Deadlines, Disposition, Effect, Envelope, ErrorCode, Facts, Failure, Limits, Opened,
    },
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
        mpsc::{self, Receiver, Sender, SyncSender, TrySendError},
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
/// Unparks a thread that parks between bounded polls.
pub(crate) struct ThreadWake(pub(crate) Thread);
impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}
struct Shared {
    sender: Sender<OpenRequest>,
    worker: Thread,
    outstanding: Arc<AtomicUsize>,
    capacity: AtomicUsize,
    pool: Pool,
    policy: PoolConfig,
    io_timeout_ms: u64,
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
#[derive(Clone)]
pub(crate) struct BridgeContext {
    pub(crate) session_id: String,
    pub(crate) stream_id: i64,
    pub(crate) version: i64,
    pub(crate) limits: Limits,
    pub(crate) deadlines: Deadlines,
    /// Woken when the worker queues a message for the bridge or ends it.
    pub(crate) waker: Option<Waker>,
}
pub(super) struct OpenRequest {
    pub(super) key: Key,
    pub(super) identity: Identity,
    pub(super) service: GitService,
    pub(super) path: String,
    pub(super) deadline: Option<u64>,
    pub(super) cancelled: Arc<AtomicBool>,
    pub(super) reply: Option<SyncSender<OpenOutcome>>,
    pub(super) selected: Option<PathBuf>,
    /// Pins the admitted key snapshot, which the registry holds only weakly,
    /// until the open completes: the connector finds it there by identity.
    pub(super) authority: Option<Arc<Entry>>,
    pub(super) progress: Progress,
    pub(super) permit: Permit,
    pub(super) context: BridgeContext,
}
pub(super) type OpenOutcome = io::Result<(EndpointAttachment, Opened)>;
/// A bridged open the worker owns until it replies.
pub(crate) struct PendingOpen {
    reply: Receiver<OpenOutcome>,
}
impl PendingOpen {
    /// The worker's reply, once it has replied; a stopped worker is a reply too.
    pub(crate) fn poll(&self) -> Poll<io::Result<(EndpointAttachment, Opened)>> {
        match self.reply.try_recv() {
            Ok(outcome) => Poll::Ready(outcome),
            Err(mpsc::TryRecvError::Empty) => Poll::Pending,
            Err(mpsc::TryRecvError::Disconnected) => Poll::Ready(Err(stopped())),
        }
    }
}
/// Bounded message bridge owned by a placement endpoint. The worker retains
/// the physical channel and pump; this handle only carries typed envelopes.
pub(crate) struct EndpointAttachment {
    // Keep the initiator half alive while the bridge owns this attachment.
    // Dropping it would cancel the shared stream before the first envelope
    // can be delivered to the physical pump.
    owner: Stream,
    inbound: SyncSender<Envelope>,
    outbound: Receiver<Envelope>,
    cancelled: Arc<AtomicBool>,
    discard: Arc<AtomicBool>,
    worker: Thread,
}
impl EndpointAttachment {
    pub(crate) fn send(&self, message: Envelope) -> io::Result<()> {
        self.inbound
            .try_send(message)
            .map_err(|error| match error {
                TrySendError::Full(_) => io::Error::from(io::ErrorKind::WouldBlock),
                TrySendError::Disconnected(_) => io::Error::from(io::ErrorKind::BrokenPipe),
            })?;
        // The worker parks between polls; the message is work for it now.
        self.worker.unpark();
        Ok(())
    }
    pub(crate) fn try_receive(&self) -> io::Result<Option<Envelope>> {
        match self.outbound.try_recv() {
            Ok(message) => Ok(Some(message)),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => Err(io::ErrorKind::BrokenPipe.into()),
        }
    }
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.owner.cancel();
    }
    /// The session serves this exchange and is then closed rather than kept
    /// idle: a setup the key's retry machine did not admit (the retry plan's
    /// §4).
    pub(crate) fn discard_after_use(&self) {
        self.discard.store(true, Ordering::Release);
    }
}
/// Sanitized endpoint outcome; native diagnostics and credential paths stay
/// local. `phase` says whether a started setup failed, which only this side
/// of the transport knows (the retry plan's §4).
#[derive(Debug)]
pub(crate) struct EndpointOpenFailure {
    pub(crate) failure: Failure,
    pub(crate) phase: Phase,
}
impl std::fmt::Display for EndpointOpenFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SSH endpoint open failed: {:?}", self.failure.code)
    }
}
impl std::error::Error for EndpointOpenFailure {}
impl EndpointOpenFailure {
    fn capture(error: io::Error, facts: Facts) -> io::Error {
        use gwz_transport::pool::Error as PoolError;
        use gwz_transport::protocol::SetupFailureCause;
        let pool = error
            .get_ref()
            .and_then(|cause| cause.downcast_ref::<PoolError>());
        let phase = pool.map_or(Phase::Other, setup_retry::phase_of);
        let (code, effect, setup_cause) = match pool {
            Some(PoolError::ConnectFailed {
                code,
                effect,
                setup_cause,
            }) => (*code, *effect, *setup_cause),
            Some(PoolError::AllocationTimeout) => (
                ErrorCode::Timeout,
                Effect::None,
                Some(SetupFailureCause::Allocation),
            ),
            Some(PoolError::ConnectTimeout) => (
                ErrorCode::Timeout,
                Effect::None,
                Some(SetupFailureCause::Aggregate),
            ),
            Some(PoolError::InteractionTimeout) => (
                ErrorCode::Timeout,
                Effect::None,
                Some(SetupFailureCause::Interaction),
            ),
            Some(PoolError::Capacity | PoolError::WouldBlock) => {
                (ErrorCode::Capacity, Effect::None, None)
            }
            Some(PoolError::Cancelled) => (ErrorCode::Cancelled, Effect::None, None),
            // The setup authenticated as an identity other than the one asked.
            Some(PoolError::IdentityMismatch) => (ErrorCode::Authentication, Effect::None, None),
            Some(PoolError::DriverLost | PoolError::Shutdown) => {
                (ErrorCode::CarrierLost, Effect::None, None)
            }
            _ => (
                match error.kind() {
                    io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied => {
                        ErrorCode::Unavailable
                    }
                    io::ErrorKind::InvalidInput => ErrorCode::InvalidRequest,
                    io::ErrorKind::TimedOut => ErrorCode::Timeout,
                    io::ErrorKind::WouldBlock => ErrorCode::Capacity,
                    io::ErrorKind::ConnectionAborted => ErrorCode::Cancelled,
                    io::ErrorKind::BrokenPipe => ErrorCode::CarrierLost,
                    _ => ErrorCode::Io,
                },
                Effect::None,
                None,
            ),
        };
        io::Error::new(
            error.kind(),
            Self {
                failure: Failure {
                    setup_cause,
                    code,
                    effect,
                    facts: Some(facts),
                },
                phase,
            },
        )
    }
}
impl OpenRequest {
    pub(super) fn has_reply(&self) -> bool {
        self.reply.is_some()
    }
    pub(super) fn reject(&mut self, kind: io::ErrorKind) {
        if let Some(reply) = self.reply.take() {
            let facts = self
                .progress
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            let _ = reply.send(Err(EndpointOpenFailure::capture(kind.into(), facts)));
            // The bridge's owner polls for the reply between its passes.
            if let Some(waker) = &self.context.waker {
                waker.wake_by_ref();
            }
        }
    }

    pub(super) fn expired(&self, now: u64) -> bool {
        self.cancelled.load(Ordering::Acquire) || self.deadline.is_some_and(|at| now >= at)
    }
    pub(super) fn complete(self, result: OpenOutcome) {
        // Release admission before publishing the reply. The physical pool now
        // bounds an active stream; pending admission remains independently bounded.
        let Self {
            reply,
            permit,
            progress,
            context,
            ..
        } = self;
        drop(permit);
        if let Some(reply) = reply {
            let result = result.map_err(|error| {
                let facts = progress.lock().unwrap_or_else(|e| e.into_inner()).clone();
                EndpointOpenFailure::capture(error, facts)
            });
            let _ = reply.send(result);
            // The bridge's owner polls for the reply between its passes.
            if let Some(waker) = context.waker {
                waker.wake();
            }
        }
    }
}
#[derive(Clone)]
pub(crate) struct Endpoint {
    shared: Arc<Shared>,
    cleanup: Duration,
}
impl Endpoint {
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
        let policy = config.clone();
        let (pool, host) = PoolHost::new(config, connector, 0)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        static NEXT_WORKER: AtomicU64 = AtomicU64::new(1);
        let id = NEXT_WORKER
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| io::Error::other("endpoint IDs exhausted"))?;
        let (sender, receiver) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let status = Status::default();
        let worker_status = status.clone();
        let client_pool = pool.clone();
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
                capacity: AtomicUsize::new(capacity),
                pool: client_pool,
                policy,
                io_timeout_ms,
                stop,
                origin,
                join: Mutex::new(Some(join)),
                status,
            }),
            cleanup: Duration::from_millis(cleanup),
        })
    }
    pub(crate) fn validate_deadlines(&self, d: &Deadlines) -> io::Result<()> {
        fn tightens(value: i64, configured: u64, zero_allowed: bool) -> bool {
            value >= 0
                && (value > 0 || zero_allowed && configured == 0)
                && (configured == 0 || value as u64 <= configured)
        }
        let p = &self.shared.policy;
        if !tightens(d.allocation_ms, p.allocation_timeout_ms, false)
            || !tightens(d.connect_ms, p.connect_timeout_ms, true)
            || !tightens(d.io_ms, self.shared.io_timeout_ms, true)
            || !tightens(d.interaction_ms, p.interaction_timeout_ms, false)
            || !tightens(d.cleanup_ms, p.cleanup_timeout_ms, false)
        {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        if d.connect_ms != 0 {
            let total = (d.allocation_ms as u64)
                .checked_add(d.connect_ms as u64)
                .and_then(|value| value.checked_add(d.interaction_ms as u64))
                .ok_or(io::ErrorKind::InvalidInput)?;
            Instant::now()
                .checked_add(Duration::from_millis(total))
                .ok_or(io::ErrorKind::InvalidInput)?;
        }
        Ok(())
    }
    pub(crate) fn start_identity_file_check(
        &self,
        selected: PathBuf,
        deadline: Option<Instant>,
    ) -> io::Result<Job<()>> {
        Job::start(deadline, self.cleanup, move |control| {
            control.check()?;
            use std::os::unix::fs::OpenOptionsExt;
            // O_NONBLOCK prevents special files (including a replaced FIFO) from
            // blocking before fstat establishes the regular-file requirement.
            let file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(selected)
                .map_err(|error| io::Error::from(error.kind()))?;
            control.check()?;
            let metadata = file
                .metadata()
                .map_err(|error| io::Error::from(error.kind()))?;
            if !metadata.file_type().is_file() {
                return Err(io::ErrorKind::InvalidInput.into());
            }
            Ok(())
        })
    }
    pub(crate) fn pending_requests(&self) -> usize {
        self.shared.outstanding.load(Ordering::Acquire)
    }
    pub(crate) fn pool(&self) -> &Pool {
        &self.shared.pool
    }
    pub(crate) fn set_request_capacity(&self, capacity: usize) {
        self.shared.capacity.store(capacity, Ordering::Release);
    }
    /// Submits a bridged open, with a selected key or the agent, and returns
    /// at once. No thread and no supervised job waits for it: the worker owns
    /// it until it replies, and its reply wakes `context.waker`. A selected
    /// identity file is admitted by the worker before every checkout,
    /// including reuse.
    pub(crate) fn start_endpoint_open(
        &self,
        key: Key,
        selected: Option<PathBuf>,
        service: GitService,
        path: &str,
        context: BridgeContext,
        cancelled: Arc<AtomicBool>,
    ) -> io::Result<PendingOpen> {
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
                (n < self.shared.capacity.load(Ordering::Acquire)).then_some(n + 1)
            })
            .map_err(|_| io::Error::new(io::ErrorKind::WouldBlock, "endpoint admission full"))?;
        let permit = Permit(self.shared.outstanding.clone());
        let (reply, result) = mpsc::sync_channel(1);
        self.validate_deadlines(&context.deadlines)?;
        let d = &context.deadlines;
        let timeout = (d.connect_ms != 0).then(|| {
            Duration::from_millis(
                (d.allocation_ms as u64)
                    .saturating_add(d.connect_ms as u64)
                    .saturating_add(d.interaction_ms as u64),
            )
        });
        let absolute = timeout
            .map(|duration| {
                Instant::now()
                    .checked_add(duration)
                    .ok_or(io::ErrorKind::InvalidInput)
            })
            .transpose()?;
        let request = OpenRequest {
            key,
            identity: Identity::Ambient,
            service,
            path: path.to_owned(),
            permit,
            reply: Some(reply),
            selected,
            authority: None,
            progress: Progress::default(),
            cancelled,
            deadline: absolute.map(|at| at.duration_since(self.shared.origin).as_millis() as u64),
            context,
        };
        if self.shared.sender.send(request).is_err() {
            return Err(stopped());
        }
        self.shared.worker.unpark();
        Ok(PendingOpen { reply: result })
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
    bridge_inbound: Receiver<Envelope>,
    bridge_outbound: SyncSender<Envelope>,
    bridge_cancelled: Arc<AtomicBool>,
    bridge_pending: Option<Envelope>,
    bridge_session: String,
    bridge_stream_id: i64,
    bridge_version: i64,
    bridge_terminal_delivered: bool,
    bridge_waker: Option<Waker>,
    discard: Arc<AtomicBool>,
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
    let stall_ms = host.stall_slot();
    stall_ms.store(io_timeout_ms, Ordering::Relaxed);
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
                let item = pending
                    .iter()
                    .find(|p| p.checkout.opening_connection() == Some(connection));
                let stall = item
                    .map(|p| {
                        let io_ms = p.request.context.deadlines.io_ms;
                        if io_ms <= 0 { 0 } else { io_ms as u64 }
                    })
                    .unwrap_or(io_timeout_ms);
                stall_ms.store(stall, Ordering::Relaxed);
                item.map(|p| p.request.progress.clone()).unwrap_or_default()
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
            let mut policy = gwz_transport::pool::Request::new(
                request.key.clone(),
                request.identity.clone(),
                Owner::new(&session, serial.to_string()),
            );
            let d = &request.context.deadlines;
            policy.allocation_timeout_ms = Some(d.allocation_ms as u64);
            policy.connect_timeout_ms = Some(d.connect_ms as u64);
            policy.interaction_timeout_ms = Some(d.interaction_ms as u64);
            match pool.checkout_until(policy, request.deadline) {
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
                        attach(host, lease, &item.request, &session, now, &mut active)
                    });
                    item.request.complete(result);
                }
            }
        }
        let mut index = 0;
        while index < active.len() {
            let exchange = &mut active[index];
            let discard = exchange.discard.load(Ordering::Acquire);
            let disposition = match host.resource(exchange.lease.as_ref().expect("active lease")) {
                Ok(resource) => {
                    let result = match resource.pump() {
                        Some(pump) => transfer(pump, exchange, &mut cx, now),
                        None => Err(()),
                    };
                    match result {
                        Ok(false) => None,
                        Ok(true) if !discard && resource.reclaim() => Some(Disposition::Reusable),
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
    now: u64,
    active: &mut Vec<Active>,
) -> OpenOutcome
where
    C::Resource: ChannelResource,
{
    if request.expired(now) {
        return Err(io::ErrorKind::TimedOut.into());
    }
    let context = &request.context;
    let config = |side| {
        let mut config = StreamConfig::new(&context.session_id, context.stream_id, side);
        config.profile_version = 2;
        let limits = &context.limits;
        config.receive_limits = limits.clone();
        config.peer_limits = limits.clone();
        config.receive_window = (limits.receive_window as usize).min(65_536);
        config.peer_receive_window = (limits.receive_window as usize).min(65_536);
        config.max_payload = (limits.data_payload as usize).min(16_384);
        let d = &context.deadlines;
        config.io_timeout_ms = d.io_ms as u64;
        config.interaction_budget_ms = d.interaction_ms as u64;
        config.close_timeout_ms = d.cleanup_ms as u64;
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
    if resource.pump().is_none() {
        return Err(io::Error::other("resource did not install a channel pump"));
    }
    let (inbound, worker_inbound) = mpsc::sync_channel(16);
    let (worker_outbound, outbound) = mpsc::sync_channel(16);
    let cancelled = Arc::new(AtomicBool::new(false));
    let discard = Arc::new(AtomicBool::new(false));
    active.push(Active {
        lease: Some(lease),
        peer,
        bridge_inbound: worker_inbound,
        bridge_outbound: worker_outbound,
        bridge_cancelled: cancelled.clone(),
        bridge_pending: None,
        bridge_session: context.session_id.clone(),
        bridge_stream_id: context.stream_id,
        bridge_version: context.version,
        bridge_terminal_delivered: false,
        bridge_waker: context.waker.clone(),
        discard: discard.clone(),
    });
    Ok((
        EndpointAttachment {
            owner: client,
            inbound,
            outbound,
            cancelled,
            discard,
            worker: thread::current(),
        },
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
    active: &mut Active,
    cx: &mut Context<'_>,
    now: u64,
) -> Result<bool, ()> {
    // Time must precede incoming Close, which switches away from the I/O clock.
    pump.advance(now);
    let mut handed = false;
    let result = (|| {
        active.peer.advance(now);
        if active.bridge_cancelled.load(Ordering::Acquire) {
            return Err(());
        }
        for _ in 0..8 {
            let mut message = match active.bridge_inbound.try_recv() {
                Ok(message) => message,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return Err(()),
            };
            message.session_id = active.bridge_session.clone();
            message.stream_id = active.bridge_stream_id;
            message.version = active.bridge_version;
            pump.deliver(message).map_err(|_| ())?;
        }
        pump.tick(cx, now).map_err(|_| ())?;
        for _ in 0..8 {
            let message = match active.bridge_pending.take() {
                Some(message) => message,
                None => match pump.poll_next_message(cx) {
                    Poll::Ready(Ok(Some(message))) => message,
                    Poll::Ready(Err(_)) => return Err(()),
                    _ => break,
                },
            };
            let terminal = matches!(
                message.kind,
                gwz_transport::protocol::MessageKind::Closed
                    | gwz_transport::protocol::MessageKind::Failed
            );
            match active.bridge_outbound.try_send(message) {
                Ok(()) => {
                    handed = true;
                    if terminal {
                        active.bridge_terminal_delivered = true;
                    }
                }
                Err(TrySendError::Full(message)) => {
                    active.bridge_pending = Some(message);
                    break;
                }
                Err(TrySendError::Disconnected(_)) => return Err(()),
            }
        }
        Ok(active.bridge_terminal_delivered && active.bridge_pending.is_none())
    })();
    if result.is_err() {
        pump.cancel();
        active.peer.disconnect();
    }
    // The bridge's owner parks between passes: wake it for what it can read.
    if handed || result != Ok(false) {
        if let Some(waker) = &active.bridge_waker {
            waker.wake_by_ref();
        }
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
        impl Endpoint {
            /// The worker's shutdown status, still readable once every clone
            /// of this endpoint is gone.
            pub(crate) fn shutdown_watch(&self) -> ssh_shutdown::ShutdownWatch {
                ssh_shutdown::ShutdownWatch(self.shared.status.clone())
            }
        }
        #[path = "ssh_worker_tests.rs"]
        mod queue_tests;
    }
}

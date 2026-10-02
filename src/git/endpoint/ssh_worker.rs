//! Shared endpoint worker. Native setup is an injected nonblocking owner; an
//! open is submitted without waiting, and its reply is an
//! [`EndpointAttachment`] that the placement endpoint drives.
use super::{
    agent_job::{Cleanup, Job},
    setup_retry::{self, Phase},
    ssh_admission::{Admissions, Reader},
    ssh_channel::{GitService, SshChannel},
    ssh_handoff::{Handoff, UrlExtras},
    ssh_key_snapshot::{Entry, Registry},
    ssh_pool::{Connector, Opening, PoolHost, Progress, Resource},
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
    /// The handoff from which each open takes what its URL holds beyond its
    /// destination, when a driver in this process deposited any (TR2.18).
    handoff: Handoff,
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
#[derive(Clone)]
pub(crate) struct Endpoint {
    shared: Arc<Shared>,
    cleanup: Duration,
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
            /// An endpoint whose opens no driver deposits extras for.
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
                Self::with_reader(config, registry, Arc::new(Registry::start), factory, io_timeout_ms)
            }
            /// `with_registry`, reading selected keys with `reader`.
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
                Self::build(config, registry, reader, factory, io_timeout_ms, Handoff::default())
            }
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

mod open_request;
pub(crate) use open_request::EndpointOpenFailure;
pub(super) use open_request::{OpenOutcome, OpenRequest};

mod endpoint;

mod runner;
use runner::run;

//! Shared endpoint worker. Native setup is an injected nonblocking owner; an
//! open is submitted without waiting, and its reply is an
//! [`EndpointAttachment`] that the placement endpoint drives.
use super::{
    agent_job::{Cleanup, Job, Supervisor},
    setup_retry::{self, Governor, Phase},
    shutdown_watch::Watch,
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
    /// Restore an idle owner only after the pump's finished close: the
    /// channel's CHANNEL_CLOSE.
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
    watch: Watch,
    /// The handoff from which each open takes what its URL holds beyond its
    /// destination, when a driver in this process deposited any (TR2.18).
    handoff: Handoff,
    /// The host's job budget, which this endpoint's identity checks draw on.
    supervisor: Supervisor,
    /// The limit machines of this endpoint's pool, which its pool host
    /// reports every connection's state change to.
    governor: Governor,
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
    /// What the checkout asked the pool for; a retry asks it again, fresh.
    policy: gwz_transport::pool::Request,
}
/// How long an open waits for a closing connection of its key and identity
/// before it goes to the pool (GwzTransportSshBackgroundCloseDesign §6).
const WAIT_FOR_CLOSE_MS: u64 = 250;
/// An open that waits, once, for a closing exchange to end, and then checks a
/// connection out as any open does.
struct Deferred {
    request: OpenRequest,
    policy: gwz_transport::pool::Request,
    /// The closing exchange this open waits for.
    claim: i64,
    until: u64,
}
/// The close that goes on after a terminal has been handed to the member,
/// while the exchange keeps its lease (design §4).
struct Closing {
    /// When the close is given up on: the exchange's `cleanup_ms` after the
    /// handoff.
    deadline: u64,
    /// The terminal was a failure: there is no close to wait for.
    failed: bool,
    /// The open that waits for this close to end, if one does.
    claimed: Option<i64>,
}
/// Where an exchange stands after a pass of the worker.
enum Turn {
    Running,
    /// The member has its result and the channel's close goes on.
    Closing,
    /// The channel's close is complete: the connection can be reclaimed.
    Finished,
    /// The exchange is over, and its connection is not reusable.
    Over,
}
struct Active {
    /// What an open for this exchange's connection must match to wait for it.
    key: Key,
    identity: Identity,
    cleanup_ms: u64,
    closing: Option<Closing>,
    terminal_failed: bool,
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
    /// The open's reply, until the channel is open.
    held: Option<Held>,
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
/// The configuration of one end of an open's bridged stream.
fn stream_config(context: &BridgeContext, side: Side) -> StreamConfig {
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
    if side == Side::Endpoint {
        // The pump reads from this end only to take the client's input, so
        // while it relays the server's reply nothing forces a send: left to
        // the default batch window, an advertisement waits it out (100 ms)
        // before it reaches Git. The bridge is in-process, so there is
        // nothing to coalesce for.
        config.coalesce_delay_ms = 0;
    }
    config
}
/// Leases `lease` to `request`'s exchange. Its reply is held with the
/// exchange until the channel is open (`held`); a reused lease whose
/// connection was found dead hands the request back (`Unserved::Dead`).
fn attach<C: Connector>(
    host: &mut PoolHost<C>,
    lease: Lease,
    request: OpenRequest,
    policy: gwz_transport::pool::Request,
    session: &str,
    now: u64,
    active: &mut Vec<Active>,
) -> Option<Unserved>
where
    C::Resource: ChannelResource,
{
    let failed = |request, error| Unserved::Failed(request, error);
    if let Some(reused) = host.lost(&lease) {
        let _ = host.release(lease, Disposition::Discarded);
        return Some(if reused {
            Unserved::Dead(request, policy)
        } else {
            failed(request, io::ErrorKind::ConnectionReset.into())
        });
    }
    if request.expired(now) {
        return Some(failed(request, io::ErrorKind::TimedOut.into()));
    }
    let context = &request.context;
    let config = |side| stream_config(context, side);
    let streams = Stream::new(config(Side::Initiator))
        .and_then(|initiator| Ok((initiator, Stream::new(config(Side::Endpoint))?)));
    let ((client, peer), (stream, endpoint)) = match streams {
        Ok(streams) => streams,
        Err(error) => return Some(failed(request, io::Error::other(error))),
    };
    peer.advance(now);
    endpoint.advance(now);
    let started = (|| {
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
        Ok((connection_id, reused, facts))
    })();
    let (connection_id, reused, facts) = match started {
        Ok(started) => started,
        Err(error) => return Some(failed(request, error)),
    };
    let (inbound, worker_inbound) = mpsc::sync_channel(16);
    let (worker_outbound, outbound) = mpsc::sync_channel(16);
    let cancelled = Arc::new(AtomicBool::new(false));
    let discard = Arc::new(AtomicBool::new(false));
    let attachment = EndpointAttachment {
        owner: client,
        inbound,
        outbound,
        cancelled: cancelled.clone(),
        discard: discard.clone(),
        worker: thread::current(),
    };
    let opened = Opened {
        connection_id,
        reused,
        endpoint_id: session.into(),
        trust_owner: session.into(),
        facts,
        receive_limits: config(Side::Endpoint).receive_limits,
    };
    active.push(Active {
        key: request.key.clone(),
        identity: policy.identity.clone(),
        cleanup_ms: context.deadlines.cleanup_ms as u64,
        closing: None,
        terminal_failed: false,
        lease: Some(lease),
        peer,
        bridge_inbound: worker_inbound,
        bridge_outbound: worker_outbound,
        bridge_cancelled: cancelled,
        bridge_pending: None,
        bridge_session: context.session_id.clone(),
        bridge_stream_id: context.stream_id,
        bridge_version: context.version,
        bridge_terminal_delivered: false,
        bridge_waker: context.waker.clone(),
        discard,
        held: Some(Held {
            request,
            policy,
            reused,
            reply: (attachment, opened),
        }),
    });
    None
}
/// One pass over an exchange the member is still using. It ends when the
/// terminal has been handed over; the close then goes on (`close_step`).
fn transfer(
    pump: &mut SshPump<SshChannel>,
    active: &mut Active,
    cx: &mut Context<'_>,
    now: u64,
    discard: bool,
) -> Result<Turn, ()> {
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
            let failed = message.kind == gwz_transport::protocol::MessageKind::Failed;
            let terminal = failed || message.kind == gwz_transport::protocol::MessageKind::Closed;
            match active.bridge_outbound.try_send(message) {
                Ok(()) => {
                    handed = true;
                    if terminal {
                        active.bridge_terminal_delivered = true;
                        active.terminal_failed = failed;
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
    if result? {
        // The bound on the close runs from here, for a failed terminal too.
        active.closing = Some(Closing {
            deadline: now.saturating_add(active.cleanup_ms),
            failed: active.terminal_failed,
            claimed: None,
        });
        return Ok(settle(pump, active, now, discard));
    }
    Ok(Turn::Running)
}
/// One pass over an exchange whose member has its terminal. The member can no
/// longer cancel or drop the exchange: neither is read here. The pump runs on
/// until the channel's close ends, fails, or reaches its bound.
fn close_step(
    pump: &mut SshPump<SshChannel>,
    active: &mut Active,
    cx: &mut Context<'_>,
    now: u64,
    discard: bool,
) -> Result<Turn, ()> {
    pump.advance(now);
    if pump.tick(cx, now).is_err() {
        pump.cancel();
        return Err(());
    }
    Ok(settle(pump, active, now, discard))
}
/// Whether a handed-over exchange is over (design §4): a failed terminal,
/// discard-after-use and a retired channel end it at once, a finished close
/// makes the connection reusable, and the bound ends any other.
fn settle(pump: &SshPump<SshChannel>, active: &Active, now: u64, discard: bool) -> Turn {
    let closing = active.closing.as_ref().expect("a handed-over exchange");
    if closing.failed || discard {
        Turn::Over
    } else if pump.finished() {
        Turn::Finished
    } else if pump.retired() || now >= closing.deadline {
        Turn::Over
    } else {
        Turn::Closing
    }
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

mod held;
use held::{Held, Unserved};

mod runner;
use runner::run;

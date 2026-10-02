use super::*;
use crate::git::endpoint::{
    placement_endpoint::{EndpointError, PlacementEndpoint},
    setup_retry,
    shared_reservation::Authority,
    ssh_channel::GitService,
    ssh_destination::Destination,
    ssh_handoff::{Handoff, UrlExtras},
    ssh_worker::ThreadWake,
    stream_io::BlockingStream,
};
use gwz_transport::{
    mux::{self, Mux, Owner, Phase},
    protocol::*,
    stream::{self, MessageEndpoint, Stream},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::{Future, poll_fn},
    io,
    pin::{Pin, pin},
    sync::{
        Condvar, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    task::{Context, Poll, Waker},
    thread,
    time::Instant,
};
mod capacity;
mod close;
mod driver;
mod local_link;
mod passes;
mod port;
mod requests;
mod wait;
pub(super) use local_link::LocalLink;
use wait::Wait;
pub(crate) use driver::SshOpenFailure;
pub type Attachment = (String, Envelope);
const CLEANUP: Duration = Duration::from_secs(5);
const CHECK_MS: u64 = 120_000;
pub(super) fn limits() -> Limits {
    let mut limits = binding::default_limits();
    // Match the existing endpoint stream's bounded receive window.
    limits.receive_window = 65536;
    limits
}
/// A stream's next message, kept across passes: while it waits, the stream's
/// next change wakes the pass that forwards the message.
pub(super) type NextMessage =
    Pin<Box<dyn Future<Output = Result<Option<Envelope>, stream::Error>> + Send>>;
pub(super) fn next_message(peer: &Arc<MessageEndpoint>) -> NextMessage {
    let peer = peer.clone();
    Box::pin(async move { peer.next_message().await })
}
fn mux_config() -> mux::Config {
    mux::Config {
        limits: limits(),
        ..Default::default()
    }
}
static SERIAL: AtomicU64 = AtomicU64::new(1);
pub(super) fn unique() -> ModelResult<String> {
    SERIAL
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
        .map(|n| format!("placement-{}-{n}", std::process::id()))
        .map_err(|_| unavailable("transport IDs exhausted"))
}
fn protocol_failure(code: gwz_transport::protocol::ErrorCode) -> Failure {
    Failure {
        detail: None,
        setup_cause: None,
        code,
        effect: Effect::None,
        facts: None,
    }
}
fn mux_error(error: mux::Error) -> ModelError {
    match error {
        mux::Error::InvalidRequest | mux::Error::Protocol => {
            invalid("invalid transport session message")
        }
        mux::Error::Rejected => unsupported("endpoint negotiation rejected"),
        _ => unavailable("transport session unavailable"),
    }
}
struct Entry {
    request: String,
    stream: Stream,
    peer: Arc<MessageEndpoint>,
    opened: bool,
    report_open_failure: bool,
    opening_cancel_effect: Effect,
    reply: Arc<Wait<Result<(BlockingStream, Opened), Failure>>>,
    deadline: Option<Instant>,
    pending: Option<Envelope>,
    next: Option<NextMessage>,
    observe: Arc<dyn Fn(i64, &Opened) + Send + Sync>,
    facts: Arc<dyn Fn(&Facts) + Send + Sync>,
}
struct Check {
    request: String,
    result: Arc<Wait<Result<(), Failure>>>,
}
struct Registration {
    operation: Option<String>,
    sealed: Option<Instant>,
    // Logical mux retirement is one-shot; physical cleanup can outlive it.
    mux_retired: bool,
    result: Option<CleanupReport>,
    /// The request's `--max-retries`, which bounds how long its opens wait.
    max_retries: u32,
}
struct State {
    owner: Option<Owner>,
    port: Option<mux::Port>,
    session: Option<String>,
    registrations: BTreeMap<String, Registration>,
    used: BTreeSet<String>,
    closed: bool,
    streams: BTreeMap<i64, Entry>,
    checks: BTreeMap<i64, Check>,
    engine: Option<PlacementEndpoint>,
    https: Option<super::https_endpoint::HttpsEndpoint>,
    authority: Option<Authority>,
    installed_capacity: Option<pool::Capacity>,
    prefer_https: bool,
    endpoint_config: Option<binding::EndpointConfig>,
    pending: Option<Attachment>,
    incoming: Option<Attachment>,
    io_timeout_ms: u64,
    connect_timeout_ms: u64,
    ssh_allocation_ms: u64,
    ssh_interaction_ms: u64,
    /// A driver's handoff to the endpoint session in its process, when it
    /// has one: each SSH open's URL extras go through it (TR2.18).
    handoff: Option<Handoff>,
}
struct Event {
    wakes: Mutex<BTreeMap<u64, Waker>>,
    next: AtomicU64,
}
impl Event {
    fn new() -> Self {
        Self {
            wakes: Mutex::new(BTreeMap::new()),
            next: AtomicU64::new(1),
        }
    }
    fn signal(&self) {
        let wakes = std::mem::take(&mut *self.wakes.lock().unwrap_or_else(|e| e.into_inner()));
        for (_, wake) in wakes {
            wake.wake();
        }
    }
}
struct Listener<'a> {
    event: &'a Event,
    id: u64,
}
impl Listener<'_> {
    fn arm(&self, cx: &Context<'_>) -> bool {
        let mut wakes = self.event.wakes.lock().unwrap_or_else(|e| e.into_inner());
        if wakes.len() >= 128 && !wakes.contains_key(&self.id) {
            return false;
        }
        wakes.insert(self.id, cx.waker().clone());
        true
    }
}
impl Drop for Listener<'_> {
    fn drop(&mut self) {
        self.event
            .wakes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
    }
}
pub(super) struct Session {
    state: Mutex<State>,
    event: Event,
    origin: Instant,
    capacity_gate: AtomicBool,
    admission_gate: AtomicBool,
    test_hooks: TestHooks,
    /// The thread that runs the session's passes.
    passes: OnceLock<thread::Thread>,
}
cfg_if::cfg_if! {
    if #[cfg(test)] {
        struct TestHooks {
            hold_retirement: AtomicBool,
            waiting_retirement: AtomicBool,
            hold_pump: AtomicBool,
            pumps: std::sync::atomic::AtomicUsize,
        }
        impl TestHooks {
            fn new() -> Self {
                Self {
                    hold_retirement: AtomicBool::new(false),
                    waiting_retirement: AtomicBool::new(false),
                    hold_pump: AtomicBool::new(false),
                    pumps: std::sync::atomic::AtomicUsize::new(0),
                }
            }
            fn should_hold_pump(&self) -> bool {
                self.hold_pump.load(Ordering::Acquire)
            }
            fn pumped(&self) {
                self.pumps.fetch_add(1, Ordering::AcqRel);
            }
            fn should_hold_retirement(&self) -> bool {
                if self.hold_retirement.load(Ordering::Acquire) {
                    self.waiting_retirement.store(true, Ordering::Release);
                    true
                } else {
                    false
                }
            }
        }
    } else {
        struct TestHooks;
        impl TestHooks {
            fn new() -> Self { Self }
            fn should_hold_retirement(&self) -> bool { false }
            fn should_hold_pump(&self) -> bool { false }
            fn pumped(&self) {}
        }
    }
}
impl Session {
    cfg_if::cfg_if! { if #[cfg(test)] {
        pub(super) fn capacity_for_test(&self) -> Option<pool::Capacity> {
            self.state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .engine
                .as_ref()
                .map(|engine| engine.pool().capacity())
        }
        pub(super) fn ssh_counts_for_test(&self) -> Option<pool::Counts> {
            self.state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .engine
                .as_ref()
                .map(|engine| engine.pool().counts())
        }
        pub(super) fn https_counts_for_test(&self) -> Option<pool::Counts> {
            self.state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .https
                .as_ref()
                .map(|endpoint| endpoint.pool().counts())
        }
        pub(super) fn hold_admission_for_test(&self) {
            assert!(!self.admission_gate.swap(true, Ordering::AcqRel));
        }
        pub(super) fn release_admission_for_test(&self) {
            assert!(self.admission_gate.swap(false, Ordering::AcqRel));
            self.event.signal();
        }
        pub(super) fn hold_capacity_for_test(&self) {
            assert!(!self.capacity_gate.swap(true, Ordering::AcqRel));
        }
        pub(super) fn release_capacity_for_test(&self) {
            assert!(self.capacity_gate.swap(false, Ordering::AcqRel));
            self.event.signal();
        }
        pub(super) fn hold_pump_for_test(&self, hold: bool) {
            self.test_hooks.hold_pump.store(hold, Ordering::Release);
            // A pass already under way holds the state lock; waiting for it
            // puts the hold in force before this returns.
            drop(self.state.lock().unwrap_or_else(|error| error.into_inner()));
        }
        pub(super) fn pumps_for_test(&self) -> usize {
            self.test_hooks.pumps.load(Ordering::Acquire)
        }
        pub(super) fn hold_retirement_for_test(&self) {
            self.test_hooks.hold_retirement.store(true, Ordering::Release);
        }
        pub(super) fn retirement_waiting_for_test(&self) -> bool {
            self.test_hooks.waiting_retirement.load(Ordering::Acquire)
        }
    } }
    fn empty() -> State {
        State {
            owner: None,
            port: None,
            session: None,
            registrations: BTreeMap::new(),
            used: BTreeSet::new(),
            closed: false,
            streams: BTreeMap::new(),
            checks: BTreeMap::new(),
            engine: None,
            https: None,
            authority: None,
            installed_capacity: None,
            prefer_https: false,
            endpoint_config: None,
            pending: None,
            incoming: None,
            io_timeout_ms: 9000,
            connect_timeout_ms: 30_000,
            ssh_allocation_ms: 30_000,
            ssh_interaction_ms: 120_000,
            handoff: None,
        }
    }
    /// A driver session. `handoff` is shared with the endpoint session its
    /// carrier reaches in this process, if any.
    pub(super) fn driver(
        io_timeout_ms: u64,
        connect_timeout_ms: u64,
        handoff: Option<Handoff>,
    ) -> ModelResult<(Arc<Self>, TransportPort)> {
        Self::driver_with_ssh_budgets(io_timeout_ms, connect_timeout_ms, handoff, 30_000, 120_000)
    }
    pub(super) fn driver_with_ssh_budgets(
        io_timeout_ms: u64,
        connect_timeout_ms: u64,
        handoff: Option<Handoff>,
        allocation_ms: u64,
        interaction_ms: u64,
    ) -> ModelResult<(Arc<Self>, TransportPort)> {
        let mut state = Self::empty();
        state.io_timeout_ms = io_timeout_ms;
        state.connect_timeout_ms = connect_timeout_ms;
        state.ssh_allocation_ms = allocation_ms;
        state.ssh_interaction_ms = interaction_ms.min(120_000);
        state.handoff = handoff;
        let id = unique()?;
        let (owner, port) = Owner::new(Mux::initiator(&id, mux_config()).map_err(mux_error)?);
        state.owner = Some(owner);
        state.port = Some(port);
        state.session = Some(id);
        let session = Self::start(state)?;
        Ok((session.clone(), TransportPort(Arc::new(PortLease(session)))))
    }
    pub(super) fn endpoint(config: SshEndpointConfig) -> ModelResult<(Arc<Self>, TransportPort)> {
        Self::endpoint_with_https(config, None, Handoff::default())
    }
    /// An HTTPS endpoint takes the helper slots of the host that opens it.
    /// `handoff` is shared with the driver session in this process, if any.
    pub(super) fn endpoint_with_https(
        config: SshEndpointConfig,
        https: Option<(HttpsEndpointConfig, HelperSlots)>,
        handoff: Handoff,
    ) -> ModelResult<(Arc<Self>, TransportPort)> {
        let mut state = Self::empty();
        let id = unique()?;
        let authority = crate::git::endpoint::shared_reservation::Authority::new(
            config.pool.total,
            config.pool.per_host,
        );
        state.authority = Some(authority.clone());
        state.installed_capacity = Some(pool::Capacity::from(&config.pool));
        let ssh_helpers = https.as_ref().and_then(|(https, slots)| https.auth.as_ref().map(|auth|
            Arc::new(crate::git::endpoint::ssh_password_helpers::Helpers::new(auth.clone(), slots.clone()))));
        let ssh = ssh_local::connect_with_helpers(
            config.pool.clone(),
            config.home.join(".ssh/known_hosts"),
            config.agent.clone(),
            config.io_timeout_ms,
            authority.clone(),
            handoff,
            ssh_helpers,
        )
        .map_err(|_| unavailable("SSH endpoint construction failed"))?;
        if let Some((https, helper_slots)) = https {
            state.https = Some(super::https_endpoint::HttpsEndpoint::new(
                https,
                config.pool.clone(),
                config.io_timeout_ms,
                authority.clone(),
                id.clone(),
                helper_slots,
            )?);
        }
        let https_enabled = state.https.is_some();
        state.engine = Some(
            PlacementEndpoint::new(ssh, config.home, id.clone(), id.clone())
                .map_err(|_| unavailable("endpoint supervisor unavailable"))?,
        );
        state.endpoint_config = Some(binding::EndpointConfig {
            endpoint_id: id.clone(),
            trust_owner: id,
            role: EndpointRole::Driver,
            schemes: if https_enabled {
                vec![Scheme::Ssh, Scheme::Https]
            } else {
                vec![Scheme::Ssh]
            },
            policies: if https_enabled {
                vec![
                    AuthPolicy::SshAmbient,
                    AuthPolicy::SshExplicit,
                    AuthPolicy::Anonymous,
                    AuthPolicy::Gh,
                ]
            } else {
                vec![AuthPolicy::SshAmbient, AuthPolicy::SshExplicit]
            },
            limits: limits(),
        });
        let session = Self::start(state)?;
        Ok((session.clone(), TransportPort(Arc::new(PortLease(session)))))
    }
    fn listener(&self) -> Listener<'_> {
        Listener {
            event: &self.event,
            id: self.event.next.fetch_add(1, Ordering::Relaxed),
        }
    }

    pub(super) fn register(&self, request: &str, operation: Option<String>) -> ModelResult<()> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            return Err(unavailable("transport session is closed"));
        }
        if !request::identifier(request) || state.used.contains(request) || state.used.len() >= 256
        {
            return Err(invalid("invalid or exhausted request registration"));
        }
        if let Some(owner) = &state.owner {
            owner
                .register(request, operation.clone())
                .map_err(mux_error)?;
        }
        state.used.insert(request.into());
        state.registrations.insert(
            request.into(),
            Registration {
                operation,
                sealed: None,
                mux_retired: false,
                result: None,
                max_retries: setup_retry::DEFAULT_MAX_RETRIES,
            },
        );
        Ok(())
    }
}
struct PortLease(Arc<Session>);
#[derive(Clone)]
pub struct TransportPort(Arc<PortLease>);

cfg_if::cfg_if! { if #[cfg(test)] { #[path="cleanup_tests.rs"] mod cleanup_tests; } }

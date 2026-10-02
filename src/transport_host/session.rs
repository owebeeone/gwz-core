use super::*;
use crate::git::endpoint::{
    placement_endpoint::{EndpointError, PlacementEndpoint},
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
mod driver;
mod local_link;
pub(super) use local_link::LocalLink;
cfg_if::cfg_if! {
    if #[cfg(test)] {
        // The transport host's tests downcast SSH open errors to it.
        pub(super) use driver::SshOpenFailure;
    }
}
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
struct Wait<T: Clone> {
    value: Mutex<Option<T>>,
    changed: Condvar,
}
impl<T: Clone> Wait<T> {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            value: Mutex::new(None),
            changed: Condvar::new(),
        })
    }
    fn complete(&self, value: T) {
        let mut state = self.value.lock().unwrap_or_else(|e| e.into_inner());
        if state.is_none() {
            *state = Some(value);
            self.changed.notify_all();
        }
    }
    fn complete_with(&self, value: T, before: impl FnOnce()) -> bool {
        let mut state = self.value.lock().unwrap_or_else(|e| e.into_inner());
        if state.is_some() {
            return false;
        }
        before();
        *state = Some(value);
        self.changed.notify_all();
        true
    }
    fn get(&self) -> T {
        let mut state = self.value.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(value) = &*state {
                return value.clone();
            }
            state = self.changed.wait(state).unwrap_or_else(|e| e.into_inner());
        }
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
    /// A driver's handoff to the endpoint session in its process, when it
    /// has one: each SSH open's URL extras go through it (TR2.18).
    handoff: Option<Handoff>,
}
fn has_non_idle_lease(state: &State) -> bool {
    let non_idle = |counts: pool::Counts| counts.total() != counts.idle;
    state
        .engine
        .as_ref()
        .is_some_and(|engine| non_idle(engine.pool().counts()))
        || state
            .https
            .as_ref()
            .is_some_and(|endpoint| non_idle(endpoint.pool().counts()))
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
struct CapacityMutation<'a> {
    session: &'a Session,
    armed: bool,
}
impl Drop for CapacityMutation<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.session.close();
        }
    }
}
struct AdmissionLeader<'a>(&'a Session);
impl Drop for AdmissionLeader<'_> {
    fn drop(&mut self) {
        self.0.admission_gate.store(false, Ordering::Release);
        self.0.event.signal();
    }
}
struct CapacityLeader<'a>(&'a Session);
impl Drop for CapacityLeader<'_> {
    fn drop(&mut self) {
        self.0.capacity_gate.store(false, Ordering::Release);
        self.0.event.signal();
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
    fn start(state: State) -> ModelResult<Arc<Self>> {
        let session = Arc::new(Self {
            state: Mutex::new(state),
            event: Event::new(),
            origin: Instant::now(),
            capacity_gate: AtomicBool::new(false),
            admission_gate: AtomicBool::new(false),
            test_hooks: TestHooks::new(),
            passes: OnceLock::new(),
        });
        let weak = Arc::downgrade(&session);
        let passes = thread::Builder::new()
            .name("gwz-placement".into())
            .spawn(move || {
                // Whatever gives a pass work wakes this thread. The timeout
                // only runs the deadlines when nothing does.
                let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
                while let Some(session) = weak.upgrade() {
                    let moved = session.drive(&waker);
                    let done = {
                        let state = session.state.lock().unwrap_or_else(|e| e.into_inner());
                        state.closed
                            && state.engine.as_ref().is_none_or(|e| e.pending() == 0)
                            && state.https.as_ref().is_none_or(|e| e.pending() == 0)
                    };
                    session.event.signal();
                    drop(session);
                    if done {
                        break;
                    }
                    if !moved {
                        thread::park_timeout(Duration::from_millis(5));
                    }
                }
            })
            .map_err(|_| unavailable("transport supervisor unavailable"))?;
        let _ = session.passes.set(passes.thread().clone());
        Ok(session)
    }
    /// Runs a pass now: the carrier moved a message into or out of its mux.
    fn wake(&self) {
        if let Some(thread) = self.passes.get() {
            thread.unpark();
        }
    }
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
        let mut state = Self::empty();
        state.io_timeout_ms = io_timeout_ms;
        state.connect_timeout_ms = connect_timeout_ms;
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
        let ssh = ssh_local::connect_with_authority(
            config.pool.clone(),
            config.home.join(".ssh/known_hosts"),
            config.agent.clone(),
            config.io_timeout_ms,
            authority.clone(),
            handoff,
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
    pub(super) fn is_closed(&self) -> bool {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.closed
            || state
                .owner
                .as_ref()
                .is_some_and(|o| o.phase() == Phase::Closed)
    }

    pub(super) async fn admit_client_request(
        self: &Arc<Self>,
        request: &str,
        capacity: pool::Capacity,
    ) -> ModelResult<ClientRequest> {
        // A differing physical policy is refused before consuming the mux's
        // lifetime request ID. Serialize that check with local admissions.
        let deadline = Instant::now() + CLEANUP;
        let listener = self.listener();
        let _admission = poll_fn(|cx| {
            if !listener.arm(cx) {
                return Poll::Ready(Err(unavailable("transport capacity wait unavailable")));
            }
            if Instant::now() >= deadline {
                return Poll::Ready(Err(unavailable("transport capacity wait timed out")));
            }
            if self
                .admission_gate
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                Poll::Ready(Ok(AdmissionLeader(self)))
            } else {
                Poll::Pending
            }
        })
        .await?;
        {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed {
                return Err(unavailable("transport session is closed"));
            }
            if !request::identifier(request)
                || state.used.contains(request)
                || state.used.len() >= 256
            {
                return Err(invalid("invalid or exhausted request registration"));
            }
            if state.installed_capacity != Some(capacity)
                && (state
                    .registrations
                    .values()
                    .any(|record| record.result.is_none())
                    || state.engine.as_ref().is_some_and(|engine| {
                        engine.pending().saturating_sub(engine.pool().counts().idle) != 0
                    })
                    || state
                        .https
                        .as_ref()
                        .is_some_and(|endpoint| endpoint.pending() != 0)
                    || has_non_idle_lease(&state))
            {
                return Err(ModelError::new(
                    crate::model::ErrorCode::TransportCapacityConflict,
                    "transport physical capacity conflicts with live work",
                ));
            }
        }
        self.install_capacity(capacity, deadline).await?;
        if Instant::now() >= deadline {
            return Err(unavailable("transport capacity wait timed out"));
        }
        ClientRequest::new(self.clone(), request)
    }

    pub(super) async fn install_capacity(
        &self,
        capacity: pool::Capacity,
        deadline: Instant,
    ) -> ModelResult<()> {
        // One physical-policy leader owns both pools and the shared authority
        // through retirement. Followers wait without holding the state mutex.
        let gate_listener = self.listener();
        let _leader = poll_fn(|cx| {
            if !gate_listener.arm(cx) {
                return Poll::Ready(Err(unavailable("transport capacity wait unavailable")));
            }
            if Instant::now() >= deadline {
                return Poll::Ready(Err(unavailable("transport capacity wait timed out")));
            }
            if self
                .capacity_gate
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Poll::Ready(Ok(CapacityLeader(self)));
            }
            Poll::Pending
        })
        .await?;
        // A finished request can still have bounded physical cleanup behind
        // its terminal reply. Wait for that owner to retire before installing
        // the next operation's limits; live overlapping requests still fail.
        let listener = self.listener();
        let reused = poll_fn(|cx| {
            if !listener.arm(cx) {
                return Poll::Ready(Err(unavailable("transport capacity wait unavailable")));
            }
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed {
                return Poll::Ready(Err(unavailable("transport operation is active")));
            }
            // Identical physical policy shares the installed pools. In
            // particular, a live lease must not trigger a reinstall.
            if state.installed_capacity == Some(capacity) {
                return Poll::Ready(Ok(true));
            }
            if state
                .registrations
                .values()
                .any(|record| record.result.is_none())
            {
                return Poll::Ready(Err(unavailable("transport operation is active")));
            }
            // The SSH worker's pending count includes healthy idle sockets.
            // Those sockets are the resource this operation is meant to reuse;
            // waiting for them to disappear makes a sequential request time out.
            let pending = state.engine.as_ref().is_some_and(|engine| {
                engine.pending().saturating_sub(engine.pool().counts().idle) != 0
            }) || state
                .https
                .as_ref()
                .is_some_and(|endpoint| endpoint.pending() != 0);
            if !pending {
                return Poll::Ready(Ok(false));
            }
            if Instant::now() >= deadline {
                return Poll::Ready(Err(unavailable("prior transport cleanup incomplete")));
            }
            Poll::Pending
        })
        .await?;
        if reused {
            return Ok(());
        }
        let mut mutation = CapacityMutation {
            session: self,
            armed: false,
        };
        let (ssh, https, authority) = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.installed_capacity == Some(capacity) {
                return Ok(());
            }
            if state.closed
                || state.engine.as_ref().is_some_and(|engine| {
                    engine.pending().saturating_sub(engine.pool().counts().idle) != 0
                })
                || state
                    .https
                    .as_ref()
                    .is_some_and(|endpoint| endpoint.pending() != 0)
            {
                return Err(unavailable("transport operation is active"));
            }
            if state
                .registrations
                .values()
                .any(|record| record.result.is_none())
            {
                return Err(unavailable("transport operation is active"));
            }
            if Instant::now() >= deadline {
                return Err(unavailable("transport capacity wait timed out"));
            }
            if has_non_idle_lease(&state) {
                return Err(unavailable("transport capacity is active"));
            }
            let ssh = state
                .engine
                .as_ref()
                .ok_or_else(|| unavailable("SSH endpoint unavailable"))?;
            let https = state.https.as_ref().map(|endpoint| endpoint.pool().clone());
            mutation.armed = true;
            if let Some(https) = &https {
                ssh.pool()
                    .install_capacity_pair(https, capacity)
                    .map_err(|_| unavailable("transport capacity is active"))?;
            } else {
                ssh.pool()
                    .install_capacity(capacity)
                    .map_err(|_| unavailable("transport capacity is active"))?;
            }
            ssh.set_request_capacity(capacity.max_requests);
            let ssh_pool = ssh.pool().clone();
            state.installed_capacity = None;
            (
                ssh_pool,
                https,
                state
                    .authority
                    .clone()
                    .ok_or_else(|| unavailable("shared capacity unavailable"))?,
            )
        };
        let listener = self.listener();
        let retired = poll_fn(|cx| {
            if !listener.arm(cx) {
                return Poll::Ready(Err(unavailable("transport capacity wait unavailable")));
            }
            if self.test_hooks.should_hold_retirement() {
                return Poll::Pending;
            }
            if ssh.counts().closing == 0
                && https.as_ref().is_none_or(|pool| pool.counts().closing == 0)
            {
                return Poll::Ready(Ok(()));
            }
            if Instant::now() >= deadline || self.is_closed() {
                return Poll::Ready(Err(unavailable("transport capacity retirement incomplete")));
            }
            Poll::Pending
        })
        .await;
        if let Err(error) = retired {
            // The pools have already accepted the new policy. A failure to
            // finish retirement cannot leave an unpaired usable endpoint.
            self.close();
            return Err(error);
        }
        if !authority.install_capacity(capacity.total, capacity.per_host) {
            self.close();
            return Err(unavailable("shared capacity remains occupied"));
        }
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .installed_capacity = Some(capacity);
        mutation.armed = false;
        Ok(())
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
            },
        );
        Ok(())
    }
    pub(super) fn begin(&self, request: &str) -> ModelResult<()> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .owner
            .as_ref()
            .ok_or_else(|| unavailable("no initiator"))?
            .begin(request)
            .map_err(mux_error)
    }
    pub(super) async fn ready(&self) -> ModelResult<()> {
        let owner = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .owner
            .clone()
            .ok_or_else(|| unavailable("no binding"))?;
        owner.ready().await.map_err(mux_error)
    }
    pub(super) fn cancel(&self, request: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(owner) = &state.owner {
            let _ = owner.cancel(request);
        }
        if let Some(record) = state.registrations.get_mut(request) {
            record.sealed.get_or_insert(Instant::now());
        }
        if let Some(engine) = &mut state.engine {
            engine.cancel_request(request);
        }
        if let Some(engine) = &mut state.https {
            engine.cancel_request(request);
        }
        if state
            .incoming
            .as_ref()
            .is_some_and(|item| item.0 == request)
        {
            state.incoming = None;
        }
        for entry in state.streams.values().filter(|e| e.request == request) {
            // A not-yet-open stream may be retired before its endpoint reply
            // arrives. Complete its independent blocking waiter before retiring
            // the stream so cancellation never relies on a peer acknowledgment.
            entry.reply.complete(Err(Failure {
                setup_cause: None,
                code: gwz_transport::protocol::ErrorCode::Cancelled,
                effect: entry.opening_cancel_effect,
                facts: None,
            }));
            entry.stream.cancel();
        }
        for check in state.checks.values().filter(|c| c.request == request) {
            check.result.complete(Err(protocol_failure(
                gwz_transport::protocol::ErrorCode::Cancelled,
            )));
        }
        drop(state);
        self.event.signal();
    }
    pub(super) fn seal(&self, request: &str) {
        self.cancel(request);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let closed = state.closed
            || state
                .owner
                .as_ref()
                .is_some_and(|o| o.phase() == Phase::Closed);
        let pending = state
            .engine
            .as_ref()
            .map_or(0, |e| e.pending_request_count(request))
            + state
                .https
                .as_ref()
                .map_or(0, |e| e.pending_request_count(request));
        if let Some(record) = state.registrations.get_mut(request) {
            record.sealed.get_or_insert(Instant::now());
            if closed {
                record.result.get_or_insert(CleanupReport {
                    pending_local_work: pending,
                    peer_cleanup_confirmed: false,
                });
            }
        }
    }
    pub(super) async fn finish(&self, request: &str) -> CleanupReport {
        self.seal(request);
        let listener = self.listener();
        poll_fn(|cx| {
            if !listener.arm(cx) {
                self.close();
                return Poll::Ready(self.report());
            }
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            match state.registrations.get(request) {
                Some(r) if r.result.is_some() => Poll::Ready(r.result.clone().unwrap()),
                None => Poll::Ready(CleanupReport::default()),
                _ => Poll::Pending,
            }
        })
        .await
    }
    pub(super) fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Self::close_state(&mut state);
        drop(state);
        self.event.signal();
    }
    fn close_state(state: &mut State) {
        state.closed = true;
        if let Some(port) = &state.port {
            port.disconnect();
        }
        if let Some(engine) = &mut state.engine {
            engine.shutdown();
        }
        if let Some(engine) = &mut state.https {
            engine.shutdown();
        }
        for (_, entry) in std::mem::take(&mut state.streams) {
            entry.peer.disconnect();
            entry.reply.complete(Err(protocol_failure(
                gwz_transport::protocol::ErrorCode::CarrierLost,
            )));
        }
        for (_, check) in std::mem::take(&mut state.checks) {
            check.result.complete(Err(protocol_failure(
                gwz_transport::protocol::ErrorCode::CarrierLost,
            )));
        }
        state.pending = None;
        state.incoming = None;
    }
    fn report(&self) -> CleanupReport {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        CleanupReport {
            pending_local_work: state.streams.len()
                + state.checks.len()
                + state.engine.as_ref().map_or(0, |e| e.pending())
                + state.https.as_ref().map_or(0, |e| e.pending()),
            peer_cleanup_confirmed: false,
        }
    }
    pub(super) async fn cleanup(&self) -> CleanupReport {
        let deadline = Instant::now() + CLEANUP;
        let listener = self.listener();
        poll_fn(|cx| {
            let armed = listener.arm(cx);
            let report = self.report();
            if !armed || report.pending_local_work == 0 || Instant::now() >= deadline {
                Poll::Ready(report)
            } else {
                Poll::Pending
            }
        })
        .await
    }
    fn inbound_port(&self, attachment: &Attachment) -> ModelResult<mux::Port> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            return Err(unavailable("transport port is closed"));
        }
        if state.owner.is_none() {
            if attachment.1.kind != MessageKind::Bind
                || !state.registrations.contains_key(&attachment.0)
                || gwz_transport::codec::admit(&attachment.1).is_err()
            {
                Self::close_state(&mut state);
                return Err(invalid("invalid endpoint bootstrap"));
            }
            let config = state
                .endpoint_config
                .clone()
                .ok_or_else(|| invalid("endpoint missing"))?;
            let mux =
                Mux::endpoint(&attachment.1.session_id, config, mux_config()).map_err(mux_error)?;
            let (owner, port) = Owner::new(mux);
            owner.advance(self.origin.elapsed().as_millis() as u64);
            for (id, record) in &state.registrations {
                owner
                    .register(id, record.operation.clone())
                    .map_err(mux_error)?;
                if record.sealed.is_some() {
                    let _ = owner.cancel(id);
                }
            }
            state.session = Some(attachment.1.session_id.clone());
            state.owner = Some(owner);
            state.port = Some(port);
        }
        Ok(state.port.as_ref().expect("initialized port").clone())
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        Self::close_state(self.state.get_mut().unwrap_or_else(|e| e.into_inner()));
    }
}
struct PortLease(Arc<Session>);
impl Drop for PortLease {
    fn drop(&mut self) {
        self.0.close();
    }
}
#[derive(Clone)]
pub struct TransportPort(Arc<PortLease>);
impl TransportPort {
    pub async fn next_message(&self) -> ModelResult<Option<Attachment>> {
        let session = &self.0.0;
        let listener = session.listener();
        let port = poll_fn(|cx| {
            if !listener.arm(cx) {
                session.close();
                return Poll::Ready(Err(unavailable("transport waiter capacity")));
            }
            let state = session.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed {
                Poll::Ready(Ok(None))
            } else if let Some(port) = &state.port {
                Poll::Ready(Ok(Some(port.clone())))
            } else {
                Poll::Pending
            }
        })
        .await?;
        let message = match port {
            Some(port) => port.next_message().await.map_err(mux_error)?,
            None => None,
        };
        if message.is_some() {
            // A send that found the mux's queue full can go now.
            session.wake();
        }
        Ok(message)
    }
    pub async fn deliver(&self, attachment: Attachment) -> ModelResult<()> {
        let result = self
            .0
            .0
            .inbound_port(&attachment)?
            .deliver(attachment)
            .await
            .map_err(mux_error);
        if result.is_err() {
            self.0.0.close();
        }
        self.0.0.event.signal();
        self.0.0.wake();
        result
    }
    pub fn disconnect(&self) {
        self.0.0.close();
    }
}

cfg_if::cfg_if! { if #[cfg(test)] { #[path="cleanup_tests.rs"] mod cleanup_tests; } }

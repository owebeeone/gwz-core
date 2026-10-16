use super::*;
use crate::git::endpoint::{
    https_connection::MachineSetting,
    placement_endpoint::{EndpointError, PlacementEndpoint},
    setup_retry,
    shared_reservation::Authority,
    ssh_channel::GitService,
    ssh_destination::Destination,
    ssh_handoff::{Handoff, UrlExtras},
    ssh_home::{Home, KnownHostsFile},
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
/// The missing-git message's closing hint, shared with the HTTPS twin in `request/https_failure.rs`.
pub(in crate::transport_host) use driver::native_hint::native_switch_hint;
mod local_link;
mod passes;
mod port;
mod requests;
cfg_if::cfg_if! { if #[cfg(test)] { mod wake_tests; } }
mod wait;
pub(crate) use driver::{AgentFailure, SshLimitFailure, SshOpenFailure};
pub(super) use local_link::LocalLink;
use wait::Wait;
pub type Attachment = (String, Envelope);
const CLEANUP: Duration = Duration::from_secs(5);
/// How long the placement thread parks when a pass moved nothing: the longest
/// it can miss a deadline. Whatever gives a pass work wakes it before that.
const PARK: Duration = Duration::from_millis(5);
cfg_if::cfg_if! {
    if #[cfg(any(test, all(gwz_transport_candidate, gwz_session_candidate, any(unix, windows))))] {
        const CHECK_MS: u64 = 120_000;
    }
}
/// The policies a driver or endpoint with SSH and HTTPS binds.
fn ssh_and_https_endpoint_policies() -> Vec<AuthPolicy> {
    vec![
        AuthPolicy::SshAmbient,
        AuthPolicy::SshExplicit,
        AuthPolicy::Anonymous,
        AuthPolicy::Gh,
        AuthPolicy::WindowsConfigured,
        AuthPolicy::WindowsDefault,
    ]
}

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
    /// The stall bound this stream's open ran under (`--ssh-timeout`, in
    /// milliseconds; 0 when the clock is off): its reads name it when they stall.
    stall_ms: u64,
    stream: Stream,
    peer: Arc<MessageEndpoint>,
    opened: bool,
    report_open_failure: bool,
    opening_cancel_effect: Effect,
    reply: Arc<Wait<Result<(BlockingStream, Opened), Failure>>>,
    /// The open's backstop (`driver/backstop.rs`), until the open is answered.
    backstop: Option<driver::Backstop>,
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
    /// An endpoint session's handoff, where it keeps current which of its
    /// opens it holds (`driver/backstop.rs`), and what it last published there.
    holds_to: Option<Handoff>,
    published_holds: BTreeSet<(String, i64)>,
    /// The SSH home the endpoint in this process reads under, and the variable that named it (plan step 3.1): a driver
    /// prints the `known_hosts` file when it words a host-key failure, and expands `~/` identity paths under it.
    home: Option<Home>,
    /// What the machine proxy setting said when the runtime's configuration was built (Windows; decisions D6, D7): a
    /// driver refuses an https open with the setting's text before it asks the endpoint for anything, and words a failed
    /// direct connection.
    machine: MachineSetting,
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
            park_ms: AtomicU64,
            passes: std::sync::atomic::AtomicUsize,
            /// How far a test has stepped the backstop's clock.
            backstop_skew_ms: AtomicU64,
        }
        impl TestHooks {
            fn new() -> Self {
                Self {
                    hold_retirement: AtomicBool::new(false),
                    waiting_retirement: AtomicBool::new(false),
                    hold_pump: AtomicBool::new(false),
                    pumps: std::sync::atomic::AtomicUsize::new(0),
                    park_ms: AtomicU64::new(PARK.as_millis() as u64),
                    passes: std::sync::atomic::AtomicUsize::new(0),
                    backstop_skew_ms: AtomicU64::new(0),
                }
            }
            /// What a test stepped the backstop's clock by.
            fn backstop_skew_ms(&self) -> u64 {
                self.backstop_skew_ms.load(Ordering::Acquire)
            }
            /// How long the placement thread parks when a pass moved nothing.
            fn park(&self) -> Duration {
                Duration::from_millis(self.park_ms.load(Ordering::Acquire))
            }
            /// A pass is done and the placement thread is about to park.
            fn passed(&self) {
                self.passes.fetch_add(1, Ordering::AcqRel);
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
            fn backstop_skew_ms(&self) -> u64 { 0 }
            fn park(&self) -> Duration { PARK }
            fn passed(&self) {}
            fn should_hold_retirement(&self) -> bool { false }
            fn should_hold_pump(&self) -> bool { false }
            fn pumped(&self) {}
        }
    }
}
impl Session {
    /// The clock the open backstops (`driver/backstop.rs`) run on, in
    /// milliseconds since the session began. It is the passes' clock, which a
    /// test can step ahead without moving the endpoints' or the streams'.
    pub(super) fn backstop_clock_ms(&self) -> u64 {
        let elapsed = self.origin.elapsed().as_millis().min(u64::MAX as u128) as u64;
        elapsed.saturating_add(self.test_hooks.backstop_skew_ms())
    }
    /// Has this session's endpoints give their limit machines' notes to
    /// `notes` (§9), between passes.
    pub(super) fn set_notes(&self, notes: &super::NoteSink) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(engine) = state.engine.as_mut() {
            engine.set_notes(notes.clone());
        }
        if let Some(https) = state.https.as_mut() {
            https.set_notes(notes.clone());
        }
    }
    /// The redirect from https to http that ended `request`'s open of `open`, if one did (text T5). Told once.
    pub(super) fn redirect_refusal(
        &self,
        request: &str,
        open: &str,
    ) -> Option<crate::git::endpoint::https_worker::redirect_log::Refused> {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .https
            .as_ref()
            .and_then(|https| https.redirect_refusal(request, open))
    }
    /// Tells this driver which SSH home its runtime's endpoint reads under, and where that home came from.
    pub(super) fn set_home(&self, home: Option<Home>) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .home = home;
    }
    /// Tells this driver what the machine proxy setting said when its runtime's configuration was built.
    pub(super) fn set_machine(&self, machine: MachineSetting) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .machine = machine;
    }
    /// What the machine proxy setting said when this runtime's configuration was built; `Unread` where none was read.
    pub(super) fn machine(&self) -> MachineSetting {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .machine
            .clone()
    }
    /// The SSH home the endpoint reads under: the one place `known_hosts` and `~/` identities are read from.
    pub(super) fn home(&self) -> Option<Home> {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .home
            .clone()
    }
    cfg_if::cfg_if! { if #[cfg(test)] {
        pub(super) fn capacity_for_test(&self) -> Option<pool::Capacity> {
            let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.engine.as_ref().map(|engine| engine.pool().capacity())
                .or_else(|| state.https.as_ref().map(|endpoint| endpoint.pool().capacity()))
        }
        cfg_if::cfg_if! { if #[cfg(test)] {
            pub(super) fn https_client_for_test(&self) -> crate::git::endpoint::https_worker::Client {
                self.state.lock().unwrap_or_else(|error| error.into_inner()).https.as_ref().unwrap().client_for_test()
            }
            /// Runs `f` on the HTTPS endpoint under the session lock, which a
            /// pass holds throughout: between passes.
            pub(super) fn with_https_for_test<T>(&self, f: impl FnOnce(&mut super::https_endpoint::HttpsEndpoint) -> T) -> T {
                f(self.state.lock().unwrap_or_else(|error| error.into_inner()).https.as_mut().unwrap())
            }
            /// The mux owner this session's pump sends through.
            pub(super) fn mux_owner_for_test(&self) -> Owner {
                self.state.lock().unwrap_or_else(|error| error.into_inner()).owner.clone().unwrap()
            }
        } }
        pub(super) fn authority_for_test(&self) -> crate::git::endpoint::shared_reservation::Authority {
            self.state.lock().unwrap_or_else(|error| error.into_inner()).authority.clone().unwrap()
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
        /// Makes the placement thread park for `park` between passes that
        /// moved nothing, so that a test can tell a wake from the timeout.
        pub(super) fn park_for_test(&self, park: Duration) {
            self.test_hooks.park_ms.store(park.as_millis() as u64, Ordering::Release);
        }
        /// The passes the placement thread has finished.
        pub(super) fn passes_for_test(&self) -> usize {
            self.test_hooks.passes.load(Ordering::Acquire)
        }
        /// How many opens an endpoint of this process holds now.
        pub(super) fn held_opens_for_test(&self) -> usize {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            Self::held_opens(&state).len()
        }
        /// How many streams this driver is tracking, opened or not.
        pub(super) fn streams_for_test(&self) -> usize {
            self.state.lock().unwrap_or_else(|e| e.into_inner()).streams.len()
        }
        /// Whether `request` has been sealed, which cancels all of it.
        pub(super) fn sealed_for_test(&self, request: &str) -> bool {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state
                .registrations
                .get(request)
                .is_some_and(|record| record.sealed.is_some())
        }
        /// Steps the open backstops' clock ahead by `by`, and wakes a pass.
        pub(super) fn step_backstop_for_test(&self, by: Duration) {
            self.test_hooks
                .backstop_skew_ms
                .fetch_add(by.as_millis() as u64, Ordering::AcqRel);
            self.wake();
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
            holds_to: None,
            published_holds: BTreeSet::new(),
            home: None,
            machine: MachineSetting::default(),
        }
    }
    cfg_if::cfg_if! {
        if #[cfg(any(test, all(gwz_transport_candidate, gwz_session_candidate, any(unix, windows))))] {
            /// A driver session. `handoff` is shared with the endpoint session its
            /// carrier reaches in this process, if any.
            pub(super) fn driver(
                io_timeout_ms: u64,
                connect_timeout_ms: u64,
                handoff: Option<Handoff>,
            ) -> ModelResult<(Arc<Self>, TransportPort)> {
                Self::driver_with_ssh_budgets(io_timeout_ms, connect_timeout_ms, handoff, 30_000, 120_000)
            }
        }
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
    cfg_if::cfg_if! {
        if #[cfg(any(test, all(gwz_transport_candidate, gwz_session_candidate, any(unix, windows))))] {
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
                Self::endpoint_with_https_native(config.into(), https, handoff, None)
            }
        }
    }
    pub(super) fn endpoint_with_https_native(
        config: EndpointSettings,
        https: Option<(HttpsEndpointConfig, HelperSlots)>,
        handoff: Handoff,
        native: Option<NativeCaller>,
    ) -> ModelResult<(Arc<Self>, TransportPort)> {
        let mut state = Self::empty();
        let id = unique()?;
        // Where the HTTPS endpoint leaves how a failed open ended beyond its failure, for the driver that shares it.
        let ends = handoff.clone();
        state.holds_to = Some(handoff.clone());
        let authority = crate::git::endpoint::shared_reservation::Authority::new(
            config.pool.total,
            config.pool.per_host,
        );
        state.authority = Some(authority.clone());
        state.installed_capacity = Some(pool::Capacity::from(&config.pool));
        if let Some(ssh_settings) = config.ssh {
            let ssh_helpers = https.as_ref().and_then(|(https, slots)| {
                https.auth.as_ref().map(|auth| {
                    Arc::new(crate::git::endpoint::ssh_password_helpers::Helpers::new(
                        auth.clone(),
                        slots.clone(),
                    ))
                })
            });
            let ssh = ssh_local::connect_with_helpers(
                config.pool.clone(),
                // The file the driver names when it words a host-key failure (`KnownHostsFile`).
                KnownHostsFile::under(&ssh_settings.home, ssh_settings.home_source).path,
                ssh_settings.agent.clone(),
                config.io_timeout_ms,
                authority.clone(),
                handoff,
                ssh_helpers,
            )
            .map_err(|_| unavailable("SSH endpoint construction failed"))?;
            state.engine = Some(
                PlacementEndpoint::new(ssh, ssh_settings.home, id.clone(), id.clone())
                    .map_err(|_| unavailable("endpoint supervisor unavailable"))?,
            );
        }
        if let Some((https, helper_slots)) = https {
            let mut endpoint = super::https_endpoint::HttpsEndpoint::new_native(
                https,
                config.pool.clone(),
                config.io_timeout_ms,
                authority.clone(),
                id.clone(),
                helper_slots,
                native,
            )?;
            endpoint.set_handoff(ends);
            state.https = Some(endpoint);
        }
        let https_enabled = state.https.is_some();
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
                ssh_and_https_endpoint_policies()
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

cfg_if::cfg_if! { if #[cfg(test)] {
    impl Session {
        pub(super) fn endpoint_offer_for_test(&self) -> (bool, binding::EndpointConfig) {
            let state = self.state.lock().unwrap();
            (state.engine.is_some(), state.endpoint_config.clone().unwrap())
        }
    }
} }

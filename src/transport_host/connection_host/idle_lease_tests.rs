//! An idle connection's loss against the host's lease of it (findings F8, 2026-10-31): the SSH adapter's worker sees an
//! idle watch fire while the host has leased that connection and queued an exchange's start, or the host leases a
//! connection the worker has just retired. The claims decide which wins (`ssh_adapter/claims.rs`), so the exchange runs
//! on the connection or the host asks again, and a loss the worker has retired before the host's lease never ends the
//! exchange `CarrierLost` before its first byte. Before the F8 follow-ups the fixture's fresh sessions hit this under load through the bytes
//! `sshd` sends after authentication; here a gate on the real fixture connection's idle watch orders the two steps.
//!
//! The F8 follow-ups: a new session is not idle-polled before its first exchange, so those bytes cannot retire it (the
//! accepted idle-loss design); only a hang-up (the peer's FIN or reset) retires it before then, and the host asks again
//! (fix round). A retirement stands until the host has read the connection's `IdleClosed`, not only until its disposal.
//! A loss the worker retires before the host's lease is refused to the host (`NotConnected`) and the opening asks again.
//! A loss seen only after the lease (the retirement is refused while the lease stands), or not seen at all, is left to
//! the exchange, which fails on its own channel (`CarrierLost` or `Discarded`, `Effect::None`) if the loss is real. The
//! adapter is wrapped (`Held`) so a row can also hold the host's side: its reports and its next start.
//! `https_idle_lease_tests.rs` uses the same gate over the HTTPS adapter.
use super::{
    ConnectionHost,
    tests::{advertisement, block_on, is_advertisement, operation, wait_for},
};
use crate::git::endpoint::ssh_adapter::{AdapterSettings, SshAdapter, Untaken};
use crate::git::endpoint::{
    agent_auth::Evidence,
    cut_proxy::CutProxy,
    setup_retry::LocalBudget,
    ssh_channel::{GitService as ChannelService, SshChannel},
    ssh_fixture::{SshConnection, SshdFixture, server_path},
    ssh_pool::{Connector, Opening, Resource},
    ssh_pump::SshPump,
    ssh_setup::{Authenticated, NativeResource, Setup, SetupConnector},
    ssh_worker::ChannelResource,
};
use gwz_connection::{
    AuthContext, Completion, ConnectCommand, EndpointAdapter, EndpointFactory, EndpointSpec,
    Exchange, ExchangeIo, ExchangeSpec, FactoryEnv, FactoryError, GitService, HostHandle,
    Operation, ProofId, ReuseCommand,
};
use gwz_transport::{
    pool::{ConnectionId, Identity, Key},
    protocol::{AuthMethod, Disposition, Facts, Failure},
    stream::{MessageEndpoint, Stream},
};
use std::{
    io,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

/// The longest the worker waits inside a held idle poll: a row that never releases it fails, it does not hang.
const HELD_AT_MOST: Duration = Duration::from_secs(30);
/// The longest a new connection's `Connected` waits for the server's bytes after authentication.
const BYTES_AT_MOST: Duration = Duration::from_secs(5);

/// How long the host's next start waits at most, and the condition of the gate that releases it sooner.
type StartHold = (Duration, fn(&GateState) -> bool);

/// What the rows let the wrapped connections' idle watches and disposals do, and the host's side of the adapter.
#[derive(Default)]
pub(super) struct Gate {
    state: Mutex<GateState>,
    changed: Condvar,
}
#[derive(Default)]
pub(super) struct GateState {
    /// The next idle poll reports the connection lost (once), as a peer's unasked byte would.
    lose: bool,
    /// That poll first waits, inside the worker's pass, until the row releases it.
    hold_poll: bool,
    /// A held poll is waiting.
    held: bool,
    /// Disposals stay pending while set.
    hold_dispose: bool,
    /// Disposal polls seen while held.
    disposals_held: usize,
    /// Idle polls of a connection no exchange had started on yet.
    fresh_polls: usize,
    /// Hang-up polls of a connection no exchange had started on yet, and how many of them saw a hang-up.
    hangup_polls: usize,
    hung_up: usize,
    /// A new connection's `Connected` waits until its socket is readable: the bytes the server sends after
    /// authentication (or `BYTES_AT_MOST`).
    await_bytes: bool,
    /// Per new connection that waited: whether those bytes came.
    bytes_seen: Vec<bool>,
    /// The host reads no report while set (`Held::poll_completion`); the host's next start clears it once the adapter
    /// has answered that start.
    pub(super) hold_reports: bool,
    reports_waker: Option<Waker>,
    /// The host's next start waits, on its driver thread, until the condition holds of the gate, or this long.
    hold_start: Option<StartHold>,
    /// How long a held start waited, and whether its condition released it (else its bound did).
    start_held: Option<(Duration, bool)>,
    /// How each start the host made through `Held` went: `None` started, else the error's kind.
    pub(super) starts: Vec<Option<io::ErrorKind>>,
    /// The adapters' outboxes, as the host has not yet read them.
    untaken: Vec<Untaken>,
}
impl Gate {
    fn lock(&self) -> std::sync::MutexGuard<'_, GateState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub(super) fn set(&self, change: impl FnOnce(&mut GateState)) {
        change(&mut self.lock());
        self.changed.notify_all();
    }
    pub(super) fn read<T>(&self, read: impl FnOnce(&GateState) -> T) -> T {
        read(&self.lock())
    }
    /// Records the outbox of an adapter the factory made.
    pub(super) fn adapter_made(&self, untaken: Untaken) {
        self.lock().untaken.push(untaken);
    }
    /// The `IdleClosed` reports the host has not yet read, over every adapter.
    pub(super) fn idle_closed_untaken(&self) -> usize {
        let untaken = self.lock().untaken.clone();
        untaken.iter().map(Untaken::idle_closed).sum()
    }
    /// The reuse verdicts `Authorized` the host has not yet read, over every adapter.
    pub(super) fn authorized_untaken(&self) -> usize {
        let untaken = self.lock().untaken.clone();
        untaken.iter().map(Untaken::authorized).sum()
    }
    /// Whether every adapter's claims record nothing.
    pub(super) fn claims_empty(&self) -> bool {
        let untaken = self.lock().untaken.clone();
        untaken.iter().all(Untaken::claims_empty)
    }
    /// The host reads the reports again, everything that waits in the outbox in one pass.
    pub(super) fn release_reports(&self) {
        let waker = {
            let mut state = self.lock();
            state.hold_reports = false;
            state.reports_waker.take()
        };
        self.changed.notify_all();
        if let Some(waker) = waker {
            waker.wake();
        }
    }
    /// Waits until `done` holds of the gate.
    pub(super) fn wait(&self, what: &str, done: impl Fn(&GateState) -> bool) {
        let deadline = Instant::now() + HELD_AT_MOST;
        let mut state = self.lock();
        while !done(&state) {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "{what}");
            state = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
    /// The worker's idle poll: Ready once when the row asked for a loss, after the row's release if it holds it.
    fn poll_lost(&self) -> bool {
        let mut state = self.lock();
        if !state.lose {
            return false;
        }
        if state.hold_poll {
            state.held = true;
            self.changed.notify_all();
            let deadline = Instant::now() + HELD_AT_MOST;
            while state.hold_poll && Instant::now() < deadline {
                state = self
                    .changed
                    .wait_timeout(state, Duration::from_millis(50))
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
            state.held = false;
        }
        state.lose = false;
        true
    }
    fn dispose_held(&self) -> bool {
        let mut state = self.lock();
        if state.hold_dispose {
            state.disposals_held += 1;
            self.changed.notify_all();
        }
        state.hold_dispose
    }
    fn fresh_poll(&self) {
        self.lock().fresh_polls += 1;
        self.changed.notify_all();
    }
    fn hangup_poll(&self, hung_up: bool) {
        let mut state = self.lock();
        state.hangup_polls += 1;
        state.hung_up += usize::from(hung_up);
        self.changed.notify_all();
    }
    /// The host's start, before the adapter sees it: held, it waits until the row's condition holds. The condition may
    /// read the adapters' claims, which do not notify the gate, so the wait looks again every 10 ms.
    fn before_start(&self) {
        let Some((at_most, until)) = self.lock().hold_start.take() else {
            return;
        };
        let since = Instant::now();
        let deadline = since + at_most;
        let mut state = self.lock();
        while !until(&state) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            state = self
                .changed
                .wait_timeout(state, left.min(Duration::from_millis(10)))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        state.start_held = Some((since.elapsed(), until(&state)));
    }
    /// The adapter answered the host's start: it is recorded, and held reports go to the host from here.
    fn after_start(&self, outcome: Option<io::ErrorKind>) {
        let waker = {
            let mut state = self.lock();
            state.starts.push(outcome);
            state.hold_reports = false;
            state.reports_waker.take()
        };
        self.changed.notify_all();
        if let Some(waker) = waker {
            waker.wake();
        }
    }
    /// Whether the host's read of the reports is held now (its waker is kept for the release).
    fn reports_held(&self, cx: &Context<'_>) -> bool {
        let mut state = self.lock();
        if state.hold_reports {
            state.reports_waker = Some(cx.waker().clone());
        }
        state.hold_reports
    }
}

/// The adapter the host is given: `inner`, with the gate's holds on the host's side of it.
pub(super) struct Held<A> {
    pub(super) inner: A,
    pub(super) gate: Arc<Gate>,
}
impl<A: EndpointAdapter> EndpointAdapter for Held<A> {
    fn connect(&mut self, command: ConnectCommand) {
        self.inner.connect(command);
    }
    fn cancel_connect(&mut self, connection: ConnectionId) {
        self.inner.cancel_connect(connection);
    }
    fn close(&mut self, connection: ConnectionId) {
        self.inner.close(connection);
    }
    fn start_exchange(
        &mut self,
        connection: ConnectionId,
        spec: &ExchangeSpec,
        auth: &AuthContext,
    ) -> io::Result<Box<dyn ExchangeIo>> {
        self.gate.before_start();
        let started = self.inner.start_exchange(connection, spec, auth);
        self.gate
            .after_start(started.as_ref().err().map(io::Error::kind));
        started
    }
    fn authorize_reuse(&mut self, command: ReuseCommand) -> bool {
        self.inner.authorize_reuse(command)
    }
    fn cancel_reuse(&mut self, proof: ProofId) {
        self.inner.cancel_reuse(proof);
    }
    fn poll_completion(&mut self, cx: &mut Context<'_>) -> Poll<Completion> {
        if self.gate.reports_held(cx) {
            return Poll::Pending;
        }
        self.inner.poll_completion(cx)
    }
    fn poll_dispose(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        self.inner.poll_dispose(cx)
    }
}

/// The fixture's real connection, whose idle watch and disposal the gate can override.
struct Gated {
    inner: NativeResource,
    gate: Arc<Gate>,
    /// An exchange has started on it.
    started: bool,
    /// Connected, its `Connected` waiting for the server's bytes since then (`GateState::await_bytes`).
    connected: Option<(Option<Identity>, Instant)>,
}
impl Resource for Gated {
    fn poll_connected(&mut self, cx: &mut Context<'_>) -> Poll<Result<Option<Identity>, Failure>> {
        if self.connected.is_none() {
            match self.inner.poll_connected(cx) {
                Poll::Ready(Ok(identity)) if self.gate.read(|state| state.await_bytes) => {
                    self.connected = Some((identity, Instant::now()));
                }
                other => return other,
            }
        }
        let since = self
            .connected
            .as_ref()
            .map_or_else(Instant::now, |(_, since)| *since);
        // Readable: bytes (or an end) the server sent after authentication. The real watch registers the worker's
        // waker, so their arrival brings the next poll.
        let readable = self.inner.poll_idle_lost(cx).is_ready();
        if !readable && since.elapsed() < BYTES_AT_MOST {
            return Poll::Pending;
        }
        self.gate.set(|state| state.bytes_seen.push(readable));
        let (identity, _) = self.connected.take().expect("connected");
        Poll::Ready(Ok(identity))
    }
    fn poll_dispose(&mut self, cx: &mut Context<'_>, force: bool) -> Poll<io::Result<()>> {
        if self.gate.dispose_held() {
            // The worker passes again within its park interval: no waker is kept.
            return Poll::Pending;
        }
        self.inner.poll_dispose(cx, force)
    }
    fn reusable(&self) -> bool {
        self.inner.reusable()
    }
    fn reuse_evidence(&self) -> Option<Arc<Evidence>> {
        self.inner.reuse_evidence()
    }
    fn poll_idle_lost(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        if !self.started {
            self.gate.fresh_poll();
        }
        if self.gate.poll_lost() {
            return Poll::Ready(());
        }
        self.inner.poll_idle_lost(cx)
    }
    fn poll_hung_up(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        let hung_up = self.inner.poll_hung_up(cx);
        if !self.started {
            self.gate.hangup_poll(hung_up.is_ready());
        }
        hung_up
    }
    fn waiting_locally(&self) -> bool {
        self.inner.waiting_locally()
    }
    fn local_budget(&self) -> Option<LocalBudget> {
        self.inner.local_budget()
    }
    fn tcp_connect_ms(&self) -> Option<u64> {
        self.inner.tcp_connect_ms()
    }
}
impl ChannelResource for Gated {
    fn observation(&self) -> (bool, Facts) {
        self.inner.observation()
    }
    fn start_exchange(
        &mut self,
        stream: Stream,
        endpoint: MessageEndpoint,
        service: ChannelService,
        path: &str,
    ) -> io::Result<()> {
        self.started = true;
        self.inner.start_exchange(stream, endpoint, service, path)
    }
    fn pump(&mut self) -> Option<&mut SshPump<SshChannel>> {
        self.inner.pump()
    }
    fn reclaim(&mut self) -> bool {
        self.inner.reclaim()
    }
}
struct GatedConnector {
    inner: SetupConnector,
    gate: Arc<Gate>,
}
impl Connector for GatedConnector {
    type Resource = Gated;
    fn set_stall_ms(&mut self, stall_ms: u64) {
        self.inner.set_stall_ms(stall_ms);
    }
    fn setup_clock_source(&self) -> Option<(Instant, Arc<dyn Fn() -> u64 + Send + Sync>)> {
        self.inner.setup_clock_source()
    }
    fn now_ms(&self) -> Option<u64> {
        self.inner.now_ms()
    }
    fn start(
        &mut self,
        key: &Key,
        identity: &Identity,
        deadline: Option<u64>,
    ) -> Result<Gated, Failure> {
        self.start_reported(key, identity, deadline, Opening::default())
    }
    fn start_reported(
        &mut self,
        key: &Key,
        identity: &Identity,
        deadline: Option<u64>,
        opening: Opening,
    ) -> Result<Gated, Failure> {
        let inner = self
            .inner
            .start_reported(key, identity, deadline, opening)?;
        Ok(Gated {
            inner,
            gate: self.gate.clone(),
            started: false,
            connected: None,
        })
    }
}

/// The fixture's dial. The factory holds only this, so the fixture's server stops with the row, whatever the host holds.
type Dial = Arc<dyn Fn() -> io::Result<SshConnection> + Send + Sync>;
/// SSH adapters over the fixture, each setup one counted dial.
struct GatedFactory {
    dial: Dial,
    gate: Arc<Gate>,
    dials: Arc<AtomicUsize>,
}
impl EndpointFactory for GatedFactory {
    fn create(
        &self,
        _spec: &EndpointSpec,
        _env: &FactoryEnv<'_>,
    ) -> Result<Box<dyn EndpointAdapter>, FactoryError> {
        let (dial, gate, dials) = (self.dial.clone(), self.gate.clone(), self.dials.clone());
        let adapter = SshAdapter::new(
            move |origin| {
                let inner =
                    SetupConnector::isolated(origin, Duration::from_millis(500), move |_, _, _| {
                        let (dial, dials) = (dial.clone(), dials.clone());
                        let setup: Setup = Box::new(move |_| {
                            dials.fetch_add(1, Ordering::SeqCst);
                            Authenticated::new(
                                dial()?,
                                Identity::Ambient,
                                Facts {
                                    method: AuthMethod::SshKey,
                                    authenticated: Some(true),
                                    credential_offered: true,
                                    ..Facts::default()
                                },
                            )
                        });
                        Ok(setup)
                    });
                GatedConnector { inner, gate }
            },
            AdapterSettings {
                connect_timeout_ms: 20_000,
                io_timeout_ms: 20_000,
                interaction_timeout_ms: 20_000,
                cleanup_timeout_ms: 2_000,
            },
        );
        self.gate.adapter_made(adapter.untaken());
        Ok(Box::new(Held {
            inner: adapter,
            gate: self.gate.clone(),
        }))
    }
}

struct Rig {
    fixture: SshdFixture,
    /// The proxy the sessions go through, when the row cuts them.
    proxy: Option<CutProxy>,
    gate: Arc<Gate>,
    dials: Arc<AtomicUsize>,
    host: ConnectionHost,
    handle: HostHandle,
}
impl Rig {
    fn new() -> Self {
        Self::build(false)
    }
    /// Sessions go through a proxy the row can cut.
    fn proxied() -> Self {
        Self::build(true)
    }
    fn build(proxied: bool) -> Self {
        let fixture = SshdFixture::new();
        let proxy = proxied.then(|| CutProxy::start(fixture.port));
        let port = proxy.as_ref().map_or(fixture.port, |proxy| proxy.port);
        let gate = Arc::new(Gate::default());
        let dials = Arc::new(AtomicUsize::new(0));
        let host = ConnectionHost::build(
            Arc::new(GatedFactory {
                dial: Arc::new(fixture.dialer(port)),
                gate: gate.clone(),
                dials: dials.clone(),
            }),
            None,
            Arc::new(crate::transport_host::composition::process_safety()),
        )
        .unwrap();
        let handle = host.handle();
        Self {
            fixture,
            proxy,
            gate,
            dials,
            host,
            handle,
        }
    }
    fn exchange(&self, operation: &Operation) -> Exchange {
        let spec = ExchangeSpec::git(
            Key::ssh(&self.fixture.user, "127.0.0.1", self.fixture.port),
            Identity::Ambient,
            GitService::UploadPackExchange,
            &server_path(&self.fixture.url_repository),
        )
        .unwrap();
        block_on(operation.open(spec).unwrap().wait()).unwrap()
    }
    /// One exchange of `operation`, run to its clean end.
    fn serve(&self, mut exchange: Exchange) {
        let advertised = advertisement(&mut exchange).unwrap();
        assert!(is_advertisement(&advertised), "{advertised:?}");
        assert_eq!(
            block_on(exchange.finish()).disposition,
            Disposition::Reusable
        );
    }
    fn wait(&self, what: &str, done: impl Fn(gwz_transport::pool::Counts) -> bool) {
        wait_for(what, || done(self.handle.snapshot().connections));
    }
    fn close(mut self) -> bool {
        block_on(self.host.close())
    }
}

#[test]
fn an_idle_loss_seen_after_the_host_leased_the_connection_leaves_it_to_the_exchange() {
    let rig = Rig::new();
    let operation = operation(&rig.handle, "op-idle-lease");
    rig.serve(rig.exchange(&operation));
    rig.wait("the connection is idle", |counts| {
        (counts.idle, counts.leased) == (1, 0)
    });
    // Counted from here: a fresh session the fixture's server writes to after authentication can itself be seen lost
    // before its first lease and replaced, under load (findings F8), which is not what this row is about.
    let dialed = rig.dials.load(Ordering::SeqCst);
    // The worker's next pass over the idle connection sees it lost, and waits there for the row.
    rig.gate.set(|state| {
        state.lose = true;
        state.hold_poll = true;
    });
    rig.gate
        .wait("the worker polls the idle connection", |state| state.held);
    // Meanwhile the host leases it and queues the exchange's start.
    let exchange = rig.exchange(&operation);
    rig.gate.set(|state| state.hold_poll = false);
    // The lease stands: the exchange starts on that connection and runs to its clean end.
    rig.serve(exchange);
    assert_eq!(
        rig.dials.load(Ordering::SeqCst),
        dialed,
        "it ran on the leased connection"
    );
    block_on(operation.finish());
    rig.wait("the connection is idle again", |counts| {
        (counts.idle, counts.leased, counts.total()) == (1, 0, 1)
    });
    assert!(rig.close());
}

#[test]
fn a_connection_retired_on_an_idle_loss_is_refused_to_the_host_which_asks_again() {
    let rig = Rig::new();
    let operation = operation(&rig.handle, "op-idle-retired");
    rig.serve(rig.exchange(&operation));
    rig.wait("the connection is idle", |counts| {
        (counts.idle, counts.leased) == (1, 0)
    });
    // Counted from here, as in the row above.
    let dialed = rig.dials.load(Ordering::SeqCst);
    // The worker sees the idle connection lost and retires it; its disposal is held, so the host has not heard of it.
    rig.gate.set(|state| {
        state.lose = true;
        state.hold_dispose = true;
    });
    rig.gate
        .wait("the worker disposes of the lost connection", |state| {
            state.disposals_held > 0
        });
    assert_eq!(rig.handle.snapshot().connections.idle, 1);
    // The host leases what it still counts idle; the worker's retirement refuses it, and the opening asks again.
    rig.serve(rig.exchange(&operation));
    assert!(
        rig.dials.load(Ordering::SeqCst) > dialed,
        "a fresh connection served it"
    );
    rig.gate.set(|state| state.hold_dispose = false);
    block_on(operation.finish());
    rig.wait("the retired connection is gone", |counts| {
        (counts.idle, counts.leased, counts.total()) == (1, 0, 1)
    });
    assert!(rig.close());
}

/// One new session through the gate of the row below, served twice: whether the server's bytes were on its socket
/// before its first exchange. Every other claim of the row holds either way.
fn a_new_session_serves_twice() -> bool {
    let rig = Rig::new();
    rig.gate.set(|state| {
        state.await_bytes = true;
        // Released early only by an idle poll (the defect), or once the hang-up watch has passed over the session and
        // its bytes three times.
        state.hold_start = Some((Duration::from_millis(400), |state| {
            state.fresh_polls > 0 || state.hangup_polls >= 3
        }));
    });
    let operation = operation(&rig.handle, "op-fresh-bytes");
    let exchange = rig.exchange(&operation);
    let bytes = rig.gate.read(|state| {
        assert_eq!(
            state.fresh_polls, 0,
            "no idle poll of a new session before its first exchange"
        );
        assert_eq!(state.starts, [None], "the host's first lease started");
        assert_eq!(state.bytes_seen.len(), 1, "one new session");
        assert!(state.hangup_polls >= 1, "the hang-up watch looked at it");
        assert_eq!(state.hung_up, 0, "the server's bytes are not a hang-up");
        state.bytes_seen[0]
    });
    // The bytes are the channel's to read: the exchange runs to its clean end on that session.
    rig.serve(exchange);
    assert_eq!(
        rig.dials.load(Ordering::SeqCst),
        1,
        "one session, never redialled"
    );
    // Watched from here, it stays idle: the first exchange read what the server sent, and the next one reuses it.
    rig.wait("the session is idle", |counts| {
        (counts.idle, counts.leased) == (1, 0)
    });
    rig.serve(rig.exchange(&operation));
    assert_eq!(
        rig.dials.load(Ordering::SeqCst),
        1,
        "the idle session served again"
    );
    block_on(operation.finish());
    rig.wait("one idle session", |counts| {
        (counts.idle, counts.leased, counts.total()) == (1, 0, 1)
    });
    assert!(rig.close());
    bytes
}

#[test]
fn a_new_session_the_server_writes_to_before_its_first_exchange_is_not_retired_and_serves_it() {
    // F8 follow-up 1. The fixture's sshd writes to a new session after authentication (OpenSSH's host-key notice). The
    // gate holds the session's `Connected` until those bytes are on its socket, and holds the host's first start until
    // the worker polls the new session's idle watch, or, as the accepted design has it, long enough for the worker to
    // pass over the idle session several times without polling it (its park is 50 ms); only its hang-up watch looks,
    // and the bytes are not a hang-up (fix round). Now and then libssh2 reads
    // those bytes into its own buffer during authentication (1 session in 60 under load on WSL), and the socket has
    // nothing to show: that session proves the rest, and the row takes another, up to three.
    let with_bytes = (0..3).find(|_| a_new_session_serves_twice());
    assert!(
        with_bytes.is_some(),
        "the server wrote to none of three new sessions before its first exchange"
    );
}

/// F8 follow-up 1, fix round (STATE-P2-1): a new session the server hangs up on (`reset`: resets) after `Connected`
/// and before the host's first start. The idle watch is not polled, but the hang-up watch sees the end behind the
/// server's unread bytes and retires the session: the host's start is refused, the opening asks again, and a fresh
/// session serves it. Before the fix round nothing watched the session, and the exchange ended `CarrierLost` before
/// its first byte. Returns whether the server's bytes were on the socket in front of the end; every other claim holds
/// either way (with nothing unread the peek alone sees the end).
fn a_new_session_hung_up_on_before_its_first_exchange_is_refused_and_the_host_asks_again(
    reset: bool,
) -> bool {
    let rig = Rig::proxied();
    rig.gate.set(|state| {
        state.await_bytes = true;
        // Released once the worker has retired the session: a start released when the hang-up watch fires, before
        // the worker's retirement in the same step, could lease it first, which the claims rightly let stand.
        state.hold_start = Some((Duration::from_secs(5), |state| {
            state.untaken.iter().map(Untaken::retired).sum::<usize>() > 0
        }));
    });
    let operation = operation(&rig.handle, "op-fresh-hung-up");
    let spec = ExchangeSpec::git(
        Key::ssh(&rig.fixture.user, "127.0.0.1", rig.fixture.port),
        Identity::Ambient,
        GitService::UploadPackExchange,
        &server_path(&rig.fixture.url_repository),
    )
    .unwrap();
    let opening = operation.open(spec).unwrap().wait();
    // `Connected` is reported once the server's bytes are on the socket; the host's start is held. The session ends.
    rig.gate.wait("the new session connected", |state| {
        state.bytes_seen.len() == 1
    });
    let bytes = rig.gate.read(|state| state.bytes_seen[0]);
    let proxy = rig.proxy.as_ref().expect("proxied");
    if reset {
        proxy.reset_all();
    } else {
        proxy.cut_all();
    }
    // The opening is served, and its exchange runs to its clean end (not `CarrierLost` before its first byte).
    let mut exchange = block_on(opening).unwrap();
    let advertised = advertisement(&mut exchange);
    rig.gate.read(|state| {
        assert!(
            advertised.as_ref().is_ok_and(|bytes| is_advertisement(bytes)),
            "the exchange was served: {advertised:?}; start held {:?}, starts {:?}, hang-up polls {} (seen {}), idle \
             polls {}, bytes seen {:?}, dials {}",
            state.start_held,
            state.starts,
            state.hangup_polls,
            state.hung_up,
            state.fresh_polls,
            state.bytes_seen,
            rig.dials.load(Ordering::SeqCst)
        );
    });
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Reusable
    );
    rig.gate.read(|state| {
        assert_eq!(
            state.fresh_polls, 0,
            "no idle poll before the first exchange"
        );
        assert!(state.hung_up > 0, "the hang-up watch saw the end");
        assert_eq!(
            state.starts,
            [Some(io::ErrorKind::NotConnected), None],
            "the retired session was refused, and a fresh one started"
        );
    });
    assert_eq!(
        rig.dials.load(Ordering::SeqCst),
        2,
        "a fresh session served the opening"
    );
    block_on(operation.finish());
    rig.wait("one idle session", |counts| {
        (counts.idle, counts.leased, counts.total()) == (1, 0, 1)
    });
    assert!(rig.close());
    bytes
}

/// The rows require the end behind unread bytes, the case the hang-up watch reads readiness for. Now and then libssh2
/// reads the server's bytes during authentication and the socket has nothing to show: that session proves the rest,
/// and the row takes another, up to three (as the row above does).
fn hung_up_on_behind_the_servers_bytes(reset: bool) {
    let with_bytes = (0..3).find(|_| {
        a_new_session_hung_up_on_before_its_first_exchange_is_refused_and_the_host_asks_again(reset)
    });
    assert!(
        with_bytes.is_some(),
        "the server wrote to none of three new sessions before it hung up"
    );
}

#[test]
fn a_new_session_the_server_closes_before_its_first_exchange_is_refused_and_the_host_asks_again() {
    hung_up_on_behind_the_servers_bytes(false);
}

#[test]
fn a_new_session_the_server_resets_before_its_first_exchange_is_refused_and_the_host_asks_again() {
    hung_up_on_behind_the_servers_bytes(true);
}

#[test]
fn a_connection_retired_on_an_idle_loss_is_refused_until_the_host_reads_its_report() {
    // F8 follow-up 2. The worker has retired the idle connection and finished its disposal; its `IdleClosed` waits in
    // the outbox, which the gate keeps from the host, so the host still counts the connection idle and leases it. The
    // retirement still stands: the lease is refused, and the opening asks again.
    let rig = Rig::new();
    let operation = operation(&rig.handle, "op-idle-unread");
    rig.serve(rig.exchange(&operation));
    rig.wait("the connection is idle", |counts| {
        (counts.idle, counts.leased) == (1, 0)
    });
    let dialed = rig.dials.load(Ordering::SeqCst);
    rig.gate.set(|state| {
        state.hold_reports = true;
        state.lose = true;
    });
    wait_for("the worker disposed of it and reported it", || {
        rig.gate.idle_closed_untaken() == 1
    });
    assert_eq!(
        rig.handle.snapshot().connections.idle,
        1,
        "the host has not read it"
    );
    // The start that releases the reports to the host (`Held`) is the one the retirement refuses.
    rig.serve(rig.exchange(&operation));
    rig.gate.read(|state| {
        assert_eq!(
            state.starts,
            [None, Some(io::ErrorKind::NotConnected), None],
            "the retired connection was refused, and a new one started"
        );
    });
    assert_eq!(
        rig.dials.load(Ordering::SeqCst),
        dialed + 1,
        "a fresh connection served it"
    );
    block_on(operation.finish());
    rig.wait("the retired connection is gone", |counts| {
        (counts.idle, counts.leased, counts.total()) == (1, 0, 1)
    });
    assert!(rig.close());
}

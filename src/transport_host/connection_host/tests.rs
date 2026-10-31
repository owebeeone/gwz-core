//! The connection host over the SSH adapter, against the fixture `sshd` (plan step 1.3b; contract sections 4.1, 6, 8
//! and 9). Each host runs on the composition's own ports (`DriverThread`), so nothing here polls on the host's behalf:
//! the rows wait on the host's snapshot or on the opening's readiness. The fixture's server sits behind a proxy that
//! counts its connections and can cut them from the server's side.
use super::ConnectionHost;
use crate::git::endpoint::{
    agent_auth::{Authenticator, Evidence, ReuseAuthority},
    agent_job::Supervisor,
    agent_source::AgentSource,
    cut_proxy::CutProxy,
    ssh_adapter::{AdapterSettings, SshAdapter},
    ssh_fixture::{DialRetries, SshdFixture, server_path},
    ssh_key_snapshot::Registry,
    ssh_setup::{Authenticated, Setup, SetupConnector},
    ssh_tests::reuse_agent::{Agent, ROW_KEY, keygen_kind},
};
use crate::transport_host::endpoint_environment::EndpointInputs;
use gwz_connection::{
    AuthContext, AuthScope, CleanupStatus, EndpointAdapter, EndpointFactory, EndpointProfile,
    EndpointRoute, EndpointSpec, Exchange, ExchangeSpec, FactoryEnv, FactoryError, GitService,
    HostHandle, HostPhase, OpenFailure, Operation, OperationPolicy, PreparedOperation, StreamError,
    TerminalReason,
};
use gwz_transport::{
    pool::{Identity, Key},
    protocol::{AuthMethod, Disposition, Facts},
};
use std::{
    future::Future,
    io,
    pin::pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
    thread::{self, Thread},
    time::{Duration, Instant},
};

const WITHIN: Duration = Duration::from_secs(30);

struct Unpark(Thread);
impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}
/// Waits for `future` on this thread, which the future's waker unparks.
pub(super) fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(Unpark(thread::current())));
    let mut future = pin!(future);
    let deadline = Instant::now() + WITHIN;
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut Context::from_waker(&waker)) {
            return value;
        }
        assert!(Instant::now() < deadline, "the future did not end in time");
        thread::park_timeout(Duration::from_millis(50));
    }
}
pub(super) fn wait_for(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + WITHIN;
    while !done() {
        assert!(Instant::now() < deadline, "{what}");
        thread::sleep(Duration::from_millis(5));
    }
}

fn settings() -> AdapterSettings {
    AdapterSettings {
        connect_timeout_ms: 20_000,
        io_timeout_ms: 20_000,
        interaction_timeout_ms: 20_000,
        cleanup_timeout_ms: 2_000,
    }
}

/// Makes SSH adapters whose setups dial the fixture through `dial`, which may wait for a gate.
struct FixtureFactory {
    dial:
        Arc<dyn Fn() -> io::Result<crate::git::endpoint::ssh_fixture::SshConnection> + Send + Sync>,
    /// While set, a setup waits (cancellably) before it dials.
    hold: Arc<AtomicBool>,
    /// What each setup keeps for another binding's reuse (plan step 1.6): none, as a password's connection keeps none.
    evidence: Option<Arc<dyn Fn() -> Evidence + Send + Sync>>,
}
impl EndpointFactory for FixtureFactory {
    fn create(
        &self,
        _spec: &EndpointSpec,
        _env: &FactoryEnv<'_>,
    ) -> Result<Box<dyn EndpointAdapter>, FactoryError> {
        let (dial, hold) = (self.dial.clone(), self.hold.clone());
        let evidence = self.evidence.clone();
        Ok(Box::new(SshAdapter::new(
            move |origin| {
                SetupConnector::isolated(origin, Duration::from_millis(500), move |_, _, _| {
                    let (dial, hold, evidence) = (dial.clone(), hold.clone(), evidence.clone());
                    let setup: Setup = Box::new(move |control| {
                        while hold.load(Ordering::SeqCst) {
                            control.check()?;
                            thread::sleep(Duration::from_millis(5));
                        }
                        let authenticated = Authenticated::new(
                            dial()?,
                            Identity::Ambient,
                            Facts {
                                method: AuthMethod::SshKey,
                                authenticated: Some(true),
                                credential_offered: true,
                                ..Facts::default()
                            },
                        )?;
                        Ok(match &evidence {
                            Some(evidence) => authenticated.proven_by(evidence()),
                            None => authenticated,
                        })
                    });
                    Ok(setup)
                })
            },
            settings(),
        )))
    }
}

/// What a case's connections keep for another binding's reuse proof (plan step 1.6).
#[derive(Clone)]
struct Kept {
    /// `known_hosts` lists the evidence's host key.
    trusted: bool,
    /// The public blob of the agent key the connections' evidence names, and the agent holding it. None: an explicit
    /// key's evidence, which needs no possession proof.
    agent: Option<(Vec<u8>, AgentSource)>,
    /// The host as the pool keys it (lowercased) and as the URL wrote it. Some: the connections are keyed by the first,
    /// the evidence records the second, and `known_hosts` lists the fixture's own host key under the second, hashed.
    named: Option<(String, String)>,
}
impl Kept {
    fn trusted() -> Self {
        Self {
            trusted: true,
            agent: None,
            named: None,
        }
    }
}
struct Case {
    /// The host the spec's key names.
    spec_host: String,
    fixture: SshdFixture,
    proxy: Arc<CutProxy>,
    retries: DialRetries,
    hold: Arc<AtomicBool>,
    host: Mutex<ConnectionHost>,
    handle: HostHandle,
    /// The `known_hosts` file the connections' reuse proofs read, when they keep evidence.
    _known_hosts: Option<tempfile::TempDir>,
}
/// The host key the fixture connections' evidence names (a stand-in blob: only its equality is checked).
const EVIDENCE_HOST_KEY: &[u8] = b"fixture-host-key-blob";
impl Case {
    fn new() -> Self {
        Self::build(None)
    }
    /// A case whose connections keep the evidence `kept` describes.
    fn keeping(kept: Kept) -> Self {
        Self::build(Some(kept))
    }
    /// A case whose connections keep reuse evidence (an explicit key's), against a `known_hosts` file that lists the
    /// evidence's host key when `trusted` and lists nothing otherwise.
    fn with_evidence(trusted: bool) -> Self {
        Self::build(Some(Kept {
            trusted,
            ..Kept::trusted()
        }))
    }
    fn build(kept: Option<Kept>) -> Self {
        use base64::Engine as _;
        let fixture = SshdFixture::new();
        let dir = kept.as_ref().map(|_| tempfile::tempdir().unwrap());
        let named = kept.as_ref().and_then(|kept| kept.named.clone());
        let pooled_host = named
            .as_ref()
            .map_or_else(|| "127.0.0.1".to_owned(), |(pooled, _)| pooled.clone());
        let evidence_for = kept.map(|kept| {
            let dir = dir.as_ref().unwrap();
            let known_hosts = dir.path().join("known_hosts");
            let (host_key, written, line) = match &kept.named {
                // The fixture's own host key, listed hashed under the host as written.
                Some((_, written)) => {
                    let listed = fixture.known_host(written, fixture.port, true);
                    let blob = listed.split_whitespace().nth(2).unwrap();
                    let blob = base64::engine::general_purpose::STANDARD
                        .decode(blob)
                        .unwrap();
                    (blob, written.clone(), format!("{}\n", listed.trim_end()))
                }
                None => {
                    let line = format!(
                        "[127.0.0.1]:{} ssh-ed25519 {}\n",
                        fixture.port,
                        base64::engine::general_purpose::STANDARD.encode(EVIDENCE_HOST_KEY)
                    );
                    (EVIDENCE_HOST_KEY.to_vec(), pooled_host.clone(), line)
                }
            };
            std::fs::write(
                &known_hosts,
                if kept.trusted { line } else { String::new() },
            )
            .unwrap();
            let (authenticator, source) = match kept.agent {
                Some((blob, source)) => (Authenticator::Agent(blob), source),
                None => (
                    Authenticator::KeyFile("fixture-key".into()),
                    AgentSource::None,
                ),
            };
            let authority = Arc::new(ReuseAuthority::new(
                source,
                known_hosts,
                Supervisor::new(),
                Instant::now(),
                Duration::from_millis(500),
            ));
            Arc::new(move || Evidence {
                authenticator: authenticator.clone(),
                host_key: host_key.clone(),
                written_host: written.clone(),
                authority: authority.clone(),
            }) as Arc<dyn Fn() -> Evidence + Send + Sync>
        });
        let proxy = Arc::new(CutProxy::start(fixture.port));
        let retries = DialRetries::default();
        let hold = Arc::new(AtomicBool::new(false));
        let factory = FixtureFactory {
            dial: Arc::new(fixture.dialer_counting(proxy.port, retries.clone())),
            hold: hold.clone(),
            evidence: evidence_for,
        };
        let host = ConnectionHost::build(
            Arc::new(factory),
            None,
            Arc::new(crate::transport_host::composition::process_safety()),
        )
        .unwrap();
        let handle = host.handle();
        Self {
            spec_host: pooled_host,
            fixture,
            proxy,
            retries,
            hold,
            host: Mutex::new(host),
            handle,
            _known_hosts: dir,
        }
    }
    /// Sessions the proxy carried (less the connections a dial abandoned and replaced).
    fn sessions(&self) -> usize {
        self.retries.sessions_in(self.proxy.connections())
    }
    fn operation(&self, name: &str) -> Operation {
        operation(&self.handle, name)
    }
    fn spec(&self) -> ExchangeSpec {
        ExchangeSpec::git(
            Key::ssh(&self.fixture.user, &self.spec_host, self.fixture.port),
            Identity::Ambient,
            GitService::UploadPackExchange,
            // The URL-path form (a junction on Windows: the repository's own name holds a single quote, which the
            // Windows server's POSIX shell does not convert to the Windows form Git needs; `fixture_host`).
            &server_path(&self.fixture.url_repository),
        )
        .unwrap()
    }
    fn exchange(&self, operation: &Operation) -> Exchange {
        block_on(operation.open(self.spec()).unwrap().wait()).unwrap()
    }
    fn wait(&self, what: &str, done: impl Fn(gwz_transport::pool::Counts) -> bool) {
        wait_for(what, || done(self.handle.snapshot().connections));
    }
    fn close(&self) -> bool {
        block_on(self.host.lock().unwrap().close())
    }
}

pub(super) fn operation(handle: &HostHandle, name: &str) -> Operation {
    operation_with(handle, name, Arc::new(()))
}
/// An operation whose authentication context holds `provider`: the adapter's retention of the context is its count.
pub(super) fn operation_with(
    handle: &HostHandle,
    name: &str,
    provider: Arc<dyn std::any::Any + Send + Sync>,
) -> Operation {
    let input = PreparedOperation::new(
        name,
        EndpointRoute::ssh_and_https(),
        EndpointProfile::new(b"fixture-sshd".to_vec(), [7; 32]).unwrap(),
        AuthContext::new(64, AuthScope::new([0; 32]), provider),
        OperationPolicy::new(4, 32, 3).unwrap(),
    )
    .unwrap();
    block_on(handle.admit(input).unwrap().acquire())
        .unwrap()
        .register()
}

/// An operation whose profile carries `inputs`, as a capture makes its profile (`endpoint_environment::capture_operation`):
/// what the production factory makes the instance's adapter from. The canonical bytes are a fixed fixture's, so a row
/// that makes two profiles gives them a `tag` apiece.
pub(super) fn operation_over(
    handle: &HostHandle,
    name: &str,
    tag: &str,
    inputs: EndpointInputs,
) -> Operation {
    operation_over_auth(
        handle,
        name,
        tag,
        inputs,
        AuthContext::new(64, AuthScope::new([0; 32]), Arc::new(())),
    )
}
/// [`operation_over`], with the context `auth`.
pub(super) fn operation_over_auth(
    handle: &HostHandle,
    name: &str,
    tag: &str,
    inputs: EndpointInputs,
    auth: AuthContext,
) -> Operation {
    use sha2::{Digest, Sha256};
    let canonical = format!("fixture-inputs/{tag}").into_bytes();
    let digest = Sha256::digest(&canonical).into();
    let profile = EndpointProfile::with_inputs(canonical, digest, Arc::new(inputs)).unwrap();
    let input = PreparedOperation::new(
        name,
        EndpointRoute::ssh_and_https(),
        profile,
        auth,
        OperationPolicy::new(4, 32, 3).unwrap(),
    )
    .unwrap();
    block_on(handle.admit(input).unwrap().acquire())
        .unwrap()
        .register()
}

/// Whether `bytes` is a pkt-line advertisement: a first line whose four hex digits give its length (more than 255 when
/// the server's capability list is long, as Git for Windows' agent string makes it), ending with a flush.
pub(super) fn is_advertisement(bytes: &[u8]) -> bool {
    let length = bytes
        .get(..4)
        .and_then(|digits| std::str::from_utf8(digits).ok())
        .and_then(|digits| usize::from_str_radix(digits, 16).ok());
    length.is_some_and(|length| (4..=bytes.len()).contains(&length)) && bytes.ends_with(b"0000")
}
/// The advertisement of one upload-pack exchange, which then ends with a flush: the server's EOF follows.
pub(super) fn advertisement(exchange: &mut Exchange) -> Result<Vec<u8>, StreamError> {
    let deadline = Instant::now() + WITHIN;
    let mut sent = 0;
    let mut out = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        assert!(Instant::now() < deadline, "the exchange did not end");
        if sent < 4 {
            match exchange.stream().write(&b"0000"[sent..]) {
                Ok(n) => sent += n,
                Err(StreamError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error),
            }
        }
        match exchange.stream().read(&mut buffer) {
            Ok(0) => return Ok(out),
            Ok(n) => out.extend_from_slice(&buffer[..n]),
            Err(StreamError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) => return Err(error),
        }
    }
}

#[test]
fn finish_a_then_b_reuses_the_idle_ssh_connection_through_the_host() {
    // B proves A's connection (plan step 1.6): its host key is listed in `known_hosts` now, so B is leased it.
    let case = Case::with_evidence(true);
    for name in ["op-a", "op-b"] {
        let operation = case.operation(name);
        let mut exchange = case.exchange(&operation);
        let advertised = advertisement(&mut exchange).unwrap();
        assert!(
            is_advertisement(&advertised),
            "a pkt-line advertisement, not {:?}",
            String::from_utf8_lossy(&advertised)
        );
        assert_eq!(
            block_on(exchange.finish()).disposition,
            Disposition::Reusable
        );
        block_on(operation.finish());
        case.wait("the connection is idle", |counts| {
            (counts.idle, counts.leased) == (1, 0)
        });
    }
    assert_eq!(case.sessions(), 1, "one SSH connection served A and B");
    assert!(case.close());
}

#[test]
fn a_connection_that_keeps_no_evidence_serves_only_the_binding_that_used_it() {
    // A password's or a helper's connection can be proved by no other binding (contract 7.1): B opens its own, and A's
    // stays idle for A.
    let case = Case::new();
    for name in ["op-a", "op-b"] {
        let operation = case.operation(name);
        let mut exchange = case.exchange(&operation);
        advertisement(&mut exchange).unwrap();
        block_on(exchange.finish());
        block_on(operation.finish());
    }
    case.wait("both connections idle", |counts| {
        (counts.idle, counts.leased) == (2, 0)
    });
    assert_eq!(case.sessions(), 2);
    assert!(case.close());
}

#[test]
fn a_host_key_that_is_no_longer_trusted_revokes_the_connection_and_its_idle_siblings() {
    // Two idle connections with one host key, A's. `known_hosts` lists nothing now: B's proof of the first fails, both
    // are retired, and B is served by a connection of its own, which the next proof of nobody can reach.
    let case = Case::with_evidence(false);
    let a = case.operation("op-a");
    let mut first = case.exchange(&a);
    let mut second = case.exchange(&a);
    advertisement(&mut first).unwrap();
    advertisement(&mut second).unwrap();
    block_on(first.finish());
    block_on(second.finish());
    block_on(a.finish());
    case.wait("both connections idle", |counts| {
        (counts.idle, counts.leased) == (2, 0)
    });
    let b = case.operation("op-b");
    let mut exchange = case.exchange(&b);
    advertisement(&mut exchange).unwrap();
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Reusable
    );
    block_on(b.finish());
    case.wait("only B's connection is left", |counts| {
        (counts.idle, counts.leased, counts.total()) == (1, 0, 1)
    });
    assert_eq!(case.sessions(), 3, "B's connection is a third one");
    assert!(case.close());
}

#[test]
fn the_adapter_keeps_nothing_of_a_connects_context_once_the_connect_completes() {
    // Contract 7 (step 1.5 retention rule): the context given with a connect is dropped when the connect completes,
    // and nothing of it stays on the idle connection.
    let case = Case::new();
    let provider = Arc::new(());
    let a = operation_with(&case.handle, "op-a", provider.clone());
    let held = Arc::strong_count(&provider);
    let mut exchange = case.exchange(&a);
    advertisement(&mut exchange).unwrap();
    block_on(exchange.finish());
    case.wait("the connection is idle", |counts| {
        (counts.idle, counts.leased) == (1, 0)
    });
    assert_eq!(
        Arc::strong_count(&provider),
        held,
        "the idle connection holds none of the operation's context"
    );
    block_on(a.finish());
    assert!(case.close());
}

#[test]
fn a_dead_idle_ssh_connection_is_detected_and_discarded() {
    let case = Case::new();
    let a = case.operation("op-a");
    let mut exchange = case.exchange(&a);
    advertisement(&mut exchange).unwrap();
    block_on(exchange.finish());
    block_on(a.finish());
    case.wait("idle", |counts| counts.idle == 1);
    // The server's side closes it: with no operation and no caller, the host's driver drops it.
    case.proxy.cut_all();
    case.wait("the dead idle connection is discarded", |counts| {
        counts.total() == 0
    });
    let b = case.operation("op-b");
    let mut exchange = case.exchange(&b);
    advertisement(&mut exchange).unwrap();
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Reusable
    );
    assert_eq!(
        case.sessions(),
        2,
        "the next operation got a fresh connection"
    );
    block_on(b.finish());
    assert!(case.close());
}

#[test]
fn a_binding_fault_closes_only_that_binding() {
    let case = Case::new();
    let a = case.operation("op-a");
    let b = case.operation("op-b");
    let mut exchange_a = case.exchange(&a);
    let mut exchange_b = case.exchange(&b);
    assert_eq!(case.handle.snapshot().connections.leased, 2);
    // A's binding is cancelled: its stream fails by name, its connection is closed.
    a.cancel_handle().cancel();
    assert!(matches!(
        advertisement(&mut exchange_a),
        Err(StreamError::Cancelled)
    ));
    assert_eq!(
        block_on(exchange_a.finish()).disposition,
        Disposition::Discarded
    );
    // B's exchange, on its own connection, goes on and returns it to idle.
    advertisement(&mut exchange_b).unwrap();
    assert_eq!(
        block_on(exchange_b.finish()).disposition,
        Disposition::Reusable
    );
    case.wait("A's connection gone, B's idle", |counts| {
        (counts.idle, counts.leased, counts.total()) == (1, 0, 1)
    });
    assert_eq!(case.handle.snapshot().phase, HostPhase::Open);
    assert!(block_on(a.finish()).cancelled);
    assert!(!block_on(b.finish()).cancelled);
    assert!(case.close());
}

#[test]
fn a_physical_fault_retires_that_connection_and_the_next_operation_gets_a_fresh_one() {
    let case = Case::new();
    let a = case.operation("op-a");
    let mut exchange = case.exchange(&a);
    // The connection dies under the leased exchange.
    case.proxy.cut_all();
    assert!(advertisement(&mut exchange).is_err());
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Discarded
    );
    case.wait("the broken connection is gone", |counts| {
        counts.total() == 0
    });
    assert_eq!(
        case.handle.snapshot().phase,
        HostPhase::Open,
        "the host is intact"
    );
    assert!(!block_on(a.finish()).cancelled);
    let b = case.operation("op-b");
    let mut exchange = case.exchange(&b);
    advertisement(&mut exchange).unwrap();
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Reusable
    );
    assert_eq!(case.sessions(), 2);
    block_on(b.finish());
    assert!(case.close());
}

#[test]
fn an_ssh_opening_completes_with_no_caller_poll() {
    let case = Case::new();
    let a = case.operation("op-a");
    let wait = a.open(case.spec()).unwrap().wait();
    // Nothing polls the opening: the driver connects, leases and starts the exchange.
    wait_for("the driver started the exchange", || wait.is_ready());
    assert_eq!(case.handle.snapshot().connections.leased, 1);
    let mut exchange = block_on(wait).unwrap();
    advertisement(&mut exchange).unwrap();
    block_on(exchange.finish());
    block_on(a.finish());
    assert!(case.close());
}

#[test]
fn cancel_during_connect_ends_the_opening_and_disposes_of_the_setup() {
    let case = Case::new();
    case.hold.store(true, Ordering::SeqCst);
    let a = case.operation("op-a");
    let wait = a.open(case.spec()).unwrap().wait();
    case.wait("connecting", |counts| counts.opening == 1);
    a.cancel_handle().cancel();
    assert!(matches!(block_on(wait), Err(OpenFailure::Cancelled)));
    case.wait("the cancelled setup is disposed", |counts| {
        counts.total() == 0
    });
    assert_eq!(case.sessions(), 0, "it never dialed");
    case.hold.store(false, Ordering::SeqCst);
    let b = case.operation("op-b");
    let mut exchange = case.exchange(&b);
    advertisement(&mut exchange).unwrap();
    block_on(exchange.finish());
    block_on(b.finish());
    assert!(block_on(a.finish()).cancelled);
    assert!(case.close());
}

#[test]
fn owner_close_stops_the_driver_and_an_in_flight_open_ends_with_a_named_error() {
    let case = Case::new();
    case.hold.store(true, Ordering::SeqCst);
    let a = case.operation("op-a");
    let wait = a.open(case.spec()).unwrap().wait();
    case.wait("connecting", |counts| counts.opening == 1);
    // The close disposes of the waiting setup within its bound and reports it complete.
    assert!(case.close());
    assert!(matches!(
        block_on(wait),
        Err(OpenFailure::Closing(TerminalReason::HostClosing))
    ));
    assert_eq!(case.handle.snapshot().phase, HostPhase::Closed);
    assert!(matches!(
        block_on(a.finish()).cleanup,
        CleanupStatus::Complete
    ));
}

#[test]
fn the_runtime_builds_its_ssh_adapters_through_the_production_factory() {
    use crate::transport_host::{SshEndpointConfig, TransportRuntime, driver_tests};
    let fixture = SshdFixture::new();
    let home = driver_tests::endpoint_home(&fixture);
    let runtime = TransportRuntime::new(SshEndpointConfig::fixture(home.clone(), None)).unwrap();
    let (handle, keys) = runtime.connection_for_test().unwrap();
    let keys: Registry = keys.expect("an SSH runtime's factory has a key registry");
    // The selected key, admitted as the endpoint admits one (plan step 1.5 moves this into the operation's context).
    let key = Key::ssh(&fixture.user, "127.0.0.1", fixture.port);
    let mut job = keys
        .start(
            key.clone(),
            home.join("client_ed25519"),
            None,
            Duration::from_secs(5),
        )
        .unwrap();
    let loaded = block_on(std::future::poll_fn(|cx| job.poll_result(cx))).unwrap();
    let entry = keys.intern(loaded, || Ok(())).unwrap();
    // The factory makes the instance from the operation's profile: the runtime's own settings, as a capture of the same
    // environment would give them.
    let inputs = EndpointInputs {
        local: SshEndpointConfig::fixture(home.clone(), None).into(),
        tls: Default::default(),
    };
    let operation = operation_over(&handle, "op-factory", "ssh-runtime", inputs);
    let spec = ExchangeSpec::git(
        key,
        entry.identity(),
        GitService::UploadPackAdvertisement,
        &server_path(&fixture.url_repository),
    )
    .unwrap();
    let mut exchange = block_on(operation.open(spec).unwrap().wait()).unwrap();
    let advertised = advertisement(&mut exchange).unwrap();
    assert!(
        is_advertisement(&advertised),
        "a pkt-line advertisement, not {:?}",
        String::from_utf8_lossy(&advertised)
    );
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Reusable
    );
    block_on(operation.finish());
    assert_eq!(handle.snapshot().connections.idle, 1);
    assert_eq!(block_on(runtime.shutdown()).pending_local_work, 0);
    assert_eq!(handle.snapshot().phase, HostPhase::Closed);
}

// Phase 1 integration fix-up, round 1 (dev-docs/GwzPhase1Integration-RemPlan.md): the SSH adapter's proof jobs, against
// the platform's private agent (`reuse_agent`: Unix `ssh-agent` on a private socket, the pipe agent on Windows).

/// A private agent holding one fresh key, and the key's public blob (what a connection's evidence names).
struct AgentKey {
    _dir: tempfile::TempDir,
    agent: Agent,
    key: std::path::PathBuf,
    blob: Vec<u8>,
}
impl AgentKey {
    fn new() -> Self {
        use base64::Engine as _;
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("agent_key");
        keygen_kind(&key, ROW_KEY);
        let public = std::fs::read_to_string(key.with_extension("pub")).unwrap();
        let blob = base64::engine::general_purpose::STANDARD
            .decode(public.split_whitespace().nth(1).unwrap())
            .unwrap();
        let agent = Agent::start(dir.path(), &[&key]);
        Self {
            _dir: dir,
            agent,
            key,
            blob,
        }
    }
    /// A case whose connections keep this key's evidence and a trusted host key.
    fn case(&self) -> Case {
        Case::keeping(Kept {
            agent: Some((self.blob.clone(), self.agent.source())),
            ..Kept::trusted()
        })
    }
}

/// `count` exchanges of `operation` open at once on as many connections, each run to its clean end.
fn serve_together(case: &Case, operation: &Operation, count: usize) {
    let waits: Vec<_> = (0..count)
        .map(|_| operation.open(case.spec()).unwrap().wait())
        .collect();
    let exchanges: Vec<_> = waits
        .into_iter()
        .map(|wait| block_on(wait).unwrap())
        .collect();
    for mut exchange in exchanges {
        let advertised = advertisement(&mut exchange).unwrap();
        assert!(is_advertisement(&advertised), "{advertised:?}");
        assert_eq!(
            block_on(exchange.finish()).disposition,
            Disposition::Reusable
        );
    }
}
/// A's `count` connections, left idle.
fn idle_after_a(case: &Case, count: usize) {
    let a = case.operation("op-a");
    serve_together(case, &a, count);
    block_on(a.finish());
    case.wait("A's connections are idle", |counts| {
        (counts.idle, counts.leased) == (count, 0)
    });
    assert_eq!(case.sessions(), count);
}

#[test]
fn p1_int_rem_concurrent_leases_of_one_binding_share_one_proof_job() {
    // Code P2-2 = State P3-1: A leaves three idle connections; B opens three at once. One proof job signs once (the
    // agent holds the first answer back, so every lease is waiting when it ends) and all three are Authorized: B is
    // served by A's three connections, no new one.
    let agent = AgentKey::new();
    let case = agent.case();
    idle_after_a(&case, 3);
    agent.agent.delay_next(400);
    let b = case.operation("op-b");
    serve_together(&case, &b, 3);
    assert_eq!(agent.agent.signs(), 1, "one sign request for three leases");
    assert_eq!(case.sessions(), 3, "A's connections served B");
    block_on(b.finish());
    assert!(case.close());
}

#[test]
fn p1_int_rem_a_cancelled_waiter_leaves_the_shared_proof_running_for_the_rest() {
    let agent = AgentKey::new();
    let case = agent.case();
    idle_after_a(&case, 2);
    agent.agent.delay_next(400);
    let b = case.operation("op-b");
    let first = b.open(case.spec()).unwrap().wait();
    let second = b.open(case.spec()).unwrap().wait();
    wait_for("B's proof reached the agent", || agent.agent.signs() >= 1);
    drop(first);
    let mut exchange = block_on(second).unwrap();
    assert!(is_advertisement(&advertisement(&mut exchange).unwrap()));
    block_on(exchange.finish());
    assert_eq!(agent.agent.signs(), 1);
    assert_eq!(
        case.sessions(),
        2,
        "the surviving waiter was served by A's connection"
    );
    block_on(b.finish());
    assert!(case.close());
}

#[test]
fn p1_int_rem_a_proof_outlives_its_only_cancelled_waiter_and_is_kept_for_the_binding() {
    let agent = AgentKey::new();
    let case = agent.case();
    idle_after_a(&case, 1);
    agent.agent.delay_next(400);
    let b = case.operation("op-b");
    let wait = b.open(case.spec()).unwrap().wait();
    wait_for("B's proof reached the agent", || agent.agent.signs() == 1);
    drop(wait);
    // The proof's answer arrives after the cancellation, and is kept for B.
    thread::sleep(Duration::from_millis(900));
    let mut exchange = case.exchange(&b);
    assert!(is_advertisement(&advertisement(&mut exchange).unwrap()));
    block_on(exchange.finish());
    assert_eq!(
        agent.agent.signs(),
        1,
        "B's second lease asked the agent nothing"
    );
    assert_eq!(case.sessions(), 1, "A's connection served B");
    block_on(b.finish());
    assert!(case.close());
}

#[test]
fn p1_int_rem_a_key_withdrawn_from_the_agent_revokes_the_connection_and_its_siblings() {
    // Code P3-3: the possession leg on the live path. B's proof signs once and the agent has withdrawn the key: the
    // verdict is Revoked, A's two connections are retired, and B has a connection of its own. With possession forced to
    // none the trust leg alone would authorize B and this row would see two connections, not three.
    let agent = AgentKey::new();
    let case = agent.case();
    idle_after_a(&case, 2);
    agent.agent.withdraw(&agent.key);
    let b = case.operation("op-b");
    let mut exchange = case.exchange(&b);
    assert!(is_advertisement(&advertisement(&mut exchange).unwrap()));
    block_on(exchange.finish());
    assert_eq!(agent.agent.signs(), 1, "B's proof asked the agent once");
    block_on(b.finish());
    case.wait("only B's connection is left", |counts| {
        (counts.idle, counts.leased, counts.total()) == (1, 0, 1)
    });
    assert_eq!(case.sessions(), 3, "B's connection is a third one");
    assert!(case.close());
}

#[test]
fn p1_int_rem_the_agent_proof_authorizes_a_held_key_once() {
    let agent = AgentKey::new();
    let case = agent.case();
    idle_after_a(&case, 1);
    let b = case.operation("op-b");
    for _ in 0..2 {
        let mut exchange = case.exchange(&b);
        assert!(is_advertisement(&advertisement(&mut exchange).unwrap()));
        block_on(exchange.finish());
    }
    assert_eq!(agent.agent.signs(), 1, "the proof is kept for the binding");
    assert_eq!(case.sessions(), 1);
    block_on(b.finish());
    assert!(case.close());
}

#[test]
fn p1_int_rem_trust_is_rechecked_under_the_host_as_the_url_wrote_it() {
    // Code P3-1 = State P3-3: `known_hosts` lists the host hashed, in the case the URL wrote. B's re-check names both
    // spellings, as the setup's did, so B is Authorized and A's connection is reused.
    let case = Case::keeping(Kept {
        named: Some(("githost.example".into(), "GitHost.Example".into())),
        ..Kept::trusted()
    });
    for name in ["op-a", "op-b"] {
        let operation = case.operation(name);
        let mut exchange = case.exchange(&operation);
        assert!(is_advertisement(&advertisement(&mut exchange).unwrap()));
        block_on(exchange.finish());
        block_on(operation.finish());
        case.wait("the connection is idle", |counts| {
            (counts.idle, counts.leased) == (1, 0)
        });
    }
    assert_eq!(case.sessions(), 1, "one SSH connection served A and B");
    assert!(case.close());
}

#[test]
fn p1_int_rem_a_failed_proof_retires_no_sibling_whose_own_proof_is_running() {
    // Code P3-2 = State P3-2: A leaves S1 and S2. B leases one and its proof signs, the answer held back; the agent then
    // withdraws the key and C's proof of the other is refused, which retires the idle connections of that key. The one
    // B's proof is running on is not retired: B's proof passes and B is served by it, C by a connection of its own.
    let agent = AgentKey::new();
    let case = agent.case();
    idle_after_a(&case, 2);
    agent.agent.delay_next(1500);
    let b = case.operation("op-b");
    let wait_b = b.open(case.spec()).unwrap().wait();
    wait_for("B's proof reached the agent", || agent.agent.signs() == 1);
    // B's request has been answered by the agent; only the proxy holds the reply back.
    thread::sleep(Duration::from_millis(150));
    agent.agent.withdraw(&agent.key);
    let c = case.operation("op-c");
    let mut exchange_c = case.exchange(&c);
    assert!(is_advertisement(&advertisement(&mut exchange_c).unwrap()));
    block_on(exchange_c.finish());
    let mut exchange_b = block_on(wait_b).unwrap();
    assert!(is_advertisement(&advertisement(&mut exchange_b).unwrap()));
    block_on(exchange_b.finish());
    assert_eq!(case.sessions(), 3, "A's two, and C's own");
    block_on(b.finish());
    block_on(c.finish());
    assert!(case.close());
}

#[test]
fn p1_int_rem_a_proof_that_passes_on_a_connection_lost_meanwhile_is_unprovable() {
    // Code P3-2 = State P3-2: B's proof of A's connection is running (the agent holds the answer back) when the peer
    // cuts the connection and the adapter disposes of it. The proof passes, but the connection is gone: B is not told
    // `Authorized` (which would fail its exchange) but `Unprovable`, and is served by a connection of its own.
    let agent = AgentKey::new();
    let case = agent.case();
    idle_after_a(&case, 1);
    agent.agent.delay_next(1000);
    let b = case.operation("op-b");
    let wait = b.open(case.spec()).unwrap().wait();
    wait_for("B's proof reached the agent", || agent.agent.signs() >= 1);
    case.proxy.cut_all();
    // The idle watch notices within a few of the worker's passes; the agent's answer is still held back.
    thread::sleep(Duration::from_millis(400));
    let mut exchange = block_on(wait).unwrap();
    assert!(is_advertisement(&advertisement(&mut exchange).unwrap()));
    block_on(exchange.finish());
    assert_eq!(case.sessions(), 2, "B has a connection of its own");
    block_on(b.finish());
    assert!(case.close());
}

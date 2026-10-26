//! The connection host over the SSH adapter, against the fixture `sshd` (plan step 1.3b; contract sections 4.1, 6, 8
//! and 9). Each host runs on the composition's own ports (`DriverThread`), so nothing here polls on the host's behalf:
//! the rows wait on the host's snapshot or on the opening's readiness. The fixture's server sits behind a proxy that
//! counts its connections and can cut them from the server's side.
use super::ConnectionHost;
use crate::git::endpoint::{
    cut_proxy::CutProxy,
    ssh_adapter::{AdapterSettings, SshAdapter},
    ssh_fixture::{DialRetries, SshdFixture, server_path},
    ssh_key_snapshot::Registry,
    ssh_setup::{Authenticated, Setup, SetupConnector},
};
use gwz_connection::{
    AuthContext, CleanupStatus, EndpointAdapter, EndpointFactory, EndpointProfile, EndpointRoute,
    EndpointSpec, Exchange, ExchangeSpec, FactoryEnv, FactoryError, GitService, HostHandle,
    HostPhase, OpenFailure, Operation, OperationPolicy, PreparedOperation, StreamError,
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
}
impl EndpointFactory for FixtureFactory {
    fn create(
        &self,
        _spec: &EndpointSpec,
        _env: &FactoryEnv<'_>,
    ) -> Result<Box<dyn EndpointAdapter>, FactoryError> {
        let (dial, hold) = (self.dial.clone(), self.hold.clone());
        Ok(Box::new(SshAdapter::new(
            move |origin| {
                SetupConnector::isolated(origin, Duration::from_millis(500), move |_, _, _| {
                    let (dial, hold) = (dial.clone(), hold.clone());
                    let setup: Setup = Box::new(move |control| {
                        while hold.load(Ordering::SeqCst) {
                            control.check()?;
                            thread::sleep(Duration::from_millis(5));
                        }
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
                })
            },
            settings(),
        )))
    }
}

struct Case {
    fixture: SshdFixture,
    proxy: Arc<CutProxy>,
    retries: DialRetries,
    hold: Arc<AtomicBool>,
    host: Mutex<ConnectionHost>,
    handle: HostHandle,
}
impl Case {
    fn new() -> Self {
        let fixture = SshdFixture::new();
        let proxy = Arc::new(CutProxy::start(fixture.port));
        let retries = DialRetries::default();
        let hold = Arc::new(AtomicBool::new(false));
        let factory = FixtureFactory {
            dial: Arc::new(fixture.dialer_counting(proxy.port, retries.clone())),
            hold: hold.clone(),
        };
        let host = ConnectionHost::build(
            Arc::new(factory),
            None,
            Arc::new(crate::transport_host::composition::process_safety()),
        )
        .unwrap();
        let handle = host.handle();
        Self {
            fixture,
            proxy,
            retries,
            hold,
            host: Mutex::new(host),
            handle,
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
            Key::ssh(&self.fixture.user, "127.0.0.1", self.fixture.port),
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
    let input = PreparedOperation::new(
        name,
        EndpointRoute::ssh_and_https(),
        EndpointProfile::new(b"fixture-sshd".to_vec(), [7; 32]).unwrap(),
        AuthContext::new(64, Arc::new(())),
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
    let case = Case::new();
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
    let operation = operation(&handle, "op-factory");
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

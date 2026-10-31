//! R3b of the Phase 1 remediation (State P3-2(d), Code P3-3; contract sections 4.1, 7 and 10): what the composition's
//! factory and ports take from the host's cleanup domain and from the clocks, and what the SSH adapter's setup is given.
//! The rows run through a `ConnectionHost` on the production `CoreFactory`, with profiles that real captures made:
//! - the factory holds the host's setup-job, cleanup-record and helper-slot ledgers to the budgets of
//!   `FactoryEnv::domain().config()`, and a setup that finds the job budget spent waits for a place and ends typed at the
//!   connect bound;
//! - the driver's `Clock::epoch` moves when the wall clock drifts from the monotonic origin by more than the idle interval,
//!   and a host that sees it does not lease an idle connection across the jump;
//! - the SSH adapter's connect reaches the connector with the context of the operation it connects for.
use super::{
    ConnectionHost,
    factory_profile_tests::{admitted, capture, https_host, opened, ssh_host},
    https_tests::{ADVERTISEMENT, body, port_of},
    tests::{block_on, wait_for},
};
use crate::{
    git::endpoint::{
        https_auth::{self, HelperSlots},
        https_fixture::{Handler, Server, response},
        shared_reservation::Authority,
        ssh_adapter::{AdapterSettings, SshAdapter},
        ssh_setup::SetupConnector,
    },
    transport_host::{
        composition::process_safety,
        connection_factory::{CoreFactory, HttpsHost, SchemeRouter, SshHost},
        connection_host::{DriverPorts, DriverThread, HostParts, WallClock},
    },
};
use gwz_connection::{
    Clock, DomainConfig, EndpointAdapter, EndpointFactory, EndpointProfile, EndpointRoute,
    EndpointSpec, ExchangeSpec, FactoryEnv, FactoryError, GitService, OpenFailure, OperationPolicy,
    PreparedOperation,
};
use gwz_transport::{
    pool::{Config as PoolConfig, Identity, Key},
    protocol::{self, Disposition},
};
use std::{
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, Ordering},
    },
    task::Waker,
    time::{Duration, Instant},
};

/// A wall clock the row moves by hand, in milliseconds since the Unix epoch.
fn hand_wall() -> (Arc<AtomicI64>, WallClock) {
    let source = Arc::new(AtomicI64::new(1_700_000_000_000));
    let read = source.clone();
    (source, Arc::new(move || read.load(Ordering::SeqCst)))
}
fn host_on(
    ssh: Option<SshHost>,
    https: Option<HttpsHost>,
    domain: DomainConfig,
    wall: WallClock,
) -> ConnectionHost {
    let factory = CoreFactory::new(ssh, https);
    let registry = factory.registry();
    ConnectionHost::build_on(
        Arc::new(factory),
        registry,
        Arc::new(process_safety()),
        HostParts { domain, wall },
    )
    .unwrap()
}

/// Waits for `future` on this thread for up to `limit` (the shared `block_on` gives up at 30 s, which is this row's bound).
fn block_on_within<F: std::future::Future>(future: F, limit: Duration) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl std::task::Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut future = std::pin::pin!(future);
    let deadline = Instant::now() + limit;
    loop {
        if let std::task::Poll::Ready(value) = future
            .as_mut()
            .poll(&mut std::task::Context::from_waker(&waker))
        {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "the future did not end within {limit:?}"
        );
        std::thread::park_timeout(Duration::from_millis(50));
    }
}

/// A fixture HTTPS server, a home and the CA file its certificate chains to.
struct Https {
    runtime: tokio::runtime::Runtime,
    server: Server,
    dir: tempfile::TempDir,
}
impl Https {
    fn start() -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let handler: Handler = Arc::new(|_| {
            Box::pin(async {
                response(
                    200,
                    protocol::GitService::UploadPackAdvertisement,
                    ADVERTISEMENT,
                )
            })
        });
        let server = runtime.block_on(Server::start(handler));
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("home")).unwrap();
        std::fs::write(dir.path().join("ca.pem"), &server.ca).unwrap();
        Self {
            runtime,
            server,
            dir,
        }
    }
    /// One operation's capture: this server's CA is its trust.
    fn capture(&self) -> crate::transport_host::endpoint_environment::OperationCapture {
        let home = self.dir.path().join("home");
        let ca = self.dir.path().join("ca.pem");
        capture(&[
            ("HOME", home.as_os_str()),
            ("SSL_CERT_FILE", ca.as_os_str()),
        ])
    }
    fn spec(&self) -> ExchangeSpec {
        ExchangeSpec::git(
            Key::https("localhost", port_of(&self.server)),
            Identity::Https,
            GitService::UploadPackAdvertisement,
            "/repo",
        )
        .unwrap()
    }
    fn accepted(&self) -> usize {
        self.server.connections.load(Ordering::SeqCst)
    }
}

#[test]
fn the_factory_holds_the_hosts_ledgers_to_the_domains_budgets() {
    let authority = Authority::new(256, 256);
    let slots = HelperSlots::new();
    let ssh = {
        let mut ssh = ssh_host();
        ssh.authority = authority.clone();
        ssh
    };
    let https = HttpsHost {
        helper_slots: slots.clone(),
        authority: authority.clone(),
    };
    let budgets = DomainConfig {
        setup_jobs: 2,
        retained_cleanup: 3,
        helper_slots: 5,
    };
    let (_, wall) = hand_wall();
    let mut host = host_on(Some(ssh), Some(https), budgets, wall);
    let handle = host.handle();
    let supervisor = authority.supervisor();
    // Nothing is held to the domain before the factory makes an adapter: the factory is what draws on it.
    assert_eq!(supervisor.limits(), (64, 64));
    assert_eq!(slots.available(), 8);
    let dir = tempfile::tempdir().unwrap();
    let operation = admitted(
        &handle,
        "op-budgets",
        capture(&[("HOME", dir.path().as_os_str())]),
    );
    assert_eq!(
        supervisor.limits(),
        (2, 3),
        "the setup-job and cleanup-record ledgers are not the domain's"
    );
    assert_eq!(
        slots.available(),
        5,
        "the helper slots are not the domain's"
    );
    // And the ledger enforces them: two places, then the third waits.
    let waker = Waker::noop();
    let first = supervisor.reserve(waker).expect("a first place");
    let second = supervisor.reserve(waker).expect("a second place");
    assert_eq!(
        supervisor.reserve(waker).err().map(|e| e.kind()),
        Some(io::ErrorKind::WouldBlock),
        "a third setup job was admitted on a budget of two"
    );
    drop((first, second));
    block_on(operation.finish());
    assert!(block_on(host.close()));
}

#[test]
fn a_setup_on_a_spent_job_budget_waits_for_a_place_and_ends_typed_at_the_connect_bound() {
    let https = Https::start();
    let authority = Authority::new(256, 256);
    let budgets = DomainConfig {
        setup_jobs: 1,
        ..DomainConfig::default()
    };
    let (_, wall) = hand_wall();
    let mut host = host_on(
        None,
        Some(HttpsHost {
            helper_slots: HelperSlots::new(),
            authority: authority.clone(),
        }),
        budgets,
        wall,
    );
    let handle = host.handle();
    let operation = admitted(&handle, "op-spent", https.capture());
    // The one place the domain gives setups is taken: the connect has nowhere to start its setup.
    let held = authority
        .supervisor()
        .reserve(Waker::noop())
        .expect("the domain's one place");
    // The pool's connect clock runs while the setup waits for a place (the opening is held to it, not given more), and
    // ends it `ConnectTimeout`: the default, 30 s, which the host takes from no operation, so the row waits it out.
    let bound = Duration::from_millis(PoolConfig::default().connect_timeout_ms);
    let started = Instant::now();
    let outcome = block_on_within(
        operation.open(https.spec()).unwrap().wait(),
        bound + Duration::from_secs(30),
    );
    let elapsed = started.elapsed();
    assert!(
        matches!(
            outcome,
            Err(OpenFailure::Pool(
                gwz_transport::pool::Error::ConnectTimeout
            ))
        ),
        "a setup on a spent budget did not end at the connect bound: {:?}",
        outcome.err()
    );
    assert!(
        elapsed + Duration::from_secs(1) >= bound,
        "decided at {elapsed:?}, before the bound {bound:?}"
    );
    assert_eq!(https.accepted(), 0, "a setup ran with no place");
    // The place comes back: the same operation is served.
    drop(held);
    let mut exchange = opened(&operation, https.spec()).expect("the open after the place returned");
    assert_eq!(body(&mut exchange).unwrap(), ADVERTISEMENT);
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Reusable
    );
    block_on(operation.finish());
    assert!(block_on(host.close()));
    drop(https.runtime);
}

#[test]
fn the_drivers_epoch_follows_the_wall_clock_against_the_monotonic_origin() {
    let idle_ms = PoolConfig::default().idle_timeout_ms;
    let (source, wall) = hand_wall();
    let ports = DriverPorts(DriverThread::new(wall, idle_ms));
    let first = ports.epoch();
    assert_eq!(
        ports.epoch(),
        first,
        "a wall clock that keeps time moved the epoch"
    );
    // A drift under the idle interval is not a suspend.
    source.fetch_add(idle_ms as i64 * 2 / 3, Ordering::SeqCst);
    assert_eq!(
        ports.epoch(),
        first,
        "a drift under the idle interval moved the epoch"
    );
    // A second drift that, with the first, passes the interval is one: short suspends age an idle connection together.
    source.fetch_add(idle_ms as i64 * 2 / 3, Ordering::SeqCst);
    let moved = ports.epoch();
    assert_ne!(
        moved, first,
        "drifts that add up past the idle interval did not move the epoch"
    );
    assert_eq!(
        ports.epoch(),
        moved,
        "the epoch moved again with no further drift"
    );
    // The wall clock set back by more than the interval is a jump too.
    source.fetch_sub(idle_ms as i64 * 2, Ordering::SeqCst);
    assert_ne!(
        ports.epoch(),
        moved,
        "a wall clock set back past the interval did not move the epoch"
    );
}

#[test]
fn an_idle_connection_is_not_leased_across_a_wall_clock_jump() {
    let https = Https::start();
    let idle_ms = PoolConfig::default().idle_timeout_ms as i64;
    let (source, wall) = hand_wall();
    let mut host = host_on(None, Some(https_host()), DomainConfig::default(), wall);
    let handle = host.handle();
    let serve = |name: &str| {
        let operation = admitted(&handle, name, https.capture());
        let mut exchange = opened(&operation, https.spec()).expect("the host served the open");
        assert_eq!(body(&mut exchange).unwrap(), ADVERTISEMENT);
        assert_eq!(
            block_on(exchange.finish()).disposition,
            Disposition::Reusable
        );
        block_on(operation.finish());
        wait_for("the connection is idle", || {
            handle.snapshot().connections.idle == 1
        });
    };
    serve("op-a");
    assert_eq!(https.accepted(), 1);
    // A drift of two thirds of the idle interval is not a suspend: the idle connection is reused.
    source.fetch_add(idle_ms * 2 / 3, Ordering::SeqCst);
    serve("op-b");
    assert_eq!(https.accepted(), 1, "the idle connection was not reused");
    // The next drift takes the two past the interval: the connection's age cannot be trusted, so it is not leased.
    source.fetch_add(idle_ms * 2 / 3, Ordering::SeqCst);
    serve("op-c");
    assert_eq!(
        https.accepted(),
        2,
        "an idle connection was leased across a wall-clock jump past the idle interval"
    );
    assert!(block_on(host.close()));
    drop(https.runtime);
}

/// An SSH adapter whose connector records the helper executable of the context each setup is given, and whose setups fail.
struct Recording {
    seen: Arc<Mutex<Vec<Option<std::path::PathBuf>>>>,
}
impl EndpointFactory for Recording {
    fn create(
        &self,
        _spec: &EndpointSpec,
        _env: &FactoryEnv<'_>,
    ) -> Result<Box<dyn EndpointAdapter>, FactoryError> {
        let seen = self.seen.clone();
        let adapter = SshAdapter::new(
            move |origin| {
                SetupConnector::isolated(
                    origin,
                    Duration::from_millis(500),
                    move |_, _, opening| {
                        seen.lock().unwrap().push(
                            opening
                                .auth
                                .as_ref()
                                .and_then(https_auth::OperationAuth::of)
                                .map(|auth| auth.helper().executable.clone()),
                        );
                        Err(io::Error::other(
                            "the row's setup fails after it is given its context",
                        ))
                    },
                )
            },
            AdapterSettings {
                connect_timeout_ms: 20_000,
                io_timeout_ms: 20_000,
                interaction_timeout_ms: 20_000,
                cleanup_timeout_ms: 2_000,
            },
        );
        Ok(Box::new(SchemeRouter::new(Some(adapter), None)))
    }
}

#[test]
fn the_ssh_adapters_connect_gives_the_setup_the_operations_own_context() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (_, wall) = hand_wall();
    let mut host = ConnectionHost::build_on(
        Arc::new(Recording { seen: seen.clone() }),
        None,
        Arc::new(process_safety()),
        HostParts {
            domain: DomainConfig::default(),
            wall,
        },
    )
    .unwrap();
    let handle = host.handle();
    // Two operations with one profile share an instance; each names its own helper in its own context.
    for name in ["helper-of-a", "helper-of-b"] {
        let input = PreparedOperation::new(
            name,
            EndpointRoute::ssh_and_https(),
            EndpointProfile::new(b"fixture-ssh".to_vec(), [5; 32]).unwrap(),
            https_auth::OperationAuth::context(
                https_auth::Config {
                    executable: name.into(),
                    environment: Vec::new(),
                },
                None,
            ),
            OperationPolicy::new(4, 32, 0).unwrap(),
        )
        .unwrap();
        let operation = block_on(handle.admit(input).unwrap().acquire())
            .unwrap()
            .register();
        let spec = ExchangeSpec::git(
            Key::ssh("user", "127.0.0.1", 1),
            Identity::Ambient,
            GitService::UploadPackAdvertisement,
            "/repo",
        )
        .unwrap();
        assert!(
            block_on(operation.open(spec).unwrap().wait()).is_err(),
            "the row's setup succeeded"
        );
        block_on(operation.finish());
    }
    assert_eq!(handle.snapshot().endpoint_instances, 1, "one instance");
    assert_eq!(
        *seen.lock().unwrap(),
        [Some("helper-of-a".into()), Some("helper-of-b".into())],
        "a setup was not given the context of the operation it connected for"
    );
    assert!(block_on(host.close()));
}

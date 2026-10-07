//! A full local budget makes a connection setup wait, never fail (adaptive
//! concurrency design §7.5, §10.2 case 13), and the wait is not charged to the
//! connect clock: only the open's allocation bounds it, and a wait that outlasts
//! the allocation is a local failure, never a server's stall.
//!
//! The 64 jobs belong to one host's `Supervisor`, so each test fills a
//! supervisor of its own. Three local budgets are covered, each through the
//! HTTPS pool and the first also through an SSH open in its production form
//! (the pool's shared setup clock): the job budget, the connector's setup
//! slots, and the endpoint-wide reservation.
use super::{
    agent_job::{self, Job, Supervisor},
    https_connection::HttpConnector,
    https_fixture::{Server, response},
    https_pool::{HttpLease, RunningPool},
    placement_endpoint::PlacementEndpoint,
    setup_retry::{self, Phase, Verdict},
    shared_reservation::{Authority, ReservedConnector},
    ssh_key_snapshot::Registry,
    ssh_pool::{Connector, Opening, Resource},
    ssh_setup::SetupConnector,
    ssh_worker::Endpoint,
};
use gwz_transport::{
    pool::{Config, Identity, Key, Owner},
    protocol::{
        AuthPolicy, Deadlines, Destination, Disposition, Envelope, ErrorCode, Failure, GitService,
        Identity as WireIdentity, MessageKind, Open, Scheme,
    },
};
use std::{
    io,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

const PATIENCE: Duration = Duration::from_secs(10);

/// Every job of `supervisor`, taken until `release`, as other setups take them.
struct Full {
    gate: Arc<(Mutex<bool>, Condvar)>,
    held: Vec<Job<()>>,
}
impl Full {
    fn take(supervisor: &Supervisor) -> Self {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let mut held = Vec::new();
        loop {
            let hold = gate.clone();
            match Job::start(supervisor, None, PATIENCE, move |_| {
                let (released, changed) = &*hold;
                let mut released = released.lock().unwrap();
                while !*released {
                    released = changed.wait(released).unwrap();
                }
                Ok(())
            }) {
                Ok(job) => held.push(job),
                Err(error) => {
                    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
                    break;
                }
            }
        }
        assert_eq!(held.len(), agent_job::LIMIT);
        Self { gate, held }
    }
    /// Ends every held job; the reaper returns their permits.
    fn release(self) {
        let (released, changed) = &*self.gate;
        *released.lock().unwrap() = true;
        changed.notify_all();
        drop(self.held);
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

async fn server() -> Server {
    Server::start(Arc::new(|_| {
        Box::pin(async { response(200, GitService::UploadPackAdvertisement, "ok") })
    }))
    .await
}

fn https_key(server: &Server) -> Key {
    let destination = super::https_destination::Destination::parse(&server.url).unwrap();
    Key::https(destination.host(), destination.port())
}

fn connector(server: &Server, supervisor: &Supervisor) -> HttpConnector {
    HttpConnector {
        config: server.config(),
        epoch: Instant::now(),
        setup_slots: Arc::new(Semaphore::new(8)),
        supervisor: supervisor.clone(),
    }
}

#[test]
fn an_https_connection_waits_for_a_free_job_and_then_connects() {
    runtime().block_on(async {
        let server = server().await;
        let supervisor = Supervisor::new();
        let full = Full::take(&supervisor);
        let mut resource = connector(&server, &supervisor)
            .start(&https_key(&server), &Identity::Https, None)
            .expect("a full job budget queues the connection; it is not refused");
        let mut cx = Context::from_waker(Waker::noop());
        for _ in 0..25 {
            assert!(
                matches!(resource.poll_connected(&mut cx), Poll::Pending),
                "the connection must wait while no job is free"
            );
            assert!(resource.waiting_locally(), "and report that it waits");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        assert_eq!(server.connections.load(Ordering::SeqCst), 0);
        full.release();
        let until = Instant::now() + PATIENCE;
        let connected = loop {
            if let Poll::Ready(result) = resource.poll_connected(&mut cx) {
                break result;
            }
            assert!(Instant::now() < until, "the freed job was never taken");
            tokio::time::sleep(Duration::from_millis(2)).await;
        };
        assert!(connected.is_ok(), "{connected:?}");
        assert!(!resource.waiting_locally());
        assert_eq!(server.connections.load(Ordering::SeqCst), 1);
        while resource.poll_dispose(&mut cx, false).is_pending() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    });
}

/// A connector whose setups only record that they ran, then fail.
fn ssh_connector(ran: Arc<AtomicBool>, supervisor: Supervisor) -> SetupConnector {
    SetupConnector::reported(
        Instant::now(),
        Duration::from_secs(1),
        supervisor,
        move |_, _, _| {
            let ran = ran.clone();
            Ok(Box::new(move |_| {
                ran.store(true, Ordering::SeqCst);
                Err(io::ErrorKind::Other.into())
            }))
        },
    )
}

#[test]
fn an_ssh_setup_waits_for_a_free_job_and_then_runs() {
    let ran = Arc::new(AtomicBool::new(false));
    let supervisor = Supervisor::new();
    let full = Full::take(&supervisor);
    let mut resource = ssh_connector(ran.clone(), supervisor)
        .start_reported(
            &Key::ssh("git", "host", 22),
            &Identity::Ambient,
            None,
            Opening::default(),
        )
        .expect("a full job budget queues the setup; it is not refused");
    let mut cx = Context::from_waker(Waker::noop());
    for _ in 0..25 {
        assert!(matches!(resource.poll_connected(&mut cx), Poll::Pending));
        assert!(resource.waiting_locally());
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(!ran.load(Ordering::SeqCst), "the setup ran with no job");
    full.release();
    let until = Instant::now() + PATIENCE;
    let ended = loop {
        if let Poll::Ready(result) = resource.poll_connected(&mut cx) {
            break result;
        }
        assert!(Instant::now() < until, "the freed job was never taken");
        std::thread::sleep(Duration::from_millis(2));
    };
    assert!(ran.load(Ordering::SeqCst), "the setup never ran");
    let failed = ended.expect_err("the fixture's setup fails");
    assert_ne!(failed.code, ErrorCode::Capacity);
}

/// The local budget a test fills.
#[derive(Clone, Copy, Debug)]
enum Budget {
    Jobs,
    SetupSlot,
    Reservation,
}

/// What a test holds to keep a budget full, and how it frees it.
enum Held {
    Jobs(Full),
    Slot(tokio::sync::OwnedSemaphorePermit),
    Reservation(super::shared_reservation::Reservation),
}
impl Held {
    fn release(self) {
        match self {
            Held::Jobs(full) => full.release(),
            Held::Slot(permit) => drop(permit),
            Held::Reservation(reservation) => drop(reservation),
        }
    }
}

/// An HTTPS pool whose `budget` is full, and its server.
async fn full_pool(budget: Budget) -> (RunningPool, Server, Held) {
    let server = server().await;
    let authority = match budget {
        Budget::Reservation => Authority::new(1, 1),
        _ => Authority::new(256, 32),
    };
    let supervisor = authority.supervisor().clone();
    let slots = Arc::new(Semaphore::new(1));
    let held = match budget {
        Budget::Jobs => Held::Jobs(Full::take(&supervisor)),
        Budget::SetupSlot => Held::Slot(slots.clone().try_acquire_owned().unwrap()),
        Budget::Reservation => Held::Reservation(
            authority
                .try_reserve(https_key(&server).host)
                .expect("the reservation is free"),
        ),
    };
    let tls = server.config();
    let pool =
        RunningPool::with_connector(Config::default(), authority, move |epoch| HttpConnector {
            config: tls,
            epoch,
            setup_slots: slots,
            supervisor,
        })
        .unwrap();
    (pool, server, held)
}

async fn checkout(
    pool: &RunningPool,
    server: &Server,
    allocation_ms: u64,
    connect_ms: u64,
) -> Result<HttpLease, (Failure, Phase)> {
    pool.client
        .checkout(
            https_key(server),
            Owner::new("session", "operation"),
            allocation_ms,
            connect_ms,
            &CancellationToken::new(),
        )
        .await
}

#[test]
fn a_wait_on_a_local_budget_outlasts_the_connect_clock_and_then_connects() {
    for budget in [Budget::Jobs, Budget::SetupSlot, Budget::Reservation] {
        runtime().block_on(async {
            let (mut pool, server, held) = full_pool(budget).await;
            // A 100 ms connect clock, a 5 s allocation.
            let (lease, ()) = tokio::join!(checkout(&pool, &server, 5_000, 100), async {
                tokio::time::sleep(Duration::from_millis(400)).await;
                held.release();
            });
            let lease = lease.unwrap_or_else(|(failed, phase)| {
                panic!("{budget:?}: the wait failed as {failed:?} in {phase:?}")
            });
            assert_eq!(server.connections.load(Ordering::SeqCst), 1, "{budget:?}");
            lease.finish(Disposition::Discarded).unwrap();
            assert_eq!(pool.shutdown(Duration::from_secs(5)).await, 0);
        });
    }
}

#[test]
fn a_wait_on_a_local_budget_that_outlasts_the_allocation_is_a_local_failure() {
    for budget in [Budget::Jobs, Budget::SetupSlot, Budget::Reservation] {
        runtime().block_on(async {
            let (mut pool, server, held) = full_pool(budget).await;
            let began = Instant::now();
            let result = checkout(&pool, &server, 300, 100).await;
            let waited = began.elapsed();
            let Err((failed, phase)) = result else {
                panic!("{budget:?}: a full budget cannot lease a connection");
            };
            assert_eq!(failed.code, ErrorCode::Capacity, "{budget:?}: {failed:?}");
            assert_eq!(failed.setup_cause, None, "{budget:?}");
            assert_eq!(
                setup_retry::classify(&failed, phase),
                Verdict::Return,
                "{budget:?}: {failed:?} in {phase:?}"
            );
            assert!(
                waited >= Duration::from_millis(250),
                "{budget:?}: the connect clock ended the wait after {waited:?}"
            );
            assert_eq!(server.connections.load(Ordering::SeqCst), 0, "{budget:?}");
            held.release();
            assert_eq!(pool.shutdown(Duration::from_secs(5)).await, 0);
        });
    }
}

/// A placement endpoint over a real `SetupConnector`, as the transport host
/// builds it: the pool's shared setup clock is installed for every open.
fn ssh_endpoint(ran: Arc<AtomicBool>) -> (PlacementEndpoint, Supervisor) {
    let registry = Registry::new();
    let supervisor = registry.supervisor();
    let authority = Authority::new(256, 32);
    let endpoint = Endpoint::with_registry(
        Config::default(),
        registry,
        move |origin, registry| {
            let ran = ran.clone();
            ReservedConnector::new(
                SetupConnector::reported(
                    origin,
                    Duration::from_secs(1),
                    registry.supervisor(),
                    move |_, _, _| {
                        let ran = ran.clone();
                        Ok(Box::new(move |_| {
                            ran.store(true, Ordering::SeqCst);
                            Err(io::ErrorKind::PermissionDenied.into())
                        }))
                    },
                ),
                authority,
            )
        },
        1_000,
    )
    .unwrap();
    let endpoint = PlacementEndpoint::new(
        endpoint,
        std::path::PathBuf::from("/tmp"),
        "endpoint".into(),
        "owner".into(),
    )
    .unwrap();
    (endpoint, supervisor)
}

fn ambient_open(allocation_ms: i64, connect_ms: i64) -> Envelope {
    let envelope = Envelope {
        version: 2,
        session_id: "session".into(),
        stream_id: 1,
        kind: MessageKind::Open,
        open: Some(Open {
            endpoint_id: "endpoint".into(),
            operation_id: "operation".into(),
            destination: Destination {
                scheme: Scheme::Ssh,
                host: "host".into(),
                port: 22,
                path: "/repo".into(),
                ssh_username: Some("git".into()),
                https_username: None,
            },
            service: GitService::UploadPackExchange,
            identity: WireIdentity::default(),
            policy: AuthPolicy::SshAmbient,
            deadlines: Deadlines {
                allocation_ms,
                connect_ms,
                io_ms: 1_000,
                interaction_ms: 10_000,
                cleanup_ms: 1_000,
            },
            receive_limits: gwz_transport::binding::default_limits(),
        }),
        ..Default::default()
    };
    gwz_transport::codec::admit(&envelope).unwrap();
    envelope
}

/// Steps `endpoint` until it publishes its open's reply, or `patience` ends.
fn reply(
    endpoint: &mut PlacementEndpoint,
    origin: Instant,
    patience: Duration,
) -> Option<Envelope> {
    let mut cx = Context::from_waker(Waker::noop());
    while origin.elapsed() < patience {
        endpoint
            .step(origin.elapsed().as_millis() as u64, &mut cx)
            .unwrap();
        if let Some(outbound) = endpoint.take_outbound() {
            return Some(outbound.envelope);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    None
}

#[test]
fn an_ssh_open_waits_on_a_full_job_budget_beyond_its_connect_clock_and_then_runs() {
    let ran = Arc::new(AtomicBool::new(false));
    let (mut endpoint, supervisor) = ssh_endpoint(ran.clone());
    let full = Full::take(&supervisor);
    // A 100 ms connect clock, a 5 s allocation.
    endpoint
        .accept("request".into(), ambient_open(5_000, 100))
        .unwrap();
    let origin = Instant::now();
    assert!(
        reply(&mut endpoint, origin, Duration::from_millis(400)).is_none(),
        "the open replied while its wait was still inside its allocation"
    );
    assert!(!ran.load(Ordering::SeqCst));
    full.release();
    let answer = reply(&mut endpoint, Instant::now(), PATIENCE).expect("the open replies");
    assert!(ran.load(Ordering::SeqCst), "the setup never ran");
    let failed = answer.open_failed.expect("the fixture's setup fails");
    assert_ne!(failed.code, ErrorCode::Capacity, "{failed:?}");
    // The wait was not a failed attempt: the setup ran at its first.
    let attempt = failed
        .detail
        .as_ref()
        .and_then(|detail| detail.retry_attempt.as_ref())
        .map(|count| count.attempt);
    assert!(attempt.is_none_or(|attempt| attempt == 1), "{failed:?}");
    endpoint.shutdown();
}

#[test]
fn an_ssh_open_whose_job_wait_outlasts_its_allocation_fails_locally_not_as_a_stall() {
    let ran = Arc::new(AtomicBool::new(false));
    let (mut endpoint, supervisor) = ssh_endpoint(ran.clone());
    let _full = Full::take(&supervisor);
    endpoint
        .accept("request".into(), ambient_open(300, 100))
        .unwrap();
    let origin = Instant::now();
    let answer = reply(&mut endpoint, origin, PATIENCE).expect("the open replies");
    let waited = origin.elapsed();
    let failed = answer.open_failed.expect("the wait failed");
    assert_eq!(failed.code, ErrorCode::Capacity, "{failed:?}");
    assert_eq!(failed.setup_cause, None, "{failed:?}");
    assert_eq!(
        setup_retry::classify(&failed, Phase::Setup),
        Verdict::Return,
        "{failed:?}"
    );
    assert!(
        waited >= Duration::from_millis(250),
        "the connect clock ended the wait after {waited:?}"
    );
    assert!(!ran.load(Ordering::SeqCst));
    endpoint.shutdown();
}

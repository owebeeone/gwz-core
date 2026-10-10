//! A full budget makes an open or an identity check wait, never fail.
//!
//! Many opens at once can fill two budgets that each open's setup draws on:
//! the host's supervised jobs and the key registry's reservations. A
//! request that finds either full waits for it, within its own deadline. These
//! tests fill each budget on purpose; the supervised jobs belong to the
//! endpoint's host, so that test fills its own supervisor's.
use super::{
    agent_job::{self, Job},
    placement_endpoint::{EndpointError, PlacementEndpoint},
    ssh_channel::{GitService as NativeService, SshChannel},
    ssh_key_snapshot::Registry,
    ssh_pool::{Connector, Resource},
    ssh_pump::SshPump,
    ssh_worker::{BridgeContext, ChannelResource, Endpoint, PendingOpen},
};
use gwz_transport::{
    pool::{Config, Identity, Key},
    protocol::{
        AuthPolicy, CheckIdentity, Deadlines, Destination, Effect, Envelope, ErrorCode, Failure,
        GitService, Identity as WireIdentity, IdentityMode, MessageKind, Open, Scheme,
    },
    stream::{MessageEndpoint, Stream},
};
use std::{
    io,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
    thread,
    time::{Duration, Instant},
};

const PATIENCE: Duration = Duration::from_secs(5);

/// Counts its setups and refuses each one, with no thread and no job.
struct Counted(Arc<AtomicUsize>);
struct Refused;
impl Connector for Counted {
    type Resource = Refused;
    fn start(&mut self, _: &Key, _: &Identity, _: Option<u64>) -> Result<Refused, Failure> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Refused)
    }
}
impl Resource for Refused {
    fn poll_connected(&mut self, _: &mut Context<'_>) -> Poll<Result<Option<Identity>, Failure>> {
        Poll::Ready(Err(Failure {
            detail: None,
            setup_cause: None,
            code: ErrorCode::Unavailable,
            effect: Effect::None,
            facts: None,
        }))
    }
    fn poll_dispose(&mut self, _: &mut Context<'_>, _: bool) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn reusable(&self) -> bool {
        false
    }
}
impl ChannelResource for Refused {
    fn start_exchange(
        &mut self,
        _: Stream,
        _: MessageEndpoint,
        _: NativeService,
        _: &str,
    ) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn pump(&mut self) -> Option<&mut SshPump<SshChannel>> {
        None
    }
    fn reclaim(&mut self) -> bool {
        false
    }
}

#[test]
fn a_selected_key_open_waits_for_a_key_reservation() {
    let temp = tempfile::tempdir().unwrap();
    let key = temp.path().join("client_ed25519");
    let keygen = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", ""])
        .arg("-f")
        .arg(&key)
        .output()
        .unwrap();
    assert!(keygen.status.success());
    let registry = Registry::new();
    let setups = Arc::new(AtomicUsize::new(0));
    let counted = setups.clone();
    let config = Config::default();
    let endpoint = Endpoint::with_registry(
        config.clone(),
        registry.clone(),
        move |_, _| Counted(counted),
        1_000,
    )
    .unwrap();
    // Reads in flight for other opens hold every reservation.
    let mut held = Vec::new();
    while let Ok(reservation) = registry.reserve() {
        held.push(reservation);
    }
    let open = start_selected_open(&endpoint, &config, key).unwrap();
    let until = Instant::now() + PATIENCE;
    while endpoint.shutdown_status().pending_admissions != 1 {
        if let Poll::Ready(result) = open.poll() {
            panic!(
                "the open failed while the key registry was full: {:?}",
                result.err()
            );
        }
        assert!(Instant::now() < until, "the open never reached admission");
        thread::sleep(Duration::from_millis(1));
    }
    if let Poll::Ready(result) = open.poll() {
        panic!(
            "the open failed while the key registry was full: {:?}",
            result.err()
        );
    }
    assert_eq!(setups.load(Ordering::SeqCst), 0);
    drop(held);
    let until = Instant::now() + PATIENCE;
    let error = loop {
        match open.poll() {
            Poll::Ready(Ok(_)) => panic!("the fixture refuses every setup"),
            Poll::Ready(Err(error)) => break error,
            Poll::Pending => {
                assert!(Instant::now() < until, "the open never replied");
                thread::sleep(Duration::from_millis(1));
            }
        }
    };
    assert_eq!(
        setups.load(Ordering::SeqCst),
        1,
        "once a reservation was free, the open reached its setup: {error:?}"
    );
    endpoint.shutdown();
}

#[test]
fn a_full_job_budget_leaves_opens_running_and_checks_waiting() {
    let registry = Registry::new();
    let supervisor = registry.supervisor();
    // Every supervised job is taken until released, as other setups can take them.
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let mut held = Vec::new();
    while held.len() <= agent_job::LIMIT {
        let gate = gate.clone();
        let job = Job::start(&supervisor, None, PATIENCE, move |_| {
            let (released, changed) = &*gate;
            let mut released = released.lock().unwrap();
            while !*released {
                released = changed.wait(released).unwrap();
            }
            Ok(())
        });
        match job {
            Ok(job) => held.push(job),
            Err(error) => {
                assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
                break;
            }
        }
    }
    assert_eq!(held.len(), agent_job::LIMIT);
    let setups = Arc::new(AtomicUsize::new(0));
    let counted = setups.clone();
    let mut endpoint = PlacementEndpoint::new(
        Endpoint::with_registry(
            Config::default(),
            registry,
            move |_, _| Counted(counted),
            1_000,
        )
        .unwrap(),
        std::env::temp_dir(),
        "endpoint".into(),
        "owner".into(),
    )
    .unwrap();
    // An open holds no job while the worker owns it, so it reaches its setup.
    endpoint.accept("request".into(), ambient_open(1)).unwrap();
    let mut cx = Context::from_waker(Waker::noop());
    let origin = Instant::now();
    while setups.load(Ordering::SeqCst) == 0 {
        assert!(
            origin.elapsed() < PATIENCE,
            "the open never reached its setup"
        );
        endpoint
            .step(origin.elapsed().as_millis() as u64, &mut cx)
            .unwrap();
        thread::sleep(Duration::from_millis(1));
    }
    // A check whose job the budget refuses waits for one; nothing fails.
    let file = tempfile::NamedTempFile::new().unwrap();
    assert_eq!(
        endpoint.accept("request".into(), check(2, file.path())),
        Err(EndpointError::WouldBlock)
    );
    let (released, changed) = &*gate;
    *released.lock().unwrap() = true;
    changed.notify_all();
    drop(held);
    let until = Instant::now() + PATIENCE;
    loop {
        match endpoint.accept("request".into(), check(2, file.path())) {
            Ok(()) => break,
            Err(EndpointError::WouldBlock) => {
                assert!(Instant::now() < until, "the check never found a free job");
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) => panic!("the check failed: {error:?}"),
        }
    }
    endpoint.shutdown();
}

/// Submits the open the placement endpoint would, of `repo` on `host` with
/// the identity file `selected`, over the endpoint's whole policy, without
/// waiting for the worker's reply.
fn start_selected_open(
    endpoint: &Endpoint,
    config: &Config,
    selected: PathBuf,
) -> io::Result<PendingOpen> {
    let context = BridgeContext {
        session_id: "session".into(),
        stream_id: 1,
        version: 2,
        limits: gwz_transport::binding::default_limits(),
        deadlines: Deadlines {
            allocation_ms: config.allocation_timeout_ms as i64,
            connect_ms: config.connect_timeout_ms as i64,
            io_ms: 1_000,
            interaction_ms: config.interaction_timeout_ms as i64,
            cleanup_ms: config.cleanup_timeout_ms as i64,
        },
        waker: None,
    };
    endpoint.start_endpoint_open(
        Key::ssh("git", "host", 22),
        Some(selected),
        NativeService::UploadPack,
        "repo",
        context,
        Arc::new(AtomicBool::new(false)),
    )
}

fn ambient_open(stream_id: i64) -> Envelope {
    let envelope = Envelope {
        version: 2,
        session_id: "session".into(),
        stream_id,
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
                allocation_ms: 10_000,
                connect_ms: 10_000,
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

fn check(stream_id: i64, path: &Path) -> Envelope {
    Envelope {
        version: 2,
        session_id: "session".into(),
        stream_id,
        kind: MessageKind::CheckIdentity,
        check_identity: Some(CheckIdentity {
            endpoint_id: "endpoint".into(),
            operation_id: "operation".into(),
            identity: WireIdentity {
                mode: IdentityMode::ExplicitKey,
                key_path: Some(path.to_str().unwrap().into()),
                path_base: None,
            },
            timeout_ms: 10_000,
        }),
        ..Default::default()
    }
}

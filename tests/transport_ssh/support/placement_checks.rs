use super::*;
use std::{
    sync::{Arc, Mutex, mpsc},
    thread,
};

fn fixture() -> PlacementEndpoint {
    struct NoConnect;
    impl super::super::ssh_pool::Connector for NoConnect {
        type Resource = super::super::ssh_setup::NativeResource;
        fn start(
            &mut self,
            _: &Key,
            _: &gwz_transport::pool::Identity,
            _: Option<u64>,
        ) -> Result<Self::Resource, Failure> {
            panic!("identity check must not connect");
        }
    }
    PlacementEndpoint::new(
        Endpoint::with_connector(gwz_transport::pool::Config::default(), |_| NoConnect, 100)
            .unwrap(),
        PathBuf::from("/tmp"),
        "endpoint".into(),
        "owner".into(),
    )
    .unwrap()
}
fn check(path: &Path, timeout_ms: i64) -> Envelope {
    Envelope {
        version: 2,
        session_id: "session".into(),
        stream_id: 1,
        kind: MessageKind::CheckIdentity,
        check_identity: Some(gwz_transport::protocol::CheckIdentity {
            endpoint_id: "endpoint".into(),
            operation_id: "check".into(),
            identity: Identity {
                mode: IdentityMode::ExplicitKey,
                key_path: Some(path.to_str().unwrap().into()),
                path_base: None,
            },
            timeout_ms,
        }),
        ..Default::default()
    }
}
#[test]
fn blocked_check_times_out_before_physical_disposal() {
    let mut endpoint = fixture();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let job = Job::start(None, Duration::from_millis(1), move |_| {
        entered_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        Ok(())
    })
    .unwrap();
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let envelope = check(Path::new("/tmp/unused"), 10);
    let key = ("request".into(), 1);
    endpoint
        .requests
        .insert(key.clone(), request_state(&envelope, "check".into()));
    endpoint.checks.push(CheckJob {
        key,
        job,
        deadline: 10,
        cancelled: false,
    });
    let mut cx = Context::from_waker(std::task::Waker::noop());
    endpoint.step(10, &mut cx).unwrap();
    let result = endpoint.take_outbound();
    let pending = endpoint.pending_request_count("request");
    endpoint.step(11, &mut cx).unwrap();
    let duplicate = endpoint.take_outbound();
    release_tx.send(()).unwrap();
    for _ in 0..500 {
        endpoint.step(12, &mut cx).unwrap();
        if endpoint.pending_request_count("request") == 0 {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        result
            .expect("deadline terminal must not await disposal")
            .envelope
            .identity_check_failed
            .unwrap()
            .code,
        ErrorCode::Timeout
    );
    assert!(pending > 0, "blocked physical work must stay charged");
    assert!(duplicate.is_none());
    assert_eq!(endpoint.pending_request_count("request"), 0);
}
#[test]
fn fifo_identity_without_writer_rejects_without_blocking() {
    use std::{ffi::CString, os::unix::ffi::OsStrExt, os::unix::fs::OpenOptionsExt};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity-fifo");
    let cpath = CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
    let mut endpoint = fixture();
    endpoint
        .accept("request".into(), check(&path, 1000))
        .unwrap();
    let mut cx = Context::from_waker(std::task::Waker::noop());
    let until = Instant::now() + Duration::from_millis(200);
    let result = loop {
        endpoint.step(0, &mut cx).unwrap();
        if let Some(result) = endpoint.take_outbound() {
            break Some(result);
        }
        if Instant::now() >= until {
            break None;
        }
        thread::sleep(Duration::from_millis(1));
    };
    // Release the original buggy blocking open before asserting, so red leaves no hung job.
    let _writer = std::fs::OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&path);
    assert_eq!(
        result
            .expect("special file admission blocked")
            .envelope
            .identity_check_failed
            .unwrap()
            .code,
        ErrorCode::InvalidRequest
    );
}

#[test]
fn unsupported_deadline_policy_has_no_physical_or_queued_work() {
    let mut endpoint = fixture();
    for (index, deadlines) in [
        gwz_transport::protocol::Deadlines {
            allocation_ms: i64::MAX,
            connect_ms: 1,
            io_ms: 1,
            interaction_ms: i64::MAX,
            cleanup_ms: 1,
        },
        gwz_transport::protocol::Deadlines {
            allocation_ms: 1,
            connect_ms: 1,
            io_ms: 1,
            interaction_ms: 1,
            cleanup_ms: i64::MAX,
        },
    ]
    .into_iter()
    .enumerate()
    {
        let envelope = Envelope {
            version: 2,
            session_id: "session".into(),
            stream_id: index as i64 + 1,
            kind: MessageKind::Open,
            open: Some(gwz_transport::protocol::Open {
                endpoint_id: "endpoint".into(),
                operation_id: "operation".into(),
                destination: Destination {
                    scheme: gwz_transport::protocol::Scheme::Ssh,
                    host: "host".into(),
                    port: 22,
                    path: "/repo".into(),
                    ssh_username: Some("git".into()),
                },
                service: GitService::UploadPackExchange,
                identity: Identity::default(),
                policy: gwz_transport::protocol::AuthPolicy::SshAmbient,
                deadlines,
                receive_limits: gwz_transport::binding::default_limits(),
            }),
            ..Default::default()
        };
        gwz_transport::codec::admit(&envelope).unwrap();
        endpoint.accept("request".into(), envelope).unwrap();
        assert_eq!(
            endpoint
                .take_outbound()
                .unwrap()
                .envelope
                .open_failed
                .unwrap()
                .code,
            ErrorCode::InvalidRequest
        );
        assert!(endpoint.opens.is_empty());
        assert!(endpoint.queued_opens.is_empty());
        assert_eq!(endpoint.endpoint.pending_requests(), 0);
    }
}

/// A fixture whose setup takes `setup`: each connection records when its setup
/// starts and fails once `setup` has passed. It holds no thread and no job.
struct SlowSetup {
    started: Arc<Mutex<Vec<Instant>>>,
    setup: Duration,
}
struct Setting(Instant);
impl super::super::ssh_pool::Connector for SlowSetup {
    type Resource = Setting;
    fn start(
        &mut self,
        _: &Key,
        _: &gwz_transport::pool::Identity,
        _: Option<u64>,
    ) -> Result<Setting, Failure> {
        let now = Instant::now();
        self.started.lock().unwrap().push(now);
        Ok(Setting(now + self.setup))
    }
}
impl super::super::ssh_pool::Resource for Setting {
    fn poll_connected(
        &mut self,
        _: &mut Context<'_>,
    ) -> Poll<Result<Option<gwz_transport::pool::Identity>, Failure>> {
        if Instant::now() < self.0 {
            return Poll::Pending;
        }
        Poll::Ready(Err(Failure {
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
impl super::super::ssh_worker::ChannelResource for Setting {
    fn start_exchange(
        &mut self,
        _: gwz_transport::stream::Stream,
        _: gwz_transport::stream::MessageEndpoint,
        _: NativeService,
        _: &str,
    ) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn pump(
        &mut self,
    ) -> Option<&mut super::super::ssh_pump::SshPump<super::super::ssh_channel::SshChannel>> {
        None
    }
    fn reclaim(&mut self) -> bool {
        false
    }
}
/// An endpoint over the slow fixture, with the operation limits the transport
/// host installs in its pool when the operation starts.
fn slow_endpoint(
    setup: Duration,
    capacity: gwz_transport::pool::Capacity,
) -> (PlacementEndpoint, Arc<Mutex<Vec<Instant>>>) {
    let started = Arc::new(Mutex::new(Vec::new()));
    let connector = SlowSetup {
        started: started.clone(),
        setup,
    };
    let endpoint = PlacementEndpoint::new(
        Endpoint::with_connector(gwz_transport::pool::Config::default(), |_| connector, 9_000)
            .unwrap(),
        PathBuf::from("/tmp"),
        "endpoint".into(),
        "owner".into(),
    )
    .unwrap();
    endpoint.pool().install_capacity(capacity).unwrap();
    (endpoint, started)
}
fn ambient_open(stream_id: i64, host: &str) -> Envelope {
    let envelope = Envelope {
        version: 2,
        session_id: "session".into(),
        stream_id,
        kind: MessageKind::Open,
        open: Some(gwz_transport::protocol::Open {
            endpoint_id: "endpoint".into(),
            operation_id: "operation".into(),
            destination: Destination {
                scheme: gwz_transport::protocol::Scheme::Ssh,
                host: host.into(),
                port: 22,
                path: "/repo".into(),
                ssh_username: Some("git".into()),
            },
            service: GitService::UploadPackExchange,
            identity: Identity::default(),
            policy: gwz_transport::protocol::AuthPolicy::SshAmbient,
            deadlines: gwz_transport::protocol::Deadlines {
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
/// Steps the endpoint, discarding its replies, until `count` setups started.
fn step_until_started(
    endpoint: &mut PlacementEndpoint,
    started: &Mutex<Vec<Instant>>,
    count: usize,
    origin: Instant,
    limit: Duration,
) {
    let mut cx = Context::from_waker(std::task::Waker::noop());
    while started.lock().unwrap().len() < count && origin.elapsed() < limit {
        endpoint
            .step(origin.elapsed().as_millis() as u64, &mut cx)
            .unwrap();
        while endpoint.take_outbound().is_some() {}
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn opens_within_the_operation_per_host_limit_all_start_within_one_setup() {
    const OPENS: usize = 32;
    let setup = Duration::from_millis(400);
    // An operation at the retry plan's defaults: --max-per-host 32, --jobs 100.
    let (mut endpoint, started) = slow_endpoint(
        setup,
        gwz_transport::pool::Capacity {
            per_user_host: 32,
            per_host: 32,
            total: 256,
            max_requests: 1024,
        },
    );
    let origin = Instant::now();
    for stream_id in 1..=OPENS as i64 {
        endpoint
            .accept("request".into(), ambient_open(stream_id, "host"))
            .unwrap();
    }
    step_until_started(
        &mut endpoint,
        &started,
        OPENS,
        origin,
        4 * setup + Duration::from_secs(5),
    );
    let started = started.lock().unwrap().clone();
    endpoint.shutdown();
    assert_eq!(started.len(), OPENS);
    let last = started
        .iter()
        .map(|at| at.duration_since(origin))
        .max()
        .unwrap();
    assert!(
        last < setup,
        "the last of {OPENS} opens started its setup {last:?} after the first open \
         arrived; one setup takes {setup:?}"
    );
}

#[test]
fn opens_beyond_the_operation_limits_wait_until_an_open_finishes() {
    let (mut endpoint, started) = slow_endpoint(
        Duration::ZERO,
        gwz_transport::pool::Capacity {
            per_user_host: 2,
            per_host: 2,
            total: 3,
            max_requests: 1024,
        },
    );
    let origin = Instant::now();
    for (stream_id, host) in [(1, "a"), (2, "a"), (3, "a"), (4, "b"), (5, "c")] {
        endpoint
            .accept("request".into(), ambient_open(stream_id, host))
            .unwrap();
    }
    let queued: Vec<_> = endpoint
        .queued_opens
        .iter()
        .map(|queued| queued.key.1)
        .collect();
    let admitted = endpoint.opens.len();
    step_until_started(&mut endpoint, &started, 5, origin, Duration::from_secs(5));
    let started = started.lock().unwrap().len();
    endpoint.shutdown();
    // Host a's third open waits for the per-host limit, c's for the pool's total.
    assert_eq!(queued, [3, 5]);
    assert_eq!(admitted, 3);
    assert_eq!(
        started, 5,
        "a queued open starts once an admitted one finishes"
    );
}

#[test]
fn open_ceiling_keeps_half_the_job_budget_for_the_setups_that_opens_start() {
    let ceiling = |total, max_requests| {
        open_ceiling(gwz_transport::pool::Capacity {
            per_user_host: 1,
            per_host: 1,
            total,
            max_requests,
        })
    };
    // Each open holds one supervised job until its reply and starts at most one
    // more, its key read or its setup.
    assert_eq!(ceiling(1_000, 1_000), super::super::agent_job::LIMIT / 2);
    assert_eq!(ceiling(3, 1_000), 3);
    assert_eq!(ceiling(1_000, 2), 2);
}

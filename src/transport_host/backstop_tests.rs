//! The driver's per-open backstop through the transport host (gwz-core
//! dev-docs/GwzV110PreActivationReview-ReviewState.md, P2-1). It ends only
//! its own open, never the request, and it does not run while the endpoint
//! holds the open with its allocation clock stopped (adaptive concurrency
//! §5.2). Each test steps the backstop's clock past the full bound of
//! `--max-retries 3` in one move, so no test waits out the real bound.
use super::driver_tests::block_on;
use super::https_tests::{endpoint_home, fixture, meta};
use super::*;
use crate::git::endpoint::ssh_channel::GitService as SshService;
use gwz_transport::protocol::{AuthPolicy, ErrorCode, GitService};
use std::{
    io::Read,
    net::TcpListener,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

/// More than the full bound of `--max-retries 3` under the fixtures' stall
/// bounds (`open_backstop_ms`), which is below 900 s.
const PAST_THE_BOUND: Duration = Duration::from_secs(1_000);
/// Larger than the stream's receive window, so a client that does not read
/// holds the endpoint's exchange, and the connection, mid-transfer.
const BIG: usize = 16 * 1024 * 1024;

/// An HTTPS server whose `/big` repository answers with `BIG` bytes at once,
/// whose `/stall` repository never answers, and whose other paths answer
/// with a few bytes.
fn server(runtime: &tokio::runtime::Runtime) -> fixture::Server {
    runtime.block_on(fixture::Server::start(Arc::new(|request| {
        Box::pin(async move {
            let path = request.uri().path().to_owned();
            let service = GitService::UploadPackAdvertisement;
            if path.starts_with("/stall") {
                std::future::pending().await
            } else if path.starts_with("/big") {
                fixture::response(200, service, vec![b'x'; BIG])
            } else {
                fixture::response(200, service, "small")
            }
        })
    })))
}

fn host_runtime(server: &fixture::Server) -> (TransportRuntime, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let local = SshEndpointConfig::fixture(endpoint_home(root.path()), None);
    let https = HttpsEndpointConfig {
        tls: server.config(),
        auth: None,
    };
    let runtime = TransportRuntime::with_https(local, https, HelperSlots::new()).unwrap();
    (runtime, root)
}

fn request_meta(id: &str, per_host: u32) -> RequestMeta {
    let mut request = meta(id);
    request.policy = Some(crate::OperationPolicy {
        concurrency: Some(8),
        max_connections_per_host: Some(per_host.into()),
        ..Default::default()
    });
    request
}

type Opened = std::io::Result<crate::git::endpoint::stream_io::BlockingStream>;

/// Opens `url` on its own thread; the thread sends the open's answer.
fn open_https(
    context: &super::request::RequestContext,
    path: &str,
    server: &fixture::Server,
) -> mpsc::Receiver<Opened> {
    let url = server.url.replace("/repo", path);
    let context = context.clone();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let opened = context.open_https_recording(
            &url,
            GitService::UploadPackAdvertisement,
            Some(AuthPolicy::Anonymous),
            Arc::new(|_, _| {}),
            Arc::new(|_| {}),
            Arc::new(Mutex::new(None)),
        );
        let _ = sender.send(opened);
    });
    receiver
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(5));
    }
}

fn read_all(
    stream: &mut crate::git::endpoint::stream_io::BlockingStream,
    first: &[u8],
) -> std::io::Result<usize> {
    let mut total = first.len();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            return Ok(total);
        }
        total += read;
    }
}

#[test]
fn a_held_open_outlives_its_backstop_while_the_attempt_ahead_of_it_ends_alone() {
    let tokio = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let server = server(&tokio);
    let (runtime, _root) = host_runtime(&server);
    let request =
        block_on(runtime.request(request_meta("backstop-held", 1), "fetch".into())).unwrap();
    let session = request.context.session.clone();
    // A stream mid-transfer: opened, and its body is larger than the window,
    // so it is not read out.
    let mut transferring = open_https(&request.context, "/big", &server)
        .recv_timeout(Duration::from_secs(20))
        .expect("the first open answers")
        .expect("the first open succeeds");
    let mut head = [0u8; 4096];
    transferring.read_exact(&mut head).unwrap();
    // A second open takes the host's only setup: its connection is the first
    // stream's, so its attempt waits for that stream to end.
    let waiting = open_https(&request.context, "/one", &server);
    wait_until("the second open to reach the driver", || {
        session.streams_for_test() == 2
    });
    // A third open waits in the endpoint for room, its allocation clock stopped.
    let held = open_https(&request.context, "/two", &server);
    wait_until("the endpoint to hold the third open", || {
        session.held_opens_for_test() == 1
    });
    session.step_backstop_for_test(PAST_THE_BOUND);
    // The attempt's backstop ended that open, and only it.
    let Err(error) = waiting
        .recv_timeout(Duration::from_secs(20))
        .expect("the waiting open ends")
    else {
        panic!("an open with no connection to take cannot succeed");
    };
    let failure = &error
        .get_ref()
        .and_then(|cause| cause.downcast_ref::<HttpsOpenFailure>())
        .expect("an open failure carries the failure")
        .failure;
    assert_eq!(failure.code, ErrorCode::Timeout);
    assert!(!session.sealed_for_test("backstop-held"));
    // The stream mid-transfer completes.
    let total = read_all(&mut transferring, &head).expect("the stream mid-transfer completes");
    assert_eq!(total, BIG);
    transferring.close().expect("the finished stream closes");
    drop(transferring);
    // The held open was not charged for its hold: it starts in the room the
    // ended attempt left, with its budget whole, and succeeds.
    let mut opened = held
        .recv_timeout(Duration::from_secs(20))
        .expect("the held open answers once there is room")
        .expect("the held open starts and succeeds");
    assert_eq!(read_all(&mut opened, &[]).unwrap(), "small".len());
    assert!(!session.sealed_for_test("backstop-held"));
    block_on(request.finish());
    block_on(runtime.shutdown());
}

#[test]
fn an_attempt_that_runs_out_its_backstop_ends_alone_and_the_requests_other_stream_completes() {
    let tokio = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let server = server(&tokio);
    let (runtime, _root) = host_runtime(&server);
    let request =
        block_on(runtime.request(request_meta("backstop-alone", 2), "fetch".into())).unwrap();
    let session = request.context.session.clone();
    let mut first = open_https(&request.context, "/big", &server)
        .recv_timeout(Duration::from_secs(20))
        .expect("the first open answers")
        .expect("the first open succeeds");
    let mut head = [0u8; 4096];
    first.read_exact(&mut head).unwrap();
    // An open the server never answers: its attempt is under way, not held.
    let stalled = open_https(&request.context, "/stall", &server);
    wait_until("the stalled open to be at the server", || {
        server.connections.load(Ordering::SeqCst) >= 2
    });
    assert_eq!(session.held_opens_for_test(), 0);
    session.step_backstop_for_test(PAST_THE_BOUND);
    // Its backstop ends it, with a timeout, and nothing else.
    let ended = stalled
        .recv_timeout(Duration::from_secs(20))
        .expect("the stalled open ends");
    let Err(error) = ended else {
        panic!("a stalled open cannot succeed");
    };
    let failure = &error
        .get_ref()
        .and_then(|cause| cause.downcast_ref::<HttpsOpenFailure>())
        .expect("an open failure carries the failure")
        .failure;
    assert_eq!(failure.code, ErrorCode::Timeout);
    assert!(!session.sealed_for_test("backstop-alone"));
    let total = read_all(&mut first, &head).expect("the stream mid-transfer completes");
    assert_eq!(total, BIG);
    first.close().expect("the finished stream closes");
    drop(first);
    block_on(request.finish());
    block_on(runtime.shutdown());
}

/// Accepts connections and never answers, so an SSH setup stalls; counts the
/// connections it accepted.
struct Silent {
    port: u16,
    accepted: Arc<AtomicUsize>,
}
impl Silent {
    fn new() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let accepted = Arc::new(AtomicUsize::new(0));
        let counter = accepted.clone();
        thread::spawn(move || {
            let mut held = Vec::new();
            while let Ok((stream, _)) = listener.accept() {
                counter.fetch_add(1, Ordering::AcqRel);
                held.push(stream);
            }
        });
        Self { port, accepted }
    }
}

#[test]
fn an_ssh_open_held_for_its_site_outlives_its_backstop_and_starts_when_room_appears() {
    let server = Silent::new();
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".ssh")).unwrap();
    std::fs::write(home.path().join(".ssh/known_hosts"), "").unwrap();
    let config =
        SshEndpointConfig::fixture(home.path().to_path_buf(), None).with_io_timeout_ms(60_000);
    let runtime = TransportRuntime::new(config).unwrap();
    let mut request_meta = meta("backstop-ssh");
    request_meta.policy = Some(crate::OperationPolicy {
        concurrency: Some(8),
        max_connections_per_host: Some(1),
        ..Default::default()
    });
    let request = block_on(runtime.request(request_meta, "fetch".into())).unwrap();
    let session = request.context.session.clone();
    let open = |context: super::request::RequestContext| {
        let url = format!("ssh://git@127.0.0.1:{}/repo.git", server.port);
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let opened = context.open(
                &url,
                SshService::UploadPack,
                None,
                Arc::new(|_, _| {}),
                Arc::new(|_| {}),
            );
            let _ = sender.send(opened.map(|_| ()));
        });
        receiver
    };
    // The first open's attempt takes the host's only setup and stalls.
    let first = open(request.context.clone());
    wait_until("the first setup to reach the server", || {
        server.accepted.load(Ordering::Acquire) == 1
    });
    // The second waits for room, its allocation clock stopped.
    let second = open(request.context.clone());
    wait_until("the endpoint to hold the second open", || {
        session.held_opens_for_test() == 1
    });
    session.step_backstop_for_test(PAST_THE_BOUND);
    // The first open's attempt ran out its backstop and ends alone.
    let ended = first
        .recv_timeout(Duration::from_secs(20))
        .expect("the first open ends");
    assert!(ended.is_err());
    assert!(!session.sealed_for_test("backstop-ssh"));
    // The held open was not ended with it, and now has the room: it starts.
    wait_until("the held open to start its setup", || {
        server.accepted.load(Ordering::Acquire) == 2
    });
    assert!(matches!(second.try_recv(), Err(mpsc::TryRecvError::Empty)));
    assert!(!session.sealed_for_test("backstop-ssh"));
    block_on(request.finish());
    block_on(runtime.shutdown());
    assert!(
        second
            .recv_timeout(Duration::from_secs(20))
            .unwrap()
            .is_err()
    );
}

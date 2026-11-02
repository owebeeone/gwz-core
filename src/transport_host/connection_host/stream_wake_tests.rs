//! The SSH and HTTPS adapters' byte paths wake their caller (Phase 1 Code P2-4, step R2b parts C and D; contract section
//! 9): a pending read or write is woken by the data, the room or the end it waits for, `shutdown_write` half-closes the
//! request and leaves the response readable, and a dropped `write_all` future leaves `progress().accepted_write` exact.
//! The rows drive their futures with a waker that counts its wakes and re-polls only when woken: a timed-out park checks
//! the row's deadline and never re-polls, so a row cannot pass on a busy or timed re-poll (unlike `tests::block_on`).
use super::{
    ConnectionHost,
    https_tests::{ADVERTISEMENT, Case as HttpsCase},
    tests::{block_on, is_advertisement, operation, wait_for},
};
use crate::git::endpoint::{
    https_fixture::{Behaviour, Handler, response},
    ssh_adapter::{AdapterSettings, SshAdapter},
    ssh_fixture::{SshConnection, SshdFixture, run, run_output, server_path},
    ssh_setup::{Authenticated, Setup, SetupConnector},
};
use gwz_connection::{
    EndpointAdapter, EndpointFactory, EndpointSpec, Exchange, ExchangeSpec, FactoryEnv,
    FactoryError, GitService, Operation,
};
use gwz_transport::{
    pool::{Identity, Key},
    protocol::{self, AuthMethod, Disposition, Facts},
};
use http_body_util::BodyExt;
use std::{
    future::Future,
    io::{self, Write},
    pin::pin,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
    thread::{self, Thread},
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

const WITHIN: Duration = Duration::from_secs(30);
/// How long a pending read must stay unwoken while nothing arrives.
const QUIET: Duration = Duration::from_millis(200);
/// The windows a pending read may take to settle: the stream's own state changes (the read's credit leaving) wake
/// every waiter, at most a couple of times.
const MAX_SETTLE: usize = 3;
/// The windows a body write may take to fill what lies between it and a server that does not read (the body channel,
/// Hyper, the sockets and the proxy; slow on a loaded machine), and the most it may write meanwhile.
const MAX_FILL: usize = 150;
const FILL_LIMIT: usize = 256 << 20;

/// A task on this thread: its waker counts wakes and unparks the thread.
struct Counting {
    wakes: AtomicUsize,
    thread: Thread,
}
impl Wake for Counting {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.wakes.fetch_add(1, Ordering::SeqCst);
        self.thread.unpark();
    }
}
impl Counting {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            wakes: AtomicUsize::new(0),
            thread: thread::current(),
        })
    }
    fn wakes(&self) -> usize {
        self.wakes.load(Ordering::SeqCst)
    }
    /// Parks until a wake beyond `seen` arrives; a park that times out (or unparks spuriously) only checks `deadline`.
    fn wait_past(&self, seen: usize, deadline: Instant) {
        while self.wakes() == seen {
            let left = deadline
                .checked_duration_since(Instant::now())
                .expect("no wake came in time");
            thread::park_timeout(left);
        }
    }
    /// Polls `future` to its end, polling again only after a wake.
    fn drive<F: Future>(self: &Arc<Self>, future: F) -> F::Output {
        let waker = Waker::from(self.clone());
        let mut future = pin!(future);
        let deadline = Instant::now() + WITHIN;
        loop {
            let seen = self.wakes();
            if let Poll::Ready(value) = future.as_mut().poll(&mut Context::from_waker(&waker)) {
                return value;
            }
            self.wait_past(seen, deadline);
        }
    }
}

/// Makes SSH adapters whose setups dial the fixture's server.
struct FixtureFactory {
    dial: Arc<dyn Fn() -> io::Result<SshConnection> + Send + Sync>,
}
impl EndpointFactory for FixtureFactory {
    fn create(
        &self,
        _spec: &EndpointSpec,
        _env: &FactoryEnv<'_>,
    ) -> Result<Box<dyn EndpointAdapter>, FactoryError> {
        let dial = self.dial.clone();
        let settings = AdapterSettings {
            connect_timeout_ms: 20_000,
            io_timeout_ms: 20_000,
            interaction_timeout_ms: 20_000,
            cleanup_timeout_ms: 2_000,
        };
        Ok(Box::new(SshAdapter::new(
            move |origin| {
                SetupConnector::isolated(origin, Duration::from_millis(500), move |_, _, _| {
                    let dial = dial.clone();
                    let setup: Setup = Box::new(move |_control| {
                        let facts = Facts {
                            method: AuthMethod::SshKey,
                            authenticated: Some(true),
                            credential_offered: true,
                            ..Facts::default()
                        };
                        Authenticated::new(dial()?, Identity::Ambient, facts)
                    });
                    Ok(setup)
                })
            },
            settings,
        )))
    }
}
/// A host over the fixture's server.
struct Case {
    fixture: SshdFixture,
    host: ConnectionHost,
    operation: Operation,
}
impl Case {
    fn new() -> Self {
        let fixture = SshdFixture::new();
        let factory = FixtureFactory {
            dial: Arc::new(fixture.dialer(fixture.port)),
        };
        let host = ConnectionHost::build(
            Arc::new(factory),
            None,
            Arc::new(crate::transport_host::composition::process_safety()),
        )
        .unwrap();
        let operation = operation(&host.handle(), "op-wake");
        Self {
            fixture,
            host,
            operation,
        }
    }
    fn open(&self, service: GitService) -> Exchange {
        let spec = ExchangeSpec::git(
            Key::ssh(&self.fixture.user, "127.0.0.1", self.fixture.port),
            Identity::Ambient,
            service,
            &server_path(&self.fixture.url_repository),
        )
        .unwrap();
        block_on(self.operation.open(spec).unwrap().wait()).unwrap()
    }
    fn end(mut self, exchange: Exchange) {
        block_on(exchange.finish());
        block_on(self.operation.finish());
        assert!(block_on(self.host.close()));
    }
}
/// Reads `exchange`'s response to its end.
fn read_to_end(task: &Arc<Counting>, exchange: &mut Exchange) -> Vec<u8> {
    let mut response = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        match task.drive(exchange.stream().read(&mut buffer)).unwrap() {
            0 => return response,
            n => response.extend_from_slice(&buffer[..n]),
        }
    }
}

#[test]
fn r2b_a_pending_ssh_read_is_woken_by_what_arrives_not_by_a_timer() {
    let case = Case::new();
    let mut exchange = case.open(GitService::UploadPackExchange);
    let task = Counting::new();
    let mut advertised = Vec::new();
    let mut buffer = [0u8; 4096];
    while !is_advertisement(&advertised) {
        let n = task.drive(exchange.stream().read(&mut buffer)).unwrap();
        assert_ne!(n, 0, "the advertisement ended early: {advertised:?}");
        advertised.extend_from_slice(&buffer[..n]);
    }
    // Nothing follows the advertisement until the client writes: the read is pending and, once the stream settles,
    // stays unwoken. The stream wakes every waiter when its state moves, so the credit this read returned, leaving with
    // the worker's next pass, may wake it once (or twice); a re-poll after that is pending again and the next window is
    // quiet. A busy re-poll is woken in every window. The adapter keeps the registration after the future drops, so
    // the end that comes later still wakes this waker.
    let reader = Counting::new();
    {
        let mut read = pin!(exchange.stream().read(&mut buffer));
        let waker = Waker::from(reader.clone());
        for windows in 1.. {
            // Counted from before the poll: a poll that wakes its own waker is a busy re-poll.
            let seen = reader.wakes();
            assert!(
                read.as_mut()
                    .poll(&mut Context::from_waker(&waker))
                    .is_pending()
            );
            thread::sleep(QUIET);
            if reader.wakes() == seen {
                break;
            }
            assert!(
                windows < MAX_SETTLE,
                "a pending read was woken in each of {windows} windows of {QUIET:?} while nothing arrived"
            );
        }
    }
    let seen = reader.wakes();
    task.drive(exchange.stream().write_all(b"0000")).unwrap();
    reader.wait_past(seen, Instant::now() + WITHIN);
    assert_eq!(
        reader.drive(exchange.stream().read(&mut buffer)).unwrap(),
        0,
        "the client's flush ends upload-pack: the read sees the end"
    );
    case.end(exchange);
}

#[test]
fn r2b_shutdown_write_ends_an_ssh_request_and_leaves_the_response_readable() {
    let case = Case::new();
    let mut exchange = case.open(GitService::ReceivePackExchange);
    let task = Counting::new();
    task.drive(exchange.stream().shutdown_write()).unwrap();
    task.drive(exchange.stream().shutdown_write())
        .expect("a second shutdown_write is Ready too");
    // receive-pack waits for commands after its advertisement: only the request's end lets it exit, so the response's
    // end proves the server saw it.
    let response = read_to_end(&task, &mut exchange);
    assert!(
        is_advertisement(&response),
        "the response stayed readable: {:?}",
        String::from_utf8_lossy(&response)
    );
    assert!(
        task.drive(exchange.stream().write(b"0000")).is_err(),
        "a write after shutdown_write fails"
    );
    case.end(exchange);
}

/// A commit holding a 2 MiB blob of pseudo-random bytes (incompressible, far past the stream's 64 KiB buffer), and its
/// pack: the commit's id and the pack's bytes.
fn large_pack() -> (String, Vec<u8>) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    let git = || {
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(repo)
            .args([
                "-c",
                "user.name=row",
                "-c",
                "user.email=row@example.invalid",
            ])
            .args(["-c", "commit.gpgsign=false"]);
        command
    };
    run(git().args(["init", "-q", "--initial-branch=main"]));
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let blob: Vec<u8> = (0..2 << 20)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect();
    std::fs::write(repo.join("blob"), blob).unwrap();
    run(git().args(["add", "blob"]));
    run(git().args(["commit", "-q", "-m", "large"]));
    let commit = run_output(git().args(["rev-parse", "HEAD"]));
    let mut packer = git()
        .args(["pack-objects", "--revs", "--stdout", "-q"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(packer.stdin.take().unwrap(), "{commit}").unwrap();
    let output = packer.wait_with_output().unwrap();
    assert!(output.status.success(), "pack-objects failed");
    (commit, output.stdout)
}

#[test]
fn r2b_a_dropped_ssh_write_all_leaves_accepted_write_exact() {
    let (commit, pack) = large_pack();
    let line = format!(
        "{} {commit} refs/heads/main\0report-status\n",
        "0".repeat(commit.len())
    );
    let mut request = format!("{:04x}{line}0000", line.len() + 4).into_bytes();
    request.extend_from_slice(&pack);
    let case = Case::new();
    let mut exchange = case.open(GitService::ReceivePackExchange);
    let task = Counting::new();
    {
        let mut write = pin!(exchange.stream().write_all(&request));
        let waker = Waker::from(task.clone());
        assert!(
            write
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending(),
            "the request outgrew the stream's buffer"
        );
    }
    // The dropped future took a prefix, which `accepted_write` reports exactly: the caller resumes from there.
    let accepted = usize::try_from(exchange.stream().progress().accepted_write).unwrap();
    assert!(
        0 < accepted && accepted < request.len(),
        "a prefix was accepted: {accepted} of {}",
        request.len()
    );
    task.drive(exchange.stream().write_all(&request[accepted..]))
        .unwrap();
    assert_eq!(
        exchange.stream().progress().accepted_write,
        request.len() as u64
    );
    task.drive(exchange.stream().flush()).unwrap();
    let response = String::from_utf8_lossy(&read_to_end(&task, &mut exchange)).into_owned();
    assert!(
        response.contains("unpack ok") && response.contains("ok refs/heads/main"),
        "the server took every byte once: {response:?}"
    );
    let pushed = run_output(
        Command::new("git")
            .arg("--git-dir")
            .arg(&case.fixture.repository)
            .args(["rev-parse", "refs/heads/main"]),
    );
    assert_eq!(pushed, commit);
    case.end(exchange);
}

/// The request bodies the HTTPS fixture read, in order.
type Bodies = Arc<Mutex<Vec<Vec<u8>>>>;
/// An HTTPS fixture handler that takes a permit of `release` before it touches a request, then reads the request's body
/// to its end, records it in `bodies` and answers: the advertisement for a GET, `0000` for a POST.
fn holding(release: &Arc<Semaphore>, bodies: &Bodies) -> Handler {
    let (release, bodies) = (release.clone(), bodies.clone());
    Arc::new(move |request| {
        let (release, bodies) = (release.clone(), bodies.clone());
        Box::pin(async move {
            release.acquire().await.unwrap().forget();
            let post = request.method() == hyper::Method::POST;
            let body = request.into_body().collect().await.unwrap().to_bytes();
            bodies.lock().unwrap().push(body.to_vec());
            if post {
                response(200, protocol::GitService::ReceivePackExchange, &b"0000"[..])
            } else {
                response(
                    200,
                    protocol::GitService::UploadPackAdvertisement,
                    ADVERTISEMENT,
                )
            }
        })
    })
}
/// An HTTPS host over a `holding` fixture whose handler starts with `permits`, and an operation on it.
fn https_case(permits: usize) -> (HttpsCase, Operation, Arc<Semaphore>, Bodies) {
    let release = Arc::new(Semaphore::new(permits));
    let bodies = Bodies::default();
    let case = HttpsCase::with(
        |_| holding(&release, &bodies),
        Behaviour::default(),
        |_| (None, None),
        None,
    );
    let operation = case.operation("op-https-wake");
    (case, operation, release, bodies)
}
fn https_end(case: HttpsCase, operation: Operation, exchange: Exchange) {
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Reusable
    );
    block_on(operation.finish());
    assert!(case.close());
}

#[test]
fn r2b_a_pending_https_read_is_woken_by_the_response_not_by_a_timer() {
    let (case, operation, release, _) = https_case(0);
    let mut exchange = case.exchange_of(&operation, GitService::UploadPackAdvertisement);
    // The handler holds the response: the read is pending and, once the host settles, stays unwoken. A busy re-poll is
    // woken in every window. The pipe keeps the registration after the future drops, so the response still wakes it.
    let reader = Counting::new();
    let mut buffer = [0u8; 4096];
    {
        let mut read = pin!(exchange.stream().read(&mut buffer));
        let waker = Waker::from(reader.clone());
        for windows in 1.. {
            let seen = reader.wakes();
            assert!(
                read.as_mut()
                    .poll(&mut Context::from_waker(&waker))
                    .is_pending()
            );
            thread::sleep(QUIET);
            if reader.wakes() == seen {
                break;
            }
            assert!(
                windows < MAX_SETTLE,
                "a pending read was woken in each of {windows} windows of {QUIET:?} while the response was held"
            );
        }
    }
    let seen = reader.wakes();
    release.add_permits(1);
    reader.wait_past(seen, Instant::now() + WITHIN);
    assert_eq!(read_to_end(&reader, &mut exchange), ADVERTISEMENT);
    https_end(case, operation, exchange);
}

#[test]
fn r2b_shutdown_write_ends_an_https_request_body_before_any_read() {
    let (case, operation, _, bodies) = https_case(1);
    let mut exchange = case.exchange_of(&operation, GitService::ReceivePackExchange);
    let task = Counting::new();
    let request = b"0009hello0000";
    task.drive(exchange.stream().write_all(request)).unwrap();
    task.drive(exchange.stream().shutdown_write()).unwrap();
    task.drive(exchange.stream().shutdown_write())
        .expect("a second shutdown_write is Ready too");
    // The handler reads the body to its end before it answers: only the request's end, sent with no read, lets it.
    wait_for("the handler read the whole request body", || {
        !bodies.lock().unwrap().is_empty()
    });
    assert_eq!(bodies.lock().unwrap()[0], request);
    assert!(
        task.drive(exchange.stream().write(b"0000")).is_err(),
        "a write after shutdown_write fails"
    );
    assert_eq!(read_to_end(&task, &mut exchange), b"0000");
    https_end(case, operation, exchange);
}

#[test]
fn r2b_a_pending_https_body_write_is_woken_when_the_server_reads() {
    let (case, operation, release, bodies) = https_case(0);
    let mut exchange = case.exchange_of(&operation, GitService::ReceivePackExchange);
    let chunk = vec![0x5a_u8; 64 << 10];
    let writer = Counting::new();
    let waker = Waker::from(writer.clone());
    let (mut written, mut fruitless) = (0, 0);
    // The handler does not read: the body fills what lies between, then a write is pending and stays unwoken. Until then
    // a wake brings room, and the next window writes more. A wake that brings none is a re-poll: a busy one comes in
    // every window once the body is full.
    for windows in 1.. {
        let (seen, before) = (writer.wakes(), written);
        loop {
            let polled =
                pin!(exchange.stream().write(&chunk)).poll(&mut Context::from_waker(&waker));
            match polled {
                Poll::Ready(accepted) => written += accepted.unwrap(),
                Poll::Pending => break,
            }
            assert!(
                written < FILL_LIMIT,
                "the body never filled: {written} bytes"
            );
        }
        if written == before {
            fruitless += 1;
        }
        assert!(
            fruitless < MAX_SETTLE && windows < MAX_FILL,
            "a pending write was woken with no room in {fruitless} of {windows} windows of {QUIET:?} while the server \
             read nothing ({written} bytes written)"
        );
        thread::sleep(QUIET);
        if writer.wakes() == seen {
            break;
        }
    }
    let seen = writer.wakes();
    release.add_permits(1);
    writer.wait_past(seen, Instant::now() + WITHIN);
    written += writer.drive(exchange.stream().write(&chunk)).unwrap();
    assert_eq!(exchange.stream().progress().accepted_write, written as u64);
    writer.drive(exchange.stream().shutdown_write()).unwrap();
    assert_eq!(read_to_end(&writer, &mut exchange), b"0000");
    assert_eq!(
        bodies.lock().unwrap()[0].len(),
        written,
        "the server read every byte once"
    );
    https_end(case, operation, exchange);
}

//! Operator decision D14: the setup's one retry of a libssh2 WinCNG key exchange failure (-8), on a new connection
//! inside the same setup attempt. The failure is injected through [`Steps`], never left to chance, so the rows run
//! on every platform; the platform's own policy is pinned by its own row.
use super::{
    kex_retry::{KexRetry, Native},
    observed::{TRIES, establish_retrying_double_kex},
    *,
};
use crate::git::endpoint::{
    agent_job::{Control, ManualClock, Supervisor},
    ssh_fixture::SshdFixture,
    ssh_pool::{Connector, Opening, Resource},
    ssh_setup::{self, SetupConnector},
};
use gwz_transport::{pool::Identity, protocol::ErrorCode};
use ssh2::ErrorCode as Ssh2Code;
use std::{
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
    time::Duration,
};

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        /// The connections the platform's policy makes for a key exchange that always fails.
        const PLATFORM_CONNECTS: usize = 2;
    } else {
        const PLATFORM_CONNECTS: usize = 1;
    }
}

/// Steps that make the first `failures` handshakes fail with `code` without touching the session, count the
/// connects, and leave the rest to the real handshake.
struct Scripted {
    connects: Arc<AtomicUsize>,
    injected: Arc<AtomicUsize>,
    failures: usize,
    code: Ssh2Code,
    /// A manual clock moved on by this much at each injected failure: what the failed try cost.
    cost: Option<(ManualClock, Duration)>,
}

/// The counters a [`Scripted`] reports through.
#[derive(Clone, Default)]
struct Counts {
    connects: Arc<AtomicUsize>,
    injected: Arc<AtomicUsize>,
}
impl Counts {
    fn connects(&self) -> usize {
        self.connects.load(Ordering::SeqCst)
    }
    fn injected(&self) -> usize {
        self.injected.load(Ordering::SeqCst)
    }
    fn steps(&self, failures: usize) -> Scripted {
        Scripted {
            connects: self.connects.clone(),
            injected: self.injected.clone(),
            failures,
            code: KEX_FAILURE,
            cost: None,
        }
    }
}

impl Steps for Scripted {
    fn connect(&mut self, address: SocketAddr, control: &Control) -> io::Result<TcpStream> {
        self.connects.fetch_add(1, Ordering::SeqCst);
        Native.connect(address, control)
    }

    fn handshake(&mut self, session: &mut ssh2::Session) -> Result<(), ssh2::Error> {
        if self.injected.load(Ordering::SeqCst) < self.failures {
            self.injected.fetch_add(1, Ordering::SeqCst);
            if let Some((clock, cost)) = &self.cost {
                clock.advance(*cost);
            }
            return Err(ssh2::Error::new(
                self.code,
                "Unable to exchange encryption keys",
            ));
        }
        Native.handshake(session)
    }
}

/// A listener that is never served: a connect to it completes, a handshake on it is only ever injected.
fn unserved() -> (TcpListener, Key) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let key = Key::ssh("git", "127.0.0.1", listener.local_addr().unwrap().port());
    (listener, key)
}

fn address(key: &Key) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], key.port))
}

/// A control with ten seconds of aggregate budget on a clock only the test moves.
fn timed() -> (ManualClock, Arc<Control>) {
    let clock = ManualClock::new();
    let control = Control::scripted(
        Some(clock.now() + Duration::from_secs(10)),
        Duration::ZERO,
        Duration::from_secs(1),
        clock.clock(),
    );
    (clock, control)
}

/// The network setup for `key` against the steps, with the known hosts text `known`.
fn establish_against<S: Steps>(
    key: &Key,
    known: String,
    control: &Control,
    network: &mut Network<'_, S>,
) -> io::Result<(SshConnection, Vec<u8>)> {
    let address = address(key);
    establish_inner(
        key,
        &key.host,
        Path::new("unused_known_hosts"),
        control,
        move |_, _| Ok(known),
        move |_, _, _| Ok(vec![address]),
        network,
    )
}

fn network<'a, S: Steps>(steps: S, kex: KexRetry, offered: &'a dyn Fn() -> bool) -> Network<'a, S> {
    Network {
        steps,
        kex,
        credential_offered: offered,
    }
}

#[test]
fn a_key_exchange_failure_is_retried_once_on_a_new_connection_and_the_setup_succeeds() {
    let fixture = SshdFixture::new();
    let key = Key::ssh(&fixture.user, "127.0.0.1", fixture.port);
    let known = std::fs::read_to_string(&fixture.known_hosts).unwrap();
    let counts = Counts::default();
    let (_, control) = timed();
    let (connection, trusted) = establish_against(
        &key,
        known,
        &control,
        &mut network(counts.steps(1), KexRetry::Once, &|| false),
    )
    .expect("the second connection's key exchange completes");
    assert_eq!(counts.injected(), 1);
    assert_eq!(
        counts.connects(),
        2,
        "a new TCP connection, not the failed session's"
    );
    assert!(!trusted.is_empty());
    let mut connection = connection;
    assert!(!connection.session().authenticated());
    assert!(control.tcp_connect_ms().is_some());
}

#[test]
fn a_retried_key_exchange_failure_is_one_setup_attempt() {
    let fixture = SshdFixture::new();
    let key = Key::ssh(&fixture.user, "127.0.0.1", fixture.port);
    let known = std::fs::read_to_string(&fixture.known_hosts).unwrap();
    let counts = Counts::default();
    let attempts = Arc::new(AtomicUsize::new(0));
    let seen = attempts.clone();
    let steps = counts.steps(1);
    let steps = Arc::new(std::sync::Mutex::new(Some(steps)));
    let mut connector = SetupConnector::reported(
        std::time::Instant::now(),
        Duration::from_secs(1),
        Supervisor::new(),
        move |key: &Key, _: &Identity, _: Opening| -> io::Result<ssh_setup::Setup> {
            seen.fetch_add(1, Ordering::SeqCst);
            let key = key.clone();
            let known = known.clone();
            let steps = steps
                .lock()
                .unwrap()
                .take()
                .expect("one setup, one set of steps");
            Ok(Box::new(move |control| {
                establish_against(
                    &key,
                    known,
                    &control,
                    &mut network(steps, KexRetry::Once, &|| false),
                )?;
                // The setup got as far as trust; it ends here, as the connector's other rows do.
                Err(io::ErrorKind::Other.into())
            }))
        },
    );
    let mut resource = connector
        .start_reported(&key, &Identity::Ambient, None, Opening::default())
        .unwrap();
    let mut cx = Context::from_waker(Waker::noop());
    let until = std::time::Instant::now() + Duration::from_secs(20);
    let failed = loop {
        if let Poll::Ready(result) = resource.poll_connected(&mut cx) {
            break result.expect_err("the closure ends after trust");
        }
        assert!(std::time::Instant::now() < until, "the setup never ended");
        std::thread::sleep(Duration::from_millis(1));
    };
    // The closure's own ending, not a key exchange failure and not a second attempt.
    assert_eq!(failed.code, ErrorCode::Io);
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        1,
        "one attempt of the retry machine"
    );
    assert_eq!(counts.connects(), 2);
    assert_eq!(counts.injected(), 1);
    assert!(resource.tcp_connect_ms().is_some());
}

#[test]
fn a_second_key_exchange_failure_fails_as_it_does_without_the_retry() {
    let (_listener, key) = unserved();
    let mut ended = Vec::new();
    for (kex, connects) in [(KexRetry::Never, 1), (KexRetry::Once, 2)] {
        let counts = Counts::default();
        let (_, control) = timed();
        let error = establish_against(
            &key,
            String::new(),
            &control,
            &mut network(counts.steps(usize::MAX), kex, &|| false),
        )
        .map(|_| ())
        .expect_err("the key exchange never completes");
        assert_eq!(counts.connects(), connects, "{kex:?}");
        assert_eq!(counts.injected(), connects, "{kex:?}");
        ended.push(error);
    }
    // The same error, which the retry machine retries as it always did.
    assert_eq!(ended[0].kind(), ended[1].kind());
    assert_eq!(ended[1].kind(), io::ErrorKind::Other);
    for error in &ended {
        let failure = ssh_setup::failure_from_io(error);
        assert_eq!(failure.code, ErrorCode::Io);
    }
}

#[test]
fn a_key_exchange_failure_with_the_budget_nearly_spent_is_not_retried() {
    let (_listener, key) = unserved();
    // The failed try used 9.5 of the 10 seconds: the half second left is not room for another.
    for (spent, connects) in [
        (Duration::from_millis(9500), 1),
        (Duration::from_millis(100), 2),
    ] {
        let counts = Counts::default();
        let (clock, control) = timed();
        let mut steps = counts.steps(usize::MAX);
        steps.cost = Some((clock, spent));
        let error = establish_against(
            &key,
            String::new(),
            &control,
            &mut network(steps, KexRetry::Once, &|| false),
        )
        .map(|_| ())
        .expect_err("the key exchange never completes");
        assert_eq!(counts.connects(), connects, "spent {spent:?}");
        assert_eq!(error.kind(), io::ErrorKind::Other, "{error}");
    }
}

#[test]
fn a_key_exchange_failure_after_a_credential_could_have_been_offered_is_never_retried() {
    let (_listener, key) = unserved();
    let counts = Counts::default();
    let (_, control) = timed();
    let error = establish_against(
        &key,
        String::new(),
        &control,
        &mut network(counts.steps(1), KexRetry::Once, &|| true),
    )
    .map(|_| ())
    .expect_err("the key exchange failed");
    assert_eq!(counts.connects(), 1);
    assert_eq!(counts.injected(), 1);
    assert_eq!(error.kind(), io::ErrorKind::Other);
}

#[test]
fn only_a_key_exchange_failure_is_retried() {
    let (_listener, key) = unserved();
    // The library's other session errors: a banner that did not come, a socket that failed, a timeout.
    for code in [-2, -5, -7, -9, -13, -43] {
        let counts = Counts::default();
        let (_, control) = timed();
        let mut steps = counts.steps(usize::MAX);
        steps.code = Ssh2Code::Session(code);
        establish_against(
            &key,
            String::new(),
            &control,
            &mut network(steps, KexRetry::Once, &|| false),
        )
        .map(|_| ())
        .expect_err("the handshake failed");
        assert_eq!(counts.connects(), 1, "libssh2 error {code}");
    }
}

#[test]
fn a_key_exchange_failure_that_ran_the_budget_out_reports_the_timeout() {
    let (_listener, key) = unserved();
    let counts = Counts::default();
    let (clock, control) = timed();
    let mut steps = counts.steps(usize::MAX);
    steps.cost = Some((clock, Duration::from_secs(11)));
    let error = establish_against(
        &key,
        String::new(),
        &control,
        &mut network(steps, KexRetry::Once, &|| false),
    )
    .map(|_| ())
    .expect_err("the budget ran out");
    assert_eq!(counts.connects(), 1);
    assert_eq!(error.kind(), io::ErrorKind::TimedOut, "{error}");
}

#[test]
fn the_platform_policy_retries_on_windows_and_leaves_unix_as_it_was() {
    let (_listener, key) = unserved();
    let counts = Counts::default();
    let (_, control) = timed();
    let offered = || false;
    let mut platform = Network {
        steps: counts.steps(usize::MAX),
        kex: KexRetry::PLATFORM,
        credential_offered: &offered,
    };
    establish_against(&key, String::new(), &control, &mut platform)
        .map(|_| ())
        .expect_err("the key exchange never completes");
    assert_eq!(counts.connects(), PLATFORM_CONNECTS);
    assert_eq!(
        KexRetry::PLATFORM,
        if PLATFORM_CONNECTS == 2 {
            KexRetry::Once
        } else {
            KexRetry::Never
        }
    );
    // And production builds its network on that policy.
    assert_eq!(Network::production(&|| false).kex, KexRetry::PLATFORM);
}

#[test]
fn the_shared_setup_clock_gives_room_by_what_the_failed_try_spent() {
    use gwz_transport::pool::{Action, Config, Key as PoolKey, Owner, PoolMachine, Request};
    use std::sync::atomic::AtomicU64;
    let mut pool = PoolMachine::new(Config {
        connect_timeout_ms: 1000,
        ..Default::default()
    })
    .unwrap();
    pool.request(Request::new(
        PoolKey::ssh("user", "host", 22),
        Identity::Ambient,
        Owner::new("s", "o"),
    ))
    .unwrap();
    let Action::Connect { connection, .. } = pool.next_action().unwrap() else {
        panic!("connect")
    };
    let now = Arc::new(AtomicU64::new(0));
    let source = now.clone();
    let clock = pool
        .install_setup_clock(
            connection,
            Arc::new(move || source.load(Ordering::SeqCst)),
            10,
        )
        .unwrap();
    let context = crate::git::endpoint::ssh_setup_context::SetupContext::new(
        clock,
        std::time::Instant::now(),
    );
    let control = Control::new_shared(
        context,
        Duration::from_millis(100),
        crate::git::endpoint::agent_job::wall_clock(),
    );
    let mark = control.mark();
    now.store(300, Ordering::SeqCst);
    assert!(control.has_room_since(mark), "700 ms left, 300 spent");
    now.store(600, Ordering::SeqCst);
    assert!(!control.has_room_since(mark), "400 ms left, 600 spent");
}

/// A `known_hosts` file that lists nothing, for the rows that read one before they connect.
fn empty_known_hosts() -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("known_hosts");
    std::fs::write(&path, "").unwrap();
    (directory, path)
}

#[test]
fn the_rows_that_retry_a_connection_retry_a_double_key_exchange_failure_and_only_that() {
    let (_listener, key) = unserved();
    let (_directory, known) = empty_known_hosts();
    let (_, control) = timed();
    // D14's class that survived its retry: -8 twice. The row makes the connection again, up to its limit.
    let counts = Counts::default();
    let (result, tries) = establish_retrying_double_kex(
        || counts.steps(usize::MAX),
        KexRetry::Once,
        &key,
        &known,
        &control,
    );
    assert!(result.is_err());
    assert_eq!(tries.len(), TRIES);
    assert!(tries.iter().all(|tried| tried.is_double_kex_failure()));
    assert_eq!(counts.connects(), 2 * TRIES);
    // Every other libssh2 code, on its first try: the row is told, with the codes, and does not make another.
    for code in [-2, -5, -7, -9, -13, -43] {
        let counts = Counts::default();
        let (result, tries) = establish_retrying_double_kex(
            || {
                let mut steps = counts.steps(usize::MAX);
                steps.code = Ssh2Code::Session(code);
                steps
            },
            KexRetry::Once,
            &key,
            &known,
            &control,
        );
        assert!(result.is_err(), "libssh2 error {code}");
        assert_eq!(tries.len(), 1, "libssh2 error {code} was retried");
        assert_eq!(tries[0].handshakes, [Some(Ssh2Code::Session(code))]);
        assert!(!tries[0].is_double_kex_failure());
        assert_eq!(counts.connects(), 1, "libssh2 error {code}");
    }
    // A single -8 (the platform's policy is not to retry it, as on Unix) is not the double either.
    let counts = Counts::default();
    let (result, tries) = establish_retrying_double_kex(
        || counts.steps(usize::MAX),
        KexRetry::Never,
        &key,
        &known,
        &control,
    );
    assert!(result.is_err());
    assert_eq!(tries.len(), 1);
    assert_eq!(tries[0].handshakes, [Some(KEX_FAILURE)]);
}

#[test]
fn a_double_key_exchange_failure_then_a_connection_that_completes_is_one_retry_by_the_row() {
    let fixture = SshdFixture::new();
    let key = Key::ssh(&fixture.user, "127.0.0.1", fixture.port);
    let (_, control) = timed();
    let counts = Counts::default();
    // The first try's two handshakes fail (-8 twice, D14's retry included); the second try is the real thing.
    let (result, tries) = establish_retrying_double_kex(
        || counts.steps(2),
        KexRetry::Once,
        &key,
        &fixture.known_hosts,
        &control,
    );
    result.expect("the second try's handshake completes");
    assert_eq!(tries.len(), 2);
    assert!(tries[0].is_double_kex_failure());
    assert_eq!(tries[1].handshakes, [None]);
    assert_eq!(counts.connects(), 3);
}

//! The pool's network deadline bounds the adapters' own setups (Phase 1 State P3-2(e), step R2b part A; contract
//! sections 4.1 and 8). The crate pool's connect bound is a fixed 30 s, so each row takes about that long in real time
//! (lane owner's decision OQ3). The host's `CancelConnect` is withheld from the adapter here: the setup must end by its
//! own deadline, the earlier of its connect bound and the span the `ConnectCommand` carries, and report `Timeout` by
//! the pool's deadline, whatever its own bound is (none, or longer than the pool's).
use super::{ConnectionHost, tests::operation};
use crate::git::endpoint::{
    https_adapter::{Counters, HttpsAdapter, HttpsInputs, HttpsSettings},
    https_auth::HelperSlots,
    https_connection,
    shared_reservation::Authority,
    ssh_adapter::{AdapterSettings, SshAdapter},
    ssh_setup::{Setup, SetupConnector},
};
use crate::transport_host::connection_factory::SchemeRouter;
use gwz_connection::{
    AuthContext, Completion, ConnectCommand, EndpointAdapter, EndpointFactory, EndpointSpec,
    ExchangeIo, ExchangeSpec, FactoryEnv, FactoryError, GitService, OpenFailure,
};
use gwz_transport::{
    pool::{ConnectionId, Identity, Key},
    protocol::ErrorCode,
};
use std::{
    io,
    net::TcpListener,
    sync::{Arc, Condvar, Mutex},
    task::{Context, Poll},
    thread,
    time::{Duration, Instant},
};

/// The crate pool's connect bound (`gwz_transport::pool::Config::default`), which the instance pools keep.
const POOL_CONNECT: Duration = Duration::from_millis(30_000);
/// How late after the pool's deadline the adapter's own end may still arrive (the worker's queue and park tick).
const SLACK: Duration = Duration::from_secs(5);

/// The connect failures the wrapped adapter reported, by when.
#[derive(Default)]
struct Reported {
    failures: Mutex<Vec<(Instant, ErrorCode)>>,
    changed: Condvar,
}
impl Reported {
    /// The first failure reported by `until`, if any.
    fn first_by(&self, until: Instant) -> Option<(Instant, ErrorCode)> {
        let mut failures = self.failures.lock().unwrap();
        loop {
            if let Some(first) = failures.first() {
                return Some(*first);
            }
            let left = until.checked_duration_since(Instant::now())?;
            failures = self.changed.wait_timeout(failures, left).unwrap().0;
        }
    }
}
/// The adapter under test, with the host's `cancel_connect` withheld and its connect failures recorded.
struct Withholding {
    inner: Box<dyn EndpointAdapter>,
    reported: Arc<Reported>,
}
impl EndpointAdapter for Withholding {
    fn connect(&mut self, command: ConnectCommand) {
        self.inner.connect(command);
    }
    fn cancel_connect(&mut self, _connection: ConnectionId) {}
    fn close(&mut self, connection: ConnectionId) {
        self.inner.close(connection);
    }
    fn start_exchange(
        &mut self,
        connection: ConnectionId,
        spec: &ExchangeSpec,
        auth: &AuthContext,
    ) -> io::Result<Box<dyn ExchangeIo>> {
        self.inner.start_exchange(connection, spec, auth)
    }
    fn poll_completion(&mut self, cx: &mut Context<'_>) -> Poll<Completion> {
        let polled = self.inner.poll_completion(cx);
        if let Poll::Ready(Completion::Connected {
            result: Err(failure),
            ..
        }) = &polled
        {
            self.reported
                .failures
                .lock()
                .unwrap()
                .push((Instant::now(), failure.code));
            self.reported.changed.notify_all();
        }
        polled
    }
    fn poll_dispose(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        self.inner.poll_dispose(cx)
    }
}
type Make = dyn Fn(&FactoryEnv<'_>) -> Box<dyn EndpointAdapter> + Send + Sync;
struct Factory {
    make: Box<Make>,
    reported: Arc<Reported>,
}
impl EndpointFactory for Factory {
    fn create(
        &self,
        _spec: &EndpointSpec,
        env: &FactoryEnv<'_>,
    ) -> Result<Box<dyn EndpointAdapter>, FactoryError> {
        Ok(Box::new(Withholding {
            inner: (self.make)(env),
            reported: self.reported.clone(),
        }))
    }
}

/// Opens one exchange of `key` on a host over the adapters `make` gives and asserts that the stalled setup ends by
/// itself, `Timeout`, at the pool's deadline, and that the opening fails.
fn ends_at_the_pool_deadline(key: Key, identity: Identity, make: Box<Make>) {
    let reported = Arc::new(Reported::default());
    let factory = Factory {
        make,
        reported: reported.clone(),
    };
    let mut host = ConnectionHost::build(
        Arc::new(factory),
        None,
        Arc::new(crate::transport_host::composition::process_safety()),
    )
    .unwrap();
    let handle = host.handle();
    let op = operation(&handle, "op-deadline");
    let spec =
        ExchangeSpec::git(key, identity, GitService::UploadPackAdvertisement, "/repo").unwrap();
    let started = Instant::now();
    let wait = op.open(spec).unwrap().wait();
    let first = reported.first_by(started + POOL_CONNECT + SLACK);
    let Some((at, code)) = first else {
        panic!(
            "the setup did not end by itself within the pool's deadline ({POOL_CONNECT:?} + {SLACK:?})"
        );
    };
    let elapsed = at.duration_since(started);
    assert_eq!(
        code,
        ErrorCode::Timeout,
        "the setup's own deadline ended it"
    );
    assert!(
        elapsed + Duration::from_secs(1) >= POOL_CONNECT,
        "it ended at the pool's deadline, not before: {elapsed:?}"
    );
    let opened = super::tests::block_on(wait);
    assert!(
        matches!(opened, Err(OpenFailure::Pool(_))),
        "the opening failed by the pool's bound: {:?}",
        opened.err()
    );
    super::tests::block_on(op.finish());
    assert!(super::tests::block_on(host.close()));
}

#[test]
fn p3_2e_an_ssh_setup_with_no_connect_bound_ends_at_the_pool_deadline() {
    let settings = AdapterSettings {
        // None of its own: only the pool's deadline, carried in the command, bounds it.
        connect_timeout_ms: 0,
        io_timeout_ms: 20_000,
        interaction_timeout_ms: 20_000,
        cleanup_timeout_ms: 2_000,
    };
    ends_at_the_pool_deadline(
        Key::ssh("git", "127.0.0.1", 22),
        Identity::Ambient,
        Box::new(move |_| {
            Box::new(SshAdapter::new(
                |origin| {
                    SetupConnector::isolated(origin, Duration::from_millis(500), |_, _, _| {
                        // A setup that never completes, ending only when its job's deadline (or a cancel) says so.
                        let setup: Setup = Box::new(|control| {
                            loop {
                                control.check()?;
                                thread::sleep(Duration::from_millis(5));
                            }
                        });
                        Ok(setup)
                    })
                },
                settings,
            ))
        }),
    );
}

#[test]
fn p3_2e_an_https_setup_whose_bound_is_longer_ends_at_the_pool_deadline() {
    // A listener that takes the TCP connection and never answers the TLS handshake.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let inputs = HttpsInputs {
        tls: https_connection::Config::default(),
        helper_slots: HelperSlots::new(),
        authority: Authority::new(256, 256),
        settings: HttpsSettings {
            allocation_timeout_ms: 120_000,
            // Longer than the pool's: the earlier, the pool's, holds.
            connect_timeout_ms: 120_000,
            io_timeout_ms: 20_000,
            interaction_timeout_ms: 20_000,
            cleanup_timeout_ms: 2_000,
        },
        counters: Counters::default(),
    };
    ends_at_the_pool_deadline(
        Key::https("127.0.0.1", port),
        Identity::Https,
        Box::new(move |env| {
            let adapter = HttpsAdapter::new(inputs.clone(), env.safety().clone());
            Box::new(SchemeRouter::new(None, Some(adapter)))
        }),
    );
    drop(listener);
}

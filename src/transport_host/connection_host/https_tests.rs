//! The connection host over the HTTPS adapter, against the fixture HTTPS servers (plan step 1.3c; contract sections
//! 4.1, 6, 7.2, 7.3, 8 and 9). Each host runs on the composition's own ports (`DriverThread`), so nothing here polls
//! on the host's behalf: the rows wait on the host's snapshot or on the exchange's stream. The adapters are made as the
//! production factory makes them (a `SchemeRouter` over an `HttpsAdapter`), from the fixture server's configuration;
//! the factory keeps each adapter's scope records for the rows to read.
use super::{
    ConnectionHost,
    tests::{block_on, operation, wait_for},
};
use crate::{
    git::endpoint::{
        cut_proxy::CutProxy,
        https_adapter::{Binding, HttpsAdapter, HttpsInputs, HttpsSettings, Scopes},
        https_auth::{self, HelperSlots},
        https_fixture::{Behaviour, ConnectionId as Served, Handler, Server, response},
        https_worker::native::NativeCaller,
        shared_reservation::Authority,
    },
    transport_host::connection_factory::SchemeRouter,
};
use bytes::Bytes;
use gwz_connection::{
    AuthContext, AuthScope, EndpointProfile, EndpointRoute, OpenFailure, Operation,
    OperationPolicy, PreparedOperation,
};
use gwz_connection::{
    EndpointAdapter, EndpointFactory, EndpointSpec, Exchange, ExchangeSpec, FactoryEnv,
    FactoryError, GitService, HostHandle, HostPhase, ProcessSafety, SafetyHold, SafetyState,
    SafetyUnavailable, SharedProcessSafety, StreamError,
};
use gwz_transport::{
    pool::{Identity, Key},
    protocol::{self, Disposition},
};
use hyper::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use std::{
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub(super) const ADVERTISEMENT: &[u8] = b"001e# service=git-upload-pack\n0000";

fn settings() -> HttpsSettings {
    HttpsSettings {
        allocation_timeout_ms: 20_000,
        connect_timeout_ms: 20_000,
        io_timeout_ms: 20_000,
        interaction_timeout_ms: 20_000,
        cleanup_timeout_ms: 2_000,
    }
}
fn inputs(server: &Server, native: Option<NativeCaller>) -> HttpsInputs {
    HttpsInputs {
        tls: server.config(),
        helper_slots: HelperSlots::new(),
        native,
        authority: Authority::new(256, 256),
        settings: settings(),
    }
}

/// Makes the production adapter shape over `inputs`, keeping each adapter's scope records.
struct Factory {
    inputs: HttpsInputs,
    scopes: Arc<Mutex<Vec<Scopes>>>,
}
impl EndpointFactory for Factory {
    fn create(
        &self,
        _spec: &EndpointSpec,
        env: &FactoryEnv<'_>,
    ) -> Result<Box<dyn EndpointAdapter>, FactoryError> {
        let adapter = HttpsAdapter::new(self.inputs.clone(), env.safety().clone());
        self.scopes.lock().unwrap().push(adapter.scopes());
        Ok(Box::new(SchemeRouter::new(None, Some(adapter))))
    }
}

/// A gate that cannot demonstrate its scope (contract 7.3): the composition's when its Windows object is unavailable.
pub(super) struct Unavailable;
impl ProcessSafety for Unavailable {
    fn state(&self) -> SafetyState {
        SafetyState::Unavailable(SafetyUnavailable::ObjectNotCreated)
    }
    fn hold(&self) -> Result<SafetyHold, SafetyUnavailable> {
        Err(SafetyUnavailable::ObjectNotCreated)
    }
}

/// What the fixture server saw of each request: whether it carried `Authorization`, and on which connection.
pub(super) type Seen = Arc<Mutex<Vec<(bool, usize)>>>;
fn recording(
    seen: &Seen,
    answer: impl Fn(bool) -> hyper::Response<http_body_util::Full<Bytes>> + Send + Sync + 'static,
) -> Handler {
    let (seen, answer) = (seen.clone(), Arc::new(answer));
    Arc::new(move |request| {
        let authorized = request.headers().contains_key(AUTHORIZATION);
        let served = request.extensions().get::<Served>().map_or(0, |id| id.0);
        seen.lock().unwrap().push((authorized, served));
        let answer = answer.clone();
        Box::pin(async move { answer(authorized) })
    })
}
fn advertised(status: u16) -> hyper::Response<http_body_util::Full<Bytes>> {
    response(
        status,
        protocol::GitService::UploadPackAdvertisement,
        if status == 200 { ADVERTISEMENT } else { b"" },
    )
}
/// 401 with `offers` until a request carries `Authorization`, then the advertisement.
pub(super) fn challenging(seen: &Seen, offers: &'static str) -> Handler {
    recording(seen, move |authorized| {
        if authorized {
            return advertised(200);
        }
        let mut challenge = advertised(401);
        challenge
            .headers_mut()
            .insert(WWW_AUTHENTICATE, offers.parse().unwrap());
        challenge
    })
}

pub(super) struct Case {
    /// The fixture server runs on it.
    _runtime: tokio::runtime::Runtime,
    server: Server,
    proxy: CutProxy,
    pub(super) seen: Seen,
    scopes: Arc<Mutex<Vec<Scopes>>>,
    host: Mutex<ConnectionHost>,
    pub(super) handle: HostHandle,
    /// The helper `Case::operation` gives its operations (their own, in their contexts: the instance holds none).
    helper: Option<https_auth::Config>,
}
impl Case {
    fn new(handler: impl FnOnce(&Seen) -> Handler) -> Self {
        Self::with(handler, Behaviour::default(), |_| (None, None), None)
    }
    /// A case over the fixture server `handler` makes, behaving as `behaviour`, with the helper configuration and
    /// native caller `routes` gives (from the case's temporary directory), on the gate `safety` (the composition's
    /// when `None`).
    pub(super) fn with(
        handler: impl FnOnce(&Seen) -> Handler,
        behaviour: Behaviour,
        routes: impl FnOnce(&std::path::Path) -> (Option<https_auth::Config>, Option<NativeCaller>),
        safety: Option<SharedProcessSafety>,
    ) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let seen = Seen::default();
        let server = runtime.block_on(Server::start_behaving(handler(&seen), behaviour));
        let proxy = CutProxy::start(port_of(&server));
        let directory = tempfile::tempdir().unwrap();
        let (helper, native) = routes(directory.path());
        // The helper fixture's files stay for the case's life.
        std::mem::forget(directory);
        let scopes = Arc::default();
        let factory = Factory {
            inputs: inputs(&server, native),
            scopes: Arc::clone(&scopes),
        };
        let safety = safety
            .unwrap_or_else(|| Arc::new(crate::transport_host::composition::process_safety()));
        let host = ConnectionHost::build(Arc::new(factory), None, safety).unwrap();
        let handle = host.handle();
        Self {
            _runtime: runtime,
            server,
            proxy,
            seen,
            scopes,
            host: Mutex::new(host),
            handle,
            helper,
        }
    }
    /// An operation whose context holds the case's helper (or none: a context of another provider).
    pub(super) fn operation(&self, name: &str) -> Operation {
        operation_of(&self.handle, name, context_of(self.helper.clone()))
    }
    /// The spec of an advertisement through the proxy (`through`) or straight to the server.
    fn spec(&self, through: bool) -> ExchangeSpec {
        self.spec_for(through, GitService::UploadPackAdvertisement)
    }
    fn spec_for(&self, through: bool, service: GitService) -> ExchangeSpec {
        self.spec_at(through, service, "/repo")
    }
    fn spec_at(&self, through: bool, service: GitService, path: &str) -> ExchangeSpec {
        let port = if through {
            self.proxy.port
        } else {
            port_of(&self.server)
        };
        ExchangeSpec::git(
            Key::https("localhost", port),
            Identity::Https,
            service,
            path,
        )
        .unwrap()
    }
    /// An exchange of `service` at the repository `path` through the proxy.
    pub(super) fn exchange_at(
        &self,
        operation: &Operation,
        service: GitService,
        path: &str,
    ) -> Exchange {
        block_on(
            operation
                .open(self.spec_at(true, service, path))
                .unwrap()
                .wait(),
        )
        .unwrap()
    }
    /// An exchange of `service` through the proxy, once the host has leased it a connection.
    pub(super) fn exchange_of(&self, operation: &Operation, service: GitService) -> Exchange {
        block_on(operation.open(self.spec_for(true, service)).unwrap().wait()).unwrap()
    }
    pub(super) fn exchange(
        &self,
        operation: &gwz_connection::Operation,
        through: bool,
    ) -> Exchange {
        block_on(operation.open(self.spec(through)).unwrap().wait()).unwrap()
    }
    pub(super) fn wait(&self, what: &str, done: impl Fn(gwz_transport::pool::Counts) -> bool) {
        wait_for(what, || done(self.handle.snapshot().connections));
    }
    /// The one adapter's scope records.
    pub(super) fn scopes(
        &self,
    ) -> Vec<(
        gwz_transport::pool::ConnectionId,
        crate::git::endpoint::https_adapter::ConnectionScope,
    )> {
        let scopes = self.scopes.lock().unwrap();
        assert_eq!(scopes.len(), 1, "one endpoint instance");
        scopes[0].all()
    }
    pub(super) fn accepted(&self) -> usize {
        self.server.connections.load(Ordering::SeqCst)
    }
    pub(super) fn close(&self) -> bool {
        block_on(self.host.lock().unwrap().close())
    }
}
/// The authentication context of an operation whose credential helper is `helper`: core's own provider, whose scope is
/// the digest of the helper's executable and environment. Without one the provider is another's, which holds no helper.
pub(super) fn context_of(helper: Option<https_auth::Config>) -> AuthContext {
    match helper {
        Some(helper) => https_auth::OperationAuth::context(helper),
        None => AuthContext::new(64, AuthScope::new([0; 32]), Arc::new(())),
    }
}
/// An operation over `auth`, registered on `handle`.
pub(super) fn operation_of(handle: &HostHandle, name: &str, auth: AuthContext) -> Operation {
    let input = PreparedOperation::new(
        name,
        EndpointRoute::ssh_and_https(),
        EndpointProfile::new(b"fixture-https".to_vec(), [7; 32]).unwrap(),
        auth,
        OperationPolicy::new(4, 32, 3).unwrap(),
    )
    .unwrap();
    block_on(handle.admit(input).unwrap().acquire())
        .unwrap()
        .register()
}
fn port_of(server: &Server) -> u16 {
    let authority = server.url.trim_start_matches("https://localhost:");
    authority.split('/').next().unwrap().parse().unwrap()
}

/// Reads the exchange's response body to its end.
pub(super) fn body(exchange: &mut Exchange) -> Result<Vec<u8>, StreamError> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut out = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        assert!(Instant::now() < deadline, "the exchange did not end");
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
fn served(handler_seen: &Seen) -> Handler {
    recording(handler_seen, |_| advertised(200))
}

#[test]
fn finish_a_then_b_reuses_the_idle_https_connection_through_the_host() {
    let case = Case::new(served);
    for name in ["op-a", "op-b"] {
        let operation = operation(&case.handle, name);
        let mut exchange = case.exchange(&operation, true);
        assert_eq!(body(&mut exchange).unwrap(), ADVERTISEMENT);
        assert_eq!(
            block_on(exchange.finish()).disposition,
            Disposition::Reusable
        );
        block_on(operation.finish());
        case.wait("the connection is idle", |counts| {
            (counts.idle, counts.leased) == (1, 0)
        });
    }
    assert_eq!(case.accepted(), 1, "one HTTPS connection served A and B");
    let scopes = case.scopes();
    assert_eq!(scopes.len(), 1);
    assert_eq!(scopes[0].1.binding, Binding::Anonymous);
    assert!(
        !scopes[0].1.binding.per_exchange(),
        "no credential bound it"
    );
    assert!(case.close());
}

#[test]
fn a_dead_idle_https_connection_is_detected_and_discarded() {
    let case = Case::with(
        served,
        Behaviour {
            idle: Some(Duration::from_millis(300)),
            ..Behaviour::default()
        },
        |_| (None, None),
        None,
    );
    let a = operation(&case.handle, "op-a");
    let mut exchange = case.exchange(&a, false);
    body(&mut exchange).unwrap();
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Reusable
    );
    block_on(a.finish());
    case.wait("idle", |counts| counts.idle == 1);
    // The server closes its idle keep-alive connection: with no operation and no caller, the host's driver drops it.
    case.wait("the dead idle connection is discarded", |counts| {
        counts.total() == 0
    });
    let b = operation(&case.handle, "op-b");
    let mut exchange = case.exchange(&b, false);
    assert_eq!(body(&mut exchange).unwrap(), ADVERTISEMENT);
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Reusable
    );
    assert_eq!(
        case.accepted(),
        2,
        "the next operation got a fresh connection"
    );
    block_on(b.finish());
    assert!(case.close());
}

#[test]
fn a_second_https_opening_at_per_host_one_ends_in_allocation_timeout_at_the_bound() {
    // Phase 1 remediation R1 (Code P2-1 = State P2-1; the State review's REVIEW18-ALLOC): the first exchange holds the
    // only connection the operation may have, and the host is quiet, so nothing wakes the driver but the second
    // opening's own submission. The opening still ends, typed, at the pool's allocation bound (the default, 30 s: the
    // host takes no shorter bound from its operations, so the row waits it out).
    let case = Case::new(served);
    let input = PreparedOperation::new(
        "op-limited",
        EndpointRoute::ssh_and_https(),
        EndpointProfile::new(b"fixture-https".to_vec(), [7; 32]).unwrap(),
        context_of(None),
        OperationPolicy::new(4, 1, 3).unwrap(),
    )
    .unwrap();
    let operation = block_on(case.handle.admit(input).unwrap().acquire())
        .unwrap()
        .register();
    let first = case.exchange(&operation, true);
    case.wait("the one connection is leased", |counts| counts.leased == 1);
    // The driver settles and parks with no deadline of its own.
    thread::sleep(Duration::from_secs(1));
    let bound = Duration::from_millis(gwz_transport::pool::Config::default().allocation_timeout_ms);
    let slack = Duration::from_secs(5);
    let started = Instant::now();
    let mut second = Box::pin(operation.open(case.spec(true)).unwrap().wait());
    let waker = std::task::Waker::noop();
    let outcome = loop {
        if let std::task::Poll::Ready(outcome) = second
            .as_mut()
            .poll(&mut std::task::Context::from_waker(waker))
        {
            break outcome;
        }
        assert!(
            started.elapsed() < bound + slack,
            "the opening was still undecided {:?} after it was made",
            started.elapsed()
        );
        thread::sleep(Duration::from_millis(20));
    };
    let elapsed = started.elapsed();
    assert!(
        matches!(
            outcome,
            Err(OpenFailure::Pool(
                gwz_transport::pool::Error::AllocationTimeout
            ))
        ),
        "{:?}",
        outcome.err()
    );
    assert!(
        elapsed + Duration::from_secs(1) >= bound,
        "decided at {elapsed:?}, before the bound {bound:?}"
    );
    assert_eq!(
        case.handle.snapshot().connections.leased,
        1,
        "the first is untouched"
    );
    drop(second);
    first.abort();
    block_on(operation.finish());
    assert!(case.close());
}

#[test]
fn a_binding_fault_closes_only_that_binding() {
    let case = Case::new(served);
    let a = operation(&case.handle, "op-a");
    let b = operation(&case.handle, "op-b");
    let mut exchange_a = case.exchange(&a, true);
    let mut exchange_b = case.exchange(&b, true);
    assert_eq!(case.handle.snapshot().connections.leased, 2);
    // A's binding is cancelled: its stream fails by name, its connection is closed.
    a.cancel_handle().cancel();
    assert!(matches!(body(&mut exchange_a), Err(StreamError::Cancelled)));
    assert_eq!(
        block_on(exchange_a.finish()).disposition,
        Disposition::Discarded
    );
    // B's exchange, on its own connection, goes on and returns it to idle.
    assert_eq!(body(&mut exchange_b).unwrap(), ADVERTISEMENT);
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
    // The first request is held until the row releases it; every later one is answered at once.
    let release = Arc::new(tokio::sync::Notify::new());
    let held = release.clone();
    let requests = Arc::new(AtomicUsize::new(0));
    let arrived = requests.clone();
    let case = Case::new(move |_| {
        Arc::new(move |_| {
            let (held, first) = (held.clone(), requests.fetch_add(1, Ordering::SeqCst) == 0);
            Box::pin(async move {
                if first {
                    held.notified().await;
                }
                advertised(200)
            })
        })
    });
    let a = operation(&case.handle, "op-a");
    let mut exchange = case.exchange(&a, true);
    wait_for("the request reached the server", || {
        arrived.load(Ordering::SeqCst) == 1
    });
    // The connection dies under the leased exchange.
    case.proxy.cut_all();
    assert!(body(&mut exchange).is_err());
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Discarded
    );
    release.notify_waiters();
    case.wait("the broken connection is gone", |counts| {
        counts.total() == 0
    });
    assert_eq!(
        case.handle.snapshot().phase,
        HostPhase::Open,
        "the host is intact"
    );
    assert!(!block_on(a.finish()).cancelled);
    let b = operation(&case.handle, "op-b");
    let mut exchange = case.exchange(&b, true);
    assert_eq!(body(&mut exchange).unwrap(), ADVERTISEMENT);
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Reusable
    );
    assert_eq!(case.accepted(), 2);
    block_on(b.finish());
    assert!(case.close());
}

#[test]
fn the_runtime_builds_its_https_adapters_through_the_production_factory() {
    use crate::transport_host::{
        HttpsEndpointConfig, SshEndpointConfig, TransportRuntime, https_tests::endpoint_home,
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let seen = Seen::default();
    let server = runtime.block_on(Server::start(served(&seen)));
    let root = tempfile::tempdir().unwrap();
    let transport = TransportRuntime::with_https(
        SshEndpointConfig::fixture(endpoint_home(root.path()), None),
        HttpsEndpointConfig {
            tls: server.config(),
            auth: None,
        },
        HelperSlots::new(),
    )
    .unwrap();
    let (handle, _) = transport.connection_for_test().unwrap();
    let operation = operation(&handle, "op-factory");
    let spec = ExchangeSpec::git(
        Key::https("localhost", port_of(&server)),
        Identity::Https,
        GitService::UploadPackAdvertisement,
        "/repo",
    )
    .unwrap();
    let mut exchange = block_on(operation.open(spec).unwrap().wait()).unwrap();
    assert_eq!(body(&mut exchange).unwrap(), ADVERTISEMENT);
    assert_eq!(
        block_on(exchange.finish()).disposition,
        Disposition::Reusable
    );
    block_on(operation.finish());
    assert_eq!(handle.snapshot().connections.idle, 1);
    assert_eq!(block_on(transport.shutdown()).pending_local_work, 0);
    assert_eq!(handle.snapshot().phase, HostPhase::Closed);
    drop(runtime);
}

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
fn inputs(
    server: &Server,
    auth: Option<https_auth::Config>,
    native: Option<NativeCaller>,
) -> HttpsInputs {
    HttpsInputs {
        tls: server.config(),
        auth,
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
        let (auth, native) = routes(directory.path());
        // The helper fixture's files stay for the case's life.
        std::mem::forget(directory);
        let scopes = Arc::default();
        let factory = Factory {
            inputs: inputs(&server, auth, native),
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
        }
    }
    /// The spec of an advertisement through the proxy (`through`) or straight to the server.
    fn spec(&self, through: bool) -> ExchangeSpec {
        let port = if through {
            self.proxy.port
        } else {
            port_of(&self.server)
        };
        ExchangeSpec::git(
            Key::https("localhost", port),
            Identity::Https,
            GitService::UploadPackAdvertisement,
            "/repo",
        )
        .unwrap()
    }
    pub(super) fn exchange(
        &self,
        operation: &gwz_connection::Operation,
        through: bool,
    ) -> Exchange {
        block_on(operation.open(self.spec(through)).unwrap().wait()).unwrap()
    }
    fn wait(&self, what: &str, done: impl Fn(gwz_transport::pool::Counts) -> bool) {
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
    fn accepted(&self) -> usize {
        self.server.connections.load(Ordering::SeqCst)
    }
    pub(super) fn close(&self) -> bool {
        block_on(self.host.lock().unwrap().close())
    }
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

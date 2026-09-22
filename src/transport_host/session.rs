use super::*;
use crate::git::endpoint::{
    placement_endpoint::{EndpointError, PlacementEndpoint},
    ssh_channel::GitService,
    ssh_destination::Destination,
    stream_io::BlockingStream,
};
use gwz_transport::{
    mux::{self, Mux, Owner, Phase},
    protocol::*,
    stream::{self, MessageEndpoint, Stream},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::{Future, poll_fn},
    io,
    pin::pin,
    sync::{
        Condvar,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    task::{Context, Poll, Waker},
    thread,
    time::Instant,
};
mod driver;
pub type Attachment = (String, Envelope);
const CLEANUP: Duration = Duration::from_secs(5);
const CHECK_MS: u64 = 120_000;
pub(super) fn limits() -> Limits {
    let mut limits = binding::default_limits();
    // Match the existing endpoint stream's bounded receive window.
    limits.receive_window = 65536;
    limits
}
fn mux_config() -> mux::Config {
    mux::Config {
        limits: limits(),
        ..Default::default()
    }
}
static SERIAL: AtomicU64 = AtomicU64::new(1);
pub(super) fn unique() -> ModelResult<String> {
    SERIAL
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
        .map(|n| format!("placement-{}-{n}", std::process::id()))
        .map_err(|_| unavailable("transport IDs exhausted"))
}
fn protocol_failure(code: gwz_transport::protocol::ErrorCode) -> Failure {
    Failure {
        code,
        effect: Effect::None,
        facts: None,
    }
}
fn mux_error(error: mux::Error) -> ModelError {
    match error {
        mux::Error::InvalidRequest | mux::Error::Protocol => {
            invalid("invalid transport session message")
        }
        mux::Error::Rejected => unsupported("endpoint negotiation rejected"),
        _ => unavailable("transport session unavailable"),
    }
}
struct Wait<T: Clone> {
    value: Mutex<Option<T>>,
    changed: Condvar,
}
impl<T: Clone> Wait<T> {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            value: Mutex::new(None),
            changed: Condvar::new(),
        })
    }
    fn complete(&self, value: T) {
        let mut state = self.value.lock().unwrap_or_else(|e| e.into_inner());
        if state.is_none() {
            *state = Some(value);
            self.changed.notify_all();
        }
    }
    fn get(&self) -> T {
        let mut state = self.value.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(value) = &*state {
                return value.clone();
            }
            state = self.changed.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }
}
struct Entry {
    request: String,
    stream: Stream,
    peer: Arc<MessageEndpoint>,
    opened: bool,
    report_open_failure: bool,
    reply: Arc<Wait<Result<(BlockingStream, Opened), Failure>>>,
    deadline: Option<Instant>,
    pending: Option<Envelope>,
    observe: Arc<dyn Fn(i64, &Opened) + Send + Sync>,
    facts: Arc<dyn Fn(&Facts) + Send + Sync>,
}
struct Check {
    request: String,
    result: Arc<Wait<Result<(), Failure>>>,
}
struct Registration {
    operation: Option<String>,
    sealed: Option<Instant>,
    result: Option<CleanupReport>,
}
struct State {
    owner: Option<Owner>,
    port: Option<mux::Port>,
    session: Option<String>,
    registrations: BTreeMap<String, Registration>,
    used: BTreeSet<String>,
    closed: bool,
    streams: BTreeMap<i64, Entry>,
    checks: BTreeMap<i64, Check>,
    engine: Option<PlacementEndpoint>,
    https: Option<super::https_endpoint::HttpsEndpoint>,
    prefer_https: bool,
    endpoint_config: Option<binding::EndpointConfig>,
    pending: Option<Attachment>,
    incoming: Option<Attachment>,
    io_timeout_ms: u64,
}
struct Event {
    wakes: Mutex<BTreeMap<u64, Waker>>,
    next: AtomicU64,
}
impl Event {
    fn new() -> Self {
        Self {
            wakes: Mutex::new(BTreeMap::new()),
            next: AtomicU64::new(1),
        }
    }
    fn signal(&self) {
        let wakes = std::mem::take(&mut *self.wakes.lock().unwrap_or_else(|e| e.into_inner()));
        for (_, wake) in wakes {
            wake.wake();
        }
    }
}
struct Listener<'a> {
    event: &'a Event,
    id: u64,
}
impl Listener<'_> {
    fn arm(&self, cx: &Context<'_>) -> bool {
        let mut wakes = self.event.wakes.lock().unwrap_or_else(|e| e.into_inner());
        if wakes.len() >= 128 && !wakes.contains_key(&self.id) {
            return false;
        }
        wakes.insert(self.id, cx.waker().clone());
        true
    }
}
impl Drop for Listener<'_> {
    fn drop(&mut self) {
        self.event
            .wakes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
    }
}
pub(super) struct Session {
    state: Mutex<State>,
    event: Event,
    origin: Instant,
}
impl Session {
    fn start(state: State) -> ModelResult<Arc<Self>> {
        let session = Arc::new(Self {
            state: Mutex::new(state),
            event: Event::new(),
            origin: Instant::now(),
        });
        let weak = Arc::downgrade(&session);
        thread::Builder::new()
            .name("gwz-placement".into())
            .spawn(move || {
                while let Some(session) = weak.upgrade() {
                    session.drive();
                    let done = {
                        let state = session.state.lock().unwrap_or_else(|e| e.into_inner());
                        state.closed
                            && state.engine.as_ref().is_none_or(|e| e.pending() == 0)
                            && state.https.as_ref().is_none_or(|e| e.pending() == 0)
                    };
                    session.event.signal();
                    drop(session);
                    if done {
                        break;
                    }
                    thread::park_timeout(Duration::from_millis(5));
                }
            })
            .map_err(|_| unavailable("transport supervisor unavailable"))?;
        Ok(session)
    }
    fn empty() -> State {
        State {
            owner: None,
            port: None,
            session: None,
            registrations: BTreeMap::new(),
            used: BTreeSet::new(),
            closed: false,
            streams: BTreeMap::new(),
            checks: BTreeMap::new(),
            engine: None,
            https: None,
            prefer_https: false,
            endpoint_config: None,
            pending: None,
            incoming: None,
            io_timeout_ms: 3000,
        }
    }
    pub(super) fn driver(io_timeout_ms: u64) -> ModelResult<(Arc<Self>, TransportPort)> {
        let mut state = Self::empty();
        state.io_timeout_ms = io_timeout_ms;
        let id = unique()?;
        let (owner, port) = Owner::new(Mux::initiator(&id, mux_config()).map_err(mux_error)?);
        state.owner = Some(owner);
        state.port = Some(port);
        state.session = Some(id);
        let session = Self::start(state)?;
        Ok((session.clone(), TransportPort(Arc::new(PortLease(session)))))
    }
    pub(super) fn endpoint(config: SshEndpointConfig) -> ModelResult<(Arc<Self>, TransportPort)> {
        Self::endpoint_with_https(config, None)
    }
    pub(super) fn endpoint_with_https(
        config: SshEndpointConfig,
        https: Option<HttpsEndpointConfig>,
    ) -> ModelResult<(Arc<Self>, TransportPort)> {
        let mut state = Self::empty();
        let id = unique()?;
        let authority = crate::git::endpoint::shared_reservation::Authority::new(
            config.pool.total,
            config.pool.per_host,
        );
        let ssh = ssh_local::connect_with_authority(
            config.pool.clone(),
            config.home.join(".ssh/known_hosts"),
            config.agent.clone(),
            config.io_timeout_ms,
            authority.clone(),
        )
        .map_err(|_| unavailable("SSH endpoint construction failed"))?;
        if let Some(https) = https {
            state.https = Some(super::https_endpoint::HttpsEndpoint::new(
                https,
                config.pool.clone(),
                config.io_timeout_ms,
                authority,
                id.clone(),
            )?);
        }
        let https_enabled = state.https.is_some();
        state.engine = Some(
            PlacementEndpoint::new(ssh, config.home, id.clone(), id.clone())
                .map_err(|_| unavailable("endpoint supervisor unavailable"))?,
        );
        state.endpoint_config = Some(binding::EndpointConfig {
            endpoint_id: id.clone(),
            trust_owner: id,
            role: EndpointRole::Driver,
            schemes: if https_enabled {
                vec![Scheme::Ssh, Scheme::Https]
            } else {
                vec![Scheme::Ssh]
            },
            policies: if https_enabled {
                vec![
                    AuthPolicy::SshAmbient,
                    AuthPolicy::SshExplicit,
                    AuthPolicy::Anonymous,
                    AuthPolicy::Gh,
                ]
            } else {
                vec![AuthPolicy::SshAmbient, AuthPolicy::SshExplicit]
            },
            limits: limits(),
        });
        let session = Self::start(state)?;
        Ok((session.clone(), TransportPort(Arc::new(PortLease(session)))))
    }
    fn listener(&self) -> Listener<'_> {
        Listener {
            event: &self.event,
            id: self.event.next.fetch_add(1, Ordering::Relaxed),
        }
    }
    pub(super) fn is_closed(&self) -> bool {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.closed
            || state
                .owner
                .as_ref()
                .is_some_and(|o| o.phase() == Phase::Closed)
    }
    pub(super) fn register(&self, request: &str, operation: Option<String>) -> ModelResult<()> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            return Err(unavailable("transport session is closed"));
        }
        if !request::identifier(request) || state.used.contains(request) || state.used.len() >= 256
        {
            return Err(invalid("invalid or exhausted request registration"));
        }
        if let Some(owner) = &state.owner {
            owner
                .register(request, operation.clone())
                .map_err(mux_error)?;
        }
        state.used.insert(request.into());
        state.registrations.insert(
            request.into(),
            Registration {
                operation,
                sealed: None,
                result: None,
            },
        );
        Ok(())
    }
    pub(super) fn begin(&self, request: &str) -> ModelResult<()> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .owner
            .as_ref()
            .ok_or_else(|| unavailable("no initiator"))?
            .begin(request)
            .map_err(mux_error)
    }
    pub(super) async fn ready(&self) -> ModelResult<()> {
        let owner = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .owner
            .clone()
            .ok_or_else(|| unavailable("no binding"))?;
        owner.ready().await.map_err(mux_error)
    }
    pub(super) fn cancel(&self, request: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(owner) = &state.owner {
            let _ = owner.cancel(request);
        }
        if let Some(record) = state.registrations.get_mut(request) {
            record.sealed.get_or_insert(Instant::now());
        }
        if let Some(engine) = &mut state.engine {
            engine.cancel_request(request);
        }
        if let Some(engine) = &mut state.https {
            engine.cancel_request(request);
        }
        if state
            .incoming
            .as_ref()
            .is_some_and(|item| item.0 == request)
        {
            state.incoming = None;
        }
        for entry in state.streams.values().filter(|e| e.request == request) {
            // A not-yet-open stream may be retired before its endpoint reply
            // arrives. Complete its independent blocking waiter before retiring
            // the stream so cancellation never relies on a peer acknowledgment.
            entry.reply.complete(Err(Failure {
                code: gwz_transport::protocol::ErrorCode::Cancelled,
                effect: Effect::Possible,
                facts: None,
            }));
            entry.stream.cancel();
        }
        for check in state.checks.values().filter(|c| c.request == request) {
            check.result.complete(Err(protocol_failure(
                gwz_transport::protocol::ErrorCode::Cancelled,
            )));
        }
        drop(state);
        self.event.signal();
    }
    pub(super) fn seal(&self, request: &str) {
        self.cancel(request);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let closed = state.closed
            || state
                .owner
                .as_ref()
                .is_some_and(|o| o.phase() == Phase::Closed);
        let pending = state
            .engine
            .as_ref()
            .map_or(0, |e| e.pending_request_count(request))
            + state
                .https
                .as_ref()
                .map_or(0, |e| e.pending_request_count(request));
        if let Some(record) = state.registrations.get_mut(request) {
            record.sealed.get_or_insert(Instant::now());
            if closed {
                record.result.get_or_insert(CleanupReport {
                    pending_local_work: pending,
                    peer_cleanup_confirmed: false,
                });
            }
        }
    }
    pub(super) async fn finish(&self, request: &str) -> CleanupReport {
        self.seal(request);
        let listener = self.listener();
        poll_fn(|cx| {
            if !listener.arm(cx) {
                self.close();
                return Poll::Ready(self.report());
            }
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            match state.registrations.get(request) {
                Some(r) if r.result.is_some() => Poll::Ready(r.result.clone().unwrap()),
                None => Poll::Ready(CleanupReport::default()),
                _ => Poll::Pending,
            }
        })
        .await
    }
    pub(super) fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Self::close_state(&mut state);
        drop(state);
        self.event.signal();
    }
    fn close_state(state: &mut State) {
        state.closed = true;
        if let Some(port) = &state.port {
            port.disconnect();
        }
        if let Some(engine) = &mut state.engine {
            engine.shutdown();
        }
        if let Some(engine) = &mut state.https {
            engine.shutdown();
        }
        for (_, entry) in std::mem::take(&mut state.streams) {
            entry.peer.disconnect();
            entry.reply.complete(Err(protocol_failure(
                gwz_transport::protocol::ErrorCode::CarrierLost,
            )));
        }
        for (_, check) in std::mem::take(&mut state.checks) {
            check.result.complete(Err(protocol_failure(
                gwz_transport::protocol::ErrorCode::CarrierLost,
            )));
        }
        state.pending = None;
        state.incoming = None;
    }
    fn report(&self) -> CleanupReport {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        CleanupReport {
            pending_local_work: state.streams.len()
                + state.checks.len()
                + state.engine.as_ref().map_or(0, |e| e.pending())
                + state.https.as_ref().map_or(0, |e| e.pending()),
            peer_cleanup_confirmed: false,
        }
    }
    pub(super) async fn cleanup(&self) -> CleanupReport {
        let deadline = Instant::now() + CLEANUP;
        let listener = self.listener();
        poll_fn(|cx| {
            let armed = listener.arm(cx);
            let report = self.report();
            if !armed || report.pending_local_work == 0 || Instant::now() >= deadline {
                Poll::Ready(report)
            } else {
                Poll::Pending
            }
        })
        .await
    }
    fn inbound_port(&self, attachment: &Attachment) -> ModelResult<mux::Port> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            return Err(unavailable("transport port is closed"));
        }
        if state.owner.is_none() {
            if attachment.1.kind != MessageKind::Bind
                || !state.registrations.contains_key(&attachment.0)
                || gwz_transport::codec::admit(&attachment.1).is_err()
            {
                Self::close_state(&mut state);
                return Err(invalid("invalid endpoint bootstrap"));
            }
            let config = state
                .endpoint_config
                .clone()
                .ok_or_else(|| invalid("endpoint missing"))?;
            let mux =
                Mux::endpoint(&attachment.1.session_id, config, mux_config()).map_err(mux_error)?;
            let (owner, port) = Owner::new(mux);
            owner.advance(self.origin.elapsed().as_millis() as u64);
            for (id, record) in &state.registrations {
                owner
                    .register(id, record.operation.clone())
                    .map_err(mux_error)?;
                if record.sealed.is_some() {
                    let _ = owner.cancel(id);
                }
            }
            state.session = Some(attachment.1.session_id.clone());
            state.owner = Some(owner);
            state.port = Some(port);
        }
        Ok(state.port.as_ref().expect("initialized port").clone())
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        Self::close_state(self.state.get_mut().unwrap_or_else(|e| e.into_inner()));
    }
}
struct PortLease(Arc<Session>);
impl Drop for PortLease {
    fn drop(&mut self) {
        self.0.close();
    }
}
#[derive(Clone)]
pub struct TransportPort(Arc<PortLease>);
impl TransportPort {
    pub async fn next_message(&self) -> ModelResult<Option<Attachment>> {
        let session = &self.0.0;
        let listener = session.listener();
        let port = poll_fn(|cx| {
            if !listener.arm(cx) {
                session.close();
                return Poll::Ready(Err(unavailable("transport waiter capacity")));
            }
            let state = session.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed {
                Poll::Ready(Ok(None))
            } else if let Some(port) = &state.port {
                Poll::Ready(Ok(Some(port.clone())))
            } else {
                Poll::Pending
            }
        })
        .await?;
        match port {
            Some(port) => port.next_message().await.map_err(mux_error),
            None => Ok(None),
        }
    }
    pub async fn deliver(&self, attachment: Attachment) -> ModelResult<()> {
        let result = self
            .0
            .0
            .inbound_port(&attachment)?
            .deliver(attachment)
            .await
            .map_err(mux_error);
        if result.is_err() {
            self.0.0.close();
        }
        self.0.0.event.signal();
        result
    }
    pub fn disconnect(&self) {
        self.0.0.close();
    }
}
pub(super) struct LocalLink {
    stop: Arc<AtomicBool>,
}
impl LocalLink {
    pub(super) fn new(core: TransportPort, peer: TransportPort) -> ModelResult<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        thread::Builder::new()
            .name("gwz-placement-local".into())
            .spawn(move || {
                let mut a = None;
                let mut b = None;
                let mut cx = Context::from_waker(Waker::noop());
                while !stopped.load(Ordering::Acquire) {
                    for (from, to, pending) in [(&core, &peer, &mut a), (&peer, &core, &mut b)] {
                        if pending.is_none() {
                            match pin!(from.next_message()).poll(&mut cx) {
                                Poll::Ready(Ok(Some(item))) => *pending = Some(item),
                                Poll::Ready(_) => {
                                    stopped.store(true, Ordering::Release);
                                    break;
                                }
                                Poll::Pending => {}
                            }
                        }
                        if let Some(item) = pending.as_ref() {
                            match pin!(to.deliver(item.clone())).poll(&mut cx) {
                                Poll::Ready(Ok(())) => *pending = None,
                                Poll::Ready(Err(_)) => {
                                    stopped.store(true, Ordering::Release);
                                    break;
                                }
                                Poll::Pending => {}
                            }
                        }
                    }
                    thread::park_timeout(Duration::from_millis(2));
                }
                core.disconnect();
                peer.disconnect();
            })
            .map_err(|_| unavailable("local forwarding unavailable"))?;
        Ok(Self { stop })
    }
    pub(super) fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }
}
impl Drop for LocalLink {
    fn drop(&mut self) {
        self.stop();
    }
}

cfg_if::cfg_if! { if #[cfg(test)] { #[path="cleanup_tests.rs"] mod cleanup_tests; } }

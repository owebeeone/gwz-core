//! Private HTTPS side of the existing placement session. The synchronous host
//! only admits messages and polls completion; an owned runtime drives HTTP.
use super::{Arc, Duration, HttpsEndpointConfig, ModelResult, NativeCaller, pool, unavailable};
use crate::git::endpoint::{
    https_auth::HelperSlots,
    https_operation::Refusal,
    https_policy,
    https_wake::CloseWake,
    https_worker::{
        Budget, ChallengeLease, Client, Endpoint as HttpEndpoint, FirstConnect, Input, Prepared,
        Rejection,
    },
    placement_endpoint::{EndpointError, Outbound},
    shared_reservation::Authority,
    shutdown_watch::Watch,
};
use gwz_transport::{
    protocol::*,
    stream::{self, MessageEndpoint, Stream},
};
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicBool, Ordering},
    task::{Context, Poll},
};
use tokio::{runtime::Handle, sync::oneshot, task::JoinHandle};
use tokio_util::sync::CancellationToken;

mod poll;
mod retry;
use retry::{Held, Retries, Settling};

type Key = (String, i64);
/// An attempt's preparation, what its first connect did, what the limit
/// machine made of a refusal, and the budget and challenge it leaves.
type Attempt = (
    Result<Prepared, Failure>,
    FirstConnect,
    Retry,
    Option<Rejection>,
);
struct Entry {
    envelope: Envelope,
    cancel: CancellationToken,
    preparing: Option<JoinHandle<Attempt>>,
    serving: Option<JoinHandle<()>>,
    prepared: Option<Prepared>,
    handoff: bool,
    opening_published: bool,
    /// Native authentication's fixed D survives preparation and mux backpressure.
    publication_deadline: Option<tokio::time::Instant>,
    publication_route: Option<Arc<crate::git::endpoint::https_worker::native::Authenticated>>,
    // Retain the application half until its terminal message is drained.
    stream: Option<Stream>,
    peer: Option<Arc<MessageEndpoint>>,
    /// Told whenever the stream changes, for the worker's wait to complete the close.
    wake: CloseWake,
    /// The wait for the peer's next message; the HTTP task's next write wakes it.
    next: Option<super::session::NextMessage>,
    output: Option<Envelope>,
    retired: bool,
    /// The HTTPS pool key whose retry machine decides this open's attempts.
    pool_key: pool::Key,
    /// The open while it waits for an attempt (retry.rs).
    held: Option<Held>,
    /// The attempts the open has started, against `--max-retries + 1` (§5.3).
    attempts: u32,
    /// The attempt in flight carries a test of the site's limit (§4.7).
    carries_test: Option<crate::git::endpoint::setup_retry::TestToken>,
    /// The facts of the open's attempts so far, which its one reply carries:
    /// progress on its diagnostic row (the retry plan's §5).
    facts: Option<Facts>,
}
struct Operation {
    name: String,
    _guard: crate::git::endpoint::https_operation::Dependency,
    retries: BTreeMap<String, Retry>,
}
struct Retry {
    budget: Budget,
    challenge: Option<ChallengeLease>,
}
pub(super) struct HttpsEndpoint {
    client: Client,
    runtime: Handle,
    shutdown: Option<oneshot::Sender<()>>,
    stopped: Arc<AtomicBool>,
    /// Woken when the endpoint's thread has ended and `stopped` is set.
    watch: Watch,
    entries: BTreeMap<Key, Entry>,
    operations: BTreeMap<String, Operation>,
    endpoint: String,
    trust_owner: String,
    shutting_down: bool,
    last_outbound: Option<Key>,
    /// Each operation's setup retry machines (retry.rs).
    retries: Retries,
    /// The latest time `step` was given, in its clock.
    now_ms: u64,
    /// The monotonic clock the publication checks read around the mux lock.
    clock: Clock,
}
impl HttpsEndpoint {
    cfg_if::cfg_if! { if #[cfg(test)] {
        pub(in crate::transport_host) fn client_for_test(&self) -> Client { self.client.clone() }
    } }
    pub(super) fn pool(&self) -> &pool::Pool {
        self.client.pool()
    }
    cfg_if::cfg_if! { if #[cfg(test)] {
    pub(super) fn new(
        config: HttpsEndpointConfig,
        pool: pool::Config,
        io_ms: u64,
        authority: Authority,
        endpoint: String,
        helper_slots: HelperSlots,
    ) -> ModelResult<Self> {
        Self::new_native(config, pool, io_ms, authority, endpoint, helper_slots, None)
    }
    } }
    pub(super) fn new_native(
        config: HttpsEndpointConfig,
        pool: pool::Config,
        io_ms: u64,
        authority: Authority,
        endpoint: String,
        helper_slots: HelperSlots,
        native: Option<NativeCaller>,
    ) -> ModelResult<Self> {
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let (shutdown, stop) = oneshot::channel();
        let stopped = Arc::new(AtomicBool::new(false));
        let finished = stopped.clone();
        let watch = Watch::default();
        let ended = watch.clone();
        std::thread::Builder::new().name("gwz-https".into()).spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(runtime) => runtime,
                Err(_) => { let _ = ready_tx.send(Err(())); return; }
            };
            runtime.block_on(async move {
                let mut endpoint = match HttpEndpoint::with_authority(config.tls, config.auth, pool, io_ms, authority, helper_slots) {
                    Ok(endpoint) => endpoint,
                    Err(_) => { let _ = ready_tx.send(Err(())); return; }
                };
                if let Some(native) = native { endpoint.client.set_native(native); }
                if ready_tx.send(Ok((endpoint.client.clone(), Handle::current()))).is_err() { return; }
                tokio::pin!(stop);
                loop {
                    tokio::select! {
                        _ = &mut stop => break,
                        _ = tokio::time::sleep(Duration::from_millis(5)) => {endpoint.client.reap_cleanup(Duration::from_millis(5)).await;},
                    }
                }
                // A closed host cannot orphan pending physical jobs. Keep the
                // runtime alive until the existing owner proves disposal.
                while endpoint.shutdown(Duration::from_millis(20)).await != 0 {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                finished.store(true, Ordering::Release);
                ended.notify();
            });
        }).map_err(|_| unavailable("HTTPS supervisor unavailable"))?;
        let (client, runtime) = ready_rx
            .recv()
            .map_err(|_| unavailable("HTTPS construction failed"))?
            .map_err(|_| unavailable("HTTPS construction failed"))?;
        Ok(Self {
            client,
            runtime,
            shutdown: Some(shutdown),
            stopped,
            watch,
            entries: BTreeMap::new(),
            operations: BTreeMap::new(),
            trust_owner: endpoint.clone(),
            endpoint,
            shutting_down: false,
            last_outbound: None,
            retries: Retries::new(),
            now_ms: 0,
            clock: Clock::monotonic(),
        })
    }
    /// The operation's `--max-retries`, which its admission installs before
    /// its first open. An operation never given one retries three times.
    pub(super) fn set_max_retries(&mut self, request: &str, max_retries: u32) {
        self.retries.set_max_retries(request, max_retries);
    }
    pub(super) fn owns(&self, request: &str, id: i64) -> bool {
        self.entries.contains_key(&(request.into(), id))
    }
    pub(super) fn accept(
        &mut self,
        request: String,
        envelope: Envelope,
    ) -> Result<(), EndpointError> {
        if self.shutting_down {
            return Err(EndpointError::Shutdown);
        }
        let key = (request.clone(), envelope.stream_id);
        if envelope.kind != MessageKind::Open {
            if let Some(entry) = self.entries.get_mut(&key) {
                if envelope.kind == MessageKind::Cancel {
                    cancel_entry(entry);
                    // Its attempt ends with no verdict, which frees a probe,
                    // and a held open starts none.
                    if entry.preparing.is_some() && !entry.retired {
                        self.retries.abandoned(&key, &entry.pool_key);
                    }
                    entry.held = None;
                    return Ok(());
                }
                if entry.cancel.is_cancelled() {
                    return Ok(());
                }
                if entry.prepared.is_some()
                    && matches!(envelope.kind, MessageKind::Data | MessageKind::EndWrite)
                {
                    entry.handoff = true;
                    let delivered = entry
                        .peer
                        .as_ref()
                        .ok_or(EndpointError::Protocol)?
                        .deliver(envelope);
                    entry.wake.notify();
                    delivered.map_err(|_| EndpointError::Protocol)?;
                    return Ok(());
                }
                if let Some(peer) = &entry.peer {
                    if !entry.retired {
                        let delivered = peer.deliver(envelope);
                        entry.wake.notify();
                        delivered.map_err(|_| EndpointError::Protocol)?;
                    }
                }
            }
            return Ok(());
        }
        if self.entries.len() >= 64 {
            return Err(EndpointError::WouldBlock);
        }
        if self.entries.contains_key(&key) {
            return Err(EndpointError::Duplicate);
        }
        let open = envelope
            .open
            .as_ref()
            .ok_or(EndpointError::InvalidRequest)?;
        if open.endpoint_id != self.endpoint || open.destination.scheme != Scheme::Https {
            return Err(EndpointError::InvalidRequest);
        }
        // Its connection's TLS configuration builds while the open is prepared.
        self.client.prebuild_tls(&self.runtime);
        if !self.operations.contains_key(&request) {
            if self.operations.len() >= 64 {
                return Err(EndpointError::WouldBlock);
            }
            // Request registration is the route lifetime owner. A remote or an
            // RPC dropping must not retire discovery routes used later by it.
            let name = super::session::unique().map_err(|_| EndpointError::Capacity)?;
            let guard = self
                .client
                .operation(&name)
                .map_err(|refusal| match refusal {
                    // The table is full of operations that still have dependents:
                    // the open waits, as it waits for a stream.
                    Refusal::WouldBlock => EndpointError::WouldBlock,
                    // A name made for this request is never sealed already.
                    Refusal::Sealed => EndpointError::InvalidRequest,
                })?;
            // The operation's limit machines start SATURATED at the per-host
            // limit its admission installed in the pool (adaptive concurrency
            // design §4.1), named as its attempts name it. They adapt unless
            // the operation retries nothing (`--max-retries 0`, §5.3).
            let capacity = self.pool().capacity();
            self.client.governor().begin_operation(
                &name,
                capacity.per_host.min(capacity.per_user_host),
                self.retries.max_retries(&request) > 0,
                self.client.pool_now(),
            );
            self.operations.insert(
                request.clone(),
                Operation {
                    name,
                    _guard: guard,
                    retries: BTreeMap::new(),
                },
            );
        }
        let operation = self
            .operations
            .get_mut(&request)
            .expect("registered operation");
        let retry_key = retry_key(open);
        let mut retry = if open.policy == AuthPolicy::Gh {
            operation
                .retries
                .remove(&retry_key)
                .unwrap_or_else(|| Retry {
                    budget: self.client.budget_for_open(&open.deadlines),
                    challenge: None,
                })
        } else {
            Retry {
                budget: self.client.budget_for_open(&open.deadlines),
                challenge: None,
            }
        };
        retry
            .budget
            .shorten(self.client.budget_for_open(&open.deadlines));
        // The key's retry machine decides when its first attempt starts.
        let pool_key =
            pool::Key::https(open.destination.host.clone(), open.destination.port as u16);
        let held = Held::new(Some(retry), self.now_ms, open.deadlines.allocation_ms);
        self.entries.insert(
            key.clone(),
            Entry {
                envelope,
                cancel: CancellationToken::new(),
                preparing: None,
                serving: None,
                prepared: None,
                handoff: false,
                opening_published: false,
                publication_deadline: None,
                publication_route: None,
                stream: None,
                peer: None,
                wake: CloseWake::default(),
                next: None,
                output: None,
                retired: false,
                pool_key,
                held: Some(held),
                attempts: 0,
                carries_test: None,
                facts: None,
            },
        );
        self.admit(&key);
        Ok(())
    }
    // Called on every retry of the host's pending outbound slot. Taking a
    // receipt is not publication; only successful mux send linearizes Opened.
    // This check is an early exit: `publication_check` decides again under
    // the mux lock.
    pub(super) fn before_handoff(&mut self, request: &str, message: &mut Envelope) {
        if message.kind != MessageKind::Opened {
            return;
        }
        if let Some(entry) = self.entries.get_mut(&(request.into(), message.stream_id)) {
            if let Some(code) =
                publication_refusal(&entry.cancel, entry.publication_deadline, &self.clock)
            {
                fail_publication(entry, message, code);
            }
        }
    }
    /// The final check of an Opened that carries native D, for the pump to
    /// run under the mux lock that would queue it (`Owner::send_if`): time
    /// can pass between `before_handoff` and that lock. None for any other
    /// message, which the pump sends as before.
    pub(super) fn publication_check(
        &self,
        request: &str,
        message: &Envelope,
    ) -> Option<PublicationCheck> {
        if message.kind != MessageKind::Opened {
            return None;
        }
        let entry = self.entries.get(&(request.into(), message.stream_id))?;
        Some(PublicationCheck {
            deadline: entry.publication_deadline?,
            cancel: entry.cancel.clone(),
            clock: self.clock.clone(),
        })
    }
    /// After a refusal under the mux lock: the failure `before_handoff` makes.
    pub(super) fn refuse_publication(
        &mut self,
        request: &str,
        message: &mut Envelope,
        code: ErrorCode,
    ) {
        if let Some(entry) = self.entries.get_mut(&(request.into(), message.stream_id)) {
            fail_publication(entry, message, code);
        }
    }
    pub(super) fn handed_off(&mut self, request: &str, message: &Envelope) {
        if message.kind == MessageKind::Opened {
            if let Some(entry) = self.entries.get_mut(&(request.into(), message.stream_id)) {
                entry.opening_published = true;
                if entry.publication_deadline.is_some()
                    && https_policy::advertisement(
                        entry.envelope.open.as_ref().expect("Open").service,
                    )
                    && entry.prepared.is_some()
                {
                    entry.handoff = true;
                }
                entry.publication_deadline = None;
                entry.publication_route = None;
            }
        }
    }
    pub(super) fn cancel_request(&mut self, request: &str) {
        for ((id, _), entry) in &mut self.entries {
            if id == request {
                entry.cancel.cancel();
                entry.prepared = None;
                entry.handoff = false;
                entry.retired = true;
                entry.output = None;
                entry.held = None;
                if let Some(peer) = &entry.peer {
                    peer.disconnect();
                }
            }
        }
        if let Some(operation) = self.operations.remove(request) {
            self.client.finish_operation(&operation.name);
            self.client
                .governor()
                .end_operation(&operation.name, self.client.pool_now());
        }
        // Its opens are all finished: no wake starts an attempt for it.
        self.retries.remove(request);
    }
    pub(super) fn pending_request_count(&self, request: &str) -> usize {
        self.entries.keys().filter(|(id, _)| id == request).count() + self.client.pending_cleanup()
    }
    pub(super) fn pending(&self) -> usize {
        self.entries.len()
            + if self.shutting_down {
                usize::from(!self.stopped.load(Ordering::Acquire))
            } else {
                self.client.pending_cleanup()
            }
    }
    pub(super) fn shutdown(&mut self) {
        if self.shutting_down {
            return;
        }
        self.shutting_down = true;
        let requests: Vec<_> = self.operations.keys().cloned().collect();
        for request in requests {
            self.cancel_request(&request);
        }
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}
impl Drop for HttpsEndpoint {
    fn drop(&mut self) {
        self.shutdown();
    }
}
fn retry_key(open: &Open) -> String {
    format!(
        "{:?}|{}|{}|{}|{:?}",
        open.service,
        open.destination.host,
        open.destination.port,
        open.destination.path,
        open.destination.https_username
    )
}

fn cancel_entry(entry: &mut Entry) {
    entry.cancel.cancel();
    entry.handoff = false;
    if !entry.opening_published {
        let facts = entry
            .output
            .as_ref()
            .and_then(|m| m.opened.as_ref())
            .map(|o| o.facts.clone());
        entry.prepared = None;
        entry.output = Some(cancelled_open(&entry.envelope, facts));
    } else if let Some(prepared) = entry.prepared.take() {
        // Opened was published, but the initiator supplied no POST data yet.
        if let Some(peer) = &entry.peer {
            let _ = peer.fail_terminal(Failure {
                detail: None,
                setup_cause: None,
                code: ErrorCode::Cancelled,
                effect: Effect::None,
                facts: Some(prepared.opened.facts.clone()),
            });
        }
    }
    // Retain preparing/serving handles. Their owners settle cancellation and
    // physical disposal; after handoff the HTTP task owns terminal effects.
}
fn publication_expired(deadline: Option<tokio::time::Instant>, now: tokio::time::Instant) -> bool {
    deadline.is_some_and(|until| now >= until)
}
/// A native Opened's final check (`HttpsEndpoint::publication_check`).
pub(super) struct PublicationCheck {
    deadline: tokio::time::Instant,
    cancel: CancellationToken,
    clock: Clock,
}
impl PublicationCheck {
    /// Why the Opened may not publish, read now.
    pub(super) fn refusal(self) -> Option<ErrorCode> {
        publication_refusal(&self.cancel, Some(self.deadline), &self.clock)
    }
}
/// The open's cancellation first, then a fresh clock reading against D, at
/// which the Opened has expired.
fn publication_refusal(
    cancel: &CancellationToken,
    deadline: Option<tokio::time::Instant>,
    clock: &Clock,
) -> Option<ErrorCode> {
    if cancel.is_cancelled() {
        Some(ErrorCode::Cancelled)
    } else if publication_expired(deadline, clock.now()) {
        Some(ErrorCode::Timeout)
    } else {
        None
    }
}
/// Replaces an Opened that must not publish with its open's one terminal
/// failure. The facts it observed stay; the authenticated route is revoked and
/// prepared ownership discarded, while physical and native cleanup stay
/// charged to their owners until disposal.
fn fail_publication(entry: &mut Entry, message: &mut Envelope, code: ErrorCode) {
    let facts = message.opened.as_ref().map(|opened| opened.facts.clone());
    *message = failed_open(&entry.envelope, code, facts);
    entry.cancel.cancel();
    if let Some(route) = &entry.publication_route {
        route.revoke();
    }
    entry.output = None;
    entry.prepared = None;
    entry.handoff = false;
    entry.retired = true;
    if let Some(peer) = &entry.peer {
        peer.disconnect();
    }
}
cfg_if::cfg_if! { if #[cfg(test)] {
    /// tokio's monotonic clock, or the readings a test scripts in its place.
    #[derive(Clone)]
    struct Clock(Option<Arc<dyn Fn() -> tokio::time::Instant + Send + Sync>>);
    impl Clock {
        fn monotonic() -> Self {
            Self(None)
        }
        fn now(&self) -> tokio::time::Instant {
            self.0.as_ref().map_or_else(tokio::time::Instant::now, |read| read())
        }
    }
} else {
    /// tokio's monotonic clock.
    #[derive(Clone)]
    struct Clock;
    impl Clock {
        fn monotonic() -> Self {
            Self
        }
        fn now(&self) -> tokio::time::Instant {
            tokio::time::Instant::now()
        }
    }
} }
fn cancelled_open(envelope: &Envelope, facts: Option<Facts>) -> Envelope {
    failed_open(envelope, ErrorCode::Cancelled, facts)
}
fn failed_open(envelope: &Envelope, code: ErrorCode, facts: Option<Facts>) -> Envelope {
    Envelope {
        version: envelope.version,
        session_id: envelope.session_id.clone(),
        stream_id: envelope.stream_id,
        kind: MessageKind::OpenFailed,
        open_failed: Some(Failure {
            detail: None,
            setup_cause: None,
            code,
            effect: Effect::None,
            facts,
        }),
        ..Default::default()
    }
}

cfg_if::cfg_if! { if #[cfg(test)] {
    #[path = "cancellation_tests.rs"]
    mod cancellation_tests;
    #[path = "https_cancel_mux_tests.rs"]
    mod https_cancel_mux_tests;
    mod carrier_tests;
    mod down_tests;
    mod requeue_tests;
    mod retry_tests;
    mod stale_action_tests;
} }
// Needs no HTTPS server, so it runs on Windows too.
cfg_if::cfg_if! { if #[cfg(test)] { mod wake_tests; } }

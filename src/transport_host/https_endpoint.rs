//! Private HTTPS side of the existing placement session. The synchronous host
//! only admits messages and polls completion; an owned runtime drives HTTP.
use super::{Arc, Duration, HttpsEndpointConfig, ModelResult, pool, unavailable};
use crate::git::endpoint::{
    https_auth::HelperSlots,
    https_policy,
    https_worker::{
        Budget, ChallengeLease, Client, Endpoint as HttpEndpoint, FirstConnect, Input, Prepared,
    },
    placement_endpoint::{EndpointError, Outbound},
    shared_reservation::Authority,
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

mod retry;
use retry::{Held, Retries};

type Key = (String, i64);
/// An attempt's preparation, what its first connect did, and the budget and
/// challenge it leaves.
type Attempt = (Result<Prepared, Failure>, FirstConnect, Retry);
struct Entry {
    envelope: Envelope,
    cancel: CancellationToken,
    preparing: Option<JoinHandle<Attempt>>,
    serving: Option<JoinHandle<()>>,
    prepared: Option<Prepared>,
    handoff: bool,
    opening_published: bool,
    // Retain the application half until its terminal message is drained.
    stream: Option<Stream>,
    peer: Option<Arc<MessageEndpoint>>,
    /// The wait for the peer's next message; the HTTP task's next write wakes it.
    next: Option<super::session::NextMessage>,
    output: Option<Envelope>,
    retired: bool,
    /// The HTTPS pool key whose retry machine decides this open's attempts.
    pool_key: pool::Key,
    /// The open while it waits for an attempt (retry.rs).
    held: Option<Held>,
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
}
impl HttpsEndpoint {
    pub(super) fn pool(&self) -> &pool::Pool {
        self.client.pool()
    }
    pub(super) fn new(
        config: HttpsEndpointConfig,
        pool: pool::Config,
        io_ms: u64,
        authority: Authority,
        endpoint: String,
        helper_slots: HelperSlots,
    ) -> ModelResult<Self> {
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let (shutdown, stop) = oneshot::channel();
        let stopped = Arc::new(AtomicBool::new(false));
        let finished = stopped.clone();
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
            entries: BTreeMap::new(),
            operations: BTreeMap::new(),
            trust_owner: endpoint.clone(),
            endpoint,
            shutting_down: false,
            last_outbound: None,
            retries: Retries::new(),
            now_ms: 0,
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
                    entry
                        .peer
                        .as_ref()
                        .ok_or(EndpointError::Protocol)?
                        .deliver(envelope)
                        .map_err(|_| EndpointError::Protocol)?;
                    return Ok(());
                }
                if let Some(peer) = &entry.peer {
                    if !entry.retired {
                        peer.deliver(envelope)
                            .map_err(|_| EndpointError::Protocol)?;
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
                .map_err(|_| EndpointError::Capacity)?;
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
                stream: None,
                peer: None,
                next: None,
                output: None,
                retired: false,
                pool_key,
                held: Some(held),
                facts: None,
            },
        );
        self.admit(&key);
        Ok(())
    }
    pub(super) fn step(&mut self, now: u64, cx: &mut Context<'_>) -> Result<(), EndpointError> {
        self.now_ms = self.now_ms.max(now);
        for operation in self.operations.values_mut() {
            for retry in operation.retries.values_mut() {
                if retry
                    .challenge
                    .as_ref()
                    .is_some_and(ChallengeLease::expired)
                {
                    retry.challenge = None;
                }
            }
        }
        for ((request, _), entry) in &mut self.entries {
            if let Some(task) = entry.preparing.as_mut() {
                if let Poll::Ready(result) = Pin::new(task).poll(cx) {
                    entry.preparing = None;
                    let (mut result, connect, mut retry) =
                        result.map_err(|_| EndpointError::Protocol)?;
                    if entry.cancel.is_cancelled() && result.is_ok() {
                        let facts = result
                            .as_ref()
                            .ok()
                            .map(|prepared| prepared.opened.facts.clone());
                        result = Err(Failure {
                            setup_cause: None,
                            code: ErrorCode::Cancelled,
                            effect: Effect::None,
                            facts,
                        });
                    }
                    if self.shutting_down || entry.retired {
                        drop(result);
                        continue;
                    }
                    // The key learns how the attempt's setup went; a retried
                    // open waits for its next attempt (retry.rs). A cancelled
                    // one told it at its cancellation.
                    if !entry.cancel.is_cancelled() {
                        let member = (request.clone(), entry.envelope.stream_id);
                        let settled = self.retries.settle(
                            self.now_ms,
                            member,
                            entry,
                            result,
                            connect,
                            &mut retry,
                        );
                        let Some(settled) = settled else {
                            continue;
                        };
                        result = settled;
                    }
                    let mut receipt = Envelope {
                        version: entry.envelope.version,
                        session_id: entry.envelope.session_id.clone(),
                        stream_id: entry.envelope.stream_id,
                        ..Default::default()
                    };
                    match result {
                        Ok(mut prepared) => {
                            prepared.opened.endpoint_id = self.endpoint.clone();
                            prepared.opened.trust_owner = self.trust_owner.clone();
                            let limits = entry
                                .envelope
                                .open
                                .as_ref()
                                .expect("Open")
                                .receive_limits
                                .clone();
                            prepared.opened.receive_limits = limits.clone();
                            receipt.kind = MessageKind::Opened;
                            receipt.opened = Some(prepared.opened.clone());
                            let mut config = stream::Config::new(
                                &receipt.session_id,
                                receipt.stream_id,
                                stream::Side::Endpoint,
                            );
                            config.profile_version = 2;
                            config.io_timeout_ms = prepared.io_timeout_ms();
                            config.receive_window = limits.receive_window as usize;
                            config.peer_receive_window = limits.receive_window as usize;
                            config.max_payload =
                                config.max_payload.min(limits.data_payload as usize);
                            config.receive_limits = limits.clone();
                            config.peer_limits = limits;
                            let (stream, peer) =
                                Stream::new(config).map_err(|_| EndpointError::Protocol)?;
                            let peer = Arc::new(peer);
                            peer.advance(now);
                            if https_policy::advertisement(
                                entry.envelope.open.as_ref().expect("Open").service,
                            ) {
                                entry.serving = Some(self.runtime.spawn(prepared.serve(
                                    stream.clone(),
                                    peer.clone(),
                                    entry.cancel.clone(),
                                )));
                            } else {
                                entry.prepared = Some(prepared);
                            }
                            entry.stream = Some(stream);
                            entry.peer = Some(peer);
                        }
                        Err(failure) => {
                            let open = entry.envelope.open.as_ref().expect("Open");
                            if !entry.cancel.is_cancelled()
                                && open.policy == AuthPolicy::Anonymous
                                && https_policy::advertisement(open.service)
                                && matches!(
                                    failure.code,
                                    ErrorCode::Authentication | ErrorCode::RepositoryRefused
                                )
                                && failure
                                    .facts
                                    .as_ref()
                                    .is_some_and(|f| matches!(f.http_status, Some(401 | 404)))
                            {
                                if let Some(operation) = self.operations.get_mut(request) {
                                    if operation.retries.len() < 64 {
                                        operation.retries.insert(retry_key(open), retry);
                                    }
                                }
                            }
                            receipt.kind = MessageKind::OpenFailed;
                            receipt.open_failed = Some(failure);
                        }
                    }
                    entry.output = Some(receipt);
                }
            }
            if let Some(peer) = &entry.peer {
                peer.advance(now);
            }
            if entry.serving.is_none() {
                if entry.handoff {
                    let Some(prepared) = entry.prepared.take() else {
                        return Err(EndpointError::Protocol);
                    };
                    let Some(peer) = entry.peer.clone() else {
                        return Err(EndpointError::Protocol);
                    };
                    let Some(stream) = entry.stream.clone() else {
                        return Err(EndpointError::Protocol);
                    };
                    entry.serving = Some(self.runtime.spawn(prepared.serve(
                        stream,
                        peer.clone(),
                        entry.cancel.clone(),
                    )));
                    entry.handoff = false;
                }
            }
            if let Some(task) = entry.serving.as_mut() {
                if let Poll::Ready(result) = Pin::new(task).poll(cx) {
                    result.map_err(|_| EndpointError::Protocol)?;
                    entry.serving = None;
                }
            }
        }
        self.start_held();
        self.entries.retain(|_, entry| {
            !(entry.retired && entry.preparing.is_none() && entry.serving.is_none())
        });
        Ok(())
    }
    pub(super) fn take_outbound(&mut self, cx: &mut Context<'_>) -> Option<Outbound> {
        let keys: Vec<_> = self
            .entries
            .keys()
            .filter(|key| self.last_outbound.as_ref().is_none_or(|last| *key > last))
            .chain(
                self.entries
                    .keys()
                    .filter(|key| self.last_outbound.as_ref().is_some_and(|last| *key <= last)),
            )
            .cloned()
            .collect();
        for key in keys {
            let entry = self.entries.get_mut(&key).expect("live key");
            let request = &key.0;
            if entry.retired {
                continue;
            }
            let mut message = entry.output.take();
            if message.is_none() {
                if let Some(peer) = &entry.peer {
                    let next = entry
                        .next
                        .get_or_insert_with(|| super::session::next_message(peer));
                    if let Poll::Ready(result) = next.as_mut().poll(cx) {
                        entry.next = None;
                        message = result.ok().flatten();
                    }
                }
            }
            if let Some(envelope) = message {
                if matches!(
                    envelope.kind,
                    MessageKind::OpenFailed | MessageKind::Closed | MessageKind::Failed
                ) {
                    entry.retired = true;
                }
                self.last_outbound = Some(key.clone());
                return Some(Outbound {
                    request: request.clone(),
                    envelope,
                });
            }
        }
        None
    }
    // Called on every retry of the host's pending outbound slot. Taking a
    // receipt is not publication; only successful mux send linearizes Opened.
    pub(super) fn before_handoff(&mut self, request: &str, message: &mut Envelope) {
        if let Some(entry) = self.entries.get_mut(&(request.into(), message.stream_id)) {
            if message.kind == MessageKind::Opened && entry.cancel.is_cancelled() {
                let facts = message.opened.as_ref().map(|opened| opened.facts.clone());
                *message = cancelled_open(&entry.envelope, facts);
                entry.output = None;
                entry.prepared = None;
                entry.handoff = false;
                entry.retired = true;
            }
        }
    }
    pub(super) fn handed_off(&mut self, request: &str, message: &Envelope) {
        if message.kind == MessageKind::Opened {
            if let Some(entry) = self.entries.get_mut(&(request.into(), message.stream_id)) {
                entry.opening_published = true;
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
        "{:?}|{}|{}|{}",
        open.service, open.destination.host, open.destination.port, open.destination.path
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
fn cancelled_open(envelope: &Envelope, facts: Option<Facts>) -> Envelope {
    Envelope {
        version: envelope.version,
        session_id: envelope.session_id.clone(),
        stream_id: envelope.stream_id,
        kind: MessageKind::OpenFailed,
        open_failed: Some(Failure {
            setup_cause: None,
            code: ErrorCode::Cancelled,
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
    mod retry_tests;
} }

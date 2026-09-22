//! Private HTTPS side of the existing placement session. The synchronous host
//! only admits messages and polls completion; an owned runtime drives HTTP.
use super::{Arc, Duration, HttpsEndpointConfig, ModelResult, pool, unavailable};
use crate::git::endpoint::{
    https_policy,
    https_worker::{Budget, Client, Endpoint as HttpEndpoint, Input, Prepared},
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
type Key = (String, i64);
struct Entry {
    envelope: Envelope,
    cancel: CancellationToken,
    preparing: Option<JoinHandle<(Result<Prepared, Failure>, Budget)>>,
    serving: Option<JoinHandle<()>>,
    // Retain the application half until its terminal message is drained.
    stream: Option<Stream>,
    peer: Option<Arc<MessageEndpoint>>,
    output: Option<Envelope>,
    retired: bool,
}
struct Operation {
    name: String,
    _guard: crate::git::endpoint::https_operation::Dependency,
    retries: BTreeMap<String, Budget>,
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
}
impl HttpsEndpoint {
    pub(super) fn new(
        config: HttpsEndpointConfig,
        pool: pool::Config,
        io_ms: u64,
        authority: Authority,
        endpoint: String,
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
                let mut endpoint = match HttpEndpoint::with_authority(config.tls, config.auth, pool, io_ms, authority) {
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
        })
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
                    entry.cancel.cancel();
                    // Let the HTTP owner emit the terminal with its effect and
                    // facts. Delivering Cancel to the byte machine first would
                    // make fail_terminal a no-op and leave the mux route live.
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
        let host = if open.destination.host.contains(':') {
            format!("[{}]", open.destination.host)
        } else {
            open.destination.host.clone()
        };
        let input = Input {
            destination: format!(
                "https://{host}:{}{}",
                open.destination.port, open.destination.path
            ),
            service: open.service,
            policy: open.policy,
            session: envelope.session_id.clone(),
            operation: operation.name.clone(),
        };
        let retry_key = retry_key(open);
        let mut budget = if open.policy == AuthPolicy::Gh {
            operation
                .retries
                .remove(&retry_key)
                .unwrap_or_else(|| self.client.budget_for_open(&open.deadlines))
        } else {
            self.client.budget_for_open(&open.deadlines)
        };
        budget.shorten(self.client.budget_for_open(&open.deadlines));
        let cancel = CancellationToken::new();
        let cancelled = cancel.clone();
        let client = self.client.clone();
        let preparing = self.runtime.spawn(async move {
            let result = client.prepare_budget(input, &cancelled, &mut budget).await;
            (result, budget)
        });
        self.entries.insert(
            key,
            Entry {
                envelope,
                cancel,
                preparing: Some(preparing),
                serving: None,
                stream: None,
                peer: None,
                output: None,
                retired: false,
            },
        );
        Ok(())
    }
    pub(super) fn step(&mut self, now: u64, cx: &mut Context<'_>) -> Result<(), EndpointError> {
        for ((request, _), entry) in &mut self.entries {
            if let Some(task) = entry.preparing.as_mut() {
                if let Poll::Ready(result) = Pin::new(task).poll(cx) {
                    entry.preparing = None;
                    let (mut result, budget) = result.map_err(|_| EndpointError::Protocol)?;
                    if entry.cancel.is_cancelled() && result.is_ok() {
                        let facts = result
                            .as_ref()
                            .ok()
                            .map(|prepared| prepared.opened.facts.clone());
                        result = Err(Failure {
                            code: ErrorCode::Cancelled,
                            effect: Effect::None,
                            facts,
                        });
                    }
                    if self.shutting_down || entry.retired {
                        drop(result);
                        continue;
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
                            entry.serving = Some(self.runtime.spawn(prepared.serve(
                                stream.clone(),
                                peer.clone(),
                                entry.cancel.clone(),
                            )));
                            entry.stream = Some(stream);
                            entry.peer = Some(peer);
                        }
                        Err(failure) => {
                            let open = entry.envelope.open.as_ref().expect("Open");
                            if open.policy == AuthPolicy::Anonymous
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
                                        operation.retries.insert(retry_key(open), budget);
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
            if let Some(task) = entry.serving.as_mut() {
                if let Poll::Ready(result) = Pin::new(task).poll(cx) {
                    result.map_err(|_| EndpointError::Protocol)?;
                    entry.serving = None;
                }
            }
        }
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
            let message = entry.output.take().or_else(|| {
                entry.peer.as_ref().and_then(|peer| {
                    match std::pin::pin!(peer.next_message()).poll(cx) {
                        Poll::Ready(Ok(Some(message))) => Some(message),
                        _ => None,
                    }
                })
            });
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
    pub(super) fn cancel_request(&mut self, request: &str) {
        for ((id, _), entry) in &mut self.entries {
            if id == request {
                entry.cancel.cancel();
                entry.retired = true;
                entry.output = None;
                if let Some(peer) = &entry.peer {
                    peer.disconnect();
                }
            }
        }
        if let Some(operation) = self.operations.remove(request) {
            self.client.finish_operation(&operation.name);
        }
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

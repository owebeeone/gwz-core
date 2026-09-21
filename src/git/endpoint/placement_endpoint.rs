//! Nonblocking endpoint-side bridge for the v2 placement mux.
//!
//! The bridge owns only request state and bounded message attachments. Physical
//! SSH work remains in [`ssh_worker::Endpoint`], whose worker thread continues
//! pumping every attached exchange while this object is stepped by the host.

use super::{
    agent_job::Job,
    ssh_channel::GitService as NativeService,
    ssh_worker::{BridgeContext, Endpoint, EndpointAttachment},
};
use gwz_transport::{
    pool::Key,
    protocol::{
        Destination, Effect, Envelope, ErrorCode, Facts, Failure, GitService, Identity,
        IdentityMode, MessageKind, Opened,
    },
};
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    path::{Path, PathBuf},
    sync::mpsc,
    task::{Context, Poll},
    time::{Duration, Instant},
};

const MAX_REQUESTS: usize = 64;
const MAX_QUEUED_INPUT: usize = 16;
const MAX_OUTBOUND: usize = 64;
const MAX_OPEN_JOBS: usize = 8;
type RequestKey = (String, i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EndpointError {
    InvalidRequest,
    Capacity,
    Duplicate,
    Shutdown,
    Protocol,
    WouldBlock,
}

struct Request {
    stream_id: i64,
    operation_id: String,
    session_id: String,
    version: i64,
    attachment: Option<EndpointAttachment>,
    queued_input: VecDeque<Envelope>,
    terminal: bool,
    deadline: Option<u64>,
}
struct OpenJob {
    key: RequestKey,
    job: Job<(EndpointAttachment, Opened)>,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    deadline: Option<u64>,
    abandoned: bool,
}
struct QueuedOpen {
    key: RequestKey,
    envelope: Envelope,
    admitted_at: u64,
    deadline: u64,
}
struct CheckJob {
    key: RequestKey,
    job: Job<()>,
    deadline: u64,
    cancelled: bool,
}
pub(crate) struct Outbound {
    pub(crate) request: String,
    pub(crate) envelope: Envelope,
}

/// Endpoint-local runtime driven by one host supervisor thread.
pub(crate) struct PlacementEndpoint {
    endpoint: Endpoint,
    home: PathBuf,
    endpoint_id: String,
    trust_owner: String,
    now_ms: u64,
    requests: BTreeMap<RequestKey, Request>,
    opens: Vec<OpenJob>,
    queued_opens: VecDeque<QueuedOpen>,
    checks: Vec<CheckJob>,
    outbound: VecDeque<Outbound>,
    terminal_outbound: VecDeque<Outbound>,
    shutting_down: bool,
    faulted: bool,
}
impl Drop for PlacementEndpoint {
    fn drop(&mut self) {
        self.shutdown();
    }
}
impl PlacementEndpoint {
    pub(crate) fn new(
        endpoint: Endpoint,
        home: PathBuf,
        endpoint_id: String,
        trust_owner: String,
    ) -> io::Result<Self> {
        if endpoint_id.is_empty()
            || trust_owner.is_empty()
            || !home.is_absolute()
            || home.to_str().is_none_or(|value| value.contains('\0'))
        {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        Ok(Self {
            endpoint,
            home,
            endpoint_id,
            trust_owner,
            now_ms: 0,
            requests: BTreeMap::new(),
            opens: Vec::new(),
            queued_opens: VecDeque::new(),
            checks: Vec::new(),
            outbound: VecDeque::new(),
            terminal_outbound: VecDeque::new(),
            shutting_down: false,
            faulted: false,
        })
    }

    /// Admit one already-correlated mux message without performing blocking I/O.
    pub(crate) fn accept(
        &mut self,
        request: String,
        envelope: Envelope,
    ) -> Result<(), EndpointError> {
        if self.shutting_down {
            return Err(EndpointError::Shutdown);
        }
        if request.is_empty() || request.len() > 128 {
            return Err(EndpointError::InvalidRequest);
        }
        if envelope.stream_id <= 0 {
            return Err(EndpointError::InvalidRequest);
        }
        let key = (request.clone(), envelope.stream_id);
        if envelope.kind == MessageKind::Open || envelope.kind == MessageKind::CheckIdentity {
            if self.requests.contains_key(&key) || self.requests.len() >= MAX_REQUESTS {
                return Err(if self.requests.contains_key(&key) {
                    EndpointError::Duplicate
                } else {
                    EndpointError::WouldBlock
                });
            }
            return match envelope.kind {
                MessageKind::Open => self.accept_open(key, envelope),
                MessageKind::CheckIdentity => self.accept_check(key, envelope),
                _ => Err(EndpointError::Protocol),
            };
        }
        let Some(state) = self.requests.get_mut(&key) else {
            // Mux admission validated this route before placing the action in
            // its queue. A physical terminal may retire it before this action
            // is drained; stale admitted input has no remaining endpoint work.
            return Ok(());
        };
        if state.terminal {
            return Ok(());
        }
        if envelope.kind == MessageKind::Cancel && state.attachment.is_none() {
            if let Some(index) = self
                .queued_opens
                .iter()
                .position(|queued| queued.key == key)
            {
                self.queued_opens.remove(index);
                state.terminal = true;
                let message = envelope_for(
                    state,
                    MessageKind::OpenFailed,
                    Some(Failure {
                        code: ErrorCode::Cancelled,
                        effect: Effect::None,
                        facts: None,
                    }),
                    None,
                );
                self.push_outbound(key.0, message);
                return Ok(());
            }
            if let Some(check) = self.checks.iter_mut().find(|check| check.key == key) {
                check.cancelled = true;
                state.terminal = true;
                let message = envelope_for(
                    state,
                    MessageKind::IdentityCheckFailed,
                    Some(Failure {
                        code: ErrorCode::Cancelled,
                        effect: Effect::None,
                        facts: None,
                    }),
                    None,
                );
                self.push_outbound(key.0, message);
                return Ok(());
            }
            if let Some(open) = self.opens.iter_mut().find(|open| open.key == key) {
                open.abandoned = true;
                open.cancelled
                    .store(true, std::sync::atomic::Ordering::Release);
                state.terminal = true;
                let message = envelope_for(
                    state,
                    MessageKind::OpenFailed,
                    Some(Failure {
                        code: ErrorCode::Cancelled,
                        effect: Effect::None,
                        facts: None,
                    }),
                    None,
                );
                self.push_outbound(key.0, message);
                return Ok(());
            }
        }
        if state.attachment.is_none() {
            if state.queued_input.len() >= MAX_QUEUED_INPUT {
                return Err(EndpointError::Capacity);
            }
            state.queued_input.push_back(envelope);
            return Ok(());
        }
        state
            .attachment
            .as_ref()
            .expect("checked attachment")
            .send(envelope)
            .map_err(|error| match error.kind() {
                io::ErrorKind::WouldBlock => EndpointError::WouldBlock,
                io::ErrorKind::BrokenPipe => EndpointError::WouldBlock,
                _ => EndpointError::Protocol,
            })
    }

    fn accept_open(&mut self, key: RequestKey, envelope: Envelope) -> Result<(), EndpointError> {
        let open = envelope
            .open
            .as_ref()
            .ok_or(EndpointError::InvalidRequest)?;
        if open.endpoint_id != self.endpoint_id || open.operation_id.is_empty() {
            return Err(EndpointError::InvalidRequest);
        }
        // Reject unsupported peer policy before queue ownership or arithmetic.
        if self.endpoint.validate_deadlines(&open.deadlines).is_err() {
            let mut state = request_state(&envelope, open.operation_id.clone());
            state.terminal = true;
            let message = envelope_for(
                &state,
                MessageKind::OpenFailed,
                Some(Failure {
                    code: ErrorCode::InvalidRequest,
                    effect: Effect::None,
                    facts: None,
                }),
                None,
            );
            self.requests.insert(key.clone(), state);
            self.push_outbound(key.0, message);
            return Ok(());
        }
        if self.opens.len() >= MAX_OPEN_JOBS {
            let state = request_state(&envelope, open.operation_id.clone());
            let now = self.now();
            let deadline = now.saturating_add(open.deadlines.allocation_ms as u64);
            self.requests.insert(key.clone(), state);
            self.queued_opens.push_back(QueuedOpen {
                key,
                envelope,
                admitted_at: now,
                deadline,
            });
            return Ok(());
        }
        let (pool_key, repository_path) = destination(&open.destination)?;
        let selected = match selected_path(&self.home, &open.identity) {
            Ok(selected) => selected,
            Err(_) => {
                let mut state = request_state(&envelope, open.operation_id.clone());
                state.terminal = true;
                let message = envelope_for(
                    &state,
                    MessageKind::OpenFailed,
                    Some(Failure {
                        code: ErrorCode::InvalidRequest,
                        effect: Effect::None,
                        facts: None,
                    }),
                    None,
                );
                self.requests.insert(key.clone(), state);
                self.push_outbound(key.0, message);
                return Ok(());
            }
        };
        let deadline =
            deadline_from_open(&open.deadlines).map(|duration| self.now().saturating_add(duration));
        self.requests.insert(
            key.clone(),
            Request {
                stream_id: envelope.stream_id,
                operation_id: open.operation_id.clone(),
                session_id: envelope.session_id.clone(),
                version: envelope.version,
                attachment: None,
                queued_input: VecDeque::new(),
                terminal: false,
                deadline,
            },
        );
        let endpoint = self.endpoint.clone();
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        let service = native_service(open.service)?;
        let context = BridgeContext {
            session_id: envelope.session_id.clone(),
            stream_id: envelope.stream_id,
            version: envelope.version,
            limits: open.receive_limits.clone(),
            deadlines: open.deadlines.clone(),
        };
        let job = match selected {
            Some(path) => endpoint.start_endpoint_selected_job(
                pool_key,
                path,
                service,
                repository_path,
                Some(context),
                worker_cancelled,
            ),
            None => endpoint.start_endpoint_ambient_job(
                pool_key,
                service,
                repository_path,
                Some(context),
                worker_cancelled,
            ),
        }
        .map_err(|_| EndpointError::Capacity)?;
        self.opens.push(OpenJob {
            key,
            job,
            cancelled,
            deadline,
            abandoned: false,
        });
        Ok(())
    }

    fn accept_check(&mut self, key: RequestKey, envelope: Envelope) -> Result<(), EndpointError> {
        let check = envelope
            .check_identity
            .as_ref()
            .ok_or(EndpointError::InvalidRequest)?;
        if check.endpoint_id != self.endpoint_id
            || check.operation_id.is_empty()
            || check.timeout_ms <= 0
            || check.identity.mode != IdentityMode::ExplicitKey
        {
            return Err(EndpointError::InvalidRequest);
        }
        let selected = match selected_path(&self.home, &check.identity) {
            Ok(Some(selected)) => selected,
            Ok(None) | Err(_) => {
                let mut state = request_state(&envelope, check.operation_id.clone());
                state.terminal = true;
                let message = envelope_for(
                    &state,
                    MessageKind::IdentityCheckFailed,
                    Some(Failure {
                        code: ErrorCode::InvalidRequest,
                        effect: Effect::None,
                        facts: None,
                    }),
                    None,
                );
                self.requests.insert(key.clone(), state);
                self.push_outbound(key.0, message);
                return Ok(());
            }
        };
        let deadline = self.now().saturating_add(check.timeout_ms as u64);
        let job = self
            .endpoint
            .start_identity_file_check(
                selected,
                Some(Instant::now() + Duration::from_millis(check.timeout_ms as u64)),
            )
            .map_err(|_| EndpointError::Capacity)?;
        self.requests.insert(
            key.clone(),
            Request {
                stream_id: envelope.stream_id,
                operation_id: check.operation_id.clone(),
                session_id: envelope.session_id,
                version: envelope.version,
                attachment: None,
                queued_input: VecDeque::new(),
                terminal: false,
                deadline: Some(deadline),
            },
        );
        self.checks.push(CheckJob {
            key,
            job,
            deadline,
            cancelled: false,
        });
        Ok(())
    }

    /// Advance bounded checks, open completions, and each live message bridge.
    pub(crate) fn step(&mut self, now_ms: u64, cx: &mut Context<'_>) -> Result<(), EndpointError> {
        self.now_ms = self.now_ms.max(now_ms);
        let now_ms = self.now_ms;
        self.finish_checks(now_ms, cx);
        self.finish_opens(now_ms, cx);
        self.start_queued(now_ms)?;
        self.flush_attachments();
        if self.faulted {
            Err(EndpointError::Capacity)
        } else {
            Ok(())
        }
    }

    fn start_queued(&mut self, now: u64) -> Result<(), EndpointError> {
        for _ in 0..self.queued_opens.len() {
            let mut queued = self.queued_opens.pop_front().expect("queued length");
            if now >= queued.deadline {
                if let Some(state) = self.requests.get_mut(&queued.key) {
                    state.terminal = true;
                    let message = envelope_for(
                        state,
                        MessageKind::OpenFailed,
                        Some(Failure {
                            code: ErrorCode::Timeout,
                            effect: Effect::None,
                            facts: None,
                        }),
                        None,
                    );
                    self.push_outbound(queued.key.0, message);
                }
            } else if self.opens.len() < MAX_OPEN_JOBS {
                let open = queued.envelope.open.as_mut().expect("admitted Open");
                open.deadlines.allocation_ms = (open.deadlines.allocation_ms as u64)
                    .saturating_sub(now.saturating_sub(queued.admitted_at))
                    .max(1) as i64;
                self.accept_open(queued.key, queued.envelope)?;
            } else {
                self.queued_opens.push_back(queued);
            }
        }
        Ok(())
    }

    fn finish_checks(&mut self, now_ms: u64, cx: &mut Context<'_>) {
        let mut index = 0;
        while index < self.checks.len() {
            // Logical completion never waits for a blocked filesystem job to
            // return. Its physical owner remains charged until disposal.
            let expired_key = {
                let check = &mut self.checks[index];
                if !check.cancelled && now_ms >= check.deadline {
                    check.cancelled = true;
                    check.job.cancel();
                    Some(check.key.clone())
                } else {
                    None
                }
            };
            if let Some(key) = expired_key {
                if let Some(state) = self.requests.get_mut(&key) {
                    if !state.terminal {
                        state.terminal = true;
                        let message = envelope_for(
                            state,
                            MessageKind::IdentityCheckFailed,
                            Some(Failure {
                                code: ErrorCode::Timeout,
                                effect: Effect::None,
                                facts: None,
                            }),
                            None,
                        );
                        self.push_outbound(key.0, message);
                    }
                }
            }
            let check = &mut self.checks[index];
            let expired = now_ms >= check.deadline;
            if expired {
                check.cancelled = true;
            }
            let result = if check.cancelled {
                match check.job.poll_disposed(cx) {
                    Poll::Ready(Ok(())) => Some(Err(if expired {
                        ErrorCode::Timeout
                    } else {
                        ErrorCode::Cancelled
                    })),
                    Poll::Ready(Err(_)) => None,
                    Poll::Pending => None,
                }
            } else {
                match check.job.poll_result(cx) {
                    Poll::Ready(Ok(())) => Some(Ok(())),
                    Poll::Ready(Err(error)) => Some(Err(match error.kind() {
                        io::ErrorKind::InvalidInput => ErrorCode::InvalidRequest,
                        io::ErrorKind::TimedOut => ErrorCode::Timeout,
                        io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied => {
                            ErrorCode::Unavailable
                        }
                        _ => ErrorCode::Io,
                    })),
                    Poll::Pending => None,
                }
            };
            let Some(result) = result else {
                index += 1;
                continue;
            };
            let key = self.checks.swap_remove(index).key;
            let Some(state) = self.requests.get_mut(&key) else {
                continue;
            };
            let already_terminal = state.terminal;
            state.terminal = true;
            if already_terminal {
                continue;
            }
            let message = match result {
                Ok(_) => envelope_for(state, MessageKind::IdentityChecked, None, None),
                Err(code) => envelope_for(
                    state,
                    MessageKind::IdentityCheckFailed,
                    Some(Failure {
                        code,
                        effect: Effect::None,
                        facts: None,
                    }),
                    None,
                ),
            };
            self.push_outbound(key.0, message);
        }
    }

    fn finish_opens(&mut self, now_ms: u64, cx: &mut Context<'_>) {
        let mut index = 0;
        while index < self.opens.len() {
            let expired = {
                let open = &mut self.opens[index];
                if !open.abandoned && open.deadline.is_some_and(|deadline| now_ms >= deadline) {
                    open.abandoned = true;
                    open.cancelled
                        .store(true, std::sync::atomic::Ordering::Release);
                    Some(open.key.clone())
                } else {
                    None
                }
            };
            if let Some(key) = expired {
                if let Some(state) = self.requests.get_mut(&key) {
                    if !state.terminal {
                        state.terminal = true;
                        let message = envelope_for(
                            state,
                            MessageKind::OpenFailed,
                            Some(Failure {
                                code: ErrorCode::Timeout,
                                effect: Effect::None,
                                facts: None,
                            }),
                            None,
                        );
                        self.push_outbound(key.0, message);
                    }
                }
            }
            let open = &mut self.opens[index];
            let result = if open.abandoned {
                match open.job.poll_disposed(cx) {
                    Poll::Ready(Ok(())) => Some(Err(io::ErrorKind::BrokenPipe.into())),
                    Poll::Ready(Err(_)) => None,
                    Poll::Pending => None,
                }
            } else {
                match open.job.poll_result(cx) {
                    Poll::Ready(result) => Some(result),
                    Poll::Pending => None,
                }
            };
            let Some(result) = result else {
                index += 1;
                continue;
            };
            let job = self.opens.swap_remove(index);
            if job.abandoned {
                if let Ok((attachment, _)) = result {
                    attachment.cancel();
                }
                if let Some(state) = self.requests.get_mut(&job.key) {
                    if !state.terminal {
                        state.terminal = true;
                        let message = envelope_for(
                            state,
                            MessageKind::OpenFailed,
                            Some(Failure {
                                code: ErrorCode::Timeout,
                                effect: Effect::None,
                                facts: None,
                            }),
                            None,
                        );
                        self.push_outbound(job.key.0, message);
                    }
                }
                continue;
            }
            match result {
                Ok((attachment, mut opened)) => {
                    opened.endpoint_id = self.endpoint_id.clone();
                    opened.trust_owner = self.trust_owner.clone();
                    if !self.requests.contains_key(&job.key) {
                        attachment.cancel();
                        continue;
                    }
                    let message = {
                        let state = self.requests.get_mut(&job.key).expect("open request");
                        let message = envelope_for(state, MessageKind::Opened, None, Some(opened));
                        state.attachment = Some(attachment);
                        message
                    };
                    self.push_outbound(job.key.0.clone(), message);
                }
                Err(error) => {
                    if !self.requests.contains_key(&job.key) {
                        continue;
                    }
                    let message = {
                        let state = self.requests.get_mut(&job.key).expect("open request");
                        state.terminal = true;
                        envelope_for(
                            state,
                            MessageKind::OpenFailed,
                            Some(failure_for(error)),
                            None,
                        )
                    };
                    self.push_outbound(job.key.0.clone(), message);
                }
            }
        }
    }

    fn flush_attachments(&mut self) {
        let requests: Vec<_> = self.requests.keys().cloned().collect();
        for request in requests {
            if self.outbound.len() >= MAX_OUTBOUND {
                break;
            }
            let Some(state) = self.requests.get_mut(&request) else {
                continue;
            };
            let Some(attachment) = state.attachment.as_ref() else {
                continue;
            };
            let mut disconnect_failure = None;
            while let Some(message) = state.queued_input.pop_front() {
                match attachment.send(message.clone()) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        state.queued_input.push_front(message);
                        break;
                    }
                    Err(_) => {
                        state.terminal = true;
                        disconnect_failure = Some(envelope_for(
                            state,
                            MessageKind::Failed,
                            Some(Failure {
                                code: ErrorCode::Io,
                                effect: Effect::Possible,
                                facts: None,
                            }),
                            None,
                        ));
                        break;
                    }
                }
            }
            for _ in 0..8 {
                match attachment.try_receive() {
                    Ok(Some(mut message)) => {
                        message.session_id = state.session_id.clone();
                        message.stream_id = state.stream_id;
                        message.version = state.version;
                        if matches!(message.kind, MessageKind::Closed | MessageKind::Failed) {
                            state.terminal = true;
                        }
                        self.outbound.push_back(Outbound {
                            request: request.0.clone(),
                            envelope: message,
                        });
                    }
                    Ok(None) => break,
                    Err(_) => {
                        state.terminal = true;
                        disconnect_failure = Some(envelope_for(
                            state,
                            MessageKind::Failed,
                            Some(Failure {
                                code: ErrorCode::Io,
                                effect: Effect::Possible,
                                facts: None,
                            }),
                            None,
                        ));
                        break;
                    }
                }
                if self.outbound.len() >= MAX_OUTBOUND {
                    break;
                }
            }
            if let Some(message) = disconnect_failure {
                self.push_outbound(request.0.clone(), message);
            }
        }
    }

    pub(crate) fn take_outbound(&mut self) -> Option<Outbound> {
        let item = self.outbound.pop_front();
        if let Some(next) = self.terminal_outbound.pop_front() {
            self.outbound.push_back(next);
        }
        if let Some(item) = &item {
            if matches!(
                item.envelope.kind,
                MessageKind::OpenFailed
                    | MessageKind::IdentityChecked
                    | MessageKind::IdentityCheckFailed
                    | MessageKind::Failed
                    | MessageKind::Closed
            ) {
                let key = (item.request.clone(), item.envelope.stream_id);
                self.requests.remove(&key);
            }
        }
        item
    }
    pub(crate) fn requeue_outbound(&mut self, item: Outbound) -> Result<(), EndpointError> {
        if self.outbound.len() >= MAX_OUTBOUND {
            return Err(EndpointError::Capacity);
        }
        self.outbound.push_front(item);
        Ok(())
    }
    pub(crate) fn pending_request(&self, request: &str) -> bool {
        self.requests.keys().any(|key| key.0 == request)
            || self.opens.iter().any(|job| job.key.0 == request)
            || self.checks.iter().any(|job| job.key.0 == request)
    }
    pub(crate) fn pending_request_count(&self, request: &str) -> usize {
        self.requests.keys().filter(|key| key.0 == request).count()
            + self.opens.iter().filter(|job| job.key.0 == request).count()
            + self
                .checks
                .iter()
                .filter(|job| job.key.0 == request)
                .count()
    }
    pub(crate) fn cancel_request(&mut self, request: &str) {
        let keys: Vec<_> = self
            .requests
            .keys()
            .filter(|key| key.0 == request)
            .cloned()
            .collect();
        for key in keys {
            let is_check = self.checks.iter().any(|check| check.key == key);
            let message = if let Some(state) = self.requests.get_mut(&key) {
                let message = if !state.terminal {
                    state.terminal = true;
                    Some(envelope_for(
                        state,
                        if is_check {
                            MessageKind::IdentityCheckFailed
                        } else if state.attachment.is_none() {
                            MessageKind::OpenFailed
                        } else {
                            MessageKind::Failed
                        },
                        Some(Failure {
                            code: ErrorCode::Cancelled,
                            effect: Effect::None,
                            facts: None,
                        }),
                        None,
                    ))
                } else {
                    None
                };
                if let Some(attachment) = &state.attachment {
                    attachment.cancel();
                }
                message
            } else {
                None
            };
            if let Some(message) = message {
                self.push_outbound(key.0, message);
            }
        }
        self.queued_opens.retain(|queued| queued.key.0 != request);
        for check in &mut self.checks {
            if check.key.0 == request {
                check.cancelled = true;
            }
        }
        for open in &mut self.opens {
            if open.key.0 == request {
                open.abandoned = true;
                open.cancelled
                    .store(true, std::sync::atomic::Ordering::Release);
            }
        }
    }
    pub(crate) fn shutdown(&mut self) {
        if self.shutting_down {
            return;
        }
        self.shutting_down = true;
        let requests: Vec<_> = self.requests.keys().cloned().collect();
        for request in requests {
            let is_check = self.checks.iter().any(|check| check.key == request);
            if let Some(state) = self.requests.get_mut(&request) {
                if !state.terminal {
                    state.terminal = true;
                    let message = envelope_for(
                        state,
                        if is_check {
                            MessageKind::IdentityCheckFailed
                        } else if state.attachment.is_none() {
                            MessageKind::OpenFailed
                        } else {
                            MessageKind::Failed
                        },
                        Some(Failure {
                            code: ErrorCode::CarrierLost,
                            effect: Effect::None,
                            facts: None,
                        }),
                        None,
                    );
                    self.push_outbound(request.0.clone(), message);
                }
            }
        }
        for state in self.requests.values() {
            if let Some(attachment) = &state.attachment {
                attachment.cancel();
            }
        }
        for open in &mut self.opens {
            open.abandoned = true;
            open.cancelled
                .store(true, std::sync::atomic::Ordering::Release);
        }
        for check in &mut self.checks {
            check.cancelled = true;
        }
        self.endpoint.shutdown();
        self.outbound.clear();
        self.terminal_outbound.clear();
        self.requests.clear();
        self.queued_opens.clear();
    }
    pub(crate) fn pending(&self) -> usize {
        self.requests.len()
            + self.opens.len()
            + self.checks.len()
            + self.outbound.len()
            + self.terminal_outbound.len()
            + self.endpoint.pending_requests()
            + self.endpoint.shutdown_status().pending_connections
    }
    fn now(&self) -> u64 {
        self.now_ms
    }
    fn push_outbound(&mut self, request: String, message: Envelope) {
        let item = Outbound {
            request,
            envelope: message,
        };
        if self.outbound.len() < MAX_OUTBOUND {
            self.outbound.push_back(item);
        } else if self.terminal_outbound.len() < 2 * MAX_REQUESTS {
            // One Opened plus one terminal can be retained per admitted route.
            // Bulk data uses only `outbound`; it cannot consume this reserve.
            self.terminal_outbound.push_back(item);
        } else {
            // An exhausted internal bound is a binding failure, never a silent
            // successful send or a request left waiting for a dropped result.
            self.faulted = true;
        }
    }
}

fn destination(value: &Destination) -> Result<(Key, String), EndpointError> {
    if value.scheme != gwz_transport::protocol::Scheme::Ssh {
        return Err(EndpointError::InvalidRequest);
    }
    let username = value
        .ssh_username
        .as_ref()
        .ok_or(EndpointError::InvalidRequest)?;
    Ok((
        Key::ssh(username.clone(), value.host.clone(), value.port as u16),
        value.path.clone(),
    ))
}
fn selected_path(home: &Path, identity: &Identity) -> Result<Option<PathBuf>, EndpointError> {
    if identity.mode != IdentityMode::ExplicitKey {
        return Ok(None);
    }
    let raw = identity
        .key_path
        .as_deref()
        .ok_or(EndpointError::InvalidRequest)?;
    if raw.starts_with('~') {
        if raw == "~" || raw.starts_with("~/") {
            return Ok(Some(home.join(raw.strip_prefix("~/").unwrap_or(""))));
        }
        return Err(EndpointError::InvalidRequest);
    }
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        Ok(Some(path))
    } else {
        let base = identity
            .path_base
            .as_deref()
            .map(PathBuf::from)
            .ok_or(EndpointError::InvalidRequest)?;
        if !base.is_absolute() {
            return Err(EndpointError::InvalidRequest);
        }
        Ok(Some(base.join(path)))
    }
}
fn native_service(service: GitService) -> Result<NativeService, EndpointError> {
    match service {
        GitService::UploadPackAdvertisement | GitService::UploadPackExchange => {
            Ok(NativeService::UploadPack)
        }
        GitService::ReceivePackAdvertisement | GitService::ReceivePackExchange => {
            Ok(NativeService::ReceivePack)
        }
    }
}
fn request_state(envelope: &Envelope, operation_id: String) -> Request {
    Request {
        stream_id: envelope.stream_id,
        operation_id,
        session_id: envelope.session_id.clone(),
        version: envelope.version,
        attachment: None,
        queued_input: VecDeque::new(),
        terminal: false,
        deadline: None,
    }
}
fn deadline_from_open(deadlines: &gwz_transport::protocol::Deadlines) -> Option<u64> {
    (deadlines.connect_ms != 0).then(|| {
        (deadlines.allocation_ms as u64)
            .saturating_add(deadlines.connect_ms as u64)
            .saturating_add(deadlines.interaction_ms as u64)
    })
}
fn envelope_for(
    state: &Request,
    kind: MessageKind,
    failure: Option<Failure>,
    opened: Option<Opened>,
) -> Envelope {
    Envelope {
        version: state.version,
        session_id: state.session_id.clone(),
        stream_id: state.stream_id,
        kind,
        open_failed: (kind == MessageKind::OpenFailed)
            .then_some(failure.clone())
            .flatten(),
        identity_check_failed: (kind == MessageKind::IdentityCheckFailed)
            .then_some(failure.clone())
            .flatten(),
        failed: (kind == MessageKind::Failed).then_some(failure).flatten(),
        opened,
        identity_checked: (kind == MessageKind::IdentityChecked)
            .then_some(gwz_transport::protocol::IdentityChecked {}),
        ..Default::default()
    }
}
fn failure_for(error: io::Error) -> Failure {
    if let Some(failure) = error
        .get_ref()
        .and_then(|cause| cause.downcast_ref::<super::ssh_worker::EndpointOpenFailure>())
    {
        return failure.0.clone();
    }
    let code = match error.kind() {
        io::ErrorKind::TimedOut => ErrorCode::Timeout,
        io::ErrorKind::PermissionDenied | io::ErrorKind::NotFound => ErrorCode::Unavailable,
        io::ErrorKind::InvalidInput => ErrorCode::InvalidRequest,
        io::ErrorKind::WouldBlock => ErrorCode::Capacity,
        io::ErrorKind::ConnectionAborted | io::ErrorKind::BrokenPipe => ErrorCode::CarrierLost,
        _ => ErrorCode::Io,
    };
    Failure {
        code,
        effect: Effect::None,
        facts: None,
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        #[path = "../../../tests/transport_ssh/support/placement_checks.rs"]
        mod check_tests;
    }
}

use super::*;

impl PlacementEndpoint {
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
                        setup_cause: None,
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
                        setup_cause: None,
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
                        setup_cause: None,
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
        if envelope.kind == MessageKind::Cancel && state.attachment.is_some() {
            // The worker's stream ends on the initiator's Cancel without a
            // terminal of its own, so the endpoint answers it, as it answers
            // a cancelled request. Both muxes then retire the route now, not
            // at the initiator's cleanup deadline.
            self.cancel_stream(&key);
            return Ok(());
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
            let mut state = request_state(&envelope);
            state.terminal = true;
            let message = envelope_for(
                &state,
                MessageKind::OpenFailed,
                Some(Failure {
                    setup_cause: None,
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
        let (pool_key, repository_path) = destination(&open.destination)?;
        if !self.admits_open(&pool_key) {
            let state = request_state(&envelope);
            let now = self.now();
            let deadline = now.saturating_add(open.deadlines.allocation_ms as u64);
            self.requests.insert(key.clone(), state);
            self.queued_opens.push_back(QueuedOpen {
                key,
                pool_key,
                envelope,
                admitted_at: now,
                deadline,
            });
            return Ok(());
        }
        let selected = match selected_path(&self.home, &open.identity) {
            Ok(selected) => selected,
            Err(_) => {
                let mut state = request_state(&envelope);
                state.terminal = true;
                let message = envelope_for(
                    &state,
                    MessageKind::OpenFailed,
                    Some(Failure {
                        setup_cause: None,
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
                session_id: envelope.session_id.clone(),
                version: envelope.version,
                attachment: None,
                queued_input: VecDeque::new(),
                terminal: false,
            },
        );
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let service = native_service(open.service);
        let context = BridgeContext {
            session_id: envelope.session_id.clone(),
            stream_id: envelope.stream_id,
            version: envelope.version,
            limits: open.receive_limits.clone(),
            deadlines: open.deadlines.clone(),
            waker: self.waker.clone(),
        };
        // The worker owns the open until it replies. No thread or supervised
        // job waits for it, so opens leave the job budget to their setups.
        let reply = match self.endpoint.start_endpoint_open(
            pool_key.clone(),
            selected,
            service,
            &repository_path,
            context,
            cancelled.clone(),
        ) {
            Ok(reply) => reply,
            Err(error) => {
                // Refused before the worker took it: fail it as its reply would.
                let message = {
                    let state = self.requests.get_mut(&key).expect("open request");
                    state.terminal = true;
                    envelope_for(
                        state,
                        MessageKind::OpenFailed,
                        Some(failure_for(error)),
                        None,
                    )
                };
                self.push_outbound(key.0, message);
                return Ok(());
            }
        };
        self.opens.push(OpenJob {
            key,
            pool_key,
            reply,
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
                let mut state = request_state(&envelope);
                state.terminal = true;
                let message = envelope_for(
                    &state,
                    MessageKind::IdentityCheckFailed,
                    Some(Failure {
                        setup_cause: None,
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
            .map_err(|error| {
                // A full job budget is backpressure: the host offers the check
                // again on a later pass. Nothing of it is held yet.
                if error.kind() == io::ErrorKind::WouldBlock {
                    EndpointError::WouldBlock
                } else {
                    EndpointError::Capacity
                }
            })?;
        self.requests.insert(
            key.clone(),
            Request {
                stream_id: envelope.stream_id,
                session_id: envelope.session_id,
                version: envelope.version,
                attachment: None,
                queued_input: VecDeque::new(),
                terminal: false,
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

    pub(super) fn start_queued(&mut self, now: u64) -> Result<(), EndpointError> {
        for _ in 0..self.queued_opens.len() {
            let mut queued = self.queued_opens.pop_front().expect("queued length");
            if now >= queued.deadline {
                if let Some(state) = self.requests.get_mut(&queued.key) {
                    state.terminal = true;
                    let message = envelope_for(
                        state,
                        MessageKind::OpenFailed,
                        Some(Failure {
                            setup_cause: None,
                            code: ErrorCode::Timeout,
                            effect: Effect::None,
                            facts: None,
                        }),
                        None,
                    );
                    self.push_outbound(queued.key.0, message);
                }
            } else if self.admits_open(&queued.pool_key) {
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

    /// An open starts while the opens in flight stay within the operation's
    /// limits, which the transport host installs in the pool before the
    /// operation's first open: the per-host and per-user ceilings of the open's
    /// host, and `open_ceiling` across every host.
    fn admits_open(&self, key: &Key) -> bool {
        let capacity = self.endpoint.pool().capacity();
        let host = self
            .opens
            .iter()
            .filter(|open| open.pool_key.host == key.host);
        self.opens.len() < open_ceiling(capacity)
            && host.clone().count() < capacity.per_host
            && host
                .filter(|open| open.pool_key.username == key.username)
                .count()
                < capacity.per_user_host
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
fn native_service(service: GitService) -> NativeService {
    match service {
        GitService::UploadPackAdvertisement | GitService::UploadPackExchange => {
            NativeService::UploadPack
        }
        GitService::ReceivePackAdvertisement | GitService::ReceivePackExchange => {
            NativeService::ReceivePack
        }
    }
}
fn deadline_from_open(deadlines: &gwz_transport::protocol::Deadlines) -> Option<u64> {
    (deadlines.connect_ms != 0).then(|| {
        (deadlines.allocation_ms as u64)
            .saturating_add(deadlines.connect_ms as u64)
            .saturating_add(deadlines.interaction_ms as u64)
    })
}

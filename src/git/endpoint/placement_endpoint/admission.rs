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
                        detail: None,
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
                        detail: None,
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
                self.retries
                    .machine(&key.0, &retry_key(&open.pool_key, &open.envelope))
                    .abandoned(&key);
                state.terminal = true;
                let message = envelope_for(
                    state,
                    MessageKind::OpenFailed,
                    Some(Failure {
                        detail: None,
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
                    detail: None,
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
        let (pool_key, _) = destination(&open.destination)?;
        let now = self.now();
        let allocation = AllocationClock::new(now, open.deadlines.allocation_ms.max(0) as u64);
        let mut state = request_state(&envelope);
        state.site = Some(pool_key.site());
        self.requests.insert(key.clone(), state);
        self.admit(
            QueuedOpen {
                key,
                pool_key,
                envelope,
                allocation,
                attempts: 0,
            },
            now,
        )
    }

    /// Starts `queued` when its key's retry machine and the operation's
    /// limits allow it, finishes it when its key is closed or its allocation
    /// ran out, and otherwise queues it. While the key holds it, its
    /// allocation clock stops.
    fn admit(&mut self, mut queued: QueuedOpen, now: u64) -> Result<(), EndpointError> {
        let decision = self
            .retries
            .machine(
                &queued.key.0,
                &retry_key(&queued.pool_key, &queued.envelope),
            )
            .decide(now);
        match decision {
            // The key's recorded failure, without the facts of the setup
            // that recorded it: this member set nothing up in this pass.
            Decision::Finish(last) => {
                let failure = Failure {
                    facts: None,
                    ..last.wire_failure()
                };
                self.fail_open(&queued.key, failure);
                Ok(())
            }
            Decision::Wait => {
                queued.allocation.stop(now);
                self.queued_opens.push_back(queued);
                Ok(())
            }
            Decision::Start => {
                let governor = self.endpoint.governor().scoped(&queued.key.0);
                let pool_now = self.endpoint.pool_now();
                let mut admission = governor.admission(&queued.pool_key, pool_now);
                // A due test of the site's limit starts through this member
                // when it cannot start otherwise, the gate shut or the site
                // full (§4.5, §4.7): never a probe on its final attempt.
                let final_attempt = queued.attempts >= self.retries.max_retries(&queued.key.0);
                let idle = self.idle_may_exist(&queued.pool_key, &admission);
                let mut carries_test = false;
                if !(admission.gate_open && (admission.room || idle))
                    && let Some(target) =
                        governor.start_test(&queued.pool_key, final_attempt, pool_now)
                {
                    admission = Admission {
                        gate_open: true,
                        target,
                        ..admission
                    };
                    carries_test = true;
                }
                if !admission.gate_open {
                    // Behind a hold of its site (§5.2): the wait is the
                    // server's word, and the allocation clock stops.
                    queued.allocation.stop(now);
                    self.queued_opens.push_back(queued);
                    return Ok(());
                }
                queued.allocation.run(now);
                let left = queued.allocation.left(now);
                // Below the ceiling a member that cannot lease an idle
                // connection waits for room in the endpoint, where its
                // allocation clock stops (§5.2), not in the pool.
                if !carries_test && !admission.room && !idle {
                    queued.allocation.stop(now);
                    self.queued_opens.push_back(queued);
                    return Ok(());
                }
                if left > 0 && self.admits_open(&queued.pool_key, admission.target) {
                    self.start_attempt(queued, now, left, carries_test);
                    return Ok(());
                }
                if carries_test {
                    governor.test_unused(&queued.pool_key, pool_now);
                }
                if left == 0 {
                    self.fail_open(&queued.key, setup_retry::allocation_timeout());
                } else {
                    self.queued_opens.push_back(queued);
                }
                Ok(())
            }
        }
    }

    /// One attempt of `queued`'s open, with the allocation it has `left` and
    /// fresh network clocks.
    fn start_attempt(&mut self, queued: QueuedOpen, now: u64, left: u64, carries_test: bool) {
        let QueuedOpen {
            key,
            pool_key,
            envelope,
            attempts,
            ..
        } = queued;
        let attempts = attempts + 1;
        let open = envelope.open.as_ref().expect("admitted Open");
        let selected = match selected_path(&self.home, &open.identity) {
            Ok(selected) => selected,
            Err(_) => {
                self.fail_open(
                    &key,
                    Failure {
                        detail: None,
                        setup_cause: None,
                        code: ErrorCode::InvalidRequest,
                        effect: Effect::None,
                        facts: None,
                    },
                );
                return;
            }
        };
        let mut deadlines = open.deadlines.clone();
        deadlines.allocation_ms = left.min(i64::MAX as u64) as i64;
        let attempt_deadline =
            deadline_from_open(&deadlines).map(|duration| now.saturating_add(duration));
        self.retries
            .machine(&key.0, &retry_key(&pool_key, &envelope))
            .start(key.clone());
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let context = BridgeContext {
            session_id: envelope.session_id.clone(),
            stream_id: envelope.stream_id,
            version: envelope.version,
            limits: open.receive_limits.clone(),
            deadlines,
            waker: self.waker.clone(),
        };
        // The worker owns the open until it replies. No thread or supervised
        // job waits for it, so opens leave the job budget to their setups.
        match self.endpoint.start_endpoint_open(
            pool_key.clone(),
            selected,
            native_service(open.service),
            &open.destination.path,
            context,
            cancelled.clone(),
        ) {
            Ok(reply) => self.opens.push(OpenJob {
                key,
                pool_key,
                reply,
                cancelled,
                deadline: attempt_deadline,
                abandoned: false,
                envelope,
                attempts,
                carries_test,
            }),
            // Refused before the worker took it: fail it as its reply would.
            Err(error) => self.attempt_failed(key, pool_key, envelope, &error, attempts, now),
        }
    }

    /// Keeps `facts`, an attempt's, among the member's facts so far, and
    /// returns them all.
    pub(super) fn keep_facts(
        &mut self,
        key: &RequestKey,
        facts: Option<gwz_transport::protocol::Facts>,
    ) -> Option<gwz_transport::protocol::Facts> {
        let state = self.requests.get_mut(key)?;
        state.facts = setup_retry::merged_facts(state.facts.take(), facts);
        state.facts.clone()
    }

    /// Ends a member's open with `failure`, unless it has ended already. The
    /// failure carries every fact of the member's attempts.
    pub(super) fn fail_open(&mut self, key: &RequestKey, failure: Failure) {
        let facts = self.keep_facts(key, failure.facts.clone());
        if let Some(state) = self.requests.get_mut(key) {
            if state.terminal {
                return;
            }
            state.terminal = true;
            let failure = Failure { facts, ..failure };
            let message = envelope_for(state, MessageKind::OpenFailed, Some(failure), None);
            self.push_outbound(key.0.clone(), message);
        }
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
                        detail: None,
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
                site: None,
                facts: None,
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
        self.report_demand();
        for _ in 0..self.queued_opens.len() {
            let queued = self.queued_opens.pop_front().expect("queued length");
            self.admit(queued, now)?;
        }
        Ok(())
    }

    /// Tells each operation's limit machines how many queued members want a
    /// new connection on a site, and how many of them could carry a probe
    /// (not on their final attempt, §4.7). A site with none queued is told
    /// zero, so a test is never started with no one to carry it.
    pub(super) fn report_demand(&mut self) {
        let pool_now = self.endpoint.pool_now();
        let mut demand: Vec<(String, Key, usize, usize)> = self
            .opens
            .iter()
            .map(|open| (open.key.0.clone(), open.pool_key.clone(), 0, 0))
            .collect();
        for queued in &self.queued_opens {
            let operation = &queued.key.0;
            let spare = queued.attempts < self.retries.max_retries(operation);
            match demand
                .iter_mut()
                .find(|(op, key, ..)| op == operation && *key == queued.pool_key)
            {
                Some(entry) => {
                    entry.2 += 1;
                    entry.3 += usize::from(spare);
                }
                None => demand.push((
                    operation.clone(),
                    queued.pool_key.clone(),
                    1,
                    usize::from(spare),
                )),
            }
        }
        for (operation, key, needing, spare) in demand {
            self.endpoint
                .governor()
                .scoped(&operation)
                .set_demand(&key, needing, spare, pool_now);
        }
    }

    /// Whether a connection of the site may be idle: more are set up than
    /// carry a stream. An overestimate costs a wait in the pool, as at the
    /// ceiling; an underestimate would hold a member that could lease.
    fn idle_may_exist(&self, key: &Key, admission: &Admission) -> bool {
        let site = key.site();
        let streaming = self
            .requests
            .values()
            .filter(|state| state.attachment.is_some() && state.site.as_ref() == Some(&site))
            .count();
        admission.connected > streaming
    }

    /// An open starts while the opens in flight stay within the operation's
    /// limits, which the transport host installs in the pool before the
    /// operation's first open: the per-host and per-user ceilings of the open's
    /// host, `open_ceiling` across every host, and `target`, the believed
    /// limit of the open's site that its limit machine sets (the ceiling
    /// until a limit is found).
    fn admits_open(&self, key: &Key, target: usize) -> bool {
        let capacity = self.endpoint.pool().capacity();
        let host = self
            .opens
            .iter()
            .filter(|open| open.pool_key.host == key.host);
        self.opens.len() < open_ceiling(capacity)
            && host.clone().count() < capacity.per_host
            && host
                .clone()
                .filter(|open| open.pool_key.username == key.username)
                .count()
                < capacity.per_user_host
            && host.filter(|open| open.pool_key.port == key.port).count() < target
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

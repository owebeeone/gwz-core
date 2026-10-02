use super::*;
impl Session {
    pub(in crate::transport_host) fn open(
        &self,
        request: &str,
        operation: &str,
        url: &str,
        service: GitService,
        identity: Identity,
        observe: Arc<dyn Fn(i64, &Opened) + Send + Sync>,
        facts: Arc<dyn Fn(&Facts) + Send + Sync>,
    ) -> io::Result<BlockingStream> {
        let mut destination = Destination::parse(url)?.ok_or(io::ErrorKind::Unsupported)?;
        // What the URL holds beyond the protocol's destination (TR2.18): the
        // host as written, when it is not the pool key's lowercased host, and
        // the password beside its user.
        let password = destination.password.take();
        let extras = (password.is_some() || destination.written_host != destination.key.host)
            .then(|| UrlExtras::new(destination.written_host.clone(), password));
        let policy = if identity.mode == IdentityMode::ExplicitKey {
            AuthPolicy::SshExplicit
        } else {
            AuthPolicy::SshAmbient
        };
        self.open_stream(
            request,
            operation,
            gwz_transport::protocol::Destination {
                scheme: Scheme::Ssh,
                ssh_username: destination.key.username,
                host: destination.key.host,
                port: destination.key.port as i64,
                path: destination.path,
            },
            match service {
                GitService::UploadPack => gwz_transport::protocol::GitService::UploadPackExchange,
                GitService::ReceivePack => gwz_transport::protocol::GitService::ReceivePackExchange,
            },
            identity,
            policy,
            None,
            None,
            observe,
            facts,
            extras,
        )
        .map_err(|failure| {
            let attempts = setup_retry::spent_budget(&failure, self.max_retries(request));
            failure_io(failure, attempts)
        })
    }
    pub(in crate::transport_host) fn open_https(
        &self,
        request: &str,
        operation: &str,
        url: &str,
        service: gwz_transport::protocol::GitService,
        policy: AuthPolicy,
        allocation_until: Option<Instant>,
        allocation_observer: Option<Arc<dyn Fn(i64) + Send + Sync>>,
        observe: Arc<dyn Fn(i64, &Opened) + Send + Sync>,
        facts: Arc<dyn Fn(&Facts) + Send + Sync>,
    ) -> Result<BlockingStream, Failure> {
        let destination = crate::git::endpoint::https_destination::Destination::parse(url)
            .map_err(protocol_failure)?;
        self.open_stream(
            request,
            operation,
            gwz_transport::protocol::Destination {
                scheme: Scheme::Https,
                ssh_username: None,
                host: destination.host().into(),
                port: destination.port() as i64,
                path: destination.url.path().into(),
            },
            service,
            Identity {
                mode: if policy == AuthPolicy::Gh {
                    IdentityMode::Ambient
                } else {
                    IdentityMode::CredentialsDisabled
                },
                ..Default::default()
            },
            policy,
            allocation_until,
            allocation_observer,
            observe,
            facts,
            None,
        )
    }
    /// Opens a stream. An SSH open's URL `extras` go to the endpoint session
    /// through the handoff they share in this process, if they share one,
    /// and are dropped otherwise (TR2.18).
    fn open_stream(
        &self,
        request: &str,
        operation: &str,
        destination: gwz_transport::protocol::Destination,
        service: gwz_transport::protocol::GitService,
        identity: Identity,
        policy: AuthPolicy,
        allocation_until: Option<Instant>,
        allocation_observer: Option<Arc<dyn Fn(i64) + Send + Sync>>,
        observe: Arc<dyn Fn(i64, &Opened) + Send + Sync>,
        facts: Arc<dyn Fn(&Facts) + Send + Sync>,
        mut extras: Option<UrlExtras>,
    ) -> Result<BlockingStream, Failure> {
        let reply = Wait::new();
        // Held until the open has its answer: dropping it drops whatever
        // extras the endpoint never took.
        let deposit;
        // A mux with no stream to spare is backpressure, not a failure: the
        // open waits for a stream to end, within its allocation deadline.
        let admit_until = allocation_until.unwrap_or_else(|| Instant::now() + ADMISSION);
        loop {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed {
                return Err(protocol_failure(
                    gwz_transport::protocol::ErrorCode::CarrierLost,
                ));
            }
            // A URL's password goes to an endpoint in this process only. One
            // in another process could not be given it, so its open refuses.
            if state.handoff.is_none() && extras.as_ref().is_some_and(|e| e.password().is_some()) {
                return Err(protocol_failure(
                    gwz_transport::protocol::ErrorCode::InvalidRequest,
                ));
            }
            let owner = state
                .owner
                .as_ref()
                .ok_or_else(|| protocol_failure(gwz_transport::protocol::ErrorCode::Unavailable))?;
            let binding = owner
                .binding()
                .ok_or_else(|| protocol_failure(gwz_transport::protocol::ErrorCode::Unavailable))?;
            let report_open_failure = destination.scheme == Scheme::Ssh;
            // Check after taking the session mutex: contention here is part of
            // admission too. Truncate sub-millisecond remainders and fail;
            // never send zero, which an endpoint could interpret as a default.
            let allocation_ms = match allocation_until {
                Some(until) => {
                    let remaining = until.saturating_duration_since(Instant::now()).as_millis();
                    if remaining == 0 {
                        return Err(setup_retry::allocation_timeout());
                    }
                    remaining.min(i64::MAX as u128) as i64
                }
                None => 30_000,
            };
            let open = Open {
                endpoint_id: binding.endpoint_id().into(),
                operation_id: operation.into(),
                destination: destination.clone(),
                service,
                identity: identity.clone(),
                policy,
                deadlines: network_deadlines(
                    state.io_timeout_ms,
                    state.connect_timeout_ms,
                    allocation_ms,
                ),
                receive_limits: binding.limits().clone(),
            };
            let opened = match &state.handoff {
                Some(handoff) => handoff.open(binding.session_id(), &mut extras, || {
                    owner.open(request, open)
                }),
                None => owner.open(request, open).map(|id| (id, None)),
            };
            let id = match opened {
                Ok((id, queued)) => {
                    deposit = queued;
                    id
                }
                Err(mux::Error::Capacity | mux::Error::WouldBlock) => {
                    drop(state);
                    if Instant::now() >= admit_until {
                        return Err(setup_retry::allocation_timeout());
                    }
                    self.wait_for_change(admit_until.saturating_duration_since(Instant::now()));
                    continue;
                }
                Err(_) => {
                    return Err(protocol_failure(
                        gwz_transport::protocol::ErrorCode::UnsupportedOperation,
                    ));
                }
            };
            if let Some(observer) = &allocation_observer {
                observer(allocation_ms);
            }
            let mut config = stream::Config::new(binding.session_id(), id, stream::Side::Initiator);
            config.profile_version = 2;
            config.receive_limits = binding.limits().clone();
            config.peer_limits = binding.limits().clone();
            config.receive_window = binding.limits().receive_window as usize;
            config.peer_receive_window = binding.limits().receive_window as usize;
            config.max_payload = config
                .max_payload
                .min(binding.limits().data_payload as usize);
            let (stream, peer) = Stream::new(config)
                .map_err(|_| protocol_failure(gwz_transport::protocol::ErrorCode::Protocol))?;
            let max_retries = state
                .registrations
                .get(request)
                .map_or(setup_retry::DEFAULT_MAX_RETRIES, |record| {
                    record.max_retries
                });
            let deadline = (state.io_timeout_ms != 0)
                .then(|| open_backstop_ms(state.io_timeout_ms, max_retries))
                .and_then(|backstop| Instant::now().checked_add(Duration::from_millis(backstop)));
            state.streams.insert(
                id,
                Entry {
                    request: request.into(),
                    stream,
                    peer: Arc::new(peer),
                    opened: false,
                    report_open_failure,
                    opening_cancel_effect: if report_open_failure {
                        Effect::Possible
                    } else {
                        Effect::None
                    },
                    reply: reply.clone(),
                    deadline,
                    pending: None,
                    next: None,
                    observe,
                    facts,
                },
            );
            break;
        }
        let answer = reply.get();
        drop(deposit);
        match answer {
            Ok((stream, _)) => Ok(stream),
            Err(failure) => Err(failure),
        }
    }
    /// Parks the calling thread until this session's state next changes, at
    /// most `limit`, and never long: the change can land before the listener.
    fn wait_for_change(&self, limit: Duration) {
        let listener = self.listener();
        let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
        let _ = listener.arm(&Context::from_waker(&waker));
        thread::park_timeout(limit.min(Duration::from_millis(10)));
    }
    pub(in crate::transport_host) fn check(
        &self,
        request: &str,
        identity: Identity,
    ) -> ModelResult<()> {
        let result = Wait::new();
        // As for an open, a full mux makes the check wait for a stream to end.
        let admit_until = Instant::now() + ADMISSION;
        loop {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed {
                return Err(unavailable("endpoint identity check unavailable"));
            }
            let owner = state
                .owner
                .as_ref()
                .ok_or_else(|| unavailable("endpoint not bound"))?;
            let id = match owner.check_identity(request, identity.clone(), CHECK_MS) {
                Ok(id) => id,
                Err(error @ (mux::Error::Capacity | mux::Error::WouldBlock)) => {
                    drop(state);
                    if Instant::now() >= admit_until {
                        return Err(mux_error(error));
                    }
                    self.wait_for_change(admit_until.saturating_duration_since(Instant::now()));
                    continue;
                }
                Err(error) => return Err(mux_error(error)),
            };
            state.checks.insert(
                id,
                Check {
                    request: request.into(),
                    result: result.clone(),
                },
            );
            break;
        }
        result.get().map_err(|failure| match failure.code {
            gwz_transport::protocol::ErrorCode::InvalidRequest => {
                invalid("invalid endpoint identity path")
            }
            gwz_transport::protocol::ErrorCode::UnsupportedOperation => {
                unsupported("endpoint identity unsupported")
            }
            gwz_transport::protocol::ErrorCode::Authentication
            | gwz_transport::protocol::ErrorCode::Unavailable => ModelError::new(
                crate::model::ErrorCode::PermissionDenied,
                "selected endpoint identity unavailable; no agent fallback",
            ),
            _ => unavailable("endpoint identity check failed"),
        })
    }
    /// One pass. `waker` wakes the thread that runs passes; the result says
    /// whether the pass moved a message, so that another pass runs at once.
    pub(super) fn drive(&self, waker: &Waker) -> bool {
        let mut moved = false;
        let mut reports: Vec<Box<dyn FnOnce() -> bool + Send>> = Vec::new();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let now = self.origin.elapsed().as_millis().min(u64::MAX as u128) as u64;
        let mut cx = Context::from_waker(waker);
        if let Some(owner) = &state.owner {
            owner.advance(now);
        }
        if state
            .owner
            .as_ref()
            .is_some_and(|o| o.phase() == Phase::Closed)
            && !state.closed
        {
            Self::close_state(&mut state);
        }
        if let Some(engine) = &mut state.engine {
            if engine.step(now, &mut cx).is_err() {
                Self::close_state(&mut state);
            }
        }
        if let Some(engine) = &mut state.https {
            if engine.step(now, &mut cx).is_err() {
                Self::close_state(&mut state);
            }
        }
        if !state.closed {
            if let Some(owner) = state.owner.clone() {
                let held = self.test_hooks.should_hold_pump();
                for _ in 0..64 {
                    if held {
                        break;
                    }
                    let item = if let Some(item) = state.incoming.take() {
                        item
                    } else {
                        match pin!(owner.next_action()).poll(&mut cx) {
                            Poll::Ready(Ok(Some(item))) => item,
                            Poll::Ready(Ok(None)) | Poll::Pending => break,
                            Poll::Ready(Err(_)) => {
                                Self::close_state(&mut state);
                                break;
                            }
                        }
                    };
                    let (request, message) = item;
                    let https = message
                        .open
                        .as_ref()
                        .is_some_and(|o| o.destination.scheme == Scheme::Https)
                        || state
                            .https
                            .as_ref()
                            .is_some_and(|e| e.owns(&request, message.stream_id));
                    if state.engine.is_some() {
                        let result = if https {
                            state
                                .https
                                .as_mut()
                                .ok_or(EndpointError::InvalidRequest)
                                .and_then(|engine| engine.accept(request.clone(), message.clone()))
                        } else {
                            state
                                .engine
                                .as_mut()
                                .expect("endpoint")
                                .accept(request.clone(), message.clone())
                        };
                        match result {
                            Ok(()) => moved = true,
                            Err(EndpointError::WouldBlock) => {
                                state.incoming = Some((request, message));
                                break;
                            }
                            Err(_) => {
                                Self::close_state(&mut state);
                                break;
                            }
                        }
                        continue;
                    }
                    moved = true;
                    if let Some(check) = state.checks.remove(&message.stream_id) {
                        let result = match message.kind {
                            MessageKind::IdentityChecked => Ok(()),
                            MessageKind::IdentityCheckFailed => Err(message
                                .identity_check_failed
                                .expect("validated check failure")),
                            _ => Err(protocol_failure(
                                gwz_transport::protocol::ErrorCode::Cancelled,
                            )),
                        };
                        check.result.complete(result);
                        continue;
                    }
                    if let Some(entry) = state.streams.get_mut(&message.stream_id) {
                        if let Some(opened) = message.opened.clone() {
                            entry.opened = true;
                            let observe = entry.observe.clone();
                            let value = opened.clone();
                            let reply = entry.reply.clone();
                            let stream = entry.stream.clone();
                            reports.push(Box::new(move || {
                                reply.complete_with(
                                    Ok((BlockingStream::new(stream), opened)),
                                    || {
                                        observe(message.stream_id, &value);
                                    },
                                );
                                true
                            }));
                        } else if let Some(failure) = message.open_failed.clone() {
                            let report = entry.facts.clone();
                            let report_open_failure = entry.report_open_failure;
                            let reply = entry.reply.clone();
                            let peer = entry.peer.clone();
                            reports.push(Box::new(move || {
                                if report_open_failure {
                                    if let Some(facts) = &failure.facts {
                                        report(facts);
                                    }
                                }
                                reply.complete(Err(failure));
                                peer.disconnect();
                                true
                            }));
                        } else {
                            let facts = message
                                .failed
                                .as_ref()
                                .and_then(|f| f.facts.clone())
                                .or_else(|| message.closed.as_ref().map(|c| c.facts.clone()));
                            let report = entry.facts.clone();
                            let peer = entry.peer.clone();
                            reports.push(Box::new(move || {
                                if let Some(facts) = facts {
                                    report(&facts);
                                }
                                peer.deliver(message).is_ok()
                            }));
                        }
                    }
                }
                // Hand the mux every message the endpoints have ready, up to a
                // bound per pass. Alternate schemes; a busy SSH stream cannot
                // starve HTTPS.
                for _ in 0..MAX_HANDOFFS {
                    if state.pending.is_none() {
                        let first = if state.prefer_https {
                            state.https.as_mut().and_then(|e| e.take_outbound(&mut cx))
                        } else {
                            state.engine.as_mut().and_then(|e| e.take_outbound())
                        };
                        let item = first.or_else(|| {
                            if state.prefer_https {
                                state.engine.as_mut().and_then(|e| e.take_outbound())
                            } else {
                                state.https.as_mut().and_then(|e| e.take_outbound(&mut cx))
                            }
                        });
                        state.pending = item.map(|out| (out.request, out.envelope));
                        state.prefer_https = !state.prefer_https;
                    }
                    let Some(mut item) = state.pending.take() else {
                        break;
                    };
                    // Local seal makes the mux own the cancellation terminal. Late
                    // physical completion is cleanup only and cannot replace it.
                    let sealed = state
                        .registrations
                        .get(&item.0)
                        .is_some_and(|r| r.sealed.is_some());
                    if sealed {
                        moved = true;
                        continue;
                    }
                    if let Some(engine) = &mut state.https {
                        engine.before_handoff(&item.0, &mut item.1);
                    }
                    match owner.send(&item.0, &item.1) {
                        Ok(()) => {
                            moved = true;
                            if let Some(engine) = &mut state.https {
                                engine.handed_off(&item.0, &item.1);
                            }
                        }
                        Err(mux::Error::WouldBlock) => {
                            state.pending = Some(item);
                            break;
                        }
                        Err(mux::Error::InvalidRequest)
                            if matches!(
                                item.1.kind,
                                MessageKind::OpenFailed
                                    | MessageKind::IdentityCheckFailed
                                    | MessageKind::Failed
                                    | MessageKind::Closed
                            ) && state.session.as_deref() == Some(&item.1.session_id) =>
                        {
                            // Mux deadline can win against the endpoint worker's
                            // terminal completion for this same admitted stream.
                            moved = true;
                        }
                        Err(_) => {
                            Self::close_state(&mut state);
                            break;
                        }
                    }
                }
                let sealed: BTreeSet<String> = state
                    .registrations
                    .iter()
                    .filter(|(_, r)| r.sealed.is_some())
                    .map(|(id, _)| id.clone())
                    .collect();
                let session = state.session.clone();
                let mut failed = false;
                'streams: for entry in state.streams.values_mut().filter(|_| !held) {
                    entry.peer.advance(now);
                    if !entry.opened {
                        if entry.deadline.is_some_and(|at| Instant::now() >= at) {
                            entry.reply.complete(Err(protocol_failure(
                                gwz_transport::protocol::ErrorCode::Timeout,
                            )));
                            let _ = owner.cancel(&entry.request);
                            entry.peer.disconnect();
                        }
                        continue;
                    }
                    // Forward what the stream has ready, up to a bound per pass.
                    // A wait for its next message stays pending, so the
                    // client's next read, write or close wakes this thread.
                    for _ in 0..MAX_STREAM_MESSAGES {
                        if entry.pending.is_none() {
                            let next = entry.next.get_or_insert_with(|| next_message(&entry.peer));
                            if let Poll::Ready(result) = next.as_mut().poll(&mut cx) {
                                entry.next = None;
                                entry.pending = result.ok().flatten();
                            }
                        }
                        let Some(message) = entry.pending.take() else {
                            break;
                        };
                        // As above: the seal made the mux own this request's
                        // cancellation, so a late client message for one of its
                        // streams is stale. The mux may already have retired
                        // the route; forwarding it would fail the whole session.
                        if sealed.contains(&entry.request) {
                            moved = true;
                            continue;
                        }
                        match owner.send(&entry.request, &message) {
                            Ok(()) => moved = true,
                            Err(mux::Error::WouldBlock) => {
                                entry.pending = Some(message);
                                break;
                            }
                            Err(mux::Error::InvalidRequest)
                                if session.as_deref() == Some(&message.session_id)
                                    && message.version == 2 =>
                            {
                                // The mux retires a stream's route the moment it
                                // admits that stream's terminal (the endpoint's
                                // Closed or Failed, or its own deadline's) and
                                // queues the terminal for this pump, which hands it
                                // to the stream only after this pass. A client
                                // message taken meanwhile is stale: drop it. `send`
                                // answers InvalidRequest for a missing route, or for
                                // a route, session or version that doesn't match;
                                // the guard rules out the last two, and the route
                                // the mux gave this stream belongs to this entry's
                                // request, so here it means the route is gone.
                                moved = true;
                            }
                            Err(_) => {
                                failed = true;
                                break 'streams;
                            }
                        }
                    }
                }
                if failed {
                    Self::close_state(&mut state);
                }
                state.streams.retain(|_, entry| {
                    held || !entry.peer.stats().terminal || entry.pending.is_some()
                });
                if !held {
                    self.test_hooks.pumped();
                }
            }
        }
        let sealed: Vec<_> = state
            .registrations
            .iter()
            .filter_map(|(id, r)| r.sealed.map(|at| (id.clone(), at)))
            .collect();
        for (id, at) in sealed {
            let pending = state
                .engine
                .as_ref()
                .is_some_and(|e| e.pending_request(&id))
                || state
                    .https
                    .as_ref()
                    .is_some_and(|e| e.pending_request_count(&id) > 0)
                || state.streams.values().any(|e| e.request == id)
                || state.checks.values().any(|e| e.request == id)
                || state.pending.as_ref().is_some_and(|p| p.0 == id)
                || state.incoming.as_ref().is_some_and(|p| p.0 == id);
            let retired = state.registrations[&id].mux_retired
                || state
                    .owner
                    .as_ref()
                    .is_none_or(|o| o.finish(&id).is_ok() || o.phase() == Phase::Closed);
            if retired {
                // finish removes the mux request. Repeating it would return
                // InvalidRequest and erase proof that retirement completed.
                state.registrations.get_mut(&id).unwrap().mux_retired = true;
            }
            if (!pending && retired) || at.elapsed() >= CLEANUP || state.closed {
                if at.elapsed() >= CLEANUP && !retired {
                    Self::close_state(&mut state);
                }
                let count = state
                    .engine
                    .as_ref()
                    .map_or(0, |e| e.pending_request_count(&id))
                    + state
                        .https
                        .as_ref()
                        .map_or(0, |e| e.pending_request_count(&id));
                if let Some(record) = state.registrations.get_mut(&id) {
                    record.result.get_or_insert(CleanupReport {
                        pending_local_work: count,
                        peer_cleanup_confirmed: false,
                    });
                }
            }
        }
        drop(state);
        moved |= !reports.is_empty();
        for report in reports {
            if !report() {
                self.close();
            }
        }
        moved
    }
}
/// The endpoint messages one pass hands to the mux, at most.
const MAX_HANDOFFS: usize = 64;
/// How long an open or a check without its own allocation deadline waits for
/// the mux to have a stream to spare; an open's default allocation is as long.
const ADMISSION: Duration = Duration::from_secs(30);
/// The messages one pass forwards from one client stream, at most.
const MAX_STREAM_MESSAGES: usize = 8;
/// How long an open waits for its endpoint's answer: each of its
/// `max_retries + 1` setup attempts may use the 120 s interaction allowance,
/// the 30 s aggregate, a stall and the Open's cleanup allowance, and the
/// waits between them come on top (the retry plan's §5 bound). The cleanup
/// allowance counts because the endpoint disposes a failed setup, joining
/// its setup thread, before it reports the failure.
fn open_backstop_ms(io_timeout_ms: u64, max_retries: u32) -> u64 {
    let attempt = OPEN_ATTEMPT_MS
        .saturating_add(io_timeout_ms)
        .saturating_add(OPEN_CLEANUP_MS);
    u64::from(max_retries)
        .saturating_add(1)
        .saturating_mul(attempt)
        .saturating_add(setup_retry::wait_bound_ms(max_retries))
}
const OPEN_ATTEMPT_MS: u64 = 150_000;
/// The cleanup allowance each Open grants its endpoint.
const OPEN_CLEANUP_MS: u64 = 5_000;
fn network_deadlines(io_timeout_ms: u64, connect_timeout_ms: u64, allocation_ms: i64) -> Deadlines {
    Deadlines {
        allocation_ms,
        connect_ms: connect_timeout_ms as i64,
        io_ms: io_timeout_ms as i64,
        interaction_ms: 120000,
        cleanup_ms: OPEN_CLEANUP_MS as i64,
    }
}
/// An SSH open's failure, and the attempt it ended as `(N, M)` when that is
/// known: its display then ends `(attempt N of M)` (the retry plan's §5).
#[derive(Debug)]
pub(crate) struct SshOpenFailure(pub(crate) Failure, pub(crate) Option<(u32, u32)>);
impl std::fmt::Display for SshOpenFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.code == gwz_transport::protocol::ErrorCode::Timeout {
            let label = setup_retry::timeout_origin(self.0.setup_cause).unwrap_or("unknown");
            write!(f, "ssh setup timeout: {label}")?;
        } else {
            write!(f, "ssh setup failed: {:?}", self.0.code)?;
        }
        if let Some((attempt, attempts)) = self.1 {
            write!(f, " (attempt {attempt} of {attempts})")?;
        }
        Ok(())
    }
}
impl std::error::Error for SshOpenFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        if self.0.code == gwz_transport::protocol::ErrorCode::Authentication {
            Some(&crate::git::endpoint::ssh_remote::AuthenticationRejected)
        } else {
            None
        }
    }
}
fn failure_io(failure: Failure, attempts: Option<(u32, u32)>) -> io::Error {
    let kind = match failure.code {
        gwz_transport::protocol::ErrorCode::Authentication => io::ErrorKind::PermissionDenied,
        gwz_transport::protocol::ErrorCode::Timeout => io::ErrorKind::TimedOut,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, SshOpenFailure(failure, attempts))
}
cfg_if::cfg_if! {
    if #[cfg(test)] {
        use crate::git::endpoint::agent_job::{
            Control, ManualClock, TimeoutReason, timeout_reason,
        };
        use gwz_transport::protocol::{Effect, ErrorCode, Facts, Failure};

        fn stamped(reason: TimeoutReason) -> Failure {
            Failure {
                setup_cause: Some(reason.setup_cause()),
                code: ErrorCode::Timeout,
                effect: Effect::None,
                facts: Some(Facts::default()),
            }
        }

        #[test]
        fn default_ssh_open_splits_stall_and_aggregate() {
            let deadlines = network_deadlines(9_000, 30_000, 30_000);
            assert_eq!(deadlines.io_ms, 9_000);
            assert_eq!(deadlines.connect_ms, 30_000);
        }

        #[test]
        fn default_https_open_splits_stall_and_aggregate() {
            let deadlines = network_deadlines(9_000, 30_000, 30_000);
            assert_eq!(deadlines.io_ms, 9_000);
            assert_eq!(deadlines.connect_ms, 30_000);
        }

        #[test]
        fn the_open_backstop_covers_every_attempt_and_the_waits_between_them() {
            // One attempt: the interaction allowance, the aggregate, a stall,
            // and the cleanup allowance the failed setup's disposal may use
            // before the endpoint reports it.
            assert_eq!(open_backstop_ms(9_000, 0), 164_000);
            // The default four attempts and their waits, at most 7.75 s.
            assert_eq!(open_backstop_ms(9_000, 3), 4 * 164_000 + 7_750);
            assert_eq!(open_backstop_ms(u64::MAX, u32::MAX), u64::MAX);
        }

        #[test]
        fn the_open_backstop_covers_the_full_bound_also_below_a_five_second_stall() {
            // The retry plan's §5 full bound: each attempt's 30 s aggregate,
            // 120 s interaction allowance and 5 s cleanup allowance, and the
            // waits. A stall shorter than the cleanup allowance does not stand
            // in for it.
            for max_retries in [0, 3, 10] {
                let full = u64::from(max_retries + 1) * (30_000 + 120_000 + 5_000)
                    + setup_retry::wait_bound_ms(max_retries);
                for stall in [1_000, 4_999, 9_000] {
                    assert!(
                        open_backstop_ms(stall, max_retries) >= full,
                        "stall {stall} ms, --max-retries {max_retries}"
                    );
                }
            }
        }

        #[test]
        fn disabled_native_timeout_clears_both_open_deadlines() {
            let deadlines = network_deadlines(0, 0, 30_000);
            assert_eq!(deadlines.io_ms, 0);
            assert_eq!(deadlines.connect_ms, 0);
        }

        #[test]
        fn idle_wait_is_a_setup_stall_not_a_peer_failure() {
            let clock = ManualClock::new();
            let start = clock.now();
            let control = Control::scripted(
                Some(start + std::time::Duration::from_secs(10)),
                std::time::Duration::from_secs(3),
                std::time::Duration::from_secs(5),
                clock.clock(),
            );
            control.begin_slice().unwrap();
            clock.advance(std::time::Duration::from_secs(3));
            let error = control.end_slice(false).unwrap_err();
            assert_eq!(timeout_reason(&error), Some(TimeoutReason::Stall));
            let reported = failure_io(stamped(TimeoutReason::Stall), None);
            assert_eq!(reported.kind(), io::ErrorKind::TimedOut);
            assert!(reported.to_string().contains("ssh setup timeout: stall"));
            assert_eq!(reported.get_ref().unwrap().downcast_ref::<SshOpenFailure>().unwrap().0.setup_cause, Some(gwz_transport::protocol::SetupFailureCause::Stall));
        }

        #[test]
        fn short_waits_past_the_aggregate_name_that_reason() {
            let clock = ManualClock::new();
            let start = clock.now();
            let control = Control::scripted(
                Some(start + std::time::Duration::from_millis(2_500)),
                std::time::Duration::from_millis(1_000),
                std::time::Duration::from_secs(5),
                clock.clock(),
            );
            for _ in 0..2 {
                control.begin_slice().unwrap();
                clock.advance(std::time::Duration::from_millis(800));
                control.end_slice(true).unwrap();
            }
            control.begin_slice().unwrap();
            clock.advance(std::time::Duration::from_millis(900));
            let error = control.end_slice(true).unwrap_err();
            assert_eq!(timeout_reason(&error), Some(TimeoutReason::Aggregate));
            let reported = failure_io(stamped(TimeoutReason::Aggregate), None);
            assert_eq!(reported.kind(), io::ErrorKind::TimedOut);
            assert!(reported.to_string().contains("ssh setup timeout: aggregate"));
            assert_eq!(reported.get_ref().unwrap().downcast_ref::<SshOpenFailure>().unwrap().0.setup_cause, Some(gwz_transport::protocol::SetupFailureCause::Aggregate));
        }

        #[test]
        fn a_spent_budget_ends_the_display_with_its_attempt() {
            let reported = failure_io(stamped(TimeoutReason::Stall), Some((4, 4)));
            assert_eq!(reported.to_string(), "ssh setup timeout: stall (attempt 4 of 4)");
            let reported = failure_io(stamped(TimeoutReason::Aggregate), Some((1, 1)));
            assert_eq!(reported.to_string(), "ssh setup timeout: aggregate (attempt 1 of 1)");
            // The reason string stays as it was where the attempt is not known.
            let reported = failure_io(stamped(TimeoutReason::Stall), None);
            assert_eq!(reported.to_string(), "ssh setup timeout: stall");
        }

        #[test]
        fn authentication_failure_stays_authentication() {
            let reported = failure_io(
                Failure {
                    setup_cause: None,
                    code: ErrorCode::Authentication,
                    effect: Effect::None,
                    facts: None,
                },
                None,
            );
            assert_eq!(reported.kind(), io::ErrorKind::PermissionDenied);
            assert!(
                reported
                    .get_ref()
                    .unwrap()
                    .downcast_ref::<SshOpenFailure>()
                    .is_some()
            );
            assert!(reported.get_ref().unwrap().source().is_some_and(|source| {
                source.is::<crate::git::endpoint::ssh_remote::AuthenticationRejected>()
            }));
        }
    }
}

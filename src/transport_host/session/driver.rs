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
        let destination = Destination::parse(url)?.ok_or(io::ErrorKind::Unsupported)?;
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
        )
        .map_err(|(_, failure)| failure_io(failure))
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
    ) -> Result<BlockingStream, (Option<i64>, Failure)> {
        let destination = crate::git::endpoint::https_destination::Destination::parse(url)
            .map_err(|failure| (None, protocol_failure(failure)))?;
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
        )
    }
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
    ) -> Result<BlockingStream, (Option<i64>, Failure)> {
        let reply = Wait::new();
        let stream_id;
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed {
                return Err((
                    None,
                    protocol_failure(gwz_transport::protocol::ErrorCode::CarrierLost),
                ));
            }
            let owner = state.owner.as_ref().ok_or_else(|| {
                (
                    None,
                    protocol_failure(gwz_transport::protocol::ErrorCode::Unavailable),
                )
            })?;
            let binding = owner.binding().ok_or_else(|| {
                (
                    None,
                    protocol_failure(gwz_transport::protocol::ErrorCode::Unavailable),
                )
            })?;
            let report_open_failure = destination.scheme == Scheme::Ssh;
            // Check after taking the session mutex: contention here is part of
            // admission too. Truncate sub-millisecond remainders and fail;
            // never send zero, which an endpoint could interpret as a default.
            let allocation_ms = match allocation_until {
                Some(until) => {
                    let remaining = until.saturating_duration_since(Instant::now()).as_millis();
                    if remaining == 0 {
                        return Err((
                            None,
                            protocol_failure(gwz_transport::protocol::ErrorCode::Timeout),
                        ));
                    }
                    remaining.min(i64::MAX as u128) as i64
                }
                None => 30_000,
            };
            let open = Open {
                endpoint_id: binding.endpoint_id().into(),
                operation_id: operation.into(),
                destination,
                service,
                identity,
                policy,
                deadlines: Deadlines {
                    allocation_ms,
                    connect_ms: state.io_timeout_ms as i64,
                    io_ms: state.io_timeout_ms as i64,
                    interaction_ms: 120000,
                    cleanup_ms: 5000,
                },
                receive_limits: binding.limits().clone(),
            };
            let id = owner.open(request, open).map_err(|_| {
                (
                    None,
                    protocol_failure(gwz_transport::protocol::ErrorCode::UnsupportedOperation),
                )
            })?;
            if let Some(observer) = &allocation_observer {
                observer(allocation_ms);
            }
            stream_id = Some(id);
            let mut config = stream::Config::new(binding.session_id(), id, stream::Side::Initiator);
            config.profile_version = 2;
            config.receive_limits = binding.limits().clone();
            config.peer_limits = binding.limits().clone();
            config.receive_window = binding.limits().receive_window as usize;
            config.peer_receive_window = binding.limits().receive_window as usize;
            config.max_payload = config
                .max_payload
                .min(binding.limits().data_payload as usize);
            let (stream, peer) = Stream::new(config).map_err(|_| {
                (
                    Some(id),
                    protocol_failure(gwz_transport::protocol::ErrorCode::Protocol),
                )
            })?;
            let deadline = (state.io_timeout_ms != 0)
                .then(|| Instant::now() + Duration::from_millis(150_000 + state.io_timeout_ms));
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
                    observe,
                    facts,
                },
            );
        }
        match reply.get() {
            Ok((stream, _)) => Ok(stream),
            Err(failure) => Err((stream_id, failure)),
        }
    }
    pub(in crate::transport_host) fn check(
        &self,
        request: &str,
        identity: Identity,
    ) -> ModelResult<()> {
        let result = Wait::new();
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed {
                return Err(unavailable("endpoint identity check unavailable"));
            }
            let owner = state
                .owner
                .as_ref()
                .ok_or_else(|| unavailable("endpoint not bound"))?;
            let id = owner
                .check_identity(request, identity, CHECK_MS)
                .map_err(mux_error)?;
            state.checks.insert(
                id,
                Check {
                    request: request.into(),
                    result: result.clone(),
                },
            );
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
    pub(super) fn drive(&self) {
        let mut reports: Vec<Box<dyn FnOnce() -> bool + Send>> = Vec::new();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let now = self.origin.elapsed().as_millis().min(u64::MAX as u128) as u64;
        let mut cx = Context::from_waker(Waker::noop());
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
                for _ in 0..64 {
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
                            Ok(()) => {}
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
                // Alternate schemes; a busy SSH stream cannot starve HTTPS.
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
                if let Some(mut item) = state.pending.take() {
                    // Local seal makes the mux own the cancellation terminal. Late
                    // physical completion is cleanup only and cannot replace it.
                    let sealed = state
                        .registrations
                        .get(&item.0)
                        .is_some_and(|r| r.sealed.is_some());
                    if !sealed {
                        if let Some(engine) = &mut state.https {
                            engine.before_handoff(&item.0, &mut item.1);
                        }
                        match owner.send(&item.0, &item.1) {
                            Ok(()) => {
                                if let Some(engine) = &mut state.https {
                                    engine.handed_off(&item.0, &item.1);
                                }
                            }
                            Err(mux::Error::WouldBlock) => state.pending = Some(item),
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
                            }
                            Err(_) => Self::close_state(&mut state),
                        }
                    }
                }
                let mut failed = false;
                for entry in state.streams.values_mut() {
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
                    if entry.pending.is_none() {
                        if let Poll::Ready(Ok(Some(message))) =
                            pin!(entry.peer.next_message()).poll(&mut cx)
                        {
                            entry.pending = Some(message);
                        }
                    }
                    if let Some(message) = entry.pending.take() {
                        match owner.send(&entry.request, &message) {
                            Ok(()) => {}
                            Err(mux::Error::WouldBlock) => entry.pending = Some(message),
                            Err(_) => {
                                failed = true;
                                break;
                            }
                        }
                    }
                }
                if failed {
                    Self::close_state(&mut state);
                }
                state
                    .streams
                    .retain(|_, entry| !entry.peer.stats().terminal || entry.pending.is_some());
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
        for report in reports {
            if !report() {
                self.close();
            }
        }
    }
}
fn failure_io(failure: Failure) -> io::Error {
    if failure.code == gwz_transport::protocol::ErrorCode::Authentication {
        return io::Error::new(
            io::ErrorKind::PermissionDenied,
            crate::git::endpoint::ssh_remote::AuthenticationRejected,
        );
    }
    io::Error::new(
        io::ErrorKind::Other,
        stream::Error::PeerFailed {
            code: failure.code,
            effect: failure.effect,
        },
    )
}

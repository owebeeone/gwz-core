use super::*;

impl Session {
    pub(in crate::transport_host) fn open(
        &self,
        request: &str,
        operation: &str,
        url: &str,
        service: GitService,
        identity: Identity,
        helpers_allowed: bool,
        observe: Arc<dyn Fn(i64, &Opened) + Send + Sync>,
        facts: Arc<dyn Fn(&Facts) + Send + Sync>,
    ) -> io::Result<BlockingStream> {
        let absent_ssh = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.engine.is_none() && state.endpoint_config.is_some()
        };
        if absent_ssh {
            return Err(io::ErrorKind::Unsupported.into());
        }
        let mut destination = Destination::parse(url)?.ok_or(io::ErrorKind::Unsupported)?;
        // What the URL holds beyond the protocol's destination (TR2.18): the
        // host as written, when it is not the pool key's lowercased host, and
        // the password beside its user.
        let password = destination.password.take();
        let extras = (!helpers_allowed
            || password.is_some()
            || destination.written_host != destination.key.host)
            .then(|| {
                UrlExtras::new(destination.written_host.clone(), password)
                    .with_helpers(helpers_allowed)
            });
        let shown = (destination.key.host.clone(), destination.key.port);
        // The agent this runtime's endpoint uses, which words a failure of the agent step.
        let handoff = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .handoff
            .clone();
        let agent = handoff
            .as_ref()
            .map(|handoff| handoff.agent().clone())
            .unwrap_or_default();
        // The stall bound this open's deadlines carry (`network_deadlines`
        // takes the same field), for a stall's message to name.
        let (stall_ms, known_hosts) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            (
                state.io_timeout_ms,
                state
                    .home
                    .as_ref()
                    .map(|home| KnownHostsFile::under(&home.path, home.source)),
            )
        };
        let key_file = key_file_name(&identity);
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
                https_username: None,
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
        .map_err(|Refusal { failure, note }| {
            let attempts = setup_retry::reported_attempt(&failure);
            // What the endpoint in this process found when it refused the host key, if it did (decision D12).
            let trust = handoff
                .as_ref()
                .and_then(|handoff| handoff.trust_found(&shown.0, shown.1));
            let mut error = open_failure_io(
                failure,
                attempts,
                stall_ms,
                shown.0.clone(),
                shown.1,
                known_hosts,
                key_file,
                &agent,
                trust,
            );
            note_ssh_end(&mut error, note, &shown.0, shown.1);
            error
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
    ) -> Result<BlockingStream, Refusal> {
        let destination = crate::git::endpoint::https_destination::Destination::parse(url)
            .map_err(protocol_failure)?;
        self.open_stream(
            request,
            operation,
            gwz_transport::protocol::Destination {
                scheme: Scheme::Https,
                ssh_username: None,
                https_username: (!destination.url.username().is_empty())
                    .then(|| destination.url.username().to_owned()),
                host: destination.host().into(),
                port: destination.port() as i64,
                path: destination.url.path().into(),
            },
            service,
            Identity {
                mode: if matches!(
                    policy,
                    AuthPolicy::Gh | AuthPolicy::WindowsConfigured | AuthPolicy::WindowsDefault
                ) {
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
    ) -> Result<BlockingStream, Refusal> {
        let reply = Wait::new();
        // Held until the open has its answer: dropping it drops whatever
        // extras the endpoint never took.
        let deposit;
        // Where this open's end note, if the endpoint leaves one, is taken from.
        let ended;
        // A mux with no stream to spare is backpressure, not a failure: the
        // open waits for a stream to end, within its allocation deadline.
        let ssh = destination.scheme == Scheme::Ssh;
        let allocation_allowance = if ssh {
            Duration::from_millis(
                self.state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .ssh_allocation_ms,
            )
        } else {
            ADMISSION
        };
        let admit_until = allocation_until.unwrap_or_else(|| Instant::now() + allocation_allowance);
        // A push's pump learns from this when the thread that owns the stream is producing and not waiting
        // for the server (GwzPushProducerBusyDesign). It goes to an endpoint in this process only.
        let mut producer = None;
        loop {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed {
                return Err(
                    protocol_failure(gwz_transport::protocol::ErrorCode::CarrierLost).into(),
                );
            }
            // A URL's password goes to an endpoint in this process only. One
            // in another process could not be given it, so its open refuses.
            if state.handoff.is_none()
                && extras
                    .as_ref()
                    .is_some_and(|e| e.password().is_some() || !e.helpers_allowed())
            {
                return Err(
                    protocol_failure(gwz_transport::protocol::ErrorCode::InvalidRequest).into(),
                );
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
            let allocation_ms = match allocation_until.or(ssh.then_some(admit_until)) {
                Some(until) => {
                    let remaining = until.saturating_duration_since(Instant::now()).as_millis();
                    if remaining == 0 {
                        return Err(setup_retry::allocation_timeout().into());
                    }
                    remaining.min(i64::MAX as u128) as i64
                }
                None => 30_000,
            };
            // The stall bound: the open's deadlines carry it, and so does its
            // stream, for a stall's message to name.
            let stall_ms = state.io_timeout_ms;
            let mut deadlines =
                network_deadlines(stall_ms, state.connect_timeout_ms, allocation_ms);
            if ssh {
                deadlines.interaction_ms = state.ssh_interaction_ms as i64;
            }
            let open = Open {
                endpoint_id: binding.endpoint_id().into(),
                operation_id: operation.into(),
                destination: destination.clone(),
                service,
                identity: identity.clone(),
                policy,
                deadlines,
                receive_limits: binding.limits().clone(),
            };
            if ssh
                && service == gwz_transport::protocol::GitService::ReceivePackExchange
                && state.handoff.is_some()
                && producer.is_none()
            {
                producer = Some(ProducerGate::new(state.producer_ceiling_ms));
            }
            let opened = match &state.handoff {
                Some(handoff) => {
                    handoff.open(binding.session_id(), &mut extras, producer.as_ref(), || {
                        owner.open(request, open)
                    })
                }
                None => owner.open(request, open).map(|id| (id, None)),
            };
            let id = match opened {
                Ok((id, queued)) => {
                    deposit = queued;
                    ended = state
                        .handoff
                        .clone()
                        .map(|handoff| (handoff, binding.session_id().to_owned(), id));
                    id
                }
                Err(mux::Error::Capacity | mux::Error::WouldBlock) => {
                    drop(state);
                    if Instant::now() >= admit_until {
                        return Err(setup_retry::allocation_timeout().into());
                    }
                    self.wait_for_change(admit_until.saturating_duration_since(Instant::now()));
                    continue;
                }
                Err(_) => {
                    return Err(protocol_failure(
                        gwz_transport::protocol::ErrorCode::UnsupportedOperation,
                    )
                    .into());
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
            let backstop = (state.io_timeout_ms != 0).then(|| {
                Backstop::new(
                    open_backstop_ms(state.io_timeout_ms, max_retries),
                    self.backstop_clock_ms(),
                )
            });
            state.streams.insert(
                id,
                Entry {
                    request: request.into(),
                    stall_ms,
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
                    backstop,
                    pending: None,
                    next: None,
                    observe,
                    facts,
                    producer: producer.take(),
                },
            );
            break;
        }
        let answer = reply.get();
        drop(deposit);
        match answer {
            Ok((stream, _)) => Ok(stream),
            Err(failure) => Err(Refusal {
                note: ended.and_then(|(handoff, session, id)| handoff.take_end(&session, id)),
                failure,
            }),
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
    // Only the Cli placement checks a path on its endpoint: the in-process endpoint is the driver's own process.
    cfg_if::cfg_if! {
        if #[cfg(all(gwz_transport_candidate, gwz_session_candidate, any(unix, windows)))] {
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
                    state.checks.insert(id, request, result.clone());
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
        }
    }
}

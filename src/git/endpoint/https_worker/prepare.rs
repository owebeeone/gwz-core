use super::*;

impl Client {
    /// The transport host's entry: one attempt of an open, continuing the
    /// budget and challenge it is given, and what the attempt's first connect
    /// did, which the open's key's retry machine learns (the retry plan's §4
    /// and §5).
    pub(crate) async fn prepare_attempt(
        &self,
        input: Input,
        cancel: &CancellationToken,
        budget: &mut Budget,
        challenge: &mut Option<ChallengeLease>,
    ) -> (Result<Prepared, Failure>, FirstConnect) {
        let (result, connect, _) = self
            .prepare_attempt_judged(input, cancel, budget, challenge)
            .await;
        (result, connect)
    }
    /// `prepare_attempt`, also saying what the limit machine made of the
    /// attempt's refusal, if the server refused it (§5.1).
    pub(crate) async fn prepare_attempt_judged(
        &self,
        input: Input,
        cancel: &CancellationToken,
        budget: &mut Budget,
        challenge: &mut Option<ChallengeLease>,
    ) -> (Result<Prepared, Failure>, FirstConnect, Option<Rejection>) {
        let mut connect = FirstConnect::None;
        let mut refusal = None;
        let result = self
            .run_attempt(input, cancel, budget, challenge, &mut connect, &mut refusal)
            .await;
        (result, connect, refusal)
    }
    /// The attempt itself, which records its first connect in `connect`.
    async fn run_attempt(
        &self,
        input: Input,
        cancel: &CancellationToken,
        budget: &mut Budget,
        challenge: &mut Option<ChallengeLease>,
        connect: &mut FirstConnect,
        refusal: &mut Option<Rejection>,
    ) -> Result<Prepared, Failure> {
        if cfg!(all(
            windows,
            gwz_transport_candidate,
            gwz_windows_https_qualification
        )) && !matches!(
            input.policy,
            AuthPolicy::Anonymous | AuthPolicy::WindowsDefault
        ) {
            return Err(failure(ErrorCode::UnsupportedOperation));
        }
        let original = Destination::parse(&input.destination).map_err(failure)?;
        if input.session.is_empty()
            || input.session.len() > 128
            || input.operation.is_empty()
            || input.operation.len() > 128
            || !matches!(
                input.policy,
                AuthPolicy::Anonymous
                    | AuthPolicy::Gh
                    | AuthPolicy::WindowsConfigured
                    | AuthPolicy::WindowsDefault
            )
        {
            return Err(failure(ErrorCode::InvalidRequest));
        }
        if cancel.is_cancelled() {
            return Err(failure(ErrorCode::Cancelled));
        }
        if input.policy == AuthPolicy::Gh && budget.allocation.as_millis() == 0 {
            return Err(helper_timeout(
                https_auth::AuthError::AllocationTimeout,
                &Facts {
                    method: AuthMethod::Gh,
                    ..Default::default()
                },
                0,
                0,
            ));
        }
        // Continuations cannot do helper/connection work on an exhausted domain.
        // Retained zero allowances are tombstones, never replaced by defaults.
        if budget.allocation.is_zero()
            || budget.cleanup.is_zero()
            || (input.policy == AuthPolicy::Gh && budget.helper.is_zero())
            || budget.connect.is_some_and(|remaining| remaining.is_zero())
            || budget.network.is_some_and(|remaining| remaining.is_zero())
        {
            return Err(failure(ErrorCode::Timeout));
        }
        let started = Instant::now();
        let mut slot = Some(acquire_slot(self.slots.clone(), budget.allocation, cancel).await?);
        budget.allocation = budget.allocation.saturating_sub(started.elapsed());
        let started = Instant::now();
        let mut dependency = Some(
            self.operation_within(&input.operation, budget.allocation, cancel)
                .await?,
        );
        budget.allocation = budget.allocation.saturating_sub(started.elapsed());
        let key = RouteKey::new(&input.operation, &original.base(), input.service);
        let mut destination = {
            let mut routes = self.routes.lock().unwrap_or_else(|e| e.into_inner());
            if https_policy::advertisement(input.service) {
                routes.admit(key.clone());
                if input.policy == AuthPolicy::Gh {
                    routes
                        .challenged(&key)
                        .map(Destination::parse)
                        .transpose()
                        .map_err(failure)?
                        .unwrap_or(original)
                } else {
                    original
                }
            } else {
                Destination::parse(routes.get(&key).map_err(failure)?).map_err(failure)?
            }
        };
        let native_policy = matches!(
            input.policy,
            AuthPolicy::WindowsConfigured | AuthPolicy::WindowsDefault
        );
        // Admission has finished; anchor once before any checkout/adoption.
        if native_policy && !budget.logical_started {
            budget.logical_started = true;
            budget.logical_deadline = budget
                .connect
                .map(|duration| {
                    Instant::now()
                        .checked_add(duration)
                        .ok_or(ErrorCode::InvalidRequest)
                })
                .transpose()
                .map_err(failure)?;
        }
        let native_route = if !https_policy::advertisement(input.service) && native_policy {
            self.routes
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .native_get(&key)
        } else {
            None
        };
        // A native service exchange requires the authenticated advertisement generation.
        let basic_route = input.policy == AuthPolicy::WindowsConfigured
            && self
                .routes
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .basic_get(&key);
        let mut hops = 0;
        let mut credential_offered = false;
        // The next checkout must open a new connection: a reused one died
        // before this hop's request was started (§6.1 (b) of
        // dev-docs/GwzTransportIdleLossDesign.md).
        let mut fresh = false;
        // The open's one retry has been used: by this attempt, or by an earlier
        // one whose connection a 401 carries here. A request that never
        // started is retried only once per open (§2.2 of the same note).
        let mut retried = false;
        let mut answer_challenge = input.policy == AuthPolicy::Gh
            || (!https_policy::advertisement(input.service) && basic_route);
        loop {
            let mut authorization = None;
            let mut facts = native_route
                .as_ref()
                .map_or_else(Facts::default, |auth| auth.facts.clone());
            facts.credential_offered |= credential_offered;
            let credential = if answer_challenge {
                facts.method = AuthMethod::Gh;
                let credential = self
                    .credential(&key, &destination, budget, cancel)
                    .await
                    .map_err(|mut failed| {
                        if let Some(facts) = failed.facts.as_mut() {
                            facts.credential_offered |= credential_offered;
                        }
                        failed
                    })?;
                authorization = Some(credential.header());
                Some(credential)
            } else {
                None
            };
            if budget.connect.is_some_and(|remaining| remaining.is_zero())
                || budget.network.is_some_and(|remaining| remaining.is_zero())
                || budget.allocation.is_zero()
            {
                return Err(with_facts(ErrorCode::Timeout, Effect::None, &facts));
            }
            if challenge.as_ref().is_some_and(ChallengeLease::expired) {
                *challenge = None;
            }
            if native_policy
                && budget
                    .logical_deadline
                    .is_some_and(|until| Instant::now() >= until)
            {
                return Err(with_facts(ErrorCode::Timeout, Effect::None, &facts));
            }
            let carried_lease = challenge.is_some();
            let lease = if let Some(carried) = challenge.take() {
                let mut carried = carried;
                if let Some(mut lease) = carried.take_for(&destination, &input) {
                    lease.reused = true;
                    lease.connect_elapsed = Duration::ZERO;
                    lease.allocation_elapsed = Duration::ZERO;
                    lease
                } else {
                    return Err(with_facts(ErrorCode::Protocol, Effect::None, &facts));
                }
            } else {
                // Only the first hop connects before the open's first request
                // byte, so its connect alone is the open's setup, and only its
                // failure keeps an origin a retry is decided by (§4).
                let first = hops == 0;
                let checkout = self.pool.checkout_scoped(
                    Key::https(destination.host(), destination.port()),
                    Owner::new(&input.session, &input.operation),
                    duration_ms(budget.allocation),
                    budget.logical_deadline.map_or_else(
                        || budget.connect.map_or(0, duration_ms),
                        |until| duration_ms(until.saturating_duration_since(Instant::now())),
                    ),
                    cancel,
                    credential
                        .as_ref()
                        .map(|c| c.scope.as_str())
                        .or_else(|| native_route.as_ref().map(|auth| auth.scope.as_str())),
                    std::mem::take(&mut fresh),
                    !retried,
                );
                let checkout = match budget.logical_deadline {
                    Some(until) => tokio::time::timeout_at(until, checkout)
                        .await
                        .unwrap_or_else(|_| Err((failure(ErrorCode::Timeout), Phase::Other))),
                    None => checkout.await,
                };
                let lease = checkout.map_err(|(error, phase)| {
                    let mut failed = with_facts(error.code, error.effect, &facts);
                    if first {
                        failed.setup_cause = error.setup_cause;
                        if phase == Phase::Setup {
                            *connect = FirstConnect::Failed;
                        }
                    }
                    failed
                })?;
                if first && !lease.reused {
                    *connect = FirstConnect::Connected;
                }
                lease
            };
            retried |= lease.retried;
            if native_policy
                && budget
                    .logical_deadline
                    .is_some_and(|until| Instant::now() >= until)
            {
                return Err(with_facts(ErrorCode::Timeout, Effect::None, &facts));
            }
            if let Some(auth) = &native_route
                && !auth.usable(&lease)
            {
                auth.revoke();
                return Err(with_facts(
                    ErrorCode::Authentication,
                    Effect::None,
                    &auth.facts,
                ));
            }
            if let Some(credential) = &credential {
                lease.scope(&credential.scope)?;
            }
            if let Some(remaining) = budget.connect.as_mut() {
                *remaining = remaining.saturating_sub(lease.connect_elapsed);
            }
            budget.allocation = budget.allocation.saturating_sub(lease.allocation_elapsed);
            let opened = Opened {
                connection_id: lease.id.clone(),
                reused: lease.reused,
                endpoint_id: "https-endpoint".into(),
                trust_owner: "endpoint-account".into(),
                facts: facts.clone(),
                receive_limits: gwz_transport::binding::default_limits(),
            };
            let mut prepared = Prepared {
                opened,
                lease: Some(lease),
                response: None,
                input: input.clone(),
                destination: destination.clone(),
                authorization,
                credential,
                native_route: native_route.clone(),
                _slot: slot.take(),
                _operation: dependency.take().expect("operation dependency"),
                protocol_error: Arc::new(AtomicBool::new(false)),
                io_ms: budget.network.map_or(0, duration_ms),
                cleanup_ms: duration_ms(budget.cleanup),
                discard: false,
            };
            if !https_policy::advertisement(input.service) {
                return Ok(prepared);
            }
            let (sender, body) = body_channel();
            drop(sender);
            let request = prepared.request(body)?;
            let connection = prepared
                .lease
                .as_ref()
                .unwrap()
                .connection
                .as_ref()
                .unwrap()
                .clone();
            let mut guard = connection.lock().await;
            if native_policy
                && budget
                    .logical_deadline
                    .is_some_and(|until| Instant::now() >= until)
            {
                return Err(with_facts(
                    ErrorCode::Timeout,
                    Effect::None,
                    &prepared.opened.facts,
                ));
            }
            let current_credential_offered = request.headers().contains_key(AUTHORIZATION);
            let offered_before = credential_offered;
            prepared.opened.facts.credential_offered =
                current_credential_offered || credential_offered;
            credential_offered = prepared.opened.facts.credential_offered;
            if prepared.opened.reused {
                // An exchange on a leased connection is an attempt of its
                // own: its window runs from here (§4.3). During a hold no
                // open begins its first exchange on a leased connection
                // (§4.5): the lease goes back unused, and the open asks
                // again when the hold has ended.
                if let Some(pooled) = prepared.lease.as_ref().and_then(HttpLease::pool_connection) {
                    let pool_key = Key::https(destination.host(), destination.port());
                    let began = self
                        .pool
                        .governor()
                        .scoped(&input.operation)
                        .exchange_begins(&pool_key, pooled, self.pool.now());
                    if !began && !carried_lease {
                        drop(guard);
                        drop(connection);
                        let Prepared {
                            _slot: returned_slot,
                            _operation: returned_dependency,
                            lease,
                            ..
                        } = prepared;
                        lease.unwrap().finish(Disposition::Reusable)?;
                        slot = returned_slot;
                        dependency = Some(returned_dependency);
                        credential_offered = offered_before;
                        self.wait_for_hold(&input.operation, &pool_key, cancel)
                            .await?;
                        continue;
                    }
                }
            }
            let header_started = Instant::now();
            let sent = tokio::select! {
                _=async { match budget.logical_deadline { Some(until) => tokio::time::sleep_until(until).await, None => std::future::pending().await } }, if native_policy => return Err(with_facts(ErrorCode::Timeout, Effect::None, &prepared.opened.facts)),
                _=cancel.cancelled()=>return Err(with_facts(ErrorCode::Cancelled, Effect::None, &prepared.opened.facts)),
                _=prepared.lease.as_ref().unwrap().cancel.cancelled()=>return Err(with_facts(if prepared.protocol_error.load(Ordering::Acquire){ErrorCode::Protocol}else{ErrorCode::Cancelled}, Effect::None, &prepared.opened.facts)),
                result=async {
                    match budget.network {
                        Some(remaining) => tokio::time::timeout(remaining, send_request(&mut guard.sender, request)).await.map_err(|_| failure(ErrorCode::Timeout)),
                        None => Ok(send_request(&mut guard.sender, request).await),
                    }
                }=>result.map_err(|error| with_facts(error.code, Effect::None, &prepared.opened.facts))?,
            };
            let mut response = match sent {
                Ok(response) => response,
                Err(SendFailure::NotStarted) if prepared.opened.reused && !retried => {
                    // Nothing reached the server: discard the connection and
                    // ask for a new one, with this attempt's slot and budget.
                    // Only a reused lease qualifies, and only an open that has
                    // not retried; the next lease is fresh, so an open is
                    // retried at most once.
                    drop(guard);
                    drop(connection);
                    let Prepared {
                        _slot: returned_slot,
                        _operation: returned_dependency,
                        lease,
                        ..
                    } = prepared;
                    lease.unwrap().finish(Disposition::Discarded)?;
                    slot = returned_slot;
                    dependency = Some(returned_dependency);
                    credential_offered = offered_before;
                    fresh = true;
                    retried = true;
                    continue;
                }
                Err(SendFailure::NotStarted) => {
                    return Err(with_facts(
                        ErrorCode::Io,
                        Effect::None,
                        &prepared.opened.facts,
                    ));
                }
                Err(SendFailure::Sent(error)) => {
                    return Err(with_facts(
                        classify_hyper_error(&error),
                        Effect::None,
                        &prepared.opened.facts,
                    ));
                }
            };
            if let Some(remaining) = budget.network.as_mut() {
                *remaining = remaining.saturating_sub(header_started.elapsed());
                if remaining.is_zero() {
                    return Err(with_facts(
                        ErrorCode::Timeout,
                        Effect::None,
                        &prepared.opened.facts,
                    ));
                }
                prepared.io_ms = duration_ms(*remaining);
            }
            if prepared.protocol_error.load(Ordering::Acquire) {
                return Err(with_facts(
                    ErrorCode::Protocol,
                    Effect::None,
                    &prepared.opened.facts,
                ));
            }
            drop(guard);
            drop(connection);
            if native_policy && response.status() == 401 {
                response = self
                    .authenticate(&mut prepared, response, &key, budget, cancel)
                    .await?;
            }
            if native_policy
                && budget
                    .logical_deadline
                    .is_some_and(|until| Instant::now() >= until)
            {
                return Err(with_facts(
                    ErrorCode::Timeout,
                    Effect::None,
                    &prepared.opened.facts,
                ));
            }
            let status = response.status().as_u16();
            prepared.opened.facts.http_status = Some(status as i64);
            *refusal = self.tell_governor(
                &input.operation,
                &prepared,
                &Key::https(destination.host(), destination.port()),
                status,
                response.headers(),
            );
            if matches!(status, 401 | 403)
                && let Some(credential) = &prepared.credential
            {
                credential.rejected.store(true, Ordering::Release);
            }
            if status == 401 && current_credential_offered {
                prepared.opened.facts.authenticated = Some(false);
            }
            match https_policy::classify(status, input.service) {
                ResponseAction::Success => {
                    validate_content(&response, input.service)
                        .map_err(|code| with_facts(code, Effect::None, &prepared.opened.facts))?;
                    self.routes
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .install(&key, &destination.base())
                        .map_err(|code| with_facts(code, Effect::None, &prepared.opened.facts))?;
                    prepared.response = Some(response);
                    return Ok(prepared);
                }
                ResponseAction::Redirect => {
                    if prepared.opened.facts.native.is_some() || budget.redirect_hops == 5 {
                        return Err(with_facts(
                            ErrorCode::UnsupportedOperation,
                            Effect::None,
                            &prepared.opened.facts,
                        ));
                    }
                    let mut values = response.headers().get_all(LOCATION).iter();
                    let location =
                        values.next().and_then(|v| v.to_str().ok()).ok_or_else(|| {
                            with_facts(
                                ErrorCode::InvalidRequest,
                                Effect::None,
                                &prepared.opened.facts,
                            )
                        })?;
                    if values.next().is_some() {
                        return Err(with_facts(
                            ErrorCode::InvalidRequest,
                            Effect::None,
                            &prepared.opened.facts,
                        ));
                    }
                    destination = destination
                        .redirect(input.service, location)
                        .map_err(|code| with_facts(code, Effect::None, &prepared.opened.facts))?;
                    drop(response);
                    // Keep admission across hops, but release physical capacity before acquiring again.
                    let Prepared {
                        _slot: returned_slot,
                        _operation: returned_dependency,
                        lease,
                        ..
                    } = prepared;
                    lease.unwrap().finish(Disposition::Discarded)?;
                    hops += 1;
                    budget.redirect_hops += 1;
                    answer_challenge = false;
                    // Re-enter with the existing slot below rather than reacquiring one.
                    slot = returned_slot;
                    dependency = Some(returned_dependency);
                    continue;
                }
                ResponseAction::Fail(code) => {
                    let mut failed = with_facts(code, Effect::None, &prepared.opened.facts);
                    let (basic, schemes) = challenges::schemes(response.headers());
                    let may_carry = self.auth.is_some()
                        && status == 401
                        && basic
                        && !current_credential_offered;
                    if status == 401 && !basic {
                        failed.detail = Some(Box::new(FailureDetail {
                            schemes: Some(schemes),
                            ..Default::default()
                        }));
                    }
                    if may_carry {
                        self.routes
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .challenge(&key, &destination.base())
                            .map_err(failure)?;
                    }
                    if may_carry {
                        let connection = prepared
                            .lease
                            .as_ref()
                            .unwrap()
                            .connection
                            .as_ref()
                            .unwrap();
                        match clean_challenge(
                            response,
                            connection,
                            cancel,
                            &prepared.lease.as_ref().unwrap().cancel,
                            budget,
                        )
                        .await
                        {
                            Ok(true) => {
                                *challenge = Some(ChallengeLease {
                                    lease: prepared.lease.take(),
                                    destination: destination.base(),
                                    session: input.session.clone(),
                                    operation: input.operation.clone(),
                                    expires: Instant::now()
                                        + budget.cleanup.min(Duration::from_secs(5)),
                                });
                                if input.policy == AuthPolicy::Gh {
                                    let Prepared {
                                        _slot: returned_slot,
                                        _operation: returned_dependency,
                                        ..
                                    } = prepared;
                                    slot = returned_slot;
                                    dependency = Some(returned_dependency);
                                    answer_challenge = true;
                                    continue;
                                }
                                return Err(failed);
                            }
                            Ok(false) => {}
                            Err(code) => failed.code = code,
                        }
                    } else {
                        drop(response);
                    }
                    let lease = prepared.lease.take().unwrap();
                    let disposed = lease.disposed.clone();
                    lease.finish(Disposition::Discarded)?;
                    // A retry must not overlap cleanup of its first attempt.
                    let cleanup_started = Instant::now();
                    let cleanup_until = cleanup_started + budget.cleanup;
                    while !disposed.load(Ordering::Acquire) {
                        if Instant::now() >= cleanup_until {
                            failed.code = ErrorCode::Timeout;
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(2)).await;
                    }
                    budget.cleanup = budget.cleanup.saturating_sub(cleanup_started.elapsed());
                    if may_carry && input.policy == AuthPolicy::Gh && failed.code == code {
                        let Prepared {
                            _slot: returned_slot,
                            _operation: returned_dependency,
                            ..
                        } = prepared;
                        slot = returned_slot;
                        dependency = Some(returned_dependency);
                        answer_challenge = true;
                        continue;
                    }
                    return Err(failed);
                }
                _ => {
                    return Err(with_facts(
                        ErrorCode::Protocol,
                        Effect::None,
                        &prepared.opened.facts,
                    ));
                }
            }
        }
    }
}
pub(super) fn duration_ms(duration: Duration) -> u64 {
    duration
        .as_millis()
        .saturating_add(u128::from(
            !duration.subsec_nanos().is_multiple_of(1_000_000),
        ))
        .min(u64::MAX as u128) as u64
}
pub(super) async fn clean_challenge(
    mut response: Response<Incoming>,
    connection: &Arc<tokio::sync::Mutex<https_connection::Connection>>,
    cancel: &CancellationToken,
    resource_cancel: &CancellationToken,
    budget: &mut Budget,
) -> Result<bool, ErrorCode> {
    let keep_alive = response.headers().get_all(CONNECTION).iter().all(|value| {
        value.to_str().is_ok_and(|text| {
            !text
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("close"))
        })
    });
    if !keep_alive {
        return Ok(false);
    }
    let started = Instant::now();
    let allowance = budget.cleanup.min(budget.network.unwrap_or(budget.cleanup));
    let until = started + allowance;
    let result = tokio::select! {
        _ = cancel.cancelled() => Err(ErrorCode::Cancelled),
        _ = resource_cancel.cancelled() => Err(ErrorCode::Cancelled),
        result = tokio::time::timeout_at(until, async {
            let mut size = 0usize;
            while let Some(frame) = response.body_mut().frame().await {
                let frame = frame.map_err(|_| ErrorCode::Protocol)?;
                if let Some(data) = frame.data_ref() {
                    size = size.saturating_add(data.len());
                    if size > 64 * 1024 {
                        return Ok(false);
                    }
                }
            }
            drop(response);
            let mut guard = connection.lock().await;
            Ok(guard.alive() && guard.sender.ready().await.is_ok() && guard.alive())
        }) => result.map_err(|_| ErrorCode::Timeout)?,
    };
    let elapsed = started.elapsed();
    budget.cleanup = budget.cleanup.saturating_sub(elapsed);
    if let Some(remaining) = budget.network.as_mut() {
        *remaining = remaining.saturating_sub(elapsed);
    }
    if cancel.is_cancelled() || resource_cancel.is_cancelled() {
        return Err(ErrorCode::Cancelled);
    }
    if budget.cleanup.is_zero() || budget.network.is_some_and(|remaining| remaining.is_zero()) {
        return Err(ErrorCode::Timeout);
    }
    result
}
pub(super) async fn acquire_helper_slot(
    slots: Arc<Semaphore>,
    until: Instant,
    cancel: &CancellationToken,
) -> Result<OwnedSemaphorePermit, https_auth::AuthError> {
    if Instant::now() >= until {
        return Err(https_auth::AuthError::AllocationTimeout);
    }
    let permit = tokio::select! {
        _ = cancel.cancelled() => Err(https_auth::AuthError::Cancelled),
        result = tokio::time::timeout_at(until, slots.acquire_owned()) => {
            result.map_err(|_| https_auth::AuthError::AllocationTimeout)?
                .map_err(|_| https_auth::AuthError::Cancelled)
        }
    }?;
    if Instant::now() >= until {
        return Err(https_auth::AuthError::AllocationTimeout);
    }
    Ok(permit)
}
async fn acquire_slot(
    slots: Arc<Semaphore>,
    allocation: Duration,
    cancel: &CancellationToken,
) -> Result<OwnedSemaphorePermit, Failure> {
    if allocation.is_zero() {
        return Err(setup_retry::allocation_timeout());
    }
    tokio::select! {
        _ = cancel.cancelled() => Err(failure(ErrorCode::Cancelled)),
        result = tokio::time::timeout(allocation, slots.acquire_owned()) => {
            result.map_err(|_| setup_retry::allocation_timeout())?
                .map_err(|_| failure(ErrorCode::Cancelled))
        }
    }
}

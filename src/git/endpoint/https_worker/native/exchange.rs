//! The native authentication exchange: Client entry points, deadline control and drop guard.
use super::owners::{Finishing, Starting};
use super::*;

struct Guard {
    starting: Option<Starting>,
    session: Option<Box<dyn Session>>,
    finishing: Option<Finishing>,
    cancellation: gwz_sspi::Cancellation,
    cleanup: Cleanup,
    operation: Option<super::super::super::https_operation::Dependency>,
    slot: Option<OwnedSemaphorePermit>,
}
impl Guard {
    fn retain(&mut self, pending: Option<Box<dyn Probe>>) {
        if let Some(probe) = pending
            && let Some(operation) = self.operation.take()
        {
            self.cleanup
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .records
                .push(Pending {
                    probe,
                    _operation: operation,
                    _slot: self.slot.take().expect("charged native cleanup"),
                });
        }
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        self.cancellation.cancel();
        if let Some(starting) = self.starting.take() {
            self.retain(Some(Box::new(starting)));
        }
        if let Some(session) = self.session.take() {
            self.retain(session.cancel());
        }
        if let Some(finishing) = self.finishing.take() {
            self.retain(Some(Box::new(finishing)));
        }
    }
}
/// The channel binding an SSPI attempt carries: the connection's own, and never an empty or substituted one. A
/// connection with none cannot be authenticated over `Negotiate` or `NTLM`; an anonymous or Basic exchange does
/// not need one.
pub(super) fn channel_binding(
    held: Option<&gwz_sspi::SecretBytes>,
) -> Option<gwz_sspi::SecretBytes> {
    held.map(|bytes| gwz_sspi::SecretBytes::new(bytes.as_bytes()))
        .filter(|bytes| !bytes.as_bytes().is_empty())
}
fn deadline(budget: &Budget) -> Result<Instant, ErrorCode> {
    let deadline = budget
        .logical_deadline
        .ok_or(ErrorCode::UnsupportedOperation)?;
    if Instant::now() >= deadline {
        return Err(ErrorCode::Timeout);
    }
    Ok(deadline)
}
fn control(cancel: &CancellationToken, lease: &HttpLease, until: Instant) -> Result<(), ErrorCode> {
    if cancel.is_cancelled() || lease.cancel.is_cancelled() {
        return Err(ErrorCode::Cancelled);
    }
    if Instant::now() >= until {
        return Err(ErrorCode::Timeout);
    }
    Ok(())
}
impl Client {
    pub(crate) fn set_native(&mut self, caller: NativeCaller) {
        self.native = Some(caller);
    }
    pub(in crate::git::endpoint::https_worker) async fn authenticate(
        &self,
        prepared: &mut Prepared,
        mut response: Response<Incoming>,
        key: &RouteKey,
        budget: &mut Budget,
        cancel: &CancellationToken,
    ) -> Result<Response<Incoming>, Failure> {
        let offers = Offers::parse(response.headers()).map_err(failure)?;
        // TD1: the logon session answers Negotiate and NTLM, alone or beside Basic or Digest, and the helper is
        // not asked. Only a challenge that has nothing for the logon session goes on to Basic.
        let Some(offer) = offers.select() else {
            return self
                .answer_basic(prepared, response, &offers, key, budget, cancel)
                .await;
        };
        let mut history = History::new(offer.scheme);
        prepared.opened.facts = history.facts.clone();
        let until =
            deadline(budget).map_err(|code| with_facts(code, Effect::None, &history.facts))?;
        offer
            .initial()
            .map_err(|code| with_facts(code, Effect::None, &history.facts))?;
        // An operation whose rounds are spent starts no further exchange, and no SSPI work.
        if budget.native_rounds >= NATIVE_ROUNDS {
            return Err(with_facts(
                ErrorCode::Protocol,
                Effect::None,
                &history.facts,
            ));
        }
        let url = prepared.destination.request(prepared.input.service);
        let overhead = url[url::Position::BeforePath..].len() + prepared.destination.authority().len()
            + https_policy::response_type(prepared.input.service).len()
            + b"GET  HTTP/1.1\r\nHost: \r\nAccept: \r\nAuthorization: \r\nTransfer-Encoding: chunked\r\n\r\n".len();
        let available = 65536usize
            .checked_sub(overhead)
            .ok_or_else(|| failure(ErrorCode::InvalidRequest))?;
        let limit = raw_limit(available, offer.scheme).map_err(failure)?;
        let caller = self
            .native
            .as_ref()
            .ok_or_else(|| with_facts(ErrorCode::Unavailable, Effect::None, &history.facts))?;
        let port = caller
            .port
            .as_ref()
            .map_err(|kind| with_facts(native_code(*kind), Effect::None, &history.facts))?;
        let connection = prepared
            .lease
            .as_ref()
            .unwrap()
            .connection
            .as_ref()
            .unwrap()
            .clone();
        // The binding of this physical connection's final origin handshake, or no attempt at all (design 8).
        let binding =
            channel_binding(connection.lock().await.binding.as_ref()).ok_or_else(|| {
                with_facts(
                    ErrorCode::UnsupportedOperation,
                    Effect::None,
                    &history.facts,
                )
            })?;
        let request = gwz_sspi::AuthRequest {
            package: if offer.scheme == NativeScheme::Negotiate {
                gwz_sspi::Package::Negotiate
            } else {
                gwz_sspi::Package::Ntlm
            },
            target: gwz_sspi::SecretText::new(&format!("HTTP/{}", prepared.destination.host()))
                .map_err(|_| failure(ErrorCode::InvalidRequest))?,
            // The identity is always the logon session's: a helper's credential answers Basic only.
            identity: gwz_sspi::Identity::CurrentLogon,
            channel_binding: binding,
            token_limit: limit,
            digest: None,
        };
        let mut guard = Guard {
            starting: None,
            session: None,
            finishing: None,
            cancellation: gwz_sspi::Cancellation::new(),
            cleanup: self.native_cleanup.clone(),
            // A further dependent of an operation the preparation already
            // holds, so only a sealed operation refuses it.
            operation: Some(
                self.operation(&prepared.input.operation)
                    .map_err(|refusal| failure(refusal.code()))?,
            ),
            slot: prepared._slot.take(),
        };
        let starting = Starting::new(port.start(request, until, guard.cancellation.clone()));
        guard.starting = Some(starting.clone());
        let result = tokio::select! {
            result = starting => result,
            _ = cancel.cancelled() => Err(ErrorCode::Cancelled),
            _ = prepared.lease.as_ref().unwrap().cancel.cancelled() => Err(ErrorCode::Cancelled),
            _ = tokio::time::sleep_until(until) => Err(ErrorCode::Timeout),
        };
        if let Err(code) = result {
            return Err(with_facts(code, Effect::None, &history.facts));
        }
        guard.session = Some(guard.starting.as_ref().unwrap().take_session());
        guard.starting = None;
        let scope = self.ids.unique().to_string();
        prepared.lease.as_ref().unwrap().scope(&scope)?;
        let mut challenge = None;
        while budget.native_rounds < NATIVE_ROUNDS {
            budget.native_rounds += 1;
            control(cancel, prepared.lease.as_ref().unwrap(), until)
                .map_err(|code| with_facts(code, Effect::None, &history.facts))?;
            let step = guard.session.as_mut().unwrap().step(challenge.take());
            let result = tokio::select! {
                result = step => result,
                _ = cancel.cancelled() => return Err(with_facts(ErrorCode::Cancelled, Effect::None, &history.facts)),
                _ = prepared.lease.as_ref().unwrap().cancel.cancelled() => return Err(with_facts(ErrorCode::Cancelled, Effect::None, &history.facts)),
                _ = tokio::time::sleep_until(until) => return Err(with_facts(ErrorCode::Timeout, Effect::None, &history.facts)),
            };
            let step = match result {
                Ok(step) => step,
                Err(error) => {
                    guard.retain(error.pending);
                    return Err(with_facts(error.code, Effect::None, &history.facts));
                }
            };
            history
                .observe(&step)
                .map_err(|code| with_facts(code, Effect::None, &history.facts))?;
            // Complete with an empty final token may still consume the prior response.
            if !step.payload.as_bytes().is_empty() {
                let header =
                    Header::token(offer.scheme, step.payload.as_bytes()).map_err(failure)?;
                prepared.authorization = Some(https_auth::SecretHeader::from_bytes(header.bytes()));
                prepared.opened.facts = history.facts.clone();
                response = self.auth_send(prepared, response, budget, cancel).await?;
                history.facts = prepared.opened.facts.clone();
            }
            control(cancel, prepared.lease.as_ref().unwrap(), until)
                .map_err(|code| with_facts(code, Effect::None, &history.facts))?;
            let status = response.status().as_u16();
            history.reject(status);
            prepared.opened.facts = history.facts.clone();
            if status == 200 {
                validate_content(&response, prepared.input.service)
                    .map_err(|code| with_facts(code, Effect::None, &history.facts))?;
                if !history.complete {
                    let final_offers = Offers::parse(response.headers())
                        .map_err(|code| with_facts(code, Effect::None, &history.facts))?;
                    let final_offer = final_offers
                        .native
                        .iter()
                        .find(|next| next.scheme == offer.scheme)
                        .ok_or_else(|| {
                            with_facts(ErrorCode::Protocol, Effect::None, &history.facts)
                        })?;
                    challenge = Some(
                        decode_challenge(final_offer.token.bytes(), limit.raw_bytes() as usize)
                            .map_err(|code| with_facts(code, Effect::None, &history.facts))?,
                    );
                    continue;
                }
                history
                    .accept(status)
                    .map_err(|code| with_facts(code, Effect::None, &history.facts))?;
                let finishing = Finishing::new(guard.session.take().unwrap().finish());
                guard.finishing = Some(finishing.clone());
                let result = tokio::select! {
                    result = finishing => result,
                    _ = cancel.cancelled() => { guard.cancellation.cancel(); Err(ErrorCode::Cancelled) },
                    _ = prepared.lease.as_ref().unwrap().cancel.cancelled() => { guard.cancellation.cancel(); Err(ErrorCode::Cancelled) },
                    _ = tokio::time::sleep_until(until) => { guard.cancellation.cancel(); Err(ErrorCode::Timeout) },
                };
                if let Err(code) = result {
                    return Err(with_facts(code, Effect::None, &history.facts));
                }
                guard.finishing = None;
                control(cancel, prepared.lease.as_ref().unwrap(), until)
                    .map_err(|code| with_facts(code, Effect::None, &history.facts))?;
                prepared.opened.facts = history.facts.clone();
                let authenticated = Arc::new(Authenticated {
                    generation: prepared.lease.as_ref().unwrap().id.clone(),
                    scope,
                    facts: history.facts,
                    pool: self.pool.clone(),
                    revoked: AtomicBool::new(false),
                });
                self.routes
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .native_install(key, authenticated.clone())
                    .map_err(failure)?;
                prepared.native_route = Some(authenticated);
                prepared._slot = guard.slot.take();
                return Ok(response);
            }
            if status != 401 || history.complete {
                return Err(with_facts(
                    ErrorCode::Authentication,
                    Effect::None,
                    &history.facts,
                ));
            }
            let next = Offers::parse(response.headers())
                .map_err(|code| with_facts(code, Effect::None, &history.facts))?;
            let offer = next
                .native
                .iter()
                .find(|next| next.scheme == offer.scheme)
                .ok_or_else(|| {
                    with_facts(ErrorCode::Authentication, Effect::None, &history.facts)
                })?;
            // A 401 that carries no continuation token for the scheme is the server's rejection of the identity:
            // terminal, with no replay and no other identity (design 7).
            if offer.token.bytes().is_empty() {
                return Err(with_facts(
                    ErrorCode::Authentication,
                    Effect::None,
                    &history.facts,
                ));
            }
            challenge = Some(
                decode_challenge(offer.token.bytes(), limit.raw_bytes() as usize)
                    .map_err(|code| with_facts(code, Effect::None, &history.facts))?,
            );
        }
        Err(with_facts(
            ErrorCode::Protocol,
            Effect::None,
            &history.facts,
        ))
    }
    /// A challenge with nothing for the logon session (TD1, TD2). `Basic` goes to the configured helper, once
    /// for the destination the credential is sent to, and a usable answer goes over `Basic`; whatever the helper
    /// cannot give ends the request. `Digest` is never answered, so a challenge that offers nothing else is
    /// refused here, naming the schemes it named, and so is `Basic` when helpers are off.
    async fn answer_basic(
        &self,
        prepared: &mut Prepared,
        response: Response<Incoming>,
        offers: &Offers,
        key: &RouteKey,
        budget: &mut Budget,
        cancel: &CancellationToken,
    ) -> Result<Response<Incoming>, Failure> {
        if !offers.basic || prepared.input.policy != AuthPolicy::WindowsConfigured {
            // The challenge is what is being refused, so its status is what the refusal reports.
            let facts = Facts {
                http_status: Some(401),
                ..prepared.opened.facts.clone()
            };
            let mut refused = with_facts(ErrorCode::Authentication, Effect::None, &facts);
            if !offers.basic {
                refused.detail = Some(Box::new(FailureDetail {
                    schemes: Some(challenges::schemes(response.headers()).1),
                    ..Default::default()
                }));
            }
            return Err(refused);
        }
        let fixed = budget.logical_deadline;
        let lookup = self.credential(key, &prepared.destination, budget, cancel);
        let credential = match fixed {
            Some(until) => tokio::time::timeout_at(until, lookup)
                .await
                .map_err(|_| failure(ErrorCode::Timeout))??,
            None => lookup.await?,
        };
        prepared.opened.facts = Facts {
            method: AuthMethod::Gh,
            ..Default::default()
        };
        prepared.lease.as_ref().unwrap().scope(&credential.scope)?;
        prepared.authorization = Some(credential.header());
        prepared.credential = Some(credential);
        let response = self.auth_send(prepared, response, budget, cancel).await?;
        prepared.opened.facts.credential_offered = true;
        if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
            // Basic follows TR1.6 OQ4(a): validated discovery at the new
            // location, with no forwarded header and a new helper lookup.
            return Ok(response);
        }
        if response.status() != 200 {
            prepared
                .credential
                .as_ref()
                .unwrap()
                .rejected
                .store(true, Ordering::Release);
            prepared.opened.facts.authenticated = Some(false);
            return Err(with_facts(
                ErrorCode::Authentication,
                Effect::None,
                &prepared.opened.facts,
            ));
        }
        validate_content(&response, prepared.input.service)
            .map_err(|code| with_facts(code, Effect::None, &prepared.opened.facts))?;
        self.routes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .basic_install(key)
            .map_err(failure)?;
        Ok(response)
    }
    async fn auth_send(
        &self,
        prepared: &mut Prepared,
        response: Response<Incoming>,
        budget: &mut Budget,
        cancel: &CancellationToken,
    ) -> Result<Response<Incoming>, Failure> {
        let connection = prepared
            .lease
            .as_ref()
            .unwrap()
            .connection
            .as_ref()
            .unwrap()
            .clone();
        let fixed = budget.logical_deadline;
        let draining = prepare::clean_challenge(
            response,
            &connection,
            cancel,
            &prepared.lease.as_ref().unwrap().cancel,
            budget,
        );
        let clean = match fixed {
            Some(until) => tokio::time::timeout_at(until, draining)
                .await
                .map_err(|_| {
                    with_facts(ErrorCode::Timeout, Effect::None, &prepared.opened.facts)
                })?,
            None => draining.await,
        }
        .map_err(|code| with_facts(code, Effect::None, &prepared.opened.facts))?;
        if !clean {
            return Err(with_facts(
                ErrorCode::Authentication,
                Effect::None,
                &prepared.opened.facts,
            ));
        }
        let (tx, body) = body_channel();
        drop(tx);
        let request = prepared.request(body)?;
        let mut connection = connection.lock().await;
        let started = Instant::now();
        let result = tokio::select! {
            _ = cancel.cancelled() => Err(ErrorCode::Cancelled),
            _ = prepared.lease.as_ref().unwrap().cancel.cancelled() => Err(ErrorCode::Cancelled),
            result = async {
                if let Some(until) = budget.logical_deadline { control(cancel, prepared.lease.as_ref().unwrap(), until)?; }
                prepared.opened.facts.credential_offered |= request.headers().contains_key(AUTHORIZATION);
                let send = connection.sender.send_request(request);
                let send = async {
                    match budget.network {
                        Some(remaining) => tokio::time::timeout(remaining, send).await.map_err(|_| ErrorCode::Timeout)?,
                        None => send.await,
                    }.map_err(|error| classify_hyper_error(&error))
                };
                match budget.logical_deadline {
                    Some(until) => tokio::time::timeout_at(until, send).await.map_err(|_| ErrorCode::Timeout)?,
                    None => send.await,
                }
            } => result,
        };
        if let Some(remaining) = budget.network.as_mut() {
            *remaining = remaining.saturating_sub(started.elapsed());
        }
        prepared.io_ms = budget.network.map_or(0, prepare::duration_ms);
        let response =
            result.map_err(|code| with_facts(code, Effect::None, &prepared.opened.facts))?;
        prepared.opened.facts.http_status = Some(response.status().as_u16() as i64);
        if prepared.protocol_error.load(Ordering::Acquire) {
            return Err(with_facts(
                ErrorCode::Protocol,
                Effect::None,
                &prepared.opened.facts,
            ));
        }
        Ok(response)
    }
}

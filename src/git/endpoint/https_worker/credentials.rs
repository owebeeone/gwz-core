//! Route-owned answers and opaque physical connection scopes.
use super::*;
use std::collections::BTreeMap;

pub(crate) struct Credential {
    secret: https_auth::Secret,
    pub(super) scope: String,
    pub(super) rejected: AtomicBool,
    pool: HttpsPool,
}
impl Drop for Credential {
    fn drop(&mut self) {
        self.pool.retire_https_scope(&self.scope);
    }
}
pub(crate) type Answers = tokio::sync::Mutex<BTreeMap<String, Result<Arc<Credential>, Failure>>>;

impl Client {
    pub(super) async fn credential(
        &self,
        key: &RouteKey,
        destination: &Destination,
        budget: &mut Budget,
        cancel: &CancellationToken,
    ) -> Result<Arc<Credential>, Failure> {
        let cell = self
            .routes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .answers(key)
            .map_err(failure)?;
        let facts = Facts {
            method: AuthMethod::Gh,
            ..Default::default()
        };
        let allocation_ms = budget.allocation.as_millis() as i64;
        let interaction_ms = budget.helper.min(Duration::from_secs(120)).as_millis() as i64;
        let until = Instant::now() + Duration::from_millis(allocation_ms as u64);
        let started = Instant::now();
        let lock = tokio::select! {
            _ = cancel.cancelled() => return Err(failure(ErrorCode::Cancelled)),
            lock = tokio::time::timeout_at(until, cell.lock()) => lock,
        };
        budget.allocation = budget.allocation.saturating_sub(started.elapsed());
        let mut answers = lock.map_err(|_| {
            helper_timeout(
                https_auth::AuthError::AllocationTimeout,
                &facts,
                allocation_ms,
                interaction_ms,
            )
        })?;
        if let Some(answer) = answers.get(&destination.base()) {
            let credential = answer.clone()?;
            if credential.rejected.load(Ordering::Acquire) {
                let facts = Facts {
                    method: AuthMethod::Gh,
                    authenticated: Some(false),
                    credential_offered: true,
                    http_status: Some(401),
                    ..Default::default()
                };
                return Err(with_facts(ErrorCode::Authentication, Effect::None, &facts));
            }
            return Ok(credential);
        }
        // One discovery reaches at most six destinations (the original and
        // five redirects), so a route holds more only when a server sends
        // later discoveries of one repository elsewhere. That is a refusal of
        // the server's redirects, like the hop limit, never a local capacity.
        if answers.len() >= 6 {
            return Err(with_facts(
                ErrorCode::UnsupportedOperation,
                Effect::None,
                &facts,
            ));
        }
        if let Some(failed) = self
            .routes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .missing_git(key)
        {
            return Err(failed);
        }
        let result: Result<Arc<Credential>, Failure> = async {
            let config = self
                .auth
                .as_ref()
                .ok_or_else(|| helper_failure(https_auth::AuthError::MissingExecutable, &facts))?;
            let started = Instant::now();
            let admitted = prepare::acquire_helper_slot(self.helpers.clone(), until, cancel).await;
            budget.allocation = budget.allocation.saturating_sub(started.elapsed());
            let helper = admitted
                .map_err(|error| helper_timeout(error, &facts, allocation_ms, interaction_ms))?;
            let secret = https_auth::lookup_until(
                &self.auth_owner,
                config,
                destination,
                &mut budget.allocation,
                Duration::from_millis(interaction_ms as u64),
                cancel,
                https_auth::LookupAdmission {
                    until,
                    endpoint_slot: Some(helper),
                },
            )
            .await
            .map_err(|error| helper_timeout(error, &facts, allocation_ms, interaction_ms))?;
            Ok(Arc::new(Credential {
                secret,
                scope: self.ids.unique().to_string(),
                rejected: AtomicBool::new(false),
                pool: self.pool.clone(),
            }))
        }
        .await;
        if result
            .as_ref()
            .err()
            .is_some_and(|f| f.code == ErrorCode::Unavailable)
        {
            self.routes
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .latch_missing_git(key, result.as_ref().err().unwrap().clone());
        }
        answers.insert(destination.base(), result.clone());
        result
    }
}
impl Client {
    /// TR1.6 OQ4 (d): whether `credential`, which a request redirected after the challenge carried, also answers
    /// `destination` on the same origin. When it does, the redirected request resends it, and the route's later
    /// requests there (the POST to the repository discovery found) are answered with it, with no lookup. It does not
    /// when the server has refused it already (draft r5 3(d): `Rejected` is terminal, so it is sent nowhere again), nor
    /// when `destination` holds another answer (another credential, or a failed lookup), which it keeps: the redirected
    /// request then goes with nothing, and only its own 401 decides. One discovery reaches at most six destinations,
    /// as for a lookup.
    pub(super) async fn resend(
        &self,
        key: &RouteKey,
        destination: &Destination,
        credential: &Arc<Credential>,
        budget: &mut Budget,
        cancel: &CancellationToken,
    ) -> Result<bool, Failure> {
        if credential.rejected.load(Ordering::Acquire) {
            return Ok(false);
        }
        let cell = self
            .routes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .answers(key)
            .map_err(failure)?;
        let facts = Facts {
            method: AuthMethod::Gh,
            ..Default::default()
        };
        let started = Instant::now();
        let lock = tokio::select! {
            _ = cancel.cancelled() => return Err(failure(ErrorCode::Cancelled)),
            lock = tokio::time::timeout_at(started + budget.allocation, cell.lock()) => lock,
        };
        budget.allocation = budget.allocation.saturating_sub(started.elapsed());
        let mut answers = lock.map_err(|_| with_facts(ErrorCode::Timeout, Effect::None, &facts))?;
        if let Some(answer) = answers.get(&destination.base()) {
            return Ok(answer
                .as_ref()
                .is_ok_and(|answer| Arc::ptr_eq(answer, credential)));
        }
        if answers.len() >= 6 {
            return Err(with_facts(
                ErrorCode::UnsupportedOperation,
                Effect::None,
                &facts,
            ));
        }
        answers.insert(destination.base(), Ok(credential.clone()));
        Ok(true)
    }
}
impl Credential {
    pub(super) fn header(&self) -> https_auth::SecretHeader {
        self.secret.header()
    }
}

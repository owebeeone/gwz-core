//! Endpoint-owned HTTP exchange; the host drives the existing message boundary.
use super::{
    https_auth,
    https_connection::{self, RequestBody, failure},
    https_destination::Destination,
    https_policy::{self, ResponseAction, RouteKey, Routes},
    https_pool::{HttpLease, HttpsPool, RunningPool},
    shared_reservation::Authority,
};
use bytes::Bytes;
use gwz_transport::{
    pool::{self, Key, Owner},
    protocol::*,
    stream::{IoState, MessageEndpoint, Stream},
};
use http_body_util::BodyExt;
use hyper::{
    Request, Response,
    body::Incoming,
    header::{ACCEPT, AUTHORIZATION, CONNECTION, CONTENT_TYPE, HOST, LOCATION},
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

mod budget;
mod prepare;
mod serve;

#[derive(Clone)]
pub(crate) struct Input {
    pub(crate) destination: String,
    pub(crate) service: GitService,
    pub(crate) policy: AuthPolicy,
    pub(crate) session: String,
    pub(crate) operation: String,
}
#[derive(Clone)]
pub(crate) struct Client {
    pool: HttpsPool,
    auth: Option<https_auth::Config>,
    slots: Arc<Semaphore>,
    helpers: Arc<Semaphore>,
    routes: Arc<Mutex<Routes>>,
    config: pool::Config,
    io_timeout_ms: u64,
    auth_owner: https_auth::AuthOwner,
    operations: super::https_operation::Operations,
}
pub(crate) struct Endpoint {
    pub(crate) client: Client,
    pool: RunningPool,
}
impl Endpoint {
    pub(crate) fn with_authority(
        tls: https_connection::Config,
        auth: Option<https_auth::Config>,
        config: pool::Config,
        io_timeout_ms: u64,
        authority: Authority,
        helper_slots: https_auth::HelperSlots,
    ) -> Result<Self, Failure> {
        if io_timeout_ms > i32::MAX as u64 {
            return Err(failure(ErrorCode::InvalidRequest));
        }
        let pool = RunningPool::with_authority(config.clone(), tls, authority)?;
        let routes = Arc::new(Mutex::new(Routes::new(64)));
        let operations = super::https_operation::Operations::new(routes.clone());
        let client = Client {
            pool: pool.client.clone(),
            auth,
            slots: Arc::new(Semaphore::new(64)),
            helpers: Arc::new(Semaphore::new(8)),
            routes,
            operations,
            auth_owner: https_auth::AuthOwner::new(helper_slots),
            config,
            io_timeout_ms,
        };
        Ok(Self { client, pool })
    }
    pub(crate) async fn shutdown(&mut self, limit: Duration) -> usize {
        let until = Instant::now() + limit;
        // Close admission before cancelling work. Every preparation holds a
        // slot through helper/network work and through the resulting stream.
        self.client.slots.close();
        self.client.helpers.close();
        self.client.auth_owner.cancel();
        self.client.pool.pool.shutdown();
        while self.client.slots.available_permits() != 64 && Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        self.client.auth_owner.reap_pending(until).await;
        let physical = self
            .pool
            .shutdown(until.saturating_duration_since(Instant::now()))
            .await;
        // Transfer to retained helpers happens before a preparation releases
        // its slot. This read order can overcount a racing completion, but
        // cannot report a false zero during that transfer.
        let active = 64 - self.client.slots.available_permits();
        let helpers = self.client.auth_owner.pending_cleanup_count();
        active + helpers + physical
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        self.client.slots.close();
        self.client.helpers.close();
        self.client.auth_owner.cancel();
        self.client.pool.pool.shutdown();
    }
}
pub(crate) struct Prepared {
    pub(crate) opened: Opened,
    lease: Option<HttpLease>,
    response: Option<Response<Incoming>>,
    input: Input,
    destination: Destination,
    authorization: Option<String>,
    _slot: OwnedSemaphorePermit,
    _operation: super::https_operation::Dependency,
    protocol_error: Arc<AtomicBool>,
    io_ms: u64,
    cleanup_ms: u64,
}
pub(crate) struct ChallengeLease {
    lease: Option<HttpLease>,
    destination: String,
    session: String,
    operation: String,
    expires: Instant,
}
impl ChallengeLease {
    pub(crate) fn expired(&self) -> bool {
        Instant::now() >= self.expires
    }
    fn take_for(&mut self, destination: &Destination, input: &Input) -> Option<HttpLease> {
        if self.destination == destination.base()
            && self.session == input.session
            && self.operation == input.operation
            && self
                .lease
                .as_ref()
                .is_some_and(|lease| !lease.cancel.is_cancelled())
        {
            self.lease.take()
        } else {
            None
        }
    }
}
impl Drop for ChallengeLease {
    fn drop(&mut self) {
        if let Some(lease) = self.lease.take() {
            let _ = lease.finish(Disposition::Discarded);
        }
    }
}
pub(crate) struct Budget {
    allocation: Duration,
    helper: Duration,
    connect: Option<Duration>,
    network: Option<Duration>,
    cleanup: Duration,
}
impl Client {
    pub(crate) fn pool(&self) -> &pool::Pool {
        &self.pool.pool
    }
    pub(crate) fn pending_cleanup(&self) -> usize {
        self.auth_owner.pending_cleanup_count() + self.pool.pool.counts().closing
    }

    pub(crate) async fn reap_cleanup(&self, limit: Duration) -> usize {
        self.auth_owner.reap_pending(Instant::now() + limit).await + self.pool.pool.counts().closing
    }

    pub(crate) fn finish_operation(&self, operation: &str) {
        self.operations.finish(operation);
    }
    pub(crate) fn operation(
        &self,
        operation: &str,
    ) -> Result<super::https_operation::Dependency, ErrorCode> {
        self.operations.acquire(operation)
    }
}

fn body_channel() -> (mpsc::Sender<std::io::Result<Bytes>>, RequestBody) {
    let (tx, rx) = mpsc::channel(1);
    (tx, RequestBody { rx })
}
fn with_facts(code: ErrorCode, effect: Effect, facts: &Facts) -> Failure {
    Failure {
        setup_cause: None,
        code,
        effect,
        facts: Some(facts.clone()),
    }
}
fn classify_hyper_error(error: &hyper::Error) -> ErrorCode {
    if error.is_parse() {
        ErrorCode::Protocol
    } else {
        ErrorCode::Io
    }
}
fn validate_content(response: &Response<Incoming>, service: GitService) -> Result<(), ErrorCode> {
    let value = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .ok_or(ErrorCode::Protocol)?;
    if !value
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .eq_ignore_ascii_case(&https_policy::response_type(service))
    {
        return Err(ErrorCode::Protocol);
    }
    Ok(())
}
cfg_if::cfg_if! { if #[cfg(test)] { #[path="https_worker_tests.rs"] mod tests; } }
cfg_if::cfg_if! { if #[cfg(test)] { #[path="https_budget_tests.rs"] mod budget_tests; } }

cfg_if::cfg_if! {
    if #[cfg(test)] {
        /// A standalone endpoint and the client's direct entry points, which
        /// the HTTPS tests drive; production enters through
        /// `prepare_budget_for_transition` with `budget_for_open`.
        impl Endpoint {
            pub(crate) fn new(
                tls: https_connection::Config,
                auth: Option<https_auth::Config>,
                config: pool::Config,
            ) -> Result<Self, Failure> {
                Self::new_with_io_timeout(tls, auth, config, 9_000)
            }
            pub(crate) fn new_with_io_timeout(
                tls: https_connection::Config,
                auth: Option<https_auth::Config>,
                config: pool::Config,
                io_timeout_ms: u64,
            ) -> Result<Self, Failure> {
                // A standalone endpoint is its own host: its own authority and slots.
                let authority = Authority::new(config.total, config.per_host);
                let helper_slots = https_auth::HelperSlots::new();
                Self::with_authority(tls, auth, config, io_timeout_ms, authority, helper_slots)
            }
        }
        impl Client {
            pub(crate) async fn prepare(
                &self,
                input: Input,
                cancel: &CancellationToken,
            ) -> Result<Prepared, Failure> {
                self.prepare_until(
                    input,
                    cancel,
                    Instant::now() + Duration::from_millis(self.config.allocation_timeout_ms),
                    Duration::from_millis(self.config.interaction_timeout_ms),
                )
                .await
            }
            /// Sole authentication replay: anonymous discovery 401/404, once. The
            /// caller retains the first receipt and publishes only the final result.
            pub(crate) async fn prepare_auto(
                &self,
                mut input: Input,
                cancel: &CancellationToken,
                first: &mut Option<Failure>,
            ) -> Result<Prepared, Failure> {
                let mut budget = self.budget();
                let mut challenge = None;
                if https_policy::receive_pack(input.service) {
                    input.policy = AuthPolicy::Gh;
                }
                let result = self
                    .prepare_budget_for_transition(input.clone(), cancel, &mut budget, &mut challenge)
                    .await;
                match result {
                    Err(f)
                        if input.policy == AuthPolicy::Anonymous
                            && https_policy::advertisement(input.service)
                            && matches!(
                                f.code,
                                ErrorCode::Authentication | ErrorCode::RepositoryRefused
                            )
                            && f.facts
                                .as_ref()
                                .is_some_and(|facts| matches!(facts.http_status, Some(401 | 404))) =>
                    {
                        *first = Some(f);
                        input.policy = AuthPolicy::Gh;
                        self.prepare_budget_for_transition(input, cancel, &mut budget, &mut challenge)
                            .await
                    }
                    other => other,
                }
            }
            pub(crate) async fn prepare_until(
                &self,
                input: Input,
                cancel: &CancellationToken,
                until: Instant,
                helper_remaining: Duration,
            ) -> Result<Prepared, Failure> {
                let mut budget = self.budget();
                budget.allocation = until.saturating_duration_since(Instant::now());
                budget.helper = helper_remaining;
                self.prepare_budget(input, cancel, &mut budget).await
            }
            /// Apply an Open request's positive deadline values as upper bounds. Zero
            /// retains the endpoint's captured setting, including zero-disabled I/O.
            pub(crate) async fn prepare_open(
                &self,
                input: Input,
                cancel: &CancellationToken,
                deadlines: &Deadlines,
            ) -> Result<Prepared, Failure> {
                let mut budget = self.budget_for_open(deadlines);
                self.prepare_budget(input, cancel, &mut budget).await
            }
            pub(crate) fn configured_deadlines(&self) -> Deadlines {
                Deadlines {
                    allocation_ms: self.config.allocation_timeout_ms as i64,
                    connect_ms: self.config.connect_timeout_ms as i64,
                    io_ms: self.io_timeout_ms as i64,
                    interaction_ms: self.config.interaction_timeout_ms as i64,
                    cleanup_ms: self.config.cleanup_timeout_ms as i64,
                }
            }
            pub(crate) async fn prepare_budget(
                &self,
                input: Input,
                cancel: &CancellationToken,
                budget: &mut Budget,
            ) -> Result<Prepared, Failure> {
                self.prepare_budget_inner(input, cancel, budget, &mut None, false)
                    .await
            }
        }
    }
}

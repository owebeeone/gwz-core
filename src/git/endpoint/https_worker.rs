//! Endpoint-owned HTTP exchange; the host drives the existing message boundary.
use super::{
    https_auth,
    https_connection::{self, RequestBody, failure},
    https_destination::Destination,
    https_policy::{self, ResponseAction, RouteKey, Routes},
    https_pool::{HttpLease, HttpsPool, RunningPool},
    https_wake::CloseWake,
    setup_retry::{self, Phase},
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
mod challenges;
pub(crate) mod credentials;
pub(crate) mod native;
mod prepare;
mod serve;
mod throttle;

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
    #[cfg(unix)]
    auth_owner: https_auth::AuthOwner,
    native: Option<native::NativeCaller>,
    native_cleanup: native::Cleanup,
    operations: super::https_operation::Operations,
    ids: Arc<gwz_ids::IdSource>,
}
impl Client {
    /// Has the endpoint's TLS configuration built on `runtime` now, for the
    /// connection an open that has just arrived will need.
    pub(crate) fn prebuild_tls(&self, runtime: &tokio::runtime::Handle) {
        self.pool.prebuild_tls(runtime);
    }
    cfg_if::cfg_if! { if #[cfg(all(test, unix))] {
        pub(crate) fn tls_builds(&self) -> usize {
            self.pool.tls_builds()
        }
    } }
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
        cfg_if::cfg_if! { if #[cfg(all(windows, gwz_transport_candidate, gwz_windows_https_qualification))] {
            if auth.is_some() { return Err(failure(ErrorCode::UnsupportedOperation)); }
            drop(helper_slots);
        } }
        let pool = RunningPool::with_authority(config.clone(), tls, authority)?;
        let routes = Arc::new(Mutex::new(Routes::new()));
        let operations = super::https_operation::Operations::new(routes.clone());
        let client = Client {
            pool: pool.client.clone(),
            auth,
            slots: Arc::new(Semaphore::new(64)),
            helpers: Arc::new(Semaphore::new(8)),
            routes,
            operations,
            ids: Arc::new(crate::operation_context::new_id_source()),
            #[cfg(unix)]
            auth_owner: https_auth::AuthOwner::new(helper_slots),
            native: None,
            native_cleanup: Arc::new(Mutex::new(native::CleanupState::default())),
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
        cfg_if::cfg_if! { if #[cfg(unix)] { self.client.auth_owner.cancel(); } }
        self.client.pool.shutdown();
        while self.client.slots.available_permits() != 64 && Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        cfg_if::cfg_if! { if #[cfg(unix)] { self.client.auth_owner.reap_pending(until).await; } }
        let physical = self
            .pool
            .shutdown(until.saturating_duration_since(Instant::now()))
            .await;
        // Transfer to retained helpers happens before a preparation releases
        // its slot. This read order can overcount a racing completion, but
        // cannot report a false zero during that transfer.
        let active = 64 - self.client.slots.available_permits();
        let helpers = self.client.helper_pending();
        active + helpers + physical + native::reap(&self.client.native_cleanup)
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        self.client.slots.close();
        self.client.helpers.close();
        cfg_if::cfg_if! { if #[cfg(unix)] { self.client.auth_owner.cancel(); } }
        self.client.pool.shutdown();
    }
}
pub(crate) struct Prepared {
    pub(crate) opened: Opened,
    lease: Option<HttpLease>,
    response: Option<Response<Incoming>>,
    input: Input,
    destination: Destination,
    authorization: Option<https_auth::SecretHeader>,
    credential: Option<Arc<credentials::Credential>>,
    native_route: Option<Arc<native::Authenticated>>,
    _slot: Option<OwnedSemaphorePermit>,
    _operation: super::https_operation::Dependency,
    protocol_error: Arc<AtomicBool>,
    io_ms: u64,
    cleanup_ms: u64,
    /// The connection serves this exchange and is then discarded, never
    /// returned to the pool for reuse.
    discard: bool,
}
impl Prepared {
    pub(crate) fn native_publication_route(&self) -> Option<Arc<native::Authenticated>> {
        self.native_route.clone()
    }
    pub(crate) fn revoke_native_route(&self) {
        if let Some(route) = &self.native_route {
            route.revoke();
        }
    }
    cfg_if::cfg_if! { if #[cfg(all(test, unix))] {
        pub(crate) fn publication_resources_for_test(&self) -> (Arc<native::Authenticated>, Arc<tokio::sync::Mutex<https_connection::Connection>>) {
            (self.native_route.clone().unwrap(), self.lease.as_ref().unwrap().connection.clone().unwrap())
        }
    } }
    /// A setup its key's retry machine does not admit for reuse: one from a
    /// generation the key has left (the retry plan's §4).
    pub(crate) fn discard_after_use(&mut self) {
        self.discard = true;
    }
}
/// What an open's first connect did, which its key's retry machine learns
/// (the retry plan's §4 and §5). Only a fresh connect for the open's first
/// request, before that request's first byte, is a setup: a redirect's
/// connect, a carried or idle connection, and the work before a connect are
/// none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FirstConnect {
    /// No fresh connect: none was needed, or none was reached.
    #[default]
    None,
    /// The fresh connect succeeded, whatever the request after it did.
    Connected,
    /// The fresh connect failed, and the preparation's failure is its.
    Failed,
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
    redirect_hops: usize,
    logical_deadline: Option<Instant>,
    logical_started: bool,
}
impl Budget {
    pub(crate) fn publication_deadline(&self) -> Option<Instant> {
        self.logical_deadline
    }
}
impl Client {
    pub(crate) fn pool(&self) -> &pool::Pool {
        &self.pool.pool
    }
    fn helper_pending(&self) -> usize {
        cfg_if::cfg_if! { if #[cfg(unix)] { self.auth_owner.pending_cleanup_count() } else { 0 } }
    }
    pub(crate) fn pending_cleanup(&self) -> usize {
        self.helper_pending() + native::reap(&self.native_cleanup) + self.pool.pool.counts().closing
    }

    pub(crate) async fn reap_cleanup(&self, limit: Duration) -> usize {
        cfg_if::cfg_if! { if #[cfg(unix)] {
            self.auth_owner.reap_pending(Instant::now() + limit).await;
        } }
        self.helper_pending() + native::reap(&self.native_cleanup) + self.pool.pool.counts().closing
    }

    cfg_if::cfg_if! { if #[cfg(test)] {
        /// The routes the endpoint holds now, across its operations.
        pub(crate) fn route_count_for_test(&self) -> usize {
            self.routes.lock().unwrap_or_else(|e| e.into_inner()).len()
        }
    } }
    pub(crate) fn finish_operation(&self, operation: &str) {
        self.operations.finish(operation);
    }
    pub(crate) fn operation(
        &self,
        operation: &str,
    ) -> Result<super::https_operation::Dependency, super::https_operation::Refusal> {
        self.operations.acquire(operation)
    }
    /// A dependent of `operation`, waiting while the table is full of
    /// operations that still have dependents, as a stream waits for its place.
    /// The wait ends with the caller's allocation, which is its own failure
    /// (`setup_retry::allocation_timeout`), or with its cancellation.
    async fn operation_within(
        &self,
        operation: &str,
        allocation: Duration,
        cancel: &CancellationToken,
    ) -> Result<super::https_operation::Dependency, Failure> {
        use super::https_operation::Refusal;
        let until = Instant::now() + allocation;
        loop {
            let freed = self.operations.freed().notified();
            tokio::pin!(freed);
            freed.as_mut().enable();
            match self.operation(operation) {
                Ok(dependency) => return Ok(dependency),
                Err(refusal @ Refusal::Sealed) => return Err(failure(refusal.code())),
                Err(Refusal::WouldBlock) => {}
            }
            tokio::select! {
                _ = cancel.cancelled() => return Err(failure(ErrorCode::Cancelled)),
                _ = tokio::time::timeout_at(until, &mut freed) => {}
            }
            if Instant::now() >= until {
                return Err(setup_retry::allocation_timeout());
            }
        }
    }
}

fn body_channel() -> (mpsc::Sender<std::io::Result<Bytes>>, RequestBody) {
    let (tx, rx) = mpsc::channel(1);
    (tx, RequestBody { rx })
}
fn with_facts(code: ErrorCode, effect: Effect, facts: &Facts) -> Failure {
    Failure {
        detail: None,
        setup_cause: None,
        code,
        effect,
        facts: Some(facts.clone()),
    }
}
/// Preserve the configured-helper phase without treating local pipes as network loss.
fn helper_failure(error: https_auth::AuthError, facts: &Facts) -> Failure {
    let mut failure = with_facts(error.code(), Effect::None, facts);
    failure.setup_cause = match error {
        https_auth::AuthError::Timeout => Some(SetupFailureCause::Interaction),
        https_auth::AuthError::AllocationTimeout => Some(SetupFailureCause::Allocation),
        https_auth::AuthError::MissingExecutable => Some(SetupFailureCause::NotFound),
        _ => None,
    };
    use gwz_transport::protocol::{FailureDetail, HelperFailureCause};
    let cause = match error {
        https_auth::AuthError::Pipe(_) => Some(HelperFailureCause::PipeFailure),
        https_auth::AuthError::ControlCharacter => Some(HelperFailureCause::ControlCharacter),
        https_auth::AuthError::UsernameColon => Some(HelperFailureCause::UsernameColon),
        https_auth::AuthError::NotUtf8 => Some(HelperFailureCause::NotUtf8),
        https_auth::AuthError::MissingNewline => Some(HelperFailureCause::MissingNewline),
        https_auth::AuthError::MissingCredential => Some(HelperFailureCause::MissingField),
        https_auth::AuthError::MalformedOutput => Some(HelperFailureCause::MalformedOutput),
        https_auth::AuthError::OutputTooLarge => Some(HelperFailureCause::OutputLimit),
        _ => None,
    };
    if let Some(cause) = cause {
        failure.detail = Some(Box::new(FailureDetail {
            helper_cause: Some(cause),
            pipe_kind: match error {
                https_auth::AuthError::Pipe(kind) => Some(format!("{kind:?}")),
                _ => None,
            },
            ..FailureDetail::default()
        }));
    }
    failure
}

pub(super) fn helper_timeout(
    error: https_auth::AuthError,
    facts: &Facts,
    allocation_ms: i64,
    interaction_ms: i64,
) -> Failure {
    let mut failed = helper_failure(error, facts);
    let allowance = match error {
        https_auth::AuthError::AllocationTimeout => Some(allocation_ms),
        https_auth::AuthError::Timeout if interaction_ms > 0 => Some(interaction_ms),
        _ => None,
    };
    if let Some(allowance) = allowance {
        failed.detail = Some(Box::new(FailureDetail {
            helper_budget_ms: Some(allowance),
            ..Default::default()
        }));
    }
    failed
}

/// How a request on a connection failed.
pub(super) enum SendFailure {
    /// Hyper never started writing it: the connection had already closed.
    NotStarted,
    Sent(hyper::Error),
}
/// Sends `request`, telling a request Hyper never started from one it wrote
/// (dev-docs/GwzTransportIdleLossDesign.md §6.1 (b)).
pub(super) async fn send_request(
    sender: &mut hyper::client::conn::http1::SendRequest<RequestBody>,
    request: Request<RequestBody>,
) -> Result<Response<Incoming>, SendFailure> {
    sender.try_send_request(request).await.map_err(|mut error| {
        if error.take_message().is_some() {
            SendFailure::NotStarted
        } else {
            SendFailure::Sent(error.into_error())
        }
    })
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
cfg_if::cfg_if! { if #[cfg(all(test, unix))] { #[path="https_worker_tests.rs"] mod tests; } }
cfg_if::cfg_if! { if #[cfg(all(test, unix))] { #[path="https_budget_tests.rs"] mod budget_tests; } }
cfg_if::cfg_if! { if #[cfg(all(test, unix))] { mod retry_tests; mod helper_budget_tests; mod credential_tests; mod setup_slot_tests; mod tls_share_tests; mod supervisor_tests; mod throttle_worker_tests; } }
// Needs no HTTPS server, so it runs on Windows too.
cfg_if::cfg_if! { if #[cfg(test)] { mod close_tests; } }

cfg_if::cfg_if! {
    if #[cfg(test)] {
        /// A standalone endpoint, the anonymous-then-Gh replay and the configured
        /// deadlines, for the HTTPS tests. They prepare through
        /// `prepare_budget_for_transition`, which is `prepare_attempt`, the
        /// transport host's HTTPS endpoint's entry, without the first connect's
        /// report; `budget()` is what `budget_for_open` gives an Open that
        /// carries the configured deadlines.
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
            /// `prepare_attempt`, the production entry, without what its
            /// first connect did, which only the transport host's retry
            /// machine reads.
            pub(crate) async fn prepare_budget_for_transition(
                &self,
                input: Input,
                cancel: &CancellationToken,
                budget: &mut Budget,
                challenge: &mut Option<ChallengeLease>,
            ) -> Result<Prepared, Failure> {
                self.prepare_attempt(input, cancel, budget, challenge).await.0
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
                            && f.detail.as_ref().is_none_or(|d| d.schemes.is_none())
                            && f.facts
                                .as_ref()
                                .is_some_and(|facts| matches!(facts.http_status, Some(401))) =>
                    {
                        *first = Some(f);
                        input.policy = AuthPolicy::Gh;
                        self.prepare_budget_for_transition(input, cancel, &mut budget, &mut challenge)
                            .await
                    }
                    other => other,
                }
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
        }
    }
}

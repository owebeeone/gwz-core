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
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HOST, LOCATION},
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
    time::{Instant, timeout},
};
use tokio_util::sync::CancellationToken;

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
    pub(crate) fn new(
        tls: https_connection::Config,
        auth: Option<https_auth::Config>,
        config: pool::Config,
    ) -> Result<Self, Failure> {
        Self::new_with_io_timeout(tls, auth, config, 3_000)
    }
    pub(crate) fn new_with_io_timeout(
        tls: https_connection::Config,
        auth: Option<https_auth::Config>,
        config: pool::Config,
        io_timeout_ms: u64,
    ) -> Result<Self, Failure> {
        let authority = Authority::new(config.total, config.per_host);
        Self::with_authority(tls, auth, config, io_timeout_ms, authority)
    }
    pub(crate) fn with_authority(
        tls: https_connection::Config,
        auth: Option<https_auth::Config>,
        config: pool::Config,
        io_timeout_ms: u64,
        authority: Authority,
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
            auth_owner: https_auth::AuthOwner::new(),
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
pub(crate) struct Budget {
    allocation: Duration,
    helper: Duration,
    connect: Option<Duration>,
    network: Option<Duration>,
    cleanup: Duration,
}
impl Budget {
    pub(crate) fn shorten(&mut self, cap: Self) {
        fn bounded(value: Option<Duration>, cap: Option<Duration>) -> Option<Duration> {
            match (value, cap) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            }
        }
        self.allocation = self.allocation.min(cap.allocation);
        self.helper = self.helper.min(cap.helper);
        self.connect = bounded(self.connect, cap.connect);
        self.network = bounded(self.network, cap.network);
        self.cleanup = self.cleanup.min(cap.cleanup);
    }
}
impl Client {
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
        if https_policy::receive_pack(input.service) {
            input.policy = AuthPolicy::Gh;
        }
        let result = self
            .prepare_budget(input.clone(), cancel, &mut budget)
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
                self.prepare_budget(input, cancel, &mut budget).await
            }
            other => other,
        }
    }
    pub(crate) fn budget(&self) -> Budget {
        budget_for_config(&self.config, self.io_timeout_ms)
    }
}

fn budget_for_config(config: &pool::Config, io_timeout_ms: u64) -> Budget {
    Budget {
        allocation: Duration::from_millis(config.allocation_timeout_ms),
        helper: Duration::from_millis(config.interaction_timeout_ms),
        connect: (config.connect_timeout_ms != 0)
            .then(|| Duration::from_millis(config.connect_timeout_ms)),
        // Active I/O consumes one cumulative budget across redirects and
        // authentication attempts; zero deliberately disables the deadline.
        network: (io_timeout_ms != 0).then(|| Duration::from_millis(io_timeout_ms)),
        cleanup: Duration::from_millis(config.cleanup_timeout_ms),
    }
}
impl Client {
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
    pub(crate) fn budget_for_open(&self, deadlines: &Deadlines) -> Budget {
        let mut budget = self.budget();
        if deadlines.allocation_ms > 0 {
            budget.allocation = budget
                .allocation
                .min(Duration::from_millis(deadlines.allocation_ms as u64));
        }
        if deadlines.connect_ms > 0 {
            let requested = Duration::from_millis(deadlines.connect_ms as u64);
            budget.connect = Some(
                budget
                    .connect
                    .map_or(requested, |current| current.min(requested)),
            );
        }
        if deadlines.interaction_ms >= 0 {
            budget.helper = budget
                .helper
                .min(Duration::from_millis(deadlines.interaction_ms as u64));
        }
        if deadlines.io_ms > 0 {
            let requested = Duration::from_millis(deadlines.io_ms as u64);
            budget.network = Some(
                budget
                    .network
                    .map_or(requested, |current| current.min(requested)),
            );
        }
        if deadlines.cleanup_ms > 0 {
            budget.cleanup = budget
                .cleanup
                .min(Duration::from_millis(deadlines.cleanup_ms as u64));
        }
        budget
    }

    pub(crate) async fn prepare_budget(
        &self,
        input: Input,
        cancel: &CancellationToken,
        budget: &mut Budget,
    ) -> Result<Prepared, Failure> {
        let original = Destination::parse(&input.destination).map_err(failure)?;
        if input.session.is_empty()
            || input.session.len() > 128
            || input.operation.is_empty()
            || input.operation.len() > 128
            || !matches!(input.policy, AuthPolicy::Anonymous | AuthPolicy::Gh)
        {
            return Err(failure(ErrorCode::InvalidRequest));
        }
        if cancel.is_cancelled() {
            return Err(failure(ErrorCode::Cancelled));
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
        let mut dependency = Some(self.operation(&input.operation).map_err(failure)?);
        let key = RouteKey::new(&input.operation, &original.base(), input.service);
        let mut destination = {
            let mut routes = self.routes.lock().unwrap_or_else(|e| e.into_inner());
            if https_policy::advertisement(input.service) {
                routes.admit(key.clone()).map_err(failure)?;
                original
            } else {
                Destination::parse(routes.get(&key).map_err(failure)?).map_err(failure)?
            }
        };
        let mut hops = 0;
        let mut credential_offered = false;
        loop {
            let mut authorization = None;
            let mut facts = Facts::default();
            facts.credential_offered = credential_offered;
            if input.policy == AuthPolicy::Gh {
                facts.method = AuthMethod::Gh;
                let config = self
                    .auth
                    .as_ref()
                    .ok_or_else(|| with_facts(ErrorCode::Authentication, Effect::None, &facts))?;
                let started = Instant::now();
                let _helper = acquire_slot(self.helpers.clone(), budget.helper, cancel)
                    .await
                    .map_err(|error| with_facts(error.code, Effect::None, &facts))?;
                budget.helper = budget.helper.saturating_sub(started.elapsed());
                let started = Instant::now();
                let secret = https_auth::lookup_owned(
                    &self.auth_owner,
                    config,
                    &destination,
                    started + budget.helper,
                    cancel,
                )
                .await
                .map_err(|error| with_facts(error.code(), Effect::None, &facts))?;
                budget.helper = budget.helper.saturating_sub(started.elapsed());
                authorization = Some(secret.header());
            }
            if budget.connect.is_some_and(|remaining| remaining.is_zero())
                || budget.network.is_some_and(|remaining| remaining.is_zero())
                || budget.allocation.is_zero()
            {
                return Err(with_facts(ErrorCode::Timeout, Effect::None, &facts));
            }
            let lease = self
                .pool
                .checkout(
                    Key::https(destination.host(), destination.port()),
                    Owner::new(&input.session, &input.operation),
                    duration_ms(budget.allocation),
                    budget.connect.map_or(0, duration_ms),
                    cancel,
                )
                .await
                .map_err(|error| with_facts(error.code, error.effect, &facts))?;
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
                _slot: slot.take().expect("admitted request slot"),
                _operation: dependency.take().expect("operation dependency"),
                protocol_error: Arc::new(AtomicBool::new(false)),
                io_ms: budget.network.map_or(0, duration_ms),
                cleanup_ms: duration_ms(budget.cleanup),
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
            prepared.opened.facts.credential_offered =
                request.headers().contains_key(AUTHORIZATION) || credential_offered;
            credential_offered = prepared.opened.facts.credential_offered;
            let header_started = Instant::now();
            let response = tokio::select! {
                _=cancel.cancelled()=>return Err(with_facts(ErrorCode::Cancelled, Effect::None, &prepared.opened.facts)),
                _=prepared.lease.as_ref().unwrap().cancel.cancelled()=>return Err(with_facts(if prepared.protocol_error.load(Ordering::Acquire){ErrorCode::Protocol}else{ErrorCode::Cancelled}, Effect::None, &prepared.opened.facts)),
                result=async {
                    match budget.network {
                        Some(remaining) => tokio::time::timeout(remaining, guard.sender.send_request(request)).await.map_err(|_| failure(ErrorCode::Timeout)),
                        None => Ok(guard.sender.send_request(request).await),
                    }
                }=>result.map_err(|error| with_facts(error.code, Effect::None, &prepared.opened.facts))?.map_err(|error| with_facts(classify_hyper_error(&error), Effect::None, &prepared.opened.facts))?,
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
            let status = response.status().as_u16();
            prepared.opened.facts.http_status = Some(status as i64);
            if status == 401 && prepared.opened.facts.credential_offered {
                prepared.opened.facts.authenticated = Some(false);
            }
            match https_policy::classify(status, input.service, false) {
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
                    if hops == 5 {
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
                    // Re-enter with the existing slot below rather than reacquiring one.
                    slot = Some(returned_slot);
                    dependency = Some(returned_dependency);
                    continue;
                }
                ResponseAction::Fail(code) => {
                    let mut failed = with_facts(code, Effect::None, &prepared.opened.facts);
                    drop(response);
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
fn duration_ms(duration: Duration) -> u64 {
    duration
        .as_millis()
        .saturating_add(u128::from(duration.subsec_nanos() % 1_000_000 != 0))
        .min(u64::MAX as u128) as u64
}
fn body_channel() -> (mpsc::Sender<std::io::Result<Bytes>>, RequestBody) {
    let (tx, rx) = mpsc::channel(1);
    (tx, RequestBody { rx })
}
fn with_facts(code: ErrorCode, effect: Effect, facts: &Facts) -> Failure {
    Failure {
        code,
        effect,
        facts: Some(facts.clone()),
    }
}
async fn acquire_slot(
    slots: Arc<Semaphore>,
    allocation: Duration,
    cancel: &CancellationToken,
) -> Result<OwnedSemaphorePermit, Failure> {
    if allocation.is_zero() {
        return Err(failure(ErrorCode::Timeout));
    }
    tokio::select! {
        _ = cancel.cancelled() => Err(failure(ErrorCode::Cancelled)),
        result = tokio::time::timeout(allocation, slots.acquire_owned()) => {
            result.map_err(|_| failure(ErrorCode::Timeout))?
                .map_err(|_| failure(ErrorCode::Cancelled))
        }
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
impl Prepared {
    pub(crate) fn io_timeout_ms(&self) -> u64 {
        self.io_ms
    }
    fn request(&mut self, body: RequestBody) -> Result<Request<RequestBody>, Failure> {
        let url = self.destination.request(self.input.service);
        let mut request = Request::builder()
            .method(if https_policy::advertisement(self.input.service) {
                "GET"
            } else {
                "POST"
            })
            .uri(&url[url::Position::BeforePath..])
            .header(HOST, self.destination.authority())
            .header(ACCEPT, https_policy::response_type(self.input.service));
        if !https_policy::advertisement(self.input.service) {
            request = request.header(
                CONTENT_TYPE,
                format!(
                    "application/x-{}-request",
                    https_policy::service_name(self.input.service)
                ),
            );
        }
        if let Some(authorization) = self.authorization.take() {
            request = request.header(AUTHORIZATION, authorization);
        }
        let mut request = request
            .body(body)
            .map_err(|_| failure(ErrorCode::InvalidRequest))?;
        if let Some(value) = request.headers_mut().get_mut(AUTHORIZATION) {
            value.set_sensitive(true);
        }
        let protocol_error = self.protocol_error.clone();
        let cancel = self.lease.as_ref().unwrap().cancel.clone();
        let count = Arc::new(AtomicUsize::new(0));
        hyper::ext::on_informational(&mut request, move |response| {
            if !matches!(response.status().as_u16(), 100 | 102 | 103)
                || count.fetch_add(1, Ordering::Relaxed) >= 8
            {
                protocol_error.store(true, Ordering::Release);
                cancel.cancel();
            }
        });
        Ok(request)
    }
    pub(crate) async fn serve(
        mut self,
        stream: Stream,
        peer: Arc<MessageEndpoint>,
        cancel: CancellationToken,
    ) {
        let resource_cancel = self.lease.as_ref().unwrap().cancel.clone();
        let facts = Arc::new(Mutex::new(self.opened.facts.clone()));
        let possible = Arc::new(AtomicBool::new(false));
        let progress = self
            .lease
            .as_ref()
            .unwrap()
            .connection
            .as_ref()
            .unwrap()
            .lock()
            .await
            .progress
            .clone();
        let mut observed = progress.load(Ordering::Relaxed);
        let mut tick = tokio::time::interval(Duration::from_millis(2));
        let result = {
            let work = self.run(&stream, &peer, facts.clone(), possible.clone());
            tokio::pin!(work);
            loop {
                tokio::select! {
                    result=&mut work=>break result,
                    _=cancel.cancelled()=>break Err(ErrorCode::Cancelled),
                    _=resource_cancel.cancelled()=>break Err(ErrorCode::Cancelled),
                    _=tick.tick()=>{
                        let now=progress.load(Ordering::Relaxed);let bytes=now.wrapping_sub(observed);observed=now;
                        if bytes>0 && peer.io_status().state==IoState::Network {let _=peer.record_io_progress(bytes.min(usize::MAX as u64) as usize);}
                    },
                }
            }
        };
        if let Err(mut code) = result {
            if self.protocol_error.load(Ordering::Acquire) {
                code = ErrorCode::Protocol;
            }
            let effect = if possible.load(Ordering::Acquire) {
                Effect::Possible
            } else {
                Effect::None
            };
            let facts = facts.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let _ = peer.fail_terminal(with_facts(code, effect, &facts));
        }
    }
    async fn run(
        &mut self,
        stream: &Stream,
        peer: &Arc<MessageEndpoint>,
        facts: Arc<Mutex<Facts>>,
        possible: Arc<AtomicBool>,
    ) -> Result<(), ErrorCode> {
        let mut response = if let Some(response) = self.response.take() {
            response
        } else {
            let (tx, body) = body_channel();
            let request = self.request(body).map_err(|e| e.code)?;
            *facts.lock().unwrap_or_else(|e| e.into_inner()) = self.opened.facts.clone();
            let producer = async {
                let mut buffer = vec![0; 16384];
                loop {
                    peer.set_io_state(IoState::Backpressure)
                        .map_err(|_| ErrorCode::Io)?;
                    let n = stream.read(&mut buffer).await.map_err(stream_code)?;
                    if n == 0 {
                        peer.set_io_state(IoState::Network).map_err(stream_code)?;
                        break;
                    }
                    peer.set_io_state(IoState::Network)
                        .map_err(|_| ErrorCode::Io)?;
                    tx.send(Ok(Bytes::copy_from_slice(&buffer[..n])))
                        .await
                        .map_err(|_| ErrorCode::Io)?;
                }
                drop(tx);
                Ok::<_, ErrorCode>(())
            };
            let connection = self
                .lease
                .as_ref()
                .unwrap()
                .connection
                .as_ref()
                .unwrap()
                .clone();
            let mut guard = connection.lock().await;
            let send = async {
                facts
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .credential_offered = request.headers().contains_key(AUTHORIZATION);
                if self.input.service == GitService::ReceivePackExchange {
                    possible.store(true, Ordering::Release);
                }
                guard
                    .sender
                    .send_request(request)
                    .await
                    .map_err(|error| classify_hyper_error(&error))
            };
            tokio::pin!(send, producer);
            tokio::select! {
                result=&mut send=>{
                    let response=result?;
                    record_response(&response,&facts);
                    check_response(&response,self.input.service)?;
                    // A final rejection is returned immediately, even when Git is
                    // blocked writing a body. Successful responses still require
                    // an explicit EndWrite before this connection can be reused.
                    producer.await?;
                    response
                },
                result=&mut producer=>{
                    let response=send.await?;
                    record_response(&response,&facts);
                    check_response(&response,self.input.service)?;
                    result?;
                    response
                },
            }
        };
        record_response(&response, &facts);
        check_response(&response, self.input.service)?;
        loop {
            peer.set_io_state(IoState::Network).map_err(stream_code)?;
            let Some(frame) = response.body_mut().frame().await else {
                break;
            };
            let frame = frame.map_err(|_| ErrorCode::Protocol)?;
            if let Ok(data) = frame.into_data() {
                peer.record_io_progress(data.len()).map_err(stream_code)?;
                peer.set_io_state(IoState::Backpressure)
                    .map_err(stream_code)?;
                for chunk in data.chunks(16384) {
                    stream.write_all(chunk).await.map_err(stream_code)?;
                }
            }
        }
        drop(response);
        peer.set_io_state(IoState::Backpressure)
            .map_err(stream_code)?;
        stream.end_write().await.map_err(stream_code)?;
        // GET has no body; wait for the peer's explicit EndWrite before close.
        if https_policy::advertisement(self.input.service) {
            let mut unexpected = [0];
            if stream.read(&mut unexpected).await.map_err(stream_code)? != 0 {
                return Err(ErrorCode::Protocol);
            }
        }
        let until = Instant::now() + Duration::from_millis(self.cleanup_ms);
        let disposition = {
            let connection = self.lease.as_ref().unwrap().connection.as_ref().unwrap();
            let mut guard = connection.lock().await;
            if matches!(
                tokio::time::timeout_at(until, guard.sender.ready()).await,
                Ok(Ok(()))
            ) {
                Disposition::Reusable
            } else {
                Disposition::Discarded
            }
        };
        loop {
            let final_facts = facts.lock().unwrap_or_else(|e| e.into_inner()).clone();
            match peer.complete_close(disposition, final_facts) {
                Ok(()) => {
                    self.lease
                        .take()
                        .expect("active HTTP lease")
                        .finish(disposition)
                        .map_err(|e| e.code)?;
                    return Ok(());
                }
                Err(gwz_transport::stream::Error::WouldBlock) => {}
                Err(error) => return Err(stream_code(error)),
            }
            if Instant::now() >= until {
                return Err(ErrorCode::Timeout);
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }
}
fn record_response(response: &Response<Incoming>, facts: &Mutex<Facts>) {
    let mut f = facts.lock().unwrap_or_else(|e| e.into_inner());
    f.http_status = Some(response.status().as_u16() as i64);
    if response.status() == 401 && f.credential_offered {
        f.authenticated = Some(false);
    }
}
fn check_response(response: &Response<Incoming>, service: GitService) -> Result<(), ErrorCode> {
    match https_policy::classify(response.status().as_u16(), service, false) {
        ResponseAction::Success => validate_content(response, service),
        ResponseAction::Fail(code) => Err(code),
        _ => Err(ErrorCode::Protocol),
    }
}
fn stream_code(error: gwz_transport::stream::Error) -> ErrorCode {
    match error {
        gwz_transport::stream::Error::Cancelled => ErrorCode::Cancelled,
        gwz_transport::stream::Error::Timeout => ErrorCode::Timeout,
        gwz_transport::stream::Error::PeerFailed { code, .. } => code,
        _ => ErrorCode::Io,
    }
}
cfg_if::cfg_if! { if #[cfg(test)] { #[path="https_worker_tests.rs"] mod tests; } }
cfg_if::cfg_if! { if #[cfg(test)] { #[path="https_budget_tests.rs"] mod budget_tests; } }

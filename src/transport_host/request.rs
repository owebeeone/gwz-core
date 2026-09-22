use super::*;
use crate::git::endpoint::{ssh_channel::GitService, stream_io::BlockingStream};
use gwz_transport::protocol::{Facts, Opened};
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone)]
pub(crate) struct RequestContext {
    pub(super) session: Arc<Session>,
    pub(super) meta: RequestMeta,
    operation: String,
    active: Arc<AtomicBool>,
    https_routes: Arc<Mutex<std::collections::BTreeMap<String, Arc<Mutex<HttpsRouteState>>>>>,
}
impl RequestContext {
    pub(super) fn new(
        session: Arc<Session>,
        meta: RequestMeta,
        operation: String,
    ) -> ModelResult<Self> {
        session.register(&meta.request_id, Some(operation.clone()))?;
        Ok(Self {
            session,
            meta,
            operation,
            active: Arc::new(AtomicBool::new(true)),
            https_routes: Arc::new(Mutex::new(std::collections::BTreeMap::new())),
        })
    }
    pub(crate) fn is_cli(&self) -> bool {
        is_cli(&self.meta)
    }
    pub(crate) fn validate(&self, meta: &RequestMeta, operation: &str) -> ModelResult<()> {
        if !self.active.load(Ordering::Acquire) || self.session.is_closed() {
            return Err(unavailable("transport request is closed"));
        }
        if &self.meta != meta || self.operation != operation {
            return Err(invalid(
                "transport scope metadata or operation does not match",
            ));
        }
        Ok(())
    }
    pub(crate) fn check_identity(&self, raw: &str) -> ModelResult<()> {
        self.validate(&self.meta, &self.operation)?;
        self.session
            .check(&self.meta.request_id, identity(raw, &self.meta))
    }
    pub(crate) fn open(
        &self,
        url: &str,
        service: GitService,
        selected: Option<String>,
        opened: Arc<dyn Fn(i64, &Opened) + Send + Sync>,
        facts: Arc<dyn Fn(&Facts) + Send + Sync>,
    ) -> io::Result<BlockingStream> {
        self.validate(&self.meta, &self.operation)
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "transport scope closed"))?;
        self.session.open(
            &self.meta.request_id,
            &self.operation,
            url,
            service,
            selected
                .as_deref()
                .map(|s| identity(s, &self.meta))
                .unwrap_or_default(),
            opened,
            facts,
        )
    }
    pub(crate) fn open_https(
        &self,
        url: &str,
        service: gwz_transport::protocol::GitService,
        policy: Option<gwz_transport::protocol::AuthPolicy>,
        opened: Arc<dyn Fn(i64, &Opened) + Send + Sync>,
        facts: Arc<dyn Fn(&Facts) + Send + Sync>,
    ) -> io::Result<BlockingStream> {
        self.open_https_recording(
            url,
            service,
            policy,
            opened,
            facts,
            Arc::new(Mutex::new(None)),
        )
    }
    pub(crate) fn open_https_recording(
        &self,
        url: &str,
        service: gwz_transport::protocol::GitService,
        policy: Option<gwz_transport::protocol::AuthPolicy>,
        opened: Arc<dyn Fn(i64, &Opened) + Send + Sync>,
        facts: Arc<dyn Fn(&Facts) + Send + Sync>,
        first_receipt: Arc<Mutex<Option<HttpsAttemptReceipt>>>,
    ) -> io::Result<BlockingStream> {
        use gwz_transport::protocol::{AuthPolicy, Effect, ErrorCode, Failure};
        let early = |code| {
            io::Error::other(HttpsOpenFailure {
                failure: Failure {
                    code,
                    effect: Effect::None,
                    facts: None,
                },
                stream_id: None,
                policy: policy.unwrap_or(AuthPolicy::Anonymous),
                anonymous: None,
            })
        };
        self.validate(&self.meta, &self.operation)
            .map_err(|_| early(ErrorCode::Cancelled))?;
        let canonical = crate::git::endpoint::https_destination::Destination::parse(url)
            .map_err(early)?
            .base();
        let route = {
            let mut routes = self.https_routes.lock().unwrap_or_else(|e| e.into_inner());
            if routes.len() >= 64 && !routes.contains_key(&canonical) {
                return Err(early(ErrorCode::Capacity));
            }
            routes.entry(canonical.clone()).or_default().clone()
        };
        // Serialize only opening, not stream exchange. The gate covers the full
        // Anonymous -> Gh pair and cache lookup. Equivalent URLs share it.
        let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut route = loop {
            self.validate(&self.meta, &self.operation)
                .map_err(|_| early(ErrorCode::Cancelled))?;
            match route.try_lock() {
                Ok(guard) => break guard,
                Err(std::sync::TryLockError::Poisoned(error)) => break error.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => {
                    if std::time::Instant::now() >= until {
                        return Err(early(ErrorCode::Timeout));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
        };
        if route.mode.is_some_and(|mode| mode != policy) {
            // Explicit Anonymous and automatic discovery have identical Open
            // shapes. Policy is fixed by the backend for this request/route;
            // switching explicit modes requires a fresh request registration.
            return Err(early(ErrorCode::InvalidRequest));
        }
        route.mode = Some(policy);
        let first = policy.unwrap_or_else(|| {
            if crate::git::endpoint::https_policy::receive_pack(service) {
                AuthPolicy::Gh
            } else {
                route.resolved
            }
        });
        if first == AuthPolicy::Anonymous
            && crate::git::endpoint::https_policy::advertisement(service)
        {
            *first_receipt.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        let mut selected = first;
        let mut result = self.session.open_https(
            &self.meta.request_id,
            &self.operation,
            &canonical,
            service,
            first,
            opened.clone(),
            facts.clone(),
        );
        let mut anonymous = first_receipt
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if policy.is_none()
            && first == AuthPolicy::Anonymous
            && crate::git::endpoint::https_policy::advertisement(service)
        {
            if let Err((stream_id, failure)) = &result {
                if matches!(
                    failure.code,
                    ErrorCode::Authentication | ErrorCode::RepositoryRefused
                ) && failure
                    .facts
                    .as_ref()
                    .is_some_and(|f| matches!(f.http_status, Some(401 | 404)))
                {
                    anonymous = Some(HttpsAttemptReceipt {
                        stream_id: *stream_id,
                        policy: first,
                        failure: failure.clone(),
                    });
                    *first_receipt.lock().unwrap_or_else(|e| e.into_inner()) = anonymous.clone();
                    selected = AuthPolicy::Gh;
                    result = if self.validate(&self.meta, &self.operation).is_err() {
                        Err((
                            None,
                            Failure {
                                code: ErrorCode::Cancelled,
                                effect: Effect::None,
                                facts: None,
                            },
                        ))
                    } else {
                        self.session.open_https(
                            &self.meta.request_id,
                            &self.operation,
                            &canonical,
                            service,
                            selected,
                            opened,
                            facts.clone(),
                        )
                    };
                }
            }
        }
        match result {
            Ok(stream) => {
                route.resolved = selected;
                Ok(stream)
            }
            Err((stream_id, failure)) => {
                if let Some(value) = &failure.facts {
                    facts(value);
                }
                Err(io::Error::other(HttpsOpenFailure {
                    failure,
                    stream_id,
                    policy: selected,
                    anonymous,
                }))
            }
        }
    }
    fn cancel(&self) {
        self.active.store(false, Ordering::Release);
        self.session.cancel(&self.meta.request_id);
    }
}
fn identity(raw: &str, meta: &RequestMeta) -> gwz_transport::protocol::Identity {
    gwz_transport::protocol::Identity {
        mode: gwz_transport::protocol::IdentityMode::ExplicitKey,
        key_path: Some(raw.into()),
        path_base: meta
            .transport
            .as_ref()
            .and_then(|t| t.endpoint_path_base.clone()),
    }
}
pub struct TransportRequest {
    pub(super) context: RequestContext,
    pub(super) backend: Option<Git2Backend>,
    local_registration: Option<ClientRequest>,
}
impl TransportRequest {
    pub(super) fn pending(
        context: RequestContext,
        local_registration: Option<ClientRequest>,
    ) -> Self {
        Self {
            context,
            backend: None,
            local_registration,
        }
    }
    pub fn backend(&self) -> &Git2Backend {
        self.backend.as_ref().expect("admitted transport scope")
    }
    pub fn cancel(&self) {
        self.context.cancel();
    }
    pub async fn finish(mut self) -> CleanupReport {
        self.context.cancel();
        let mut report = self
            .context
            .session
            .finish(&self.context.meta.request_id)
            .await;
        if let Some(local) = self.local_registration.take() {
            report.pending_local_work += local.finish().await.pending_local_work;
        }
        report
    }
}
impl Drop for TransportRequest {
    fn drop(&mut self) {
        self.context.cancel();
        self.context.session.seal(&self.context.meta.request_id);
    }
}
pub struct ClientRequest {
    session: Arc<Session>,
    request: String,
}
impl ClientRequest {
    pub(super) fn new(session: Arc<Session>, request: &str) -> ModelResult<Self> {
        session.register(request, None)?;
        Ok(Self {
            session,
            request: request.into(),
        })
    }
    pub async fn finish(self) -> CleanupReport {
        self.session.finish(&self.request).await
    }
}
impl Drop for ClientRequest {
    fn drop(&mut self) {
        self.session.cancel(&self.request);
        self.session.seal(&self.request);
    }
}
pub(super) fn is_cli(meta: &RequestMeta) -> bool {
    meta.transport.as_ref().and_then(|t| t.placement) == Some(TransportPlacement::Cli)
}
pub(super) fn validate_meta(meta: &RequestMeta, operation: &str) -> ModelResult<()> {
    if !identifier(&meta.request_id) || !identifier(operation) || meta.transport_message.is_some() {
        return Err(invalid("invalid transport request context"));
    }
    if meta.schema_version != "gwz.protocol/v0" {
        return Err(unsupported("unsupported request version"));
    }
    if let Some(base) = meta
        .transport
        .as_ref()
        .and_then(|t| t.endpoint_path_base.as_deref())
    {
        if !is_cli(meta) || base.is_empty() || base.len() > 16384 || base.contains('\0') {
            return Err(invalid("invalid endpoint path context"));
        }
    }
    Ok(())
}
pub(super) fn identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

struct HttpsRouteState {
    mode: Option<Option<gwz_transport::protocol::AuthPolicy>>,
    resolved: gwz_transport::protocol::AuthPolicy,
}
impl Default for HttpsRouteState {
    fn default() -> Self {
        Self {
            mode: None,
            resolved: gwz_transport::protocol::AuthPolicy::Anonymous,
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct HttpsAttemptReceipt {
    pub(crate) stream_id: Option<i64>,
    pub(crate) policy: gwz_transport::protocol::AuthPolicy,
    pub(crate) failure: gwz_transport::protocol::Failure,
}
#[derive(Debug)]
pub(crate) struct HttpsOpenFailure {
    pub(crate) failure: gwz_transport::protocol::Failure,
    pub(crate) stream_id: Option<i64>,
    pub(crate) policy: gwz_transport::protocol::AuthPolicy,
    pub(crate) anonymous: Option<HttpsAttemptReceipt>,
}
impl std::fmt::Display for HttpsOpenFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(status) = self
            .anonymous
            .as_ref()
            .and_then(|r| r.failure.facts.as_ref())
            .and_then(|f| f.http_status)
        {
            write!(f, "anonymous discovery returned HTTP {status}; ")?;
        }
        write!(f, "HTTPS endpoint request failed: {:?}", self.failure.code)
    }
}
impl std::error::Error for HttpsOpenFailure {}

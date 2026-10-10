//! Candidate embedding ownership for endpoint placement. The host supplies message delivery.
mod cancellable;
pub(crate) mod endpoint_environment;
mod helper_failure;
mod https_endpoint;
mod local_command;
pub use crate::git::endpoint::https_worker::native::NativeCaller;
pub use cancellable::with_cancellable_local_transport;
pub use cancellable::with_cancellable_local_transport_native;
pub use local_command::{with_local_transport, with_local_transport_native};
mod request;
mod session;
cfg_if::cfg_if! { if #[cfg(test)] { mod qualification_tests; } }
cfg_if::cfg_if! {
    // The clones that show OpenSSL's default verify paths as the trust.
    if #[cfg(all(test, unix, not(target_vendor = "apple")))] { mod ca_trust_clone_tests; }
}

// Windows parity (GwzTransportWindowsParityPlan.md): the test modules below still compile on Unix only. Each block
// belongs to one step, whose rows are in scripts/checks/windows_parity/<step>.json, and that step ungates its block in
// place (the cfg_if wrapper becomes plain declarations) without touching the others. The blank lines between blocks
// are deliberate: they keep two steps' edits from ever being adjacent lines, which git reports as a conflict.

// Permanent: the trust branch that only OpenSSL builds have (row 0.5, platform).
cfg_if::cfg_if! { if #[cfg(all(test, unix))] { mod ca_trust_tests; } }

// Step 1.8: the host-level modules that build a runtime with an SSH configuration.
cfg_if::cfg_if! {
    if #[cfg(all(test, unix))] {
        mod tests;
        mod throughput_tests;
        mod fault_tests;
        mod message_embedding_tests;
        mod https_route_scale_tests;
        mod https_compat_tests;
        mod endpoint_environment_tests;
        mod retry_tests;
    }
}

// Step 4.11: the helper-dependent modules.
cfg_if::cfg_if! {
    if #[cfg(all(test, unix))] {
        mod driver_tests;
        mod close_tests;
        mod command_tests;
        mod fetch_preflight_tests;
        mod https_tests;
        mod https_policy_tests;
        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                mod https_helper_projection_tests;
                mod ssh_helper_projection_tests;
                mod https_negotiate_projection_tests;
            }
        }
        mod cancellable_tests;
        mod cancellable_https_tests;
        mod ca_bundle_tests;
    }
}

use crate::git::endpoint::https_auth::HelperSlots;
use crate::git::endpoint::ssh_local;
use crate::{
    RequestMeta, TransportCapabilitiesRequest, TransportCapabilitiesResponse, TransportPlacement,
    git::Git2Backend,
    model::{ErrorCode, ModelError, ModelResult},
};
/// The caller's cancellation token that the cancellable entry takes, and the
/// controls that create and cancel it: gwz-session-host's, as the core session
/// contract's §5.2 transport entry takes them (1.1.0 S6.1).
pub use gwz_session_host::{CallControls, CancellationToken};
use gwz_transport::{
    binding, pool,
    protocol::{AuthPolicy, Scheme},
};
pub use request::{ClientRequest, TransportCancellation, TransportRequest};

/// Validate an operation before constructing an endpoint or consulting credentials.
pub fn validate_request_context(meta: &RequestMeta, operation_id: &str) -> ModelResult<()> {
    request::validate_meta(meta, operation_id)
}
pub(crate) use request::{HttpsAttemptReceipt, HttpsOpenFailure, RequestContext};
use session::Session;
pub(crate) use session::SshOpenFailure;
pub use session::{Attachment, TransportPort};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

cfg_if::cfg_if! {
    if #[cfg(not(windows))] {
        /// The policies an SSH and HTTPS runtime offers.
        fn ssh_and_https_policies() -> Vec<AuthPolicy> {
            vec![
                AuthPolicy::SshAmbient,
                AuthPolicy::SshExplicit,
                AuthPolicy::Anonymous,
                AuthPolicy::Gh,
            ]
        }
    } else {
        /// The policies an SSH and HTTPS runtime offers in the Windows qualification: `Gh` and `WindowsConfigured`
        /// wait for the helper runner (WH2, plan steps 4.1 to 4.5); the SSH policies end in a refusal at setup
        /// when they need an agent, which Windows has none of until Phase 3.
        fn ssh_and_https_policies() -> Vec<AuthPolicy> {
            vec![
                AuthPolicy::SshAmbient,
                AuthPolicy::SshExplicit,
                AuthPolicy::Anonymous,
                AuthPolicy::WindowsDefault,
            ]
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        /// The agent: the socket `SSH_AUTH_SOCK` names, when it is set.
        fn agent_from_environment() -> Option<PathBuf> {
            std::env::var_os("SSH_AUTH_SOCK")
                .filter(|p| !p.is_empty())
                .map(PathBuf::from)
        }
    } else {
        /// Windows has no agent source until Phase 3 of the Windows parity plan: a setup that needs an agent is
        /// refused as one with no agent is.
        fn agent_from_environment() -> Option<PathBuf> {
            None
        }
    }
}

fn apply_native_timeout(pool: &mut pool::Config, native_ms: u64) {
    if native_ms == 0 {
        pool.connect_timeout_ms = 0;
    }
}
#[derive(Clone)]
pub struct SshEndpointConfig {
    home: PathBuf,
    agent: Option<PathBuf>,
    pool: pool::Config,
    io_timeout_ms: u64,
}
impl SshEndpointConfig {
    pub fn from_environment() -> ModelResult<Self> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .ok_or_else(|| invalid("endpoint HOME is unavailable"))?;
        let agent = agent_from_environment();
        let timeout = crate::git::transport_timeout_ms();
        let mut pool = pool::Config::default();
        apply_native_timeout(&mut pool, timeout);
        Ok(Self {
            home,
            agent,
            pool,
            io_timeout_ms: timeout,
        })
    }
    cfg_if::cfg_if! {
        if #[cfg(test)] {
            pub(crate) fn fixture(home: PathBuf, agent: Option<PathBuf>) -> Self {
                Self {
                    home,
                    agent,
                    pool: pool::Config::default(),
                    io_timeout_ms: 3000,
                }
            }
            pub(crate) fn with_io_timeout_ms(mut self, io_timeout_ms: u64) -> Self {
                self.io_timeout_ms = io_timeout_ms;
                self
            }
            /// The setup's aggregate clock, which the HTTPS connect uses too.
            pub(crate) fn with_connect_timeout_ms(mut self, connect_timeout_ms: u64) -> Self {
                self.pool.connect_timeout_ms = connect_timeout_ms;
                self
            }
        }
    }
}
/// Runtime budgets do not imply an SSH engine or a HOME requirement.
#[derive(Clone)]
pub(super) struct EndpointSettings {
    ssh: Option<SshSettings>,
    pool: pool::Config,
    io_timeout_ms: u64,
}
#[derive(Clone)]
pub(super) struct SshSettings {
    home: PathBuf,
    agent: Option<PathBuf>,
}
impl From<SshEndpointConfig> for EndpointSettings {
    fn from(config: SshEndpointConfig) -> Self {
        Self {
            ssh: Some(SshSettings {
                home: config.home,
                agent: config.agent,
            }),
            pool: config.pool,
            io_timeout_ms: config.io_timeout_ms,
        }
    }
}
// Candidate-only injection; public SSH constructors remain unchanged.
#[derive(Clone)]
pub(crate) struct HttpsEndpointConfig {
    pub(crate) tls: crate::git::endpoint::https_connection::Config,
    pub(crate) auth: Option<crate::git::endpoint::https_auth::Config>,
}
#[derive(Clone, Debug, Default)]
pub struct CleanupReport {
    pub pending_local_work: usize,
    pub peer_cleanup_confirmed: bool,
}
struct RuntimeState {
    local: Arc<Session>,
    local_endpoint: Arc<Session>,
    local_link: session::LocalLink,
    cli: Option<Arc<Session>>,
    closed: bool,
    io_timeout_ms: u64,
    connect_timeout_ms: u64,
    https: bool,
}
impl Drop for RuntimeState {
    fn drop(&mut self) {
        self.local.close();
        self.local_endpoint.close();
        if let Some(cli) = &self.cli {
            cli.close();
        }
    }
}
#[derive(Clone)]
pub struct TransportRuntime(Arc<Mutex<RuntimeState>>);
impl TransportRuntime {
    pub fn new(local: SshEndpointConfig) -> ModelResult<Self> {
        Self::build(local, None)
    }
    /// `helper_slots` are the HTTPS helper slots of the host whose driver
    /// builds this runtime; its sessions' endpoints share them.
    pub(crate) fn with_https(
        local: impl Into<EndpointSettings>,
        https: HttpsEndpointConfig,
        helper_slots: HelperSlots,
    ) -> ModelResult<Self> {
        Self::build(local, Some((https, helper_slots)))
    }
    fn build(
        local: impl Into<EndpointSettings>,
        https: Option<(HttpsEndpointConfig, HelperSlots)>,
    ) -> ModelResult<Self> {
        Self::build_native(local.into(), https, None)
    }
    pub(crate) fn with_https_native(
        local: impl Into<EndpointSettings>,
        https: HttpsEndpointConfig,
        slots: HelperSlots,
        caller: NativeCaller,
    ) -> ModelResult<Self> {
        Self::build_native(local.into(), Some((https, slots)), Some(caller))
    }
    fn build_native(
        local: EndpointSettings,
        https: Option<(HttpsEndpointConfig, HelperSlots)>,
        native: Option<NativeCaller>,
    ) -> ModelResult<Self> {
        let enabled = https.is_some();
        let io_timeout_ms = local.io_timeout_ms;
        let connect_timeout_ms = local.pool.connect_timeout_ms;
        let allocation_ms = local.pool.allocation_timeout_ms;
        let interaction_ms = local.pool.interaction_timeout_ms;
        // The driver and the endpoint share this process, so each SSH open's
        // URL extras go from one to the other through this (TR2.18).
        let handoff = crate::git::endpoint::ssh_handoff::Handoff::default();
        let (endpoint, peer_port) =
            Session::endpoint_with_https_native(local, https, handoff.clone(), native)?;
        let (driver, core_port) = Session::driver_with_ssh_budgets(
            io_timeout_ms,
            connect_timeout_ms,
            Some(handoff),
            allocation_ms,
            interaction_ms,
        )?;
        let link = session::LocalLink::new(core_port, peer_port)?;
        Ok(Self(Arc::new(Mutex::new(RuntimeState {
            local: driver,
            local_endpoint: endpoint,
            local_link: link,
            cli: None,
            closed: false,
            io_timeout_ms,
            connect_timeout_ms,
            https: enabled,
        }))))
    }
    pub fn install_cli(&self) -> ModelResult<TransportPort> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            return Err(unavailable("transport runtime is closed"));
        }
        if state.cli.is_some() {
            return Err(invalid("client endpoint already installed"));
        }
        // The client's endpoint runs in another process: no handoff.
        let (session, port) = Session::driver(state.io_timeout_ms, state.connect_timeout_ms, None)?;
        state.cli = Some(session);
        Ok(port)
    }
    pub fn capabilities(
        &self,
        request: TransportCapabilitiesRequest,
    ) -> ModelResult<TransportCapabilitiesResponse> {
        if request.schema_version != "gwz.protocol/v0" {
            return Err(unsupported("unsupported protocol version"));
        }
        let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            return Err(unavailable("transport runtime is closed"));
        }
        let mut placements = vec![TransportPlacement::Local];
        if state.cli.as_ref().is_some_and(|s| !s.is_closed()) {
            placements.push(TransportPlacement::Cli);
        }
        Ok(TransportCapabilitiesResponse {
            file_identity: !cfg!(all(
                windows,
                gwz_transport_candidate,
                gwz_windows_https_qualification
            )),
            exact_agent_identity: !cfg!(all(
                windows,
                gwz_transport_candidate,
                gwz_windows_https_qualification
            )),
            message_versions: Some(vec![2]),
            placements: Some(placements),
            schemes: Some(if state.https {
                vec![Scheme::Ssh, Scheme::Https]
            } else {
                vec![Scheme::Ssh]
            }),
            auth_policies: Some(if state.https {
                ssh_and_https_policies()
            } else {
                vec![AuthPolicy::SshAmbient, AuthPolicy::SshExplicit]
            }),
            message_limits: Some(session::limits()),
        })
    }
    pub async fn request(
        &self,
        meta: RequestMeta,
        operation_id: String,
    ) -> ModelResult<TransportRequest> {
        self.open_request(meta, operation_id, None).await
    }
    /// The cancellable entry's request constructor (1.1.0 S6.1; the core
    /// session contract's §5.2): `request`, with the request's cancellation
    /// attached to the caller's token as the request registers, before it
    /// binds. A token already cancelled refuses it with `Cancelled`.
    async fn request_with_token(
        &self,
        meta: RequestMeta,
        operation_id: String,
        token: &CancellationToken,
    ) -> ModelResult<TransportRequest> {
        self.open_request(meta, operation_id, Some(token)).await
    }
    async fn open_request(
        &self,
        meta: RequestMeta,
        operation_id: String,
        token: Option<&CancellationToken>,
    ) -> ModelResult<TransportRequest> {
        request::validate_meta(&meta, &operation_id)?;
        let cli = request::is_cli(&meta);
        // The Cli placement's endpoint is beyond the port, and the session
        // protocol carries it no retry budget: it would run the default while
        // this driver counted the request's. Until that protocol carries one,
        // a budget is refused there, so the driver's record keeps the default.
        if cli
            && meta
                .policy
                .as_ref()
                .is_some_and(|policy| policy.max_retries.is_some())
        {
            return Err(unsupported(
                "--max-retries is not carried to the Cli placement's endpoint",
            ));
        }
        let (session, local_endpoint) = {
            let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed {
                return Err(unavailable("transport runtime is closed"));
            }
            if cli {
                (
                    state
                        .cli
                        .clone()
                        .ok_or_else(|| unavailable("client endpoint is not installed"))?,
                    None,
                )
            } else {
                (state.local.clone(), Some(state.local_endpoint.clone()))
            }
        };
        // Endpoint registration precedes the driver Bind even for the in-process route.
        let client_guard = if let Some(endpoint) = local_endpoint {
            let max_retries = request::max_retries(&meta);
            let policy = meta.policy.as_ref();
            let jobs = crate::operation::resolve_jobs(policy.and_then(|value| value.concurrency));
            let per_host = crate::operation::resolve_per_host(
                policy.and_then(|value| value.max_connections_per_host),
            );
            let client = endpoint
                .admit_client_request(
                    &meta.request_id,
                    pool::Capacity {
                        per_user_host: per_host,
                        per_host,
                        total: jobs.max(256),
                        max_requests: jobs.max(1024),
                    },
                )
                .await?;
            endpoint.set_max_retries(&meta.request_id, max_retries);
            Some(client)
        } else {
            None
        };
        let context = RequestContext::new(session, meta, operation_id)?;
        let mut guard = TransportRequest::pending(context, client_guard);
        if let Some(token) = token {
            guard.attach(token)?;
        }
        guard
            .context
            .session
            .begin(&guard.context.meta.request_id)?;
        guard.context.session.ready().await?;
        cfg_if::cfg_if! { if #[cfg(all(windows, gwz_transport_candidate, gwz_windows_https_qualification))] {
            guard.backend = Some(Git2Backend::without_credential_helpers().with_host_context(guard.context.clone()));
        } else {
            guard.backend = Some(Git2Backend::new().with_host_context(guard.context.clone()));
        } }
        Ok(guard)
    }
    pub async fn remove_cli(&self) -> CleanupReport {
        let cli = self.0.lock().unwrap_or_else(|e| e.into_inner()).cli.take();
        if let Some(cli) = cli {
            cli.close();
            cli.cleanup().await
        } else {
            CleanupReport::default()
        }
    }
    pub async fn shutdown(&self) -> CleanupReport {
        let (local, endpoint, cli) = {
            let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
            state.closed = true;
            state.local_link.stop();
            (
                state.local.clone(),
                state.local_endpoint.clone(),
                state.cli.take(),
            )
        };
        local.close();
        endpoint.close();
        if let Some(cli) = &cli {
            cli.close();
        }
        let mut report = endpoint.cleanup().await;
        report.pending_local_work += local.cleanup().await.pending_local_work;
        if let Some(cli) = cli {
            report.pending_local_work += cli.cleanup().await.pending_local_work;
        }
        report
    }
}
pub struct CliEndpoint(Arc<Session>);
impl CliEndpoint {
    pub fn new(config: SshEndpointConfig) -> ModelResult<(Self, TransportPort)> {
        let (session, port) = Session::endpoint(config)?;
        Ok((Self(session), port))
    }
    pub fn register_request(&self, request_id: &str) -> ModelResult<ClientRequest> {
        ClientRequest::new(self.0.clone(), request_id)
    }
    pub async fn shutdown(&self) -> CleanupReport {
        self.0.close();
        self.0.cleanup().await
    }
}
impl Drop for CliEndpoint {
    fn drop(&mut self) {
        self.0.close();
    }
}
pub fn require_cli_ssh(
    capabilities: &TransportCapabilitiesResponse,
    policy: AuthPolicy,
) -> ModelResult<()> {
    let valid = matches!(policy, AuthPolicy::SshAmbient | AuthPolicy::SshExplicit)
        && capabilities
            .message_versions
            .as_ref()
            .is_some_and(|v| v.contains(&2))
        && capabilities
            .placements
            .as_ref()
            .is_some_and(|v| v.contains(&TransportPlacement::Cli))
        && capabilities
            .schemes
            .as_ref()
            .is_some_and(|v| v.contains(&Scheme::Ssh))
        && capabilities
            .auth_policies
            .as_ref()
            .is_some_and(|v| v.contains(&policy));
    let limits = capabilities
        .message_limits
        .clone()
        .ok_or_else(|| unsupported("client message limits missing"))?;
    let mut offer = binding::offer(
        "capability-check",
        gwz_transport::protocol::EndpointRole::Driver,
    );
    offer.bind.as_mut().expect("Bind").versions = vec![2];
    offer.bind.as_mut().expect("Bind").receive_limits = limits.clone();
    let endpoint = binding::EndpointConfig {
        endpoint_id: "validation".into(),
        trust_owner: "validation".into(),
        role: gwz_transport::protocol::EndpointRole::Driver,
        schemes: vec![Scheme::Ssh],
        policies: vec![policy],
        limits,
    };
    if !valid || endpoint.accept(&offer).is_err() {
        return Err(unsupported("client SSH placement is unavailable"));
    }
    Ok(())
}
fn invalid(message: &str) -> ModelError {
    ModelError::new(ErrorCode::InvalidRequest, message)
}
fn unavailable(message: &str) -> ModelError {
    ModelError::new(ErrorCode::IoError, message)
}
fn unsupported(message: &str) -> ModelError {
    ModelError::new(ErrorCode::UnsupportedOperation, message)
}
cfg_if::cfg_if! {
    if #[cfg(test)] {
        #[test]
        fn positive_native_timeout_keeps_the_pool_aggregate() {
            let mut pool = pool::Config::default();
            let aggregate = pool.connect_timeout_ms;
            apply_native_timeout(&mut pool, 9_000);
            assert_eq!(pool.connect_timeout_ms, aggregate);
            assert_eq!(aggregate, 30_000);
            apply_native_timeout(&mut pool, 15_000);
            assert_eq!(pool.connect_timeout_ms, 30_000);
        }

        #[test]
        fn zero_native_timeout_disables_the_pool_aggregate() {
            let mut pool = pool::Config::default();
            apply_native_timeout(&mut pool, 0);
            assert_eq!(pool.connect_timeout_ms, 0);
        }
    }
}

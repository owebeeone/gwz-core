//! The Cli placement: the client endpoint a host application supplies beside the core's own, reached through a
//! `TransportPort` pair. Its wire fields (`TransportOptions.placement` and `endpoint_path_base`, the embedded
//! `transport_message` and the capability extras) are part of the ordinary protocol; the endpoint itself is in the
//! transport build, as the rest of this module is. The endpoint is the in-process client; the client owner's
//! `HostHandle` form (plan steps 1.3, 2.2c, 3.1) replaces `CliEndpoint::new(SshEndpointConfig)` when the owner's
//! production endpoint factory lands.

use super::*;
use crate::{TransportCapabilitiesRequest, TransportCapabilitiesResponse, TransportPlacement};
use gwz_transport::protocol::{AuthPolicy, Scheme};

/// The runtime's Cli driver session, once `install_cli` has installed it, and the budgets that session is
/// built with.
pub(super) struct CliSlot {
    session: Option<Arc<Session>>,
    io_timeout_ms: u64,
    connect_timeout_ms: u64,
    /// Whether the runtime's endpoint serves HTTPS too, which its capabilities report.
    https: bool,
}
impl CliSlot {
    pub(super) fn new(io_timeout_ms: u64, connect_timeout_ms: u64, https: bool) -> Self {
        Self {
            session: None,
            io_timeout_ms,
            connect_timeout_ms,
            https,
        }
    }
    pub(super) fn close(&self) {
        if let Some(cli) = &self.session {
            cli.close();
        }
    }
    /// The session, leaving the slot empty and its budgets as they were.
    pub(super) fn take(&mut self) -> Self {
        Self {
            session: self.session.take(),
            io_timeout_ms: self.io_timeout_ms,
            connect_timeout_ms: self.connect_timeout_ms,
            https: self.https,
        }
    }
    /// Waits out the session's cleanup; the work it left pending.
    pub(super) async fn cleanup(self) -> usize {
        match self.session {
            Some(cli) => cli.cleanup().await.pending_local_work,
            None => 0,
        }
    }
}

/// The session, in-process endpoint and notes a request runs on.
pub(super) type Route = (Arc<Session>, Option<Arc<Session>>, Option<NoteSink>);

/// The Cli placement's endpoint is beyond the port, and the session
/// protocol carries it no retry budget: it would run the default while
/// this driver counted the request's. Until that protocol carries one,
/// a budget is refused there, so the driver's record keeps the default.
pub(super) fn refuse_budget_for_cli(meta: &RequestMeta) -> ModelResult<()> {
    if request::is_cli(meta)
        && meta
            .policy
            .as_ref()
            .is_some_and(|policy| policy.max_retries.is_some())
    {
        return Err(unsupported(
            "--max-retries is not carried to the Cli placement's endpoint",
        ));
    }
    Ok(())
}

impl RuntimeState {
    /// Where `meta`'s request runs: the Cli driver the host installed, or the in-process endpoint.
    pub(super) fn route(&self, meta: &RequestMeta) -> ModelResult<Route> {
        if request::is_cli(meta) {
            Ok((
                self.cli
                    .session
                    .clone()
                    .ok_or_else(|| unavailable("client endpoint is not installed"))?,
                None,
                self.notes.clone(),
            ))
        } else {
            Ok((
                self.local.clone(),
                Some(self.local_endpoint.clone()),
                self.notes.clone(),
            ))
        }
    }
}

impl TransportRuntime {
    pub fn install_cli(&self) -> ModelResult<TransportPort> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            return Err(unavailable("transport runtime is closed"));
        }
        if state.cli.session.is_some() {
            return Err(invalid("client endpoint already installed"));
        }
        // The client's endpoint runs in another process: no handoff.
        let (session, port) =
            Session::driver(state.cli.io_timeout_ms, state.cli.connect_timeout_ms, None)?;
        state.cli.session = Some(session);
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
        if state.cli.session.as_ref().is_some_and(|s| !s.is_closed()) {
            placements.push(TransportPlacement::Cli);
        }
        Ok(TransportCapabilitiesResponse {
            // A selected key file is advertised on every platform: Windows has its named fixture GO (decision D30,
            // run 2026-10-15-parityrows, rows home-unset-absolute-identity and home-unset-tilde-identity).
            file_identity: true,
            // No platform selects one key from an agent yet (`--identity` names a private-key file), so no exact agent
            // identity is advertised anywhere. Decision D38 builds the selection in a separate lane; this cell
            // follows it then (GwzV110Plan.md S4.3 and S7.2, decision D30).
            exact_agent_identity: false,
            message_versions: Some(vec![2]),
            placements: Some(placements),
            schemes: Some(if state.cli.https {
                vec![Scheme::Ssh, Scheme::Https]
            } else {
                vec![Scheme::Ssh]
            }),
            auth_policies: Some(if state.cli.https {
                ssh_and_https_policies()
            } else {
                vec![AuthPolicy::SshAmbient, AuthPolicy::SshExplicit]
            }),
            message_limits: Some(session::limits()),
        })
    }
    pub async fn remove_cli(&self) -> CleanupReport {
        let cli = self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cli
            .session
            .take();
        if let Some(cli) = cli {
            cli.close();
            cli.cleanup().await
        } else {
            CleanupReport::default()
        }
    }
}

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
        /// The policies an SSH and HTTPS runtime offers on Windows: the Unix ones, whose `Gh` is the helper over
        /// `Basic`, and the two Windows ones, the logon session with the configured helper and without it.
        fn ssh_and_https_policies() -> Vec<AuthPolicy> {
            vec![
                AuthPolicy::SshAmbient,
                AuthPolicy::SshExplicit,
                AuthPolicy::Anonymous,
                AuthPolicy::Gh,
                AuthPolicy::WindowsConfigured,
                AuthPolicy::WindowsDefault,
            ]
        }
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

//! Candidate embedding ownership for endpoint placement. The host supplies message delivery.
mod cancellable;
mod cli_placement;
pub(crate) mod composition;
pub(crate) mod endpoint_environment;
mod helper_failure;
mod https_endpoint;
mod local_command;
// Step 4.6: the WinHTTP machine proxy. The grammar is portable and its tests run everywhere; Unix builds have no use for it.
cfg_if::cfg_if! { if #[cfg(any(windows, test))] { mod machine_proxy; } }
pub use crate::git::endpoint::https_worker::native::{NativeCaller, NativeHost};
pub use crate::git::endpoint::setup_retry::NoteSink;
pub use cancellable::with_cancellable_local_transport;
pub use cancellable::with_cancellable_local_transport_native;
pub use cancellable::with_cancellable_local_transport_native_noted;
pub use composition::{ProcessGate, process_safety};
pub use local_command::{
    with_local_transport, with_local_transport_native, with_local_transport_native_noted,
};
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
    if #[cfg(test)] {
        mod tests;
        mod throughput_tests;
        mod fault_tests;
        mod https_notes_tests;
        mod https_route_scale_tests;
        mod endpoint_environment_tests;
        mod retry_tests;
        mod backstop_tests;
        mod spawn_gate_chain_tests;
    }
}

// Step 3.2b: the agent step's refusals, worded from the runtime's agent source.
cfg_if::cfg_if! { if #[cfg(test)] { mod agent_source_tests; } }

// S7.1 (1.1.0) step C3a: the Cli placement is the 1.2.0 session host's transport entry, so the tests that drive it
// (the Cli harness, the embedded messages, the workspace commands and the fetch preflight over it) belong to the
// session candidate.
cfg_if::cfg_if! {
    if #[cfg(all(test, gwz_session_candidate))] {
        mod cli_driver_tests;
        mod cli_placement_tests;
        mod command_tests;
        mod fetch_preflight_tests;
        mod message_embedding_tests;
    }
}

// Step 5.1: the plain HTTP and git protocol clones this module's one test makes. The Windows qualification refused
// those remotes; retiring the switch removed the refusal, so the module runs on every platform.
cfg_if::cfg_if! { if #[cfg(test)] { mod https_compat_tests; } }

// Step 4.9: the words around a discovery redirect (T5, U9) and the WH3 matrix at the host.
cfg_if::cfg_if! { if #[cfg(test)] { mod https_redirect_tests; mod https_native_tests; } }

// Step 4.11: the helper-dependent modules. They run on every platform: a test's fake `git` is a script on Unix and
// the test executable running it under Git for Windows' shell on Windows (`helper_script::git_fixture`).
cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod driver_tests;
        mod https_tests;
        mod close_tests;
        mod producer_busy_tests;
        mod https_policy_tests;
        mod https_helper_projection_tests;
        mod ssh_helper_projection_tests;
        mod https_negotiate_projection_tests;
        mod cancellable_tests;
        mod cancellable_https_tests;
        mod ca_bundle_tests;
        mod host_key_tests;
    }
}

// Follow-up 1 of the SSH pump stall review: the large-blob push rows, ignored (they take minutes and gigabytes).
cfg_if::cfg_if! { if #[cfg(test)] { mod blob_push_tests; } }

use crate::git::endpoint::agent_source::AgentSource;
use crate::git::endpoint::https_auth::HelperSlots;
use crate::git::endpoint::ssh_home::{self, Home, HomeSource};
use crate::git::endpoint::ssh_local;
use crate::git::spawn_gate::SpawnGate;
use crate::{
    RequestMeta,
    git::Git2Backend,
    model::{ErrorCode, ModelError, ModelResult},
};
/// The caller's cancellation token that the cancellable entry takes, and the
/// controls that create and cancel it: gwz-session-host's, as the core session
/// contract's §5.2 transport entry takes them (1.1.0 S6.1).
pub use gwz_session_host::{CallControls, CancellationToken};
use gwz_transport::{binding, pool};
pub use request::{ClientRequest, TransportCancellation, TransportRequest};

/// Validate an operation before constructing an endpoint or consulting credentials.
pub fn validate_request_context(meta: &RequestMeta, operation_id: &str) -> ModelResult<()> {
    request::validate_meta(meta, operation_id)
}
pub(crate) use request::{HttpsAttemptReceipt, HttpsOpenFailure, RequestContext, policy_words};
use session::Session;
pub(crate) use session::{AgentFailure, SshLimitFailure, SshOpenFailure};
pub use session::{Attachment, TransportPort};
// S7.1 (1.1.0) step C3a: the Cli placement's public surface is the session candidate's (1.2.0).
cfg_if::cfg_if! {
    if #[cfg(all(gwz_transport_candidate, gwz_session_candidate, any(unix, windows)))] {
        pub use cli_placement::{CliEndpoint, require_cli_ssh};
    }
}
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

/// The agent the process environment's `SSH_AUTH_SOCK` selects (plan step 3.2b chooses on Windows).
fn agent_from_environment() -> AgentSource {
    AgentSource::capture(std::env::var_os("SSH_AUTH_SOCK").as_deref())
}

fn apply_native_timeout(pool: &mut pool::Config, native_ms: u64) {
    if native_ms == 0 {
        pool.connect_timeout_ms = 0;
    }
}
#[derive(Clone)]
pub struct SshEndpointConfig {
    home: PathBuf,
    home_source: HomeSource,
    agent: AgentSource,
    pool: pool::Config,
    io_timeout_ms: u64,
}
impl SshEndpointConfig {
    pub fn from_environment() -> ModelResult<Self> {
        // The same home contract as the snapshot-derived configuration (plan step 3.1), over the live process: its
        // variables and its current directory, read once, here.
        struct Live;
        impl ssh_home::Variables for Live {
            fn get(&self, name: &str) -> Option<std::ffi::OsString> {
                std::env::var_os(name)
            }
        }
        let cwd = std::env::current_dir().ok();
        let home = ssh_home::resolve(ssh_home::PLATFORM, &Live, cwd.as_deref(), &ssh_home::Disk)
            .map_err(|refusal| invalid(&refusal.to_string()))?;
        let agent = agent_from_environment();
        let timeout = crate::git::transport_timeout_ms();
        let mut pool = pool::Config::default();
        apply_native_timeout(&mut pool, timeout);
        Ok(Self {
            home: home.path,
            home_source: home.source,
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
                    home_source: HomeSource::Home,
                    agent: AgentSource::from_path(agent),
                    pool: pool::Config::default(),
                    io_timeout_ms: 3000,
                }
            }
            /// A runtime whose agent is `agent`, whatever form it has on the platform under test.
            pub(crate) fn with_agent_source(mut self, agent: AgentSource) -> Self {
                self.agent = agent;
                self
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
    /// The variable that named `home` (design section 3), which a `known_hosts` message prints.
    home_source: HomeSource,
    agent: AgentSource,
}
impl From<SshEndpointConfig> for EndpointSettings {
    fn from(config: SshEndpointConfig) -> Self {
        Self {
            ssh: Some(SshSettings {
                home: config.home,
                home_source: config.home_source,
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
    /// The Cli placement's driver session and the budgets it is built with: nothing without the session candidate.
    cli: cli_placement::CliSlot,
    closed: bool,
    /// Where the command's warnings and notes go, if it gave a reader.
    notes: Option<NoteSink>,
    /// The host's spawn gate: its helper slots' (the launches hold it), or one of its own for a runtime that has
    /// no HTTPS endpoint and so no launch. Every request's backend holds it around the spawns it makes.
    spawn_gate: SpawnGate,
}
impl Drop for RuntimeState {
    fn drop(&mut self) {
        self.local.close();
        self.local_endpoint.close();
        self.cli.close();
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
    /// The native route's runtime: its helper slots, and so its backends' spawns, take the caller's gate, the one
    /// the supervisor's launch guard takes (operator decision D31).
    pub(crate) fn with_https_native(
        local: impl Into<EndpointSettings>,
        https: HttpsEndpointConfig,
        caller: NativeCaller,
    ) -> ModelResult<Self> {
        let slots = HelperSlots::with_gate(caller.spawn_gate().clone());
        Self::build_native(local.into(), Some((https, slots)), Some(caller))
    }
    fn build_native(
        local: EndpointSettings,
        https: Option<(HttpsEndpointConfig, HelperSlots)>,
        native: Option<NativeCaller>,
    ) -> ModelResult<Self> {
        let enabled = https.is_some();
        // The launches of this host's helpers hold the gate their slots carry, so the backends of its requests must
        // hold that one. A runtime with no HTTPS endpoint has no launch and takes the process's gate.
        let spawn_gate = https
            .as_ref()
            .map(|(_, slots)| slots.gate().clone())
            .unwrap_or_else(composition::spawn_gate);
        // What the machine proxy setting said (Windows), which the driver words an https failure with (D6, D7).
        let machine = https
            .as_ref()
            .map(|(config, _)| config.tls.machine.clone())
            .unwrap_or_default();
        let io_timeout_ms = local.io_timeout_ms;
        let connect_timeout_ms = local.pool.connect_timeout_ms;
        let allocation_ms = local.pool.allocation_timeout_ms;
        let interaction_ms = local.pool.interaction_timeout_ms;
        // The home the endpoint reads under, for the driver to name the file when it words a host-key failure and to
        // expand `~/` identities under.
        let home = local.ssh.as_ref().map(|ssh| Home {
            path: ssh.home.clone(),
            source: ssh.home_source,
        });
        // The driver and the endpoint share this process, so each SSH open's
        // URL extras go from one to the other through this (TR2.18).
        let handoff = crate::git::endpoint::ssh_handoff::Handoff::with_agent(
            local
                .ssh
                .as_ref()
                .map(|ssh| ssh.agent.clone())
                .unwrap_or_default(),
        );
        let (endpoint, peer_port) =
            Session::endpoint_with_https_native(local, https, handoff.clone(), native)?;
        let (driver, core_port) = Session::driver_with_ssh_budgets(
            io_timeout_ms,
            connect_timeout_ms,
            Some(handoff),
            allocation_ms,
            interaction_ms,
        )?;
        driver.set_home(home);
        driver.set_machine(machine);
        let link = session::LocalLink::new(core_port, peer_port)?;
        Ok(Self(Arc::new(Mutex::new(RuntimeState {
            local: driver,
            local_endpoint: endpoint,
            local_link: link,
            cli: cli_placement::CliSlot::new(io_timeout_ms, connect_timeout_ms, enabled),
            closed: false,
            notes: None,
            spawn_gate,
        }))))
    }
    cfg_if::cfg_if! { if #[cfg(test)] {
        /// The SSH home the runtime's driver names and expands `~/` under.
        pub(crate) fn driver_home_for_test(&self) -> Option<Home> {
            self.0.lock().unwrap_or_else(|e| e.into_inner()).local.home()
        }
        /// Has the driver's push opens carry a producer ceiling of `ms`.
        pub(crate) fn set_producer_ceiling_ms_for_test(&self, ms: u64) {
            self.0.lock().unwrap_or_else(|e| e.into_inner()).local.set_producer_ceiling_ms_for_test(ms);
        }
    } }
    /// Has the endpoints this runtime built give their limit machines' notes
    /// to `notes` (§9). Called once, before the first request.
    pub(crate) fn set_notes(&self, notes: &NoteSink) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.local_endpoint.set_notes(notes);
        state.notes = Some(notes.clone());
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
        cfg_if::cfg_if! {
            if #[cfg(all(gwz_transport_candidate, gwz_session_candidate, any(unix, windows)))] {
                cli_placement::refuse_budget_for_cli(&meta)?;
            }
        }
        let (session, local_endpoint, notes, spawn_gate) = {
            let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed {
                return Err(unavailable("transport runtime is closed"));
            }
            cfg_if::cfg_if! {
                if #[cfg(all(gwz_transport_candidate, gwz_session_candidate, any(unix, windows)))] {
                    {
                        let (session, local_endpoint, notes) = state.route(&meta)?;
                        (session, local_endpoint, notes, state.spawn_gate.clone())
                    }
                } else {
                    (
                        state.local.clone(),
                        Some(state.local_endpoint.clone()),
                        state.notes.clone(),
                        state.spawn_gate.clone(),
                    )
                }
            }
        };
        // The in-process endpoint's session, which the request asks for the words its failures cannot carry (T5).
        let endpoint_session = local_endpoint.clone();
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
        let context = RequestContext::new(session, meta, operation_id, notes, spawn_gate)?
            .with_endpoint(endpoint_session);
        let mut guard = TransportRequest::pending(context, client_guard);
        if let Some(token) = token {
            guard.attach(token)?;
        }
        guard
            .context
            .session
            .begin(&guard.context.meta.request_id)?;
        guard.context.session.ready().await?;
        guard.backend = Some(Git2Backend::new().with_host_context(guard.context.clone()));
        Ok(guard)
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
        cli.close();
        let mut report = endpoint.cleanup().await;
        report.pending_local_work += local.cleanup().await.pending_local_work;
        report.pending_local_work += cli.cleanup().await;
        report
    }
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

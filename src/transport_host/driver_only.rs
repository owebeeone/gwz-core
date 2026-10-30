//! The driver-only construction (plan step 1.7 of dev-docs/GwzTransportReleaseConvergencePlan.md; dev-docs/
//! GwzSplitCommandChannel.md sections 3.4, 4.3 and 4.6): the core-side host `gwz server` runs. It has no endpoint of its
//! own: no `EndpointSettings`, no `Handoff`, no agent, key or helper, and no local endpoint session, so every request
//! runs on the client's endpoint beyond the [`TransportPort`] the build returns.
//!
//! | Behaviour | Clause |
//! | --- | --- |
//! | The builder names none of the types that make or feed a local endpoint, and takes only two budgets, a flag's value and the caller's environment snapshot; `scripts/checks/check_driver_only_boundary.py` holds that line over this file | plan 1.7 "no `EndpointSettings`, no `Handoff` and no local endpoint" |
//! | `build` refuses the native libgit2 transport, however it is selected (`--transport native`, `GWZ_TRANSPORT=native`, `gwz.transport native`), naming the form that selected it, before it builds anything | plan 1.7; split design 4.6 |
//! | A malformed setting is refused in the setting module's own words, and a form that does not decide (a flag over a variable) is not read | `transport_setting` design section 2 |
//! | Every request is the `cli` placement: any other placement is refused, and so is a retry budget the lane cannot carry | plan 1.7 "every request's placement is `cli`"; split design 4.6 |
//! | A route the client's endpoint does not serve (`file://`, a local path, `http://`, `git://`, the clone family's local peer) is refused by the backend each request builds, by name, before any effect. The route is judged on the URL libgit2 will connect to (after `insteadOf` and `pushInsteadOf`), by the classifier the binding binds on (`transport_binding::bound_route`), and the binding itself installs a refusing transport for a route it does not bind, so libgit2's own transport never runs: `git/gitbackend/transport_support/identity.rs`, `transport_binding.rs`, `transport.rs` | plan 1.7; split design 4.6 |
//!
//! The host makes no `gwz_connection::HostHandle`: the core's requests reach the client through the session port until
//! the consumers move onto the handle (plan phase 3), and the carrier-backed handle is step 2.2c's, which answers
//! `snapshot()` with a refusal (contract section 15). The spawn gate a request's backend holds is the process's, from
//! the composition (`composition::spawn_gate`), the one site that makes it.
use super::{CleanupReport, TransportPort, TransportRequest, cli_placement, composition, request};
use super::{RequestContext, invalid, session::Session, unavailable, unsupported};
use crate::git::{Git2Backend, spawn_gate::SpawnGate};
use crate::session_host::EnvironmentSnapshot;
use crate::transport_setting::{self, Driver, Source, Transport};
use crate::{RequestMeta, model::ModelResult};
use gwz_session_host::CancellationToken;
use std::sync::{Arc, Mutex};

/// Builds a [`DriverOnly`] host. Its inputs are the two budgets of the driver's session, the value of the transport
/// flag (the served core has none, so `None`) and, to `build`, the caller's environment snapshot.
#[derive(Clone, Copy, Debug)]
pub struct DriverOnlyBuilder {
    io_timeout_ms: u64,
    connect_timeout_ms: u64,
    flag: Option<Transport>,
}
impl DriverOnlyBuilder {
    /// A builder whose driver session stalls after `io_timeout_ms` and gives a connect `connect_timeout_ms`; `0` turns a
    /// bound off, as `--ssh-timeout 0` does.
    pub fn new(io_timeout_ms: u64, connect_timeout_ms: u64) -> Self {
        Self {
            io_timeout_ms,
            connect_timeout_ms,
            flag: None,
        }
    }
    /// The value of `--transport`, if the command line gave one.
    pub fn with_transport_flag(mut self, flag: Option<Transport>) -> Self {
        self.flag = flag;
        self
    }
    /// Builds the host and the port the carrier joins to the client's endpoint. The transport setting is resolved from
    /// `environment` first: the native route is refused, and a malformed setting is refused, before anything is built.
    pub fn build(
        self,
        environment: &EnvironmentSnapshot,
    ) -> ModelResult<(DriverOnly, TransportPort)> {
        refuse_native(self.flag, environment)?;
        // The client's endpoint runs in another process: no handoff.
        let (session, port) = Session::driver(self.io_timeout_ms, self.connect_timeout_ms, None)?;
        let state = State {
            session,
            spawn_gate: composition::spawn_gate(),
            closed: false,
        };
        Ok((DriverOnly(Arc::new(Mutex::new(state))), port))
    }
}

/// The native libgit2 transport, selected by whichever form decided, is refused; so is a setting no form can read.
fn refuse_native(flag: Option<Transport>, environment: &EnvironmentSnapshot) -> ModelResult<()> {
    // The served core has no flag form of its own: the words of a malformed setting are the Python driver's, which
    // leave `--transport` out.
    let setting = transport_setting::resolve(flag, environment)
        .map_err(|refusal| invalid(&refusal.message(Driver::Python)))?;
    if setting.transport != Transport::Native {
        return Ok(());
    }
    let selected = match &setting.source {
        Source::Flag => "--transport native".to_owned(),
        Source::Environment => "GWZ_TRANSPORT=native".to_owned(),
        Source::GlobalConfiguration(location) => {
            format!("gwz.transport native in {}", location.where_text())
        }
        Source::Default => "the default".to_owned(),
    };
    Err(unsupported(&format!(
        "a driver-only host refuses the native libgit2 transport (selected by {selected}): the served core runs gwz's \
         own transport through the client's endpoint only, and holds no agent, key or helper for libgit2 to use; \
         nothing was built"
    )))
}

struct State {
    session: Arc<Session>,
    spawn_gate: SpawnGate,
    closed: bool,
}
impl Drop for State {
    fn drop(&mut self) {
        self.session.close();
    }
}

/// The driver-only host: the driver session of the `cli` placement and nothing else. Cloning it shares the host.
#[derive(Clone)]
pub struct DriverOnly(Arc<Mutex<State>>);
impl DriverOnly {
    /// A request on the client's endpoint. The request's placement must be `cli`; the backend it holds refuses the
    /// routes that endpoint does not serve.
    pub async fn request(
        &self,
        meta: RequestMeta,
        operation_id: String,
    ) -> ModelResult<TransportRequest> {
        self.open(meta, operation_id, None).await
    }
    /// [`DriverOnly::request`], with the request's cancellation attached to `token` as it registers, before it binds. A
    /// token already cancelled refuses it with `Cancelled`.
    pub async fn request_with_token(
        &self,
        meta: RequestMeta,
        operation_id: String,
        token: &CancellationToken,
    ) -> ModelResult<TransportRequest> {
        self.open(meta, operation_id, Some(token)).await
    }
    async fn open(
        &self,
        meta: RequestMeta,
        operation_id: String,
        token: Option<&CancellationToken>,
    ) -> ModelResult<TransportRequest> {
        request::validate_meta(&meta, &operation_id)?;
        require_cli(&meta)?;
        cli_placement::refuse_budget_for_cli(&meta)?;
        let (session, spawn_gate) = {
            let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed {
                return Err(unavailable("driver-only host is closed"));
            }
            (state.session.clone(), state.spawn_gate.clone())
        };
        let context = RequestContext::new(session, meta, operation_id, None, spawn_gate)?;
        let mut guard = TransportRequest::pending(context, None);
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
    /// Closes the host and waits out the session's cleanup. A request after it is refused.
    pub async fn shutdown(&self) -> CleanupReport {
        let session = {
            let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
            state.closed = true;
            state.session.clone()
        };
        session.close();
        session.cleanup().await
    }
}

/// A request that is not the `cli` placement has no endpoint here to run on.
fn require_cli(meta: &RequestMeta) -> ModelResult<()> {
    if request::is_cli(meta) {
        return Ok(());
    }
    Err(unsupported(
        "a driver-only host serves the cli placement only: it has no endpoint of its own, so a request whose placement \
         is not cli is refused",
    ))
}

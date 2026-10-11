//! Candidate-only synchronous command embedding for the local alpha.
use super::{
    CleanupReport, EndpointSettings, HelperSlots, HttpsEndpointConfig, NoteSink, TransportRequest,
    TransportRuntime, endpoint_environment, unavailable,
};
use crate::session_host::EnvironmentSnapshot;
use crate::{RequestMeta, git::Git2Backend, model::ModelResult};

/// Own one command's shared SSH/HTTPS endpoint, including bounded cleanup on error.
/// Credentials and environment are captured on this local endpoint only.
pub fn with_local_transport<T>(
    meta: RequestMeta,
    operation: String,
    action: impl FnOnce(&Git2Backend) -> T,
) -> ModelResult<(T, CleanupReport)> {
    run(meta, operation, None, None, action)
}
/// Uses the original caller captured by the installed CLI before member fanout.
pub fn with_local_transport_native<T>(
    meta: RequestMeta,
    operation: String,
    native: super::NativeCaller,
    action: impl FnOnce(&Git2Backend) -> T,
) -> ModelResult<(T, CleanupReport)> {
    run(meta, operation, Some(native), None, action)
}
/// `with_local_transport_native`, also giving `notes` the `note: ` lines of
/// the adaptive concurrency design's §9 as the command's endpoints make them
/// (a host refusing extra connections, a long `Retry-After`, a host that
/// stops answering). The caller decides whether they are printed: none is
/// given in a machine-readable output mode.
pub fn with_local_transport_native_noted<T>(
    meta: RequestMeta,
    operation: String,
    native: super::NativeCaller,
    notes: NoteSink,
    action: impl FnOnce(&Git2Backend) -> T,
) -> ModelResult<(T, CleanupReport)> {
    run(meta, operation, Some(native), Some(notes), action)
}
fn run<T>(
    meta: RequestMeta,
    operation: String,
    native: Option<super::NativeCaller>,
    notes: Option<NoteSink>,
    action: impl FnOnce(&Git2Backend) -> T,
) -> ModelResult<(T, CleanupReport)> {
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| unavailable("local transport executor unavailable"))?;
    let (ssh, https) = environment_config(native.as_ref())?;
    let runtime = build_runtime(ssh, https, native, notes.as_ref())?;
    let request = executor.block_on(runtime.request(meta, operation))?;
    let mut command = Command {
        executor,
        runtime,
        request: Some(request),
    };
    let result = action(command.request.as_ref().unwrap().backend());
    let cleanup = command.finish();
    Ok((result, cleanup))
}
/// The command's runtime over its endpoints, which give `notes` (if any) the
/// limit machines' notes (§9).
fn build_runtime(
    ssh: EndpointSettings,
    https: HttpsEndpointConfig,
    native: Option<super::NativeCaller>,
    notes: Option<&NoteSink>,
) -> ModelResult<TransportRuntime> {
    // The command is the driver: its host's HTTPS helper slots are created
    // once here and shared by the endpoints of the sessions it opens.
    let runtime = match native {
        Some(caller) => {
            TransportRuntime::with_https_native(ssh, https, HelperSlots::new(), caller)?
        }
        None => TransportRuntime::with_https(ssh, https, HelperSlots::new())?,
    };
    if let Some(notes) = notes {
        runtime.set_notes(notes);
    }
    Ok(runtime)
}
/// The command's endpoint configuration, from the process environment as it
/// stands at the command's start: the command's environment snapshot.
fn environment_config(
    native: Option<&super::NativeCaller>,
) -> ModelResult<(EndpointSettings, HttpsEndpointConfig)> {
    let environment = EnvironmentSnapshot::from_os_pairs(std::env::vars_os())?;
    // The current directory is captured beside the environment, for a relative home (design section 3).
    let cwd = std::env::current_dir().ok();
    let direct = native
        .map(super::NativeCaller::qualification_direct)
        .unwrap_or_else(endpoint_environment::capture_qualification_proxy);
    endpoint_environment::endpoint_config_native(&environment, cwd.as_deref(), direct)
}
struct Command {
    executor: tokio::runtime::Runtime,
    runtime: TransportRuntime,
    request: Option<TransportRequest>,
}
impl Command {
    fn finish(&mut self) -> CleanupReport {
        let Some(request) = self.request.take() else {
            return CleanupReport::default();
        };
        self.executor.block_on(async {
            let operation = request.finish().await;
            let runtime = self.runtime.shutdown().await;
            // Shutdown is the final snapshot of the same owned work, not a
            // second set of jobs to add to the operation's earlier snapshot.
            CleanupReport {
                pending_local_work: runtime.pending_local_work,
                peer_cleanup_confirmed: operation.peer_cleanup_confirmed
                    && runtime.peer_cleanup_confirmed,
            }
        })
    }
}
impl Drop for Command {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

cfg_if::cfg_if! {
    if #[cfg(test)] {
        mod notes_tests;
    }
}

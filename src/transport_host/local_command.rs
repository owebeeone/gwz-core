//! Candidate-only synchronous command embedding for the local alpha.
use super::{
    CleanupReport, HelperSlots, HttpsEndpointConfig, SshEndpointConfig, TransportRequest,
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
    run(meta, operation, None, action)
}
/// Uses the original caller captured by the installed CLI before member fanout.
pub fn with_local_transport_native<T>(
    meta: RequestMeta,
    operation: String,
    native: super::NativeCaller,
    action: impl FnOnce(&Git2Backend) -> T,
) -> ModelResult<(T, CleanupReport)> {
    run(meta, operation, Some(native), action)
}
fn run<T>(
    meta: RequestMeta,
    operation: String,
    native: Option<super::NativeCaller>,
    action: impl FnOnce(&Git2Backend) -> T,
) -> ModelResult<(T, CleanupReport)> {
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| unavailable("local transport executor unavailable"))?;
    let (ssh, https) = environment_config()?;
    // The command is the driver: its host's HTTPS helper slots are created
    // once here and shared by the endpoints of the sessions it opens.
    let runtime = match native {
        Some(caller) => {
            TransportRuntime::with_https_native(ssh, https, HelperSlots::new(), caller)?
        }
        None => TransportRuntime::with_https(ssh, https, HelperSlots::new())?,
    };
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
/// The command's endpoint configuration, from the process environment as it
/// stands at the command's start: the command's environment snapshot.
fn environment_config() -> ModelResult<(SshEndpointConfig, HttpsEndpointConfig)> {
    let environment = EnvironmentSnapshot::from_os_pairs(std::env::vars_os())?;
    endpoint_environment::endpoint_config(&environment)
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

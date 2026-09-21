use std::path::Path;
use gwz_core::{FetchRequest, FetchResponse, RequestMeta, TransportOptions,
    TransportPlacement, TransportCapabilitiesRequest};
use gwz_core::model::ModelResult;
use gwz_core::transport_host::{CliEndpoint, SshEndpointConfig, TransportPort,
    TransportRuntime, require_cli_ssh};
use gwz_transport::protocol::AuthPolicy;

async fn fetch_using_client<P>(
    root: &Path,
    connect: impl FnOnce(TransportPort, TransportPort) -> ModelResult<P>,
) -> ModelResult<FetchResponse> {
    let runtime = TransportRuntime::new(SshEndpointConfig::from_environment()?)?;
    let (client, client_port) = CliEndpoint::new(SshEndpointConfig::from_environment()?)?;
    let core_port = runtime.install_cli()?;
    let pumps = connect(core_port, client_port)?;
    let capabilities = runtime.capabilities(TransportCapabilitiesRequest {
        schema_version: "gwz.protocol/v0".into(),
    })?;
    require_cli_ssh(&capabilities, AuthPolicy::SshAmbient)?;
    let meta = RequestMeta {
        request_id: "fetch-1".into(),
        schema_version: "gwz.protocol/v0".into(),
        transport: Some(TransportOptions {
            placement: Some(TransportPlacement::Cli),
            ..Default::default()
        }),
        ..Default::default()
    };
    let client_request = client.register_request(&meta.request_id)?;
    let operation_id = "op-fetch-1".to_owned();
    let scope = runtime.request(meta.clone(), operation_id.clone()).await?;
    let result = gwz_core::workspace_ops::handle_fetch(
        scope.backend(), root, FetchRequest { meta, ..Default::default() },
        operation_id,
    );
    let operation_cleanup = scope.finish().await;
    let client_cleanup = client_request.finish().await;
    let removed = runtime.remove_cli().await;
    let endpoint_cleanup = client.shutdown().await;
    let runtime_cleanup = runtime.shutdown().await;
    // An embedding records pending_local_work from these reports; it must not
    // turn transport cleanup or peer_cleanup_confirmed into Git success.
    let _ = (operation_cleanup, client_cleanup, removed,
        endpoint_cleanup, runtime_cleanup);
    drop(pumps);
    result
}

# Choosing where Git connections run

**Candidate embedding API: available only in the isolated integration harness,
not in default or released builds.** The implementation follows the accepted
endpoint-placement contract. Existing CLI commands and the default
`Git2Backend::new()` usage remain unchanged.

An endpoint opens Git-host connections and owns SSH credentials, host trust and
connection pools. Repository work stays in gwz-core. `local` means the core
machine; `cli` means the client endpoint supplied by your host application.
There is no new CLI command, server address or hosting service to configure.

## Default: connections on the core machine

Continue creating the backend as today and omit `TransportOptions.placement`.
Omission and explicit `local` both select core-local connections. No supplied
client channel is required. Existing identity options retain local behavior.
Do not set `endpoint_path_base` for local placement.

## Use a client endpoint

This is an embedding lifecycle, not a command-line walkthrough. A host must
provide asynchronous bidirectional message delivery that continues during a
running Git operation. One-way progress events and the final operation response
are insufficient. The concrete connection mechanism is the host's responsibility.

1. Construct a backend-family runtime, installing the shared local endpoint and
   optionally the client SSH endpoint with your supplied delivery hooks. Keep
   the returned runtime owner alive across operations to reuse connections.
2. Query the existing `transport_capabilities` operation. Require message version
   2, placement `cli`, scheme SSH, the intended authentication policy, and message
   limits. Missing capability fields mean unsupported. Do not send cli placement
   to an old core and rely on it to reject unknown options.
   Cache that answer only for the same live core instance/backend runtime and
   host connection generation. The host must pin the receiver through dispatch,
   with no hidden rerouting/retry. Reconnect, failover or receiver replacement
   invalidates it; query again before sending another explicit-cli operation.
   If the host cannot guarantee this affinity, cli placement is unavailable.
3. On the first operation, the runtime completes Bind/Bound using that operation's
   request id before any Git connection or workspace change. The acknowledgement
   must match the installed endpoint, scheme, policy, version and bounded limits.
4. Set `RequestMeta.transport.placement = cli`. Use the existing identity options.
   For a relative identity path, also set `endpoint_path_base` to the client's
   captured absolute directory; for example `/home/alex/project` with
   `default_identity = keys/work_key`. Both strings describe the client filesystem.
5. Run the ordinary core operation once. The runtime exchanges discrete transport
   messages while the operation executes. Inspect the ordinary response/events;
   no separate Git operation implementation belongs in your client.
6. To return to local connections, finish/cancel outstanding client operations,
   shut down their runtime binding, and omit placement on subsequent requests.
   Explicit bounded runtime shutdown releases pooled resources. Last-owner drop
   initiates cleanup too. A new runtime/session is required to reconnect later.

The candidate signatures and complete lifecycle example appear below. The exact
example is compiled by the full-core candidate tests; it is not a released API.

## Options and defaults

| Field | Meaning | Default |
|---|---|---|
| placement | `local` or `cli` for the whole operation | local |
| endpoint_path_base | Absolute directory in the CLI endpoint filesystem for relative identity paths | none; relative CLI paths then fail |
| default_identity | Existing invocation-wide SSH key selection | existing ambient selection when no other selection wins |
| remote_identities | Existing per-remote overrides | empty |
| url_scheme | Existing effective-URL preference | existing workspace/manifest policy |

Absolute key paths refer literally to the selected endpoint. `~/` resolves on
that endpoint; `~user` is unsupported. A repository's configured relative key
path also needs the client base when placement is cli. Core does not inspect
client key files. The existing `gwz auth identity` commands manage repository configuration
locally; the global `--remote-identity` option selects invocation overrides.
Neither creates or moves client keys.

All selected identity paths are checked before workspace mutation/network work;
Open checks again before using a key. A file can change after preflight, causing
a later operation failure. Selected-key failure never falls back to the agent.

## Supported routes and failures

The first implementation supports SSH client placement: SCP-style URLs, ssh://,
git+ssh:// and ssh+git://. Client HTTPS comes later. Local files, clone-family
routes, http:// and git:// cannot use cli placement. An operation whose planned
routes include unsupported placement fails before it starts changing repositories.

Missing endpoint, incompatible version, unsupported route or a closed supplied
channel produces an explicit failure. The runtime never silently switches an
explicit cli request to local credentials. Channel loss cancels active exchanges;
reconnection does not replay an interrupted push. A failed push can have remote
effects, so inspect the remote before choosing a retry.

## Attachments and results

Host adapters carry the shared transport Envelope in optional
`RequestMeta.transport_message` and `ResponseMeta.transport_message` fields,
correlated with the existing request_id. They must process these attachments
without invoking the operation again or inventing partial/final operation results.
The host must support delivery while the operation runs, bounded queues, closure
notification and control-message progress. If its existing message interface
cannot do that, client placement is unavailable until the host supplies it.

Optional observation fields endpoint_id, connection_id, stream_id and reused
identify the current attempt. Missing values mean unknown. Reuse does not mean a
credential was offered again: credential_offered is false on a reused connection,
while authenticated may describe previously proven connection authentication.
Private-member omission follows existing core policy.

This interface remains a candidate. Production availability still requires a
real supplied connection test and the applicable qualification/activation checks.

## Candidate Rust interface

These names and signatures are implemented in `gwz_core::transport_host` under
the isolated Unix candidate configuration. The production manifest does not enable
that configuration. See [the integration harness](../tests/transport_backend/README.md)
for preparation and tests; the example below is an exact-text compile fixture.
`ModelResult<T>` and generated request/response types retain their existing meanings.

```rust
pub struct SshEndpointConfig { /* private */ }
impl SshEndpointConfig {
    pub fn from_environment() -> ModelResult<Self>;
}

pub struct TransportRuntime { /* private; Clone shares one owner */ }
impl TransportRuntime {
    pub fn new(local: SshEndpointConfig) -> ModelResult<Self>;
    pub fn install_cli(&self) -> ModelResult<TransportPort>;
    pub fn capabilities(&self, request: TransportCapabilitiesRequest)
        -> ModelResult<TransportCapabilitiesResponse>;
    pub async fn request(&self, meta: RequestMeta, operation_id: String)
        -> ModelResult<TransportRequest>;
    pub async fn remove_cli(&self) -> CleanupReport;
    pub async fn shutdown(&self) -> CleanupReport;
}

pub struct CliEndpoint { /* private */ }
impl CliEndpoint {
    pub fn new(config: SshEndpointConfig) -> ModelResult<(Self, TransportPort)>;
    pub fn register_request(&self, request_id: &str) -> ModelResult<ClientRequest>;
    pub async fn shutdown(&self) -> CleanupReport;
}

pub struct TransportRequest { /* operation owner */ }
impl TransportRequest {
    pub fn backend(&self) -> &Git2Backend;
    pub fn cancel(&self);
    pub async fn finish(self) -> CleanupReport;
}
pub struct ClientRequest { /* client request registration */ }
impl ClientRequest {
    pub async fn finish(self) -> CleanupReport;
}

// Rust handoff value only; NOT another serialized host envelope.
// The String is the existing host request_id.
pub type Attachment = (String, gwz_transport::protocol::Envelope);
pub struct TransportPort { /* Clone shares queues and closure */ }
impl TransportPort {
    pub async fn next_message(&self) -> ModelResult<Option<Attachment>>;
    pub async fn deliver(&self, attachment: Attachment) -> ModelResult<()>;
    pub fn disconnect(&self);
}
pub struct CleanupReport {
    pub pending_local_work: usize,
    pub peer_cleanup_confirmed: bool,
}
pub fn require_cli_ssh(
    capabilities: &TransportCapabilitiesResponse,
    policy: gwz_transport::protocol::AuthPolicy,
) -> ModelResult<()>;
```

`from_environment` captures the endpoint's own home, agent socket and existing
startup timeout/pool policy; it performs no key/trust-file reads or connections.
The candidate facade uses a 64 KiB receive window and the shared profile's other
message limits. Each binding admits at most 256 distinct request IDs over its
lifetime and 64 concurrent streams. IDs are not recycled; replace an exhausted
CLI binding with `remove_cli`/`install_cli` and a new endpoint, or create a new
runtime for local binding exhaustion. Endpoint pool defaults include a 60-second
idle lifetime and the limits documented in the [transport pool guide](../../gwz-transport/README.md#connection-pool-api).
Bootstrap and explicit cleanup waits are bounded to five seconds; endpoint identity
checks allow up to 120 seconds. Network timing uses the core's startup timeout
(default three seconds); disabling it does not disable bounded cleanup.

Configuration is immutable after construction. Unsupported candidate/platform
support returns UnsupportedOperation. `new` starts bounded local supervisors;
resources belong to the returned owner, not the caller's executor. Physical
work and delivery progress do not run on the thread blocked inside a Git handler.

`install_cli` creates one unbound application port. Installing twice while one
is installed fails InvalidRequest; remove it before installing a replacement.
It neither opens a host connection nor performs Bind. `capabilities` is the
runtime-aware implementation of the existing operation, not a new service.
`require_cli_ssh` checks version 2, cli, SSH, the selected policy and usable limits;
it rejects missing/malformed/unsupported data before an operation is sent.
This helper checks capability contents only; it does not establish remote receiver
affinity. A split host must hold its own generation/receiver admission guard from
query through dispatch acceptance, invalidating it on replacement. The example's
same-runtime direct calls supply this affinity without another wire field.

`request` reserves the unique request/operation context, checks capabilities and
completes Bind/Bound if needed. It returns only after verifying Bound; the caller
does not construct or approve that acknowledgement. Bound installation races
owner cancellation under one state transition: if installation wins, the binding
survives and only the request is cancelled; otherwise the port/session retires,
all waiters fail, and a new port/session/endpoint is required. A failed or dropped
request future unregisters its request and applies that same cancellation rule.
It does not itself discover
or mutate repositories. Pass its backend, the same metadata and operation_id to
one ordinary handler. Reusing the scope with different metadata or operation id
is invalid. Handler preflight completes the effective route/identity checks.

The client registers the existing request_id before sending the command or
accepting its transport attachments. Its first valid Open/CheckIdentity fixes
the operation_id; later disagreement refuses. Registration persists through
cleanup; no new request id can be introduced just by sending an attachment.
Bind is allowed only under a registered request. `ClientRequest::finish` seals
that request against new work and waits for bounded cleanup; drop seals/cancels
it. A failed dispatch must also finish/drop the registration.

Port forwarding uses these directions:

| Direction | Read from | Deliver to |
|---|---|---|
| Core to client | `core_port.next_message()` | `client_port.deliver(attachment)` |
| Client to core | `client_port.next_message()` | `core_port.deliver(attachment)` |

The tuple carries the existing request_id and the shared transport Envelope;
it does not add a serialized wrapper. Owner message profile version `2` is
independent of the outer request schema version `gwz.protocol/v0`.
The in-memory executable direction fixture is
[`mux_async.rs`](../../gwz-transport/tests/mux_async.rs). It tests the lower-level
ports; the full facade is exercised by the core candidate lifecycle/command tests.

Port calls support cancellation of the waiting future. `next_message` transfers
one admitted outbound item, returning None after closure; dropping a pending
receive consumes nothing. `deliver` returns only after admission; dropping a
pending send admits nothing. An error closes that binding and wakes waiters.
One forwarding loop per direction preserves order; callers must not race delivery
futures or reorder received messages. Once next_message transfers an item, the
host must deliver it or disconnect; cancelling a forwarding loop must not silently
lose that item while leaving the session open. The host serializes only the existing
metadata attachment fields and request_id. It must bound any storage between
these calls and deliver concurrently in both directions. Cloning a port does not
create another session. `disconnect`, loss of the supplied host connection, or
last-port-owner drop invalidates it; disconnect is idempotent.

`cancel` requests cancellation of active transport work; it does not roll back
Git changes. `finish` seals new admissions and waits for owned work within the
configured cleanup deadline. Dropping a scope initiates cancellation/cleanup.
`remove_cli` invalidates that binding and cancels its requests, leaving local
routing available. `shutdown` invalidates the entire runtime/endpoint; all clones
observe closure. Both are idempotent; last-owner drop initiates the same cleanup.
Cancelling a shutdown future does not cancel cleanup. Pending supervised work
remains counted/contained after the deadline. CleanupReport describes *local*
owned work; disconnect never proves cleanup on another machine, so the port
boundary reports peer_cleanup_confirmed=false. It is not a Git success result.

## Example: configure, fetch once, remove

This example is compiled unchanged in the candidate harness. `connect` is supplied by the embedding:
it installs two independently progressing forwarding loops using the port methods
above and returns their owner. It is intentionally not a GWZ carrier API. In a
split deployment the two ports live in different processes; the example puts both
logical hosts together for clarity. Run this function on a blocking worker (the
ordinary Git handler blocks); the supplied delivery pumps run independently.

```rust
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
```

To cancel while the handler is running, invoke `scope.cancel()` from the host's
cancellation path with shared access to the scope; await/finish the handler before
consuming that scope. Early `?` returns drop the owners and initiate cancellation.
The explicit finish/shutdown path above is how callers obtain cleanup reports.
To execute locally afterward, create/use a local scope with placement omitted;
no client endpoint needs to be reinstalled.

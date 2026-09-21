# Choosing where Git connections run

**Proposed embedding API, not yet available in released builds.** This guide
specifies the interface being reviewed for endpoint placement. Existing CLI
commands and the default `Git2Backend::new()` usage remain unchanged.

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

The guide specifies semantic hooks; exact Rust constructor/method spellings will
be published with the implementation. It does not promise callable APIs today.

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
client key files. Existing `remote_identity` commands still manage repository
configuration locally; they do not create or move client keys.

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

This interface remains a candidate. Production availability requires implementation,
compatibility tests, a real supplied connection test and the activation checks.

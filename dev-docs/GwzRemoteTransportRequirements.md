# GWZ Remote Transport Requirements

Status: direction accepted 2026-09-19; implementation pending. Revised from the
2026-09-15 requirements draft following the operator's transport discussion.
The decisions in §7 replace the previously open alternatives. The companion
[design](GwzRemoteTransportDesign.md) specifies the proposed implementation;
its tuning values and library qualification items are identified separately
from accepted behaviour. No implementation or new performance result is claimed.

## 1. Purpose

Provide one transport service for GWZ SSH/HTTPS network operations. Existing
HTTP and git protocol operations remain native and local-only, as explicitly
listed in the design route table. Core opens a
virtual stream using taut messages; a mux routes those messages to the endpoint
that executes SSH or HTTPS. Initially the endpoint is either inside the core
process or at gwz-cli over the driver–core message channel. The endpoint owns
network traffic, ordinary local authentication and trust checks, and pooled
connections. The same API can later be carried over another connection, such
as iroh, without designing peer networking now.

Operator clarification: the transport package emulates a network stream with
discrete taut messages and asynchronous send/receive. The host communication
layer is supplied elsewhere. GWZ may carry generated transport messages in new
optional fields on existing requests/responses using existing request ids;
this programme does not require new CLI commands, core service methods or
physical message framing. Existing field tags and method behavior stay stable.

Connection reuse removes repeated SSH setup across repositories, phases and
operations. Placement chooses where that work runs. Fetch Phase 3 is an early
measurement consumer, not the boundary of this programme.

## 2. Terms

- **Driver**: gwz-cli, gwz-py or another caller of core.
- **Endpoint**: the transport service instance executing Git-host traffic and
  authentication in its owning process and local account context.
- **Mux**: the router selecting an endpoint when a virtual stream opens.
- **Carrier**: an in-process message delivery adapter or a driver–core message
  channel carrying the same taut-defined messages.
- **Virtual stream**: one bidirectional message conversation, carrying ordered
  data messages with variable-length byte payloads plus lifecycle messages.
- **Connection**: a reusable physical SSH or HTTPS connection at an endpoint.
- **Channel**: one SSH Git command on a connection.
- **Lease**: exclusive allocation of a connection to an active exchange. Closing
  a virtual stream releases its healthy connection after protocol cleanup.
- **Authority**: endpoint-local keys, agents, host-key trust and `gh` credentials.
- **Placement**: the selected endpoint; traffic and authority are colocated.

## 3. Baseline evidence and constraints

The following measurements and source references describe the 2026-09-15
investigation, not an up-to-date implementation inventory. The design identifies
current integration points. Line numbers in this historical snapshot may move.

### 3.1 Measurements

Measured 2026-09-15 from the operator's Mac to github.com. The prototype was a
throwaway (not committed): a registered libgit2 SSH transport whose streams are
channels on pooled `ssh2` sessions.

| Case | Result |
|---|---|
| libgit2's own SSH transport: 4 reads, a receive-pack advertisement, a depth-1 fetch | 6 connections, 16.5 s; 2.2–3.5 s each |
| The same six operations through the prototype | 1 connection, 7.1 s; reads after the first 0.67–0.74 s |
| OpenSSH, a new connection per advertisement | 2.0–3.1 s each |
| OpenSSH, 24 advertisements as channels on one connection | 0.44–0.63 s each, no failures |
| OpenSSH, 4 concurrent channels on one connection | 0.62 s in total |
| HTTPS, new connection | TLS complete within 45 ms; request about 0.30 s |
| HTTPS, reused connection | requests 0.20–0.34 s |

Not measured: pushing a pack through the prototype, and the cost of running
`gh auth git-credential` for each operation.

### 3.2 Code

- libgit2's SSH transport opens a socket and an SSH session for each stream and
  frees the session with it (libgit2 1.9.7 `transports/ssh_libssh2.c:180-199`,
  `774-897`). libgit2-sys 0.18.8 builds only this libssh2 backend
  (`build.rs:248-254`), and libgit2 never reads `~/.ssh/config`.
- A transport registered through `git2::transport::register` takes precedence over
  the built-in one, including for `git@host:path` URLs (`transport.c:56-60`,
  `98-99`). With `Transport::smart(remote, false, …)`, git2 hands the
  advertisement stream on to the negotiation (git2 0.21.0 `src/transport.rs:254-279`).
- libgit2 keeps an HTTP connection alive only within one remote connection
  (`transports/httpclient.c:1078`).
- Credentials (`src/git/gitbackend/transport_support.rs`): core reads an explicit
  identity file from its own filesystem, with no agent fallback (`:183`).
  Otherwise the agent is offered once (`:203-216`). HTTPS uses Git credential
  helpers under `CredentialHelperPolicy::AllowConfigured` (`:220-226`; the default,
  `backend.rs:44`). On the operator's machine the github.com helper is
  `gh auth git-credential`, so `gh auth` reaches gwz only through Git's helper
  configuration.
- Host keys: gwz sets no certificate callback, so libgit2 refuses keys that are not
  in `known_hosts` (`ssh_libssh2.c:752-767`).
- Timeouts are process-wide libgit2 socket settings (`transport_support.rs:56-64`).
- Remote work runs up to `max_connections_per_host` at a time, default 8
  (`src/operation/resolve_per_host.rs:2`; used by `push_member.rs:275` and
  `pull_head_member_preflight.rs:563-586`).
- Protocol (`protocol/gwz.taut.py`):
  - `RemoteSshIdentity.private_key_path` (`:1034-1036`).
  - `OperationAttribution.credential_ref`, a "driver-local credential handle; never
    a secret value". It is carried through but never used to authenticate (`:994`).
  - `TransportCapabilitiesResponse` (`:1056-1060`).
  - `TransportObservation`, with credential method, offered and authenticated
    flags, and key fingerprint (`:1064-1072`).

### 3.3 Existing requirements and rulings

- Core MUST NOT own credential storage (`GWZRequirements.md`, Non-Goals).
  Credential acquisition is delegated to caller policy, host configuration or
  adapter APIs (REQ-124).
- Explicit SSH identity MUST fail closed, without a Git CLI fallback (debt recovery
  requirements, 2026-09-06). The owner ruled out a Git CLI transport on 2026-09-06
  (gwz-dev `dev-docs/GwzRemoteAuthProposal.md` §6).
- Transport observations MUST distinguish offered from authenticated credentials.
- Core MUST be callable in-process and MUST NOT require a daemon (REQ-011,
  Non-Goals).
- Policy that varies by driver is a typed input (REQ-012).
- v0 may assume local caller authority; remote capability enforcement is deferred
  (REQ-013).
- The push plan (gwz-dev `dev-docs/GwzUrlSchemePushPlan.md`, phase 3) reduces how
  many remotes an operation contacts. This work reduces what each contact costs.

## 4. Initial scope

| Deployment | Transport endpoint |
|---|---|
| Ordinary local caller | In the core process; default |
| Unattended core | In the core process, using its local account configuration |
| Core using the driver's credentials or network access | At gwz-cli, via the driver–core message channel |
| Future gryth/iroh peer | Same API extension point; networking and endpoint management deferred |

Core remains usable without gwz-cli or a daemon. Existing driver integrations
continue to use local placement unless they negotiate and explicitly select a
remote endpoint. Selecting placement does not forward signing operations or
credentials between endpoints.

## 5. Requirements

Conventions follow [GWZRequirements.md](GWZRequirements.md).

### 5.1 General

- **G1.** Preserve current Git outcomes, SSH identity precedence and fail-closed
  behaviour, and error reporting. Intentional changes are endpoint-local path
  interpretation for remote placement, connection reuse across operations,
  and the `gh`-only HTTPS authentication restriction in G4, including refusal
  of HTTPS userinfo/query/fragment credential forms. HTTP/git retain local
  native behaviour; explicit nonlocal placement for them refuses before effects.
  SSH endpoint trust-file admission is another intentional bounded change:
  regular UTF-8, NUL-free input is limited to 4 MiB per store and 16 KiB per
  physical line (excluding its CR/LF terminator), parsed as complete lines.
  This can refuse a larger previously accepted file and accept a valid long
  line the pinned native 4,091-byte chunk reader rejected. It applies equally
  to local-core and driver-hosted endpoints. Size/encoding admission errors
  are InvalidRequest before credential access; native malformed content still
  refuses. See GwzRemoteTransportSshProductionSetup.md for differential gates.
  These supersede the original draft's blanket preservation of credential-helper
  behaviour and exact trust-file input compatibility; host/key/port matching
  and no-untrusted-host authentication remain unchanged.
- **G2.** Core MUST NOT own persistent credential storage. Explicit SSH identity
  MUST fail closed without an unrelated-key or Git CLI fallback. A local
  endpoint may use credentials transiently as the current native backend does.
- **G3.** Observations MUST identify the executing endpoint, new versus reused
  connections, and proven authentication information without exposing secrets.
  Offering a credential and authenticating with it remain distinct events.
- **G4.** Both SSH and HTTPS MUST support the endpoint model. HTTPS
  authentication MUST use `gh` at that endpoint; other authentication providers
  MUST NOT be used as fallback. HTTPS destinations MUST be validated into
  credential-free fields before Open/helper/network activity; userinfo, query
  and fragment forms MUST refuse with redacted errors, including on redirects.
  Anonymous HTTPS remains supported. Additional
  HTTPS pooling optimisation ranks below SSH reuse.
- **G5.** The contract MUST work on Windows, macOS and Linux. Capabilities that
  have not been qualified on a platform MUST NOT be advertised as supported.
- **G6.** New endpoint and stream features MUST be negotiated. Existing local
  requests remain wire-compatible; an explicitly requested unsupported feature
  MUST fail before network effects rather than being silently ignored. The
  G4 policy change is intentional, not a promise to retain all old helpers.
  A taut Bind/Bound acknowledgement MUST establish session-bound endpoint
  capabilities/limits before Open; disconnect invalidates that binding.
  Core service capabilities alone MUST NOT stand in for endpoint negotiation.

- **G7.** The GWZ transport MUST bind to its own remotes without changing the
  process-global transport registry or intercepting unrelated libgit2 callers.
  Required safe-binding support MUST be qualified before advertising endpoints.

### 5.2 Connection reuse

- **C1.** An endpoint MUST reuse eligible idle SSH connections across repositories,
  phases and successive operations while it remains alive. Reuse spans reads,
  clone/fetch, push and post-push reads.
- **C2.** Within one endpoint context, SSH pools MUST group by username, host and
  effective port; the generic transport distinguishes schemes. Repository names
  MUST NOT partition the pool. Explicit identity selection MUST be checked for
  compatibility before reuse: a connection authenticated using A MUST NOT satisfy
  a request explicitly requiring B. Ambient selection uses endpoint-local policy.
- **C3.** A stale connection MAY be replaced before an exchange has been sent.
  After transmission begins, a failure MUST surface without automatic replay of
  the Git exchange. Successful connection reuse MUST NOT be reported as a new
  credential offer. Existing connections are authenticated sessions, not a fresh
  host-trust or credential check on every lease.
- **C4.** Connections MUST be bounded per user/host, with the existing aggregate
  per-host limit retained. Opening reservations, idle, allocated and closing
  connections count against endpoint capacity. At capacity, allocation MUST
  wait in a bounded cancellable queue or return a typed refusal. One connection
  MUST serve at most one active virtual exchange in the first implementation.
- **C5.** Connection, I/O, allocation-wait and close deadlines MUST have explicit
  meanings. Idle-pool expiry MUST NOT terminate an allocated exchange. Configured
  network timeouts MUST also apply when connections are reused.
- **C6.** The pool MUST belong to the endpoint, not an operation. The default
  `connection_idle_timeout` MUST be 60 seconds, measured since return to idle.
  Idle connections MUST be actively reaped even when no new allocation occurs.
  Endpoint shutdown MUST release all connections; no daemon is required.
- **C7.** Tests MUST be able to count physical connections, channel opens, leases,
  reuse, idle expiry and discards independently of Git results.
- **C8.** HTTPS connection reuse SHOULD follow the same bounded lifecycle where
  the chosen HTTP implementation supports it. HTTP authentication remains
  request-scoped; a reusable TLS connection does not imply a reusable account.

### 5.3 Placement and credentials

- **P1.** The mux MUST route each open to the selected endpoint. Initial routes
  MUST support local core execution and execution at gwz-cli over a message
  channel. The stream MUST remain pinned to that endpoint until terminal.
  All admitted SSH URL spellings MUST select the same SSH adapter and canonical
  pool scheme. No unsupported selected scheme may fall through to a core socket.
- **P2.** Endpoint protection MUST follow the SSH/HTTPS client behaviour on the
  executing machine. Distributed lending limits, per-repository authority grants
  and endpoint permission management are deferred. Connections MAY survive an
  operation under C6. This replaces the original operation-only lending rule.
- **P3.** Local placement MUST remain the default. Placement MUST be a typed
  driver policy input, with no implicit routing based on discovered credentials.
- **P4.** Driver placement MUST execute both traffic and authentication there;
  core MUST NOT receive the driver's private keys, HTTPS tokens or a signing API.
- **P5.** Driver placement MUST support Git hosts reachable only from the driver.
- **P6.** Existing identity selection precedence MUST remain. The selected
  endpoint MUST resolve and validate winning identity paths against an explicit
  endpoint-local base. Core MUST NOT test driver paths against its filesystem.
  No portable identity naming system is required for this version.
- **P7.** The executing endpoint MUST verify SSH host keys against its local
  trust configuration and refuse unknown/mismatched keys as today. Observations
  MUST identify that endpoint as the trust decision owner.
- **P8.** `gh` MUST run at the selected endpoint. Its tokens MUST remain there and
  MUST NOT appear in data/control messages or observations.
- **P9.** Interactive requirements MUST be surfaced to the driver. Missing login
  state MUST fail actionably rather than silently starting a login workflow.
  Any supported user-interaction wait MUST be separate from network timeouts
  and MUST remain cancellable.

### 5.4 Bidirectional message streams

- **S1.** Public payloads MUST be taut-defined. The in-process and carried forms
  MUST have the same semantics. The protocol MUST use data messages containing
  variable-length bytes, not require a separate raw-byte side channel.
- **S2.** Each direction MUST preserve bytes and order. Slow consumers MUST cause
  bounded backpressure, not dropped data. There is no transparent reconnect or
  replay of an interrupted Git exchange.
- **S3.** The write adapter MUST batch partial writes, send full buffers promptly,
  and send a partial buffer when its batching timer expires. The timer starts
  with the first buffered byte and MUST NOT restart on each subsequent write.
  The initial evaluation value is 100 ms; the delay and buffer size are tunable.
- **S4.** Explicit flush MUST bypass batching delay. Graceful end-of-write MUST
  flush pending data before its terminal marker. Payload boundaries MUST NOT be
  interpreted as Git packet boundaries or as the caller's write boundaries.
- **S5.** The lifecycle MUST distinguish open, data exchange, end-of-write, graceful
  close, cancellation and failure. Graceful close returns only a healthy,
  cleaned-up connection; cancellation/drop MUST NOT masquerade as successful
  completion. Graceful API close MUST emit pending Data and EndWrite in order
  before wire Close, under a deadline starting at the API call. Unread reverse
  data is drained only for cleanup and its discard MUST be reported; Closed
  MUST NOT imply Git success. Cleanup MUST be bounded.
- **S6.** Carrier loss MUST fail its owned streams, cancel queued opens, wake
  blocked reads/writes and release leases. An explicit close message MUST NOT be
  the only way to detect teardown.
- **S7.** Flow-control and lifecycle processing MUST continue while data writes
  are blocked, so full buffers cannot prevent cancellation or credit updates.
- **S8.** A reusable crate SHOULD own message-stream lifecycle and pool mechanics,
  independent of GWZ workspace policy and of gwz-cli. Repository extraction is
  optional; it MUST NOT be a prerequisite for designing or testing the contract.

- **S9.** Framing MUST enforce finite encoded-size limits before reading or
  allocating a declared body. Decoding MUST enforce finite depth, metadata,
  collection and allocation budgets before descent/allocation, including unknown
  fields. Encoded plus decoded storage MUST count against carrier bounds;
  in-process delivery MUST apply equivalent admission limits.

## 6. Out of scope

- iroh integration, peer discovery, multi-hop routing and distributed endpoint
  permissions; the carrier/endpoint interfaces leave room for them.
- Forwarded signing, bearer-token transfer to core, and token minting services.
- A new identity namespace or reinterpretation of attribution `credential_ref`.
- A required daemon, durable stream replay, and resuming a failed push in flight.
- Concurrent channels sharing one SSH connection in the first implementation.
- New OpenSSH configuration compatibility such as host aliases or `ProxyJump`.
- Building a general driver–core RPC system. Binding this transport service to
  a bidirectional channel, including capability checks and teardown, IS in scope.

## 7. Decision record

The identifiers from the original draft are retained for traceability. Accepted
policy below comes from the 2026-09-19 discussion. Implementation choices and
qualification work are explicitly labelled; they are not measured results.

| Decision | Resolution |
|---|---|
| **D1 — lifetime** | Accepted: endpoint-owned pool, reusable across operations; 60-second idle expiry; process shutdown closes it. Replaces operation-only P2. |
| **D2 — concurrency** | Accepted: one active exchange per physical connection initially; pool parallelism, with cancellable waiting at capacity. |
| **D3 — SSH implementation** | Design proposal: `ssh2` over libssh2, preserving native Git transport. Qualify Windows agents, trust, cancellation and full-duplex pumping before finalising the dependency. No Git CLI fallback; per-remote binding prerequisite, no process-global registration. |
| **D4 — pool identity** | Accepted: endpoint-local username/host/effective-port grouping, scheme-separated; no repository component. Explicit identity compatibility is checked before reuse. |
| **D5 — HTTPS** | Accepted: the same endpoint/message model; auth only through `gh`. Design: an endpoint HTTP adapter; concrete HTTP library and parity qualification remain implementation work. Additional reuse optimisation is secondary. |
| **D6 — observations** | Accepted: additive negotiated reporting for endpoint, connection/stream identifiers and reuse, preserving offered versus authenticated semantics. |
| **D7 — placements** | Accepted: local core and driver endpoint (relay) for both transports. Forwarded authority is deferred. |
| **D8 — selection** | Accepted: explicit typed driver endpoint selection; local default; pin at open; unavailable selected endpoint refuses without fallback. |
| **D9 — trust** | Accepted: the executing endpoint's normal trust checks; no duplicated trust decision in core for driver execution. |
| **D10 — HTTPS authority** | Accepted: endpoint-local `gh`; no token forwarding or alternate authentication fallback. |
| **D11 — lending limits** | Deferred to endpoint management; no lending mechanism in this design. Endpoint-local client protections apply. |
| **D12 — identity references** | Accepted: preserve precedence and resolve paths at the executing endpoint. No portable key reference or attribution-to-authentication binding is introduced. |
| **D13 — link** | Accepted: taut bidirectional messages, bounded buffered data payloads, lifecycle and carrier-loss notification; Bind/Bound before Open, bounded pre-decode ingress, negotiated support, no silent fallback. |
| **D14 — peers** | Deferred: iroh/gryth may provide another carrier/endpoint later. No peer policy is implemented now. |
| **D15 — SSH configuration** | Retain existing native semantics for this version; OpenSSH aliases, `ProxyJump` and full config parsing are deferred. |

## 8. Acceptance and remaining work

The [design](GwzRemoteTransportDesign.md) owns the message inventory, state
transitions, integration points and acceptance matrix. Before implementation,
turn that matrix into a TDD-first implementation plan. Qualify SSH and HTTP
adapter choices; measure batching delay, payload/window sizes and capacity
settings. These are bounded implementation decisions, not a reopening of D7–D12.

No repository relocation, implementation, generated schema change or experiment
is part of this documentation revision. Future measurements must follow the
workspace `EVIDENCE.md`; the original prototype figures in §3 are historical.

# GWZ Remote Transport Implementation Plan

Status: **plan draft accepted after G46 re-review GO, 2026-09-19.
Implementation authorized; Phase 1/2 interfaces frozen, Phase 3 in progress.**
The operator requested this plan before any repository creation or implementation.
The accepted [design](GwzRemoteTransportDesign.md) and
[requirements](GwzRemoteTransportRequirements.md) control behavior. Their
draft-stage GO/GO at core `05842b38e55f109ed3663555680751811a72eb9b` does not
constitute review of this plan, a wire/API freeze, or implementation acceptance.
The [G46 re-review](GwzRemoteTransportPlanReview-G46-1.md) accepted plan SHA-256
`55120dd1af7b77818eb71fda609818b6c1bb2539a08ec9f025ab01f7d4899b99`, closing
all four P2 and three P3 findings with no new findings. This accepts the plan's
sequence and assignments only; it is a combined review, not a dual peer-blind
gate. The [remediation record](GwzRemoteTransportPlan-RemPlan.md) maps the
corrections. The operator subsequently authorized implementation and clarified that the
CLI–core communication layer is supplied elsewhere. The scope clarification in
design §4.1.1 supersedes the later custom-carrier proposal; no existing
communication interface is changed by this programme.

## 1. Outcome and scope

Deliver reusable, endpoint-owned SSH/HTTPS transport through one taut-defined
bidirectional message API. Core's mux selects an in-process endpoint or a
gwz-cli endpoint over the driver–core channel. The selected endpoint owns
credentials, trust and connection pooling; core retains Git and repository
semantics. Fetch Phase 3 is the first performance consumer, not the whole scope.

The first implementation task is to establish **the `gwz-transport` repository,
schema ownership and a working taut integration contract**. Creating an empty
repository alone does not complete that task. A minimal consumer must demonstrate
the shared generated types and bidirectional message handoff before production
transport work depends on them.

This plan adds no iroh transport, daemon, distributed endpoint management,
credential forwarding, resumable stream, or concurrent leases on one connection.
It preserves the design's ordinary endpoint trust, gh-only authenticated HTTPS,
60-second idle expiry and explicit placement without automatic fallback.

## 2. Ownership and dependency boundaries

| Location | Responsibility |
|---|---|
| New `gwz-transport` repository | Canonical taut transport schema, generated Rust types, reliable conversation runtime, generic pool mechanics and SSH/HTTPS adapters |
| `gwz-core` | Thin Git adapter, mux integration, operation context, placement validation, GWZ options/capabilities/observations and GWZ-specific generated protocol |
| `gwz-cli` | Install and host its endpoint, use the supplied communication interface, supply endpoint policy and expose configuration/diagnostics |
| `taut` / `taut-shape` implementations | Any necessary schema-composition or reliable delivery integration support; changes only where the initial proof establishes a missing capability |

Start with one transport repository. Within it, separate generic conversation
and pool code from Git service descriptors and protocol adapters by modules;
split crates only where dependency or feature isolation requires it. The
transport repository must build without gwz-core or gwz-cli. Both can depend on
it; core must never acquire a dependency on CLI.

The host installs one local endpoint and, when present, one driver binding at
runtime construction. Each endpoint instance owns its pool across operations.
`Git2Backend` clones, including those made by `operation_services()`, share the
endpoint handle; identity selections, errors and observations remain scoped to
the operation. Neither a process-global pool nor a fresh pool per backend clone
is permitted. Dropping one operation/backend clone does not shut down the shared
endpoint; runtime teardown owns endpoint shutdown. This is distinct from stream
handle cloning, which shares one stream and never acquires a second lease.

The transport package exports its canonical schema and generated Rust API.
Rust consumers reuse that API rather than independently generating a second
copy of the same transport types. Other language consumers can generate from
the exported schema. Core generates its own integration against the shared
types. Pin schema/package and generator versions, with regeneration drift checks.
The exact cross-package generator mechanism must be established in Phase 1.

The CLI–core communication layer is supplied elsewhere and its current message
interface remains unchanged. Transport messages use that interface; this project
owns their taut schema and behavior, not physical framing or carrier setup.
Prefer optional fields on existing taut requests/responses for generated
transport messages, with existing request ids for correlation and transport
stream ids only for the individual exchanges. Select exact fields/tags during
integration; preserve existing tags and service methods. No new CLI command or
core service surface is required. The package consumes asynchronous send/receive
with backpressure and closure notifications; the supplied message delivery must
progress during an active Git operation rather than wait for its final result.
Consume the supplied layer's bidirectional delivery, backpressure and closure
notifications. In-process delivery passes the same generated values. Codec
round-trip tests cover serialized payloads without inventing a transport header.
Do not reuse the one-way operation-event subscription as a substitute for the
supplied bidirectional message channel.

## 3. Delivery sequence

The Phase 1 schema/admission/message-handoff contract and Phase 2 stream/pool
runtime API are **accepted and frozen** after original Code and State reviewers
and the Surface reviewer all returned GO. The accepted implementation is
transport `28f5afb3938a2aa8af0e1e8d5b07779add6ab776`, core
`ace269896ad80aee923e2e8fd31e565c43de57ed`, taut
`733e8a78897a90f017f4726e4331aed95e8cb977`; workspace review inputs are
`9d0dc7ef5c616d64d52c296ea2fa34d83d21d73e`.
The [interface gate](GwzRemoteTransportPool-InterfaceGate.md) and workspace
`dev-docs/GwzRemoteTransportInterfaces-Checkpoint.md` record the exact scope,
89 owner tests, 18 isolated archive consumer tests, regeneration checks and
review closures. One merged correction closed one P2 and three P3 findings.
Phase 3 is **in progress**: the shared worker, supervised native agent/network
setup, selected-key admission and N3 backend/driver attachment are accepted as a
local candidate. [N3 acceptance](GwzRemoteTransportSshN3.md) records the exact tuple
and retained Code/State GO. Production qualification/activation remain outstanding;
see the current authority in
[CurrentProgramCheckpoint.md](../../dev-docs/CurrentProgramCheckpoint.md).
Phases 4–6 are not complete. The owner CI workflow is prepared locally;
remote execution, consumer CI activation and registry resolution remain
outstanding qualification/publication work. Local interface acceptance does
not claim those outcomes or a production SSH/HTTPS endpoint.
Write meaningful failing tests before
implementation; use deterministic fakes before network fixtures. A phase ends
with the named evidence and review, not simply with code present.

### Phase 1 — Repository, exported schema and message integration contract

1. Establish the repository location, package name, license compatibility,
   supported Rust/tool versions and dependency/release strategy. Add the new
   member using GWZ workspace tooling; never hand-edit managed configuration.
   Remote provisioning and package publication are separate execution actions.
2. Scaffold the independent package, canonical schema location, exported schema
   artifact, generated Rust module, regeneration command and CI drift check.
   Ordinary consumer builds must not need an adjacent workspace checkout or
   silently fetch an unpinned schema. Account for core's checked-in generation
   workflow in `protocol/regen.py` rather than assuming build.rs generates it.
3. Prove schema composition/external Rust type references with a small core
   consumer. If taut needs an extension, implement and qualify that dependency
   before layering the transport integration on top; do not hand-copy message
   definitions into core to work around it.
4. Exercise Bind/Bound/BindRejected and stream Open/Data/control/terminal
   messages through the supplied message interface. Use a fake endpoint and
   message-channel test doubles; no SSH, credentials or Git-host effects are
   needed. A real-process integration case may use an externally supplied
   communication layer, but does not require or authorize creating that layer.
5. Assign stable schema tags and define version negotiation, structured
   destinations, errors, identity fields, byte payloads and bounded ingress.
   Include both SSH and HTTPS scheme/auth/service descriptors in the Phase 1
   inventory so later adapters do not silently extend a frozen schema.
   Establish and qualify the hard ingress caps below before the wire freeze.
   Bind/Bound takes bounded minima; Open/Opened may narrow negotiated limits,
   never raise them or the local hard caps.

Use the design §4.2 starting caps: bootstrap encoded frame 64 KiB, nesting 16,
256 total collection entries, 16 KiB per string/metadata byte field and 256 KiB
decode allocation; stream encoded frame 128 KiB, Data payload 64 KiB, the same
depth/collection/metadata bounds and 512 KiB decode allocation per frame.
Phase 1 proofs must also select and record finite aggregate byte/frame budgets
with reserved control capacity. Any pre-freeze change to the design's declared
caps must be documented and reviewed. No phase may enable unbounded ingress.

The freeze objects and remaining discretion are explicit:

| Freeze object | Gate | What may still change without changing the frozen contract |
|---|---|---|
| Taut schema/tags and complete Bind/Bound/Open/data/control/terminal inventory; generated consumer types; message handoff to the supplied communication interface; hard ingress caps and negotiation rules | Phase 1 interface gate | Negotiated/session/stream limits may only narrow; no changed type, tag, meaning or raised hard cap. Any revision of a frozen hard cap requires a requirements/design amendment and re-review. |
| Pool/runtime traits and ownership API | Phase 2 interface gate, before dependent adapter integration | Construction policy within the frozen contract and applicable caps; no changed ownership or lifecycle semantics |
| SSH/HTTPS adapter APIs | Phase 3 / Phase 5 respective integration gates | Qualified implementation choices that preserve accepted behavior |
| Optional transport-message fields, request correlation, and additive placement/capability metadata on existing GWZ messages | Phase 4 compatibility/interface gate; Surface review of any changed public fields/options | Existing methods remain unchanged; omitted fields preserve local behavior |

Pool/runtime traits, adapter APIs and GWZ placement fields are excluded from
the Phase 1 freeze. Its fake endpoint proves the message contract without
claiming those later APIs are settled.

**Exit evidence:** exported package builds independently; the core consumer uses
the shared types; regeneration is reproducible; golden/round-trip fixtures cover
binary data and unknown fields; local binding/data/terminal exchanges and
serialized payload round trips pass; payload admission and decoding stay bounded;
unsupported versions produce no endpoint effects. Record assumptions on the
supplied layer's pre-allocation bounds, delivery and closure notifications.
Testing or altering that layer's physical framing is outside this package.

**Gate:** freeze exactly the Phase 1 objects in the table after these proofs
and interface review. This phase supplies the first runnable protocol skeleton,
not an advertised network transport. If the supplied layer cannot meet the
message contract, report the missing integration capability; do not implement
or change the communication layer as a workaround.

### Phase 2 — Reliable conversation runtime and deterministic pool

Implement the full state machine, byte/message adaptation, first-byte batching
timer, immediate full-buffer/flush behavior, directional credit and bounded
queues. Implement ordered EndWrite/Close, bounded reverse draining, cancellation,
final-owner drop and carrier-loss cleanup. Bind endpoint/session identities and
refuse stale messages. Stream handle clones retain one lease; backend clones
retain the shared endpoint/pool with separate operation state, as assigned in §2.

Implement exclusive connection leases, bounded cancellable waiters, opening
reservations, compatibility checks, fair allocation and idle reaping. Pool
grouping includes endpoint context, scheme, username, host and effective port;
repository is not a partition. Different explicit identities cannot reuse an
incompatible authenticated connection. Keep the two capacity domains separate:

- Endpoint construction policy starts at eight physical connections per
  user/host and eight aggregate per host across users, ports and schemes.
  Opening, idle, allocated and closing connections all count. These ceilings
  bound combined physical use across operations; requests cannot raise them.
- Existing `OperationPolicy.max_connections_per_host` continues to bound that
  operation's fan-out through `par_map_per_host`. Its default eight is a separate
  concurrent-work limit. A lower operation limit neither resizes the endpoint
  pool nor evicts another operation's connections.

Qualify these bounds together and use the agreed 60-second idle timeout.
Assign all seven timeout domains from design §10: allocation wait, connect/auth
network, active I/O, write coalescing, supported user interaction, close cleanup
and pool idle. Phase 2 owns their clock/cancellation semantics; Phases 3 and 5
bind adapter waits to the appropriate domain. Deliberate backpressure and helper
interaction must not masquerade as network stalls or idle expiry.

**Exit evidence:** deterministic clocks and fake endpoints cover every design
§11 adaptation, batching, flow-control, lifecycle, carrier-loss, pool and shutdown
case. Include exhausted credit during close, tiny-message storms, simultaneous
cancel/close, checkout/expiry races and no resource-count leaks. Include
"backend clone is not a new pool": shared endpoint identity survives successive
operation contexts and dropping one backend clone. Overlapping operations with
different per-host policy limits respect both physical ceilings; lowering one
operation's fan-out does not evict the other's connections. The batching timer
must emit partial data even when the Git adapter never calls flush. Run the same
message-contract suite through typed and serialized payload handoff (without
implementing physical delivery), then review the runtime and pool API before
dependent integrations. Pool-only transitions are host-local and keep their
deterministic resource-ledger tests; they are not serialized messages.

### Phase 3 — Safe native integration and local SSH

Qualify or extend the safe git2 binding for per-remote transport callbacks with
owned context. No process-global transport registration, synthetic URL scheme,
private-layout cast or thread-local lookup is permitted. Prove unrelated normal
and custom libgit2 transports coexist before enabling the integration.

Implement the endpoint SSH adapter and libgit2 read/write bridge. Preserve
stateful discovery/negotiation, endpoint-local identity and known_hosts behavior,
timeouts and authentication observations. Qualify ssh2/libssh2 against the
required trust/agent/key behavior on Windows, macOS and Linux; do not infer parity
from a successful connection on one developer machine.

Route every GWZ SSH network entry through the per-remote callback before
advertising local SSH endpoint support. The following is a required coverage
ledger, not a claim of current implementation; record the exact call sites and
fixture result for each row during Phase 3. N3 results below are from the isolated
full-core candidate; they do not activate or qualify a production distribution.
Tests and reproduction are linked from [N3](GwzRemoteTransportSshN3.md).

| Entry | Required Phase 3 disposition | Current implementation evidence |
|---|---|---|
| Workspace bootstrap / init-from-sources clone | New endpoint adapter for SSH | N3 driver gate: init_from_sources + workspace/member clone |
| Ordinary clone and advertisement/ref reads | New endpoint adapter for SSH | N3 backend gate: clone + remote_refs + read_remote_file |
| Materialize network work | New endpoint adapter for SSH | N3 driver gate: nested snapshot materialization, private refusal/observation cleanup |
| Fetch, including all phases | New endpoint adapter for SSH | N3 backend fetch funnel and command-driver gate |
| Tag-related network work | New endpoint adapter for SSH | N3 backend tag_fetch + tag driver |
| Pull, including preflight and verification | New endpoint adapter for SSH | N3 pull_head and pull_snapshot driver gate |
| Push and post-push verification | New endpoint adapter for SSH | N3 backend push/rejection/pushurl + push driver |
| File/local-family operations | Existing credential-free local path | N3 local clone with stopped SSH endpoint; native local-family paths retained |
| Native HTTP/git compatibility operations | Existing local-only transport | Dispatch unchanged; full native compatibility fixture remains qualification work |

Any still-native SSH entry keeps the SSH advertisement gate closed. Partial
measurements may name their covered entries but cannot claim general SSH support.
Repeat the coverage ledger for CLI placement in Phase 4 and HTTPS in Phase 5;
unsupported explicit routes must refuse before effects.

**Exit evidence:** controlled fixtures prove discovery, fetch/clone, push,
rejection, uncertain push and post-push reads; multiple repositories and repeated
operations reuse healthy connections. Exercise two successive
`operation_services()` lifetimes sharing one endpoint: the second reuses the
first's idle connection, and dropping the first clone leaves that endpoint live.
Every coverage-ledger entry has a fixture; the SSH entries use the new adapter.
All supported SSH spellings route correctly;
identity mismatch refuses; cancellation releases resources; no automatic replay
follows a possible remote effect. Validate callback lifetime, error isolation and
the network/I/O/user-interaction timeout separation defined in Phase 2.

**Milestone:** first measurable local SSH reuse. It remains partial programme
delivery; local SSH is advertised only after full SSH entry coverage and the
native binding/platform qualification above. CLI placement and HTTPS remain
unadvertised until their own gates pass.

### Phase 4 — CLI endpoint placement over the message channel

Concrete interface admission is in [the placement amendment](GwzRemoteTransportPlacementDesign.md)
(accepted after Consistency, Safety and Surface GO, with the operator's subsequent
batch-C scope clarification). A/B integration precedes in-process message embedding
qualification for both CLI/core and gwz-py/core. Wire plausibility is documented;
physical wire and separate-process proof are deferred outside this cycle.

Install the CLI-hosted endpoint and connect core's mux through the externally
supplied communication layer without changing its existing interface.
Keep both directions pumping during Git operations. Apply session binding,
endpoint-local path resolution and authority ownership; core must not look up
the remote endpoint's keys or open its Git-host connection.

Integrate transport messages through optional fields on existing taut messages,
using existing request ids. Carry typed placement and capability metadata
additively through existing request/options and capability responses; do not add
a transport RPC service or CLI command. Preserve tags and ordinary local requests. Explicit
placement on an old core, missing driver endpoint or unsupported scheme refuses
before socket/helper effects. Carry per-operation observations through success,
events and early errors without copying an earlier operation's observation row.
On SSH reuse, `credential_offered` is false for this attempt; proven authentication
facts come from the connection record with explicit reuse context. Preserve
nullable `authenticated` when proof is unavailable and private-member suppression.

Freeze the optional message attachments, correlation mapping, placement and
capability fields at the Phase 4 interface gate. Surface review applies to changed
public fields/options: names, default = local, explicit unsupported/unavailable
errors and relevant existing documentation. No new CLI surface is a deliverable.
Omission selects local deliberately; an explicit
driver request never becomes omission on failure. Driver runtime construction
installs the binding and process exit/runtime teardown drops it. A separate host
command or core service is outside this plan. Phase 6 validates this accepted surface
and completes release documentation; it does not defer this surface gate.

**Exit evidence:** the SSH lifecycle/Git fixtures pass with separate CLI and core
processes and deliberately distinct credential/trust environments. Kill either
process during allocation, connect, I/O and close; verify cleanup, stale-session
refusal and no fallback. Exercise old/new driver/core capability combinations,
including embedded core and gwz-py ordinary local requests. Assert offered,
authenticated and reused facts separately on success and errors, no copied
observation rows, and private-member suppression. Repeat the Phase 3 SSH entry
ledger for carried placement before advertising that placement.

### Phase 5 — HTTPS adapter and authentication policy

Detailed candidate admission: [HTTPS endpoint design](GwzRemoteTransportHttpsDesign.md),
currently DRAFT correction1 pending retained Consistency/Safety re-review and
Surface review of HTTPS observation semantics. It specifies two
implementation batches (endpoint/RPC, then host/command integration), reusing the
existing transport protocol and generic pool. It activates no public route.

Implement anonymous and gh-authenticated smart HTTPS at the owning endpoint,
including streaming requests/responses, connection reuse, TLS, proxy and redirect
semantics. Refuse other credential helpers. Reject disallowed userinfo, query and
fragment forms before messages/helper/network effects and revalidate redirects;
only the exact generated discovery service query exception in HTTPS Design §5
is permitted. Apply its §4 bounded discovery authentication transition and §7
scheme-specific final repository-refusal predicate; never replay POST.
Credentials and Authorization headers remain endpoint-local. Missing gh login
fails actionably without initiating a login workflow. Any supported helper or
user-interaction wait is bounded, cancellable and separate from network timeouts.

**Exit evidence:** local and CLI placements pass multi-round fetch, large clone
and push, authentication failure, anonymous access, gh lookup failure, redirect
origin isolation, bounded close/drain and cancellation fixtures. Preserve native
local HTTP/git compatibility and reject their explicit nonlocal placement.
Sentinel credentials must not appear in transport messages or diagnostics.
Prove that EndWrite, including the request-write to response-read transition,
ends the HTTP body; neither Flush nor the batching timer may do so. Repeat the
network-entry ledger for HTTPS in both placements before advertising support.

### Phase 6 — Aggregate validation, tuning and rollout readiness

Run the complete design §11 matrix on the supported platforms. Validate process
isolation, bounded memory under sustained backpressure and connection reuse across
long-lived embeddings. Validate the Phase 4 configuration/help surface and finish
migration notes and the intentional gh-only HTTPS policy change. Do not silently relax that policy for
compatibility with an old endpoint or core.

Measure cold/warm SSH, physical connects and channels, fetch Phase 3, push plus
post-push reads, large-pack throughput, latency and memory for both placements.
Compare immediate emission with coalescing settings including 100 ms. Choose
construction defaults for coalescing, buffers/queues, wait/cleanup deadlines and
the distinct endpoint pool ceilings from results, within the frozen contract
and at or below every applicable frozen hard cap. Wire fields, types, tags and
hard ingress caps are not retuned in this phase. A changed hard cap requires a
requirements/design amendment and re-review. Runtime defaults cannot override
the agreed 60-second idle default or change per-operation fan-out semantics.
100 ms is an evaluation point, not a proven optimum.
The historical prototype measurements are context, not acceptance evidence.

**Exit evidence:** attributable results at exact revisions, complete acceptance
matrix, tuned construction defaults within frozen caps, compatibility and package
build checks, and aggregate review. The network-entry ledger is complete for SSH
and HTTPS in both placements; no advertised route silently remains native.
Recheck observation attribution, offered/authenticated/reused distinctions and
private-member suppression. Record remaining unsupported capabilities explicitly;
any proposed reduction of the agreed scope requires an accepted amendment, not
a declaration that the programme is complete. Keep raw campaign data/runners in
the private evidence member under workspace EVIDENCE.md; public CI fixtures must remain independently accessible.

**Milestone:** ready for a separately authorized activation/release. Publishing,
tagging and deployment are not performed merely because this plan exists.

## 4. Dependencies and work that can proceed independently

The primary order is Phase 1 → Phase 2 → Phase 3 → Phase 4 → Phase 5 → Phase 6.
After the Phase 1 interface gate, safe git2 binding qualification can proceed
alongside the deterministic runtime work. After the pool/endpoint contract is
stable, HTTPS adapter work can proceed alongside CLI placement. Their integration
gates still require the preceding shared contract and all applicable tests.
Do not let independent work allocate conflicting schema tags or own competing
copies of the same runtime. This plan assigns work boundaries; it starts no agents.

## 5. Review, checkpoints and completion

Use the workspace review process at execution checkpoints. Interface freezes and
aggregate/activation gates require independent dual review; add Surface review
for public API freezes and specifically the Phase 4 placement/configuration
freeze before advertising it. Interior implementation checkpoints
behind accepted interfaces use the recorded alternating single-axis tier, with
escalation for blocking findings. Preserve original reviewers for focused
remediation re-verdicts, following the operator's direction; do not silently
restart a review with fresh agents. Record the applicable tier before dispatch.

At each gate record repository revisions, completed acceptance cases, remaining
capabilities, review verdicts and any remediation in the programme checkpoint.
The existing design's GO/GO cannot be reused as acceptance of new code, schema
tags or a changed architectural boundary. Update controlling requirements/design
before implementing any behavior that this work reveals must change.

Completion requires both placements, SSH and HTTPS, endpoint-local authority,
shared generated protocol ownership, bounded lifecycle/pooling, compatibility,
platform qualification and measured results. Pooling alone or the first faster
fetch does not complete the programme.

## 6. Immediate next action

N3 local candidate backend attachment and the [Phase4 placement interface](GwzRemoteTransportPlacementDesign.md)
are accepted; see the current workspace checkpoint and filed review verdicts.
[Placement batch A](GwzRemoteTransportPlacementA.md) is implemented and accepted
after retained Code/State/Surface GO: shared v2 schema, missing-field compatibility,
bounded mux/check/terminal lifecycle and in-memory tests. Surface documentation
findings are closed. [Placement batch B](GwzRemoteTransportPlacementB.md) is also
accepted after retained Code/State/Surface GO and one consolidated correction:
core host facade and shared SSH endpoint across all backend funnels, scoped metadata
enforcement, endpoint-local preflight, request cleanup ownership and the compiled
guide fixture. [Placement batch C](GwzRemoteTransportPlacementC.md) is accepted
after retained Code/State GO: transport attachments in existing operation messages
at the CLI typed/direct-handler boundary and through the actual gwz-py codec in
the same process, preserving ordinary dispatch and request IDs. Bidirectional
progress, backpressure, cancellation and logical closure pass. Full frontend
activation is not claimed. State P3-1 tracks cleanup-report/retirement assertions
before later activation; current evidence proves teardown return only.

Next: Phase5 HTTPS adapter/authentication design and interface review. The C
wire mapping is plausibility only; wire implementation/testing and iroh remain
outside this development cycle.

Keep platform and selected-source checks together in the operator-deferred batch;
production dependency/route activation and the remaining native compatibility
qualification are still open. Physical SSH and host dispatch remain outside
gwz-transport. Publication is separate.

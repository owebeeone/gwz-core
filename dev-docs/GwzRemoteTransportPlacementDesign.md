# GWZ transport — endpoint placement integration

Status: **DRAFT for Consistency, Safety and Surface review.** 2026-09-22.
This admits Phase 4 implementation, not production activation or a new carrier.

## 1. Scope, evidence and controlling documents

This amendment implements the boundary in [RemoteTransportDesign](GwzRemoteTransportDesign.md)
§§3, 4.1.1 and 10 and [RemoteTransportPlan](GwzRemoteTransportPlan.md) Phase 4.
[SSH N3](GwzRemoteTransportSshN3.md) is the accepted local candidate baseline.
The workspace checkpoint and AgentProcessRules, as amended by
GwzProcessOptimization, govern execution and review.

The CLI currently constructs Git2Backend and directly invokes core handlers
(`gwz-cli/src/globalargs/dispatch.rs`). The taut service schema describes
messages, not a supplied bidirectional production connection. Neither events
nor a final operation response supplies bidirectional transport progress.
This design defines application attachments and embedding hooks only. It does
not assert that a production carrier already exists.

Two observed local shortcuts cannot cross that boundary: identity selection
currently resolves/checks paths in core (`transport_support/identity.rs`), and
N3 repository-refusal classification uses a stream-local shared flag
(`endpoint/stream_io.rs`). Both become endpoint-owned, message-visible decisions.
Core still owns repository discovery, selection precedence, Git negotiation,
workspace mutation, retry policy and public result projection.

No new command, service method, physical framing, socket between CLI/core,
connection address, reconnect/replay scheme, or credential-management feature
is authorized. HTTPS implementation is Phase 5. Platform and selected-source
qualification remain the operator-deferred single batch. All production routes
and dependencies remain behind their existing activation gates.

## 2. Placement and runtime ownership

The host creates a transport runtime once per backend family. It installs the
core-local endpoint and optionally one CLI endpoint connection supplied by the
host. Clones/scoped backends share the runtime and its endpoint pools; each
operation has its own immutable request context and observations. Installation
is not an operation request and performs no Git-host network or credential I/O.
The existing default backend constructor remains valid and selects local.

Expose the shared SSH host implementation through a core library facade usable
by CLI embeddings; core must not import gwz-cli. Reuse the accepted supervisor,
selected-key admission, worker and pool rather than duplicate them. Physical
adapters remain outside gwz-transport. Extracting another repository is not a
prerequisite. The facade accepts host delivery callbacks and returns an owned
runtime handle with explicit bounded shutdown; last-owner drop initiates the
same cleanup without requiring an async destructor. The concrete proposed Rust interface and lifecycle example in the
[embedding guide](../docs/TransportPlacement.md#proposed-rust-interface) are part
of this freeze. The application port supplies next_message/deliver/disconnect
hooks; it does not implement an outer carrier.

Placement is an operation-wide choice: `local` or `cli`. Omission means local.
A request cannot supply an arbitrary endpoint address or choose another client's
binding. The runtime's authenticated host context chooses the installed binding;
endpoint_id and session_id are validated identifiers, not authority tokens.
Separate clients have separate runtime/binding namespaces. Explicit cli requires
core support, a verified binding and supported routes before any operation
mutation, key/helper access or Git-host network effect. Failure never falls
back. Local filesystem/family routes, http:// and git:// are local-only; Phase 4
cli admits only SCP-like SSH, ssh://, git+ssh:// and ssh+git://. Mixed planned
routes containing an unsupported destination fail whole-operation preflight.

Shutdown/disconnect invalidates that binding and all its checks/streams, wakes
waiters, and contains endpoint work under accepted bounded supervisor cleanup.
A replacement uses a new session id and fresh endpoint instance; it cannot
inherit leases, messages or in-progress Git exchanges. Healthy pooling survives
operations within a runtime, not its teardown. Explicit later requests can use
a newly bound endpoint, but an interrupted operation is never replayed.

## 3. Additive GWZ schema surface

The following tags are reserved by this amendment; existing tags/types/service
methods retain their meaning. All new fields below are optional, with missing
and null equivalent. A present malformed value is an error, never omission.
These are proposed fields until implementation and activation gates pass.

| Owner message | New field/tag | Type and omission meaning |
|---|---|---|
| TransportOptions | placement / 4 | TransportPlacement enum: local=1, cli=2; absent=local |
| TransportOptions | endpoint_path_base / 5 | string; absent=no supplied CLI-relative-path context |
| RequestMeta | transport_message / 10 | shared gwz-transport Envelope; absent=no attachment |
| ResponseMeta | transport_message / 9 | same shared Envelope; absent=no attachment |
| TransportCapabilitiesResponse | message_versions / 3 | list of integer conversation versions; absent=none |
| TransportCapabilitiesResponse | placements / 4 | list of TransportPlacement; absent=no message placement support |
| TransportCapabilitiesResponse | schemes / 5 | list of shared Scheme; absent=none |
| TransportCapabilitiesResponse | auth_policies / 6 | list of shared AuthPolicy; absent=none |
| TransportCapabilitiesResponse | message_limits / 7 | shared Limits; absent=no message placement support |
| TransportObservation | endpoint_id / 9 | string; absent=unknown |
| TransportObservation | connection_id / 10 | string; absent=no established physical connection |
| TransportObservation | stream_id / 11 | integer; absent=no stream |
| TransportObservation | reused / 12 | bool; absent=unknown, never inferred false |

Capabilities describe the core implementation's potential support, not readiness
of a peer endpoint. Only intersection with a successful Bind/Bound authorizes a
route. Advertise only actually implemented schemes/policies: Phase 4 is SSH,
ssh_ambient and ssh_explicit, not the broad default offer in binding.rs. Default
production builds do not advertise candidate placement. A registered candidate
test runtime can advertise it for qualification. Unavailable endpoint instances
do not become supported because a version number is present.

Import the owner schema/exported types using the existing transport_consumer
proof; do not copy a second Envelope definition into core. Update the candidate
regeneration path, its pinned tool dependency and external type mapping together.
Until dependency qualification/activation, generate the full candidate projection
in the isolated consumer/backend harness; normal production artifacts and Cargo
dependencies remain unchanged. Activation later selects those same pinned inputs
for the production regenerator, rather than creating another schema definition.
A generated optional field currently calls try_get and requires a map slot:
nullable is not proof of additive decoding. Before integration, qualify a
schema/generator mechanism that accepts missing *new* fields in Rust and Python,
without weakening required fields or hand-editing generated code. It must keep
retained old readers working and preserve old local request behavior. If the
pinned generator cannot express this, add that bounded generator support before
regeneration; do not silently change all historical field requirements.

The transport owner profile is independently versioned (§6). GWZ continues its
existing schema/service version. New drivers query capabilities before sending
cli placement. Missing fields or an old core imply unsupported; the driver
refuses locally instead of allowing an old decoder to ignore the placement.
The capability result belongs to one live, nonserialized host admission
generation identifying the exact receiving core instance and backend-family
runtime. The host must pin that receiver from capability query through operation
dispatch acceptance; checking a generation and then allowing an independent
routing choice is insufficient. Direct embedding uses the same TransportRuntime
object. A supplied remote host must provide equivalent connection/receiver
affinity and disable hidden rerouting/retry of admitted operations. Closure,
reconnection, failover or instance replacement invalidates that generation and
its cached capabilities before any new explicit-cli operation can be sent. Query
again on the replacement, and still refuse if it is old/unsupported. An already
sent operation interrupted by replacement fails; it is not replayed. Hosts that
cannot guarantee this affinity must report cli placement unavailable. This is a
host-adapter precondition, not a new wire field or a claim that old decoders
reject placement. Endpoint Bind/Bound cannot substitute for receiver affinity.

Old-driver/new-core local requests remain valid. Direct Rust struct-literal
source compatibility is not promised for additive generated fields: consumers
must use defaults/builders and be rebuilt together; wire compatibility is tested.

## 4. Message handoff and correlation

The host adapter supplies asynchronous, ordered bidirectional delivery of an
attachment and the existing request_id. The embedding callbacks are equivalent
to `send(request_id, Envelope)`, inbound delivery and session closure, with
bounded backpressure. These are Rust application hooks, not new RPC methods or
an independently serialized wrapper. RequestMeta/ResponseMeta fields are the
canonical attachment slots when the supplied layer carries existing messages.
The adapter projects attachment traffic into the mux independently of operation
handler dispatch. It must neither repeat an operation to send a frame, invent
partial operation results, nor hold delivery until the final response. A final
response alone is not an implementation of this contract. If a supplied host
cannot deliver attachments during execution, cli placement is unavailable there.
This programme does not invent its missing outer framing or service mechanism.

Admission of a command registers its unique live request_id and caller-owned
operation_id before binding or checks. The first request needing a binding
owns the Bind/Bound exchange (stream_id=0); concurrent requests wait on that
single bounded bootstrap. Serialize verified Bound installation against owner
cancellation/expiry/loss. If installation wins, the established binding survives
that owner's later cancellation; only its request is cancelled. If abandonment
wins, atomically retire the session, fail/wake every bootstrap waiter, close the
application port and require the host to propagate closure to the endpoint. The
endpoint retires its ready/pending binding on that closure under bounded cleanup;
no usable old binding remains and no late Bound can install it. While closure is
in flight, the mux sends no Open/check and the endpoint cannot grant authority to
another session from the old acknowledgement. Local waiters terminate without
waiting for proof of peer cleanup. Fresh bootstrap requires a new installed port,
fresh session id and endpoint instance; never resend Bind on an abandoned
session. Duplicate Bound after a successful installation is only idempotent when
it exactly matches the established acknowledgement; disagreement is a protocol
error. Shutdown retires pending and established generations under the same rule.
Later requests reuse only an established binding. The endpoint
validates each request context before accepting its checks/opens. The client host
registers its live request_id before command dispatch; the first valid Open/check
fixes operation_id, and later disagreement refuses. A frame cannot create an
unregistered request. The guide's ClientRequest owner defines registration and
retirement; its finish/drop seals new work and preserves bounded cleanup. Core's private
registry maps each positive stream_id to (request_id, operation_id, session_id,
endpoint_id); Open's operation_id must match. A positive identifier is unique
for all checks and streams in the session and is never reused; overflow retires
the session. Close, Cancel and facts must match the entire registered context.
No identifier authorizes dispatch outside its installed client binding.

A request stays registered through stream/check terminal cleanup, even after
its user-visible result. New work under a completed request is rejected; only
already-owned cleanup can finish. Retire records under the bounded cleanup
policy; unknown/retired/stale deliveries cannot create state, release leases or
attach facts to another operation. Session mismatches are discarded; malformed
messages for a live session fail that session closed. An idempotent duplicate
terminal acknowledgement cannot change an already terminal outcome.

Both delivery pumps run independently of blocking git2 reads/writes and endpoint
SSH work. Never wait while holding mux/pool/observation locks or call user hooks
under those locks. Bound host queues as well as transport queues, account for
attached-message decode/copy memory, and preserve separate control capacity.
Use negotiated transport bounds without raising frozen hard caps. The host
must bound its whole received message before allocation; a transport subfield
check after unbounded outer decode is insufficient. Dispatch transport/control
before blocking on operation work; a saturated data queue must not starve
Window, Cancel, Failed, terminal replies or shutdown. Per-session concurrency
and retained identifiers are bounded by host policy, with Capacity on admission.

## 5. Identity preflight at the owning endpoint

Core freezes the effective URLs and identity selection/source using the existing
precedence. For cli placement it treats key paths as opaque strings, applying
only bounded string/NUL/empty validation; it does not use core PathBuf rules,
HOME, metadata or OpenOptions on them. The endpoint performs native path parsing,
~/ expansion and regular-file/readability checks. `endpoint_path_base` is an
absolute path in that endpoint's filesystem. Relative paths require it. For cli,
this includes repository-local selected path strings; core's repository root
must never become their base. Absolute paths need no base; ~user remains refused.
InvocationContext.caller_cwd continues to describe the execution filesystem and
must not be repurposed. Local placement keeps existing path-base behavior;
endpoint_path_base is inapplicable there and any non-null supplied value is invalid.

Preflight includes the same invocation-selected files currently checked by
with_transport, plus each planned remote's effective selection, before the first
workspace mutation or Git-host connection. Preserve existing selection/unused
remote validation and dry-run policy. Do not weaken whole-operation preflight
by delaying all remote file checks to Open. Version 2 adds one bounded
CheckIdentity exchange per distinct selection (§6); check the full selected
set before executing the frozen plan. A last-target check failure leaves all
participants unchanged. No key bytes, resolved private paths or file diagnostics
return to core. The endpoint uses bounded supervised file-check admission with
an absolute deadline, cancellation and retained-work accounting; it cannot block
the delivery pump or create an unbounded thread per path.

Successful checks prove availability at that instant, not immutable credential
authority. They allocate no connection, authenticate nothing and retain no key
snapshot. Open must still run accepted N2 selected-key snapshot/admission before
every pool lookup. A changed/deleted file fails under normal operation semantics,
without agent fallback or claiming that preflight guaranteed later success.
Checks and Open must interpret the same raw selection and endpoint base.

## 6. Transport owner version 2 amendment

These changes belong to gwz-transport's taut schema, not a private core wire
extension. They replace the v1-only implementation restriction, not v1 meanings.
Bootstrap Envelope.version remains 1; Bind.versions offers supported versions.
Bound.version selects the highest common admitted version, and its bootstrap
envelope remains 1. All subsequent messages use that negotiated version. v1
readers receive only v1 messages; v2 placement requires v2 and cannot downgrade.
v1 local fixtures/profile remain supported. No common version rejects before
credentials or sockets. Unknown kinds/codes remain errors within a profile.

| Addition | Frozen shape |
|---|---|
| MessageKind | check_identity=16, identity_checked=17, identity_check_failed=18 |
| Envelope | check_identity / 25, identity_checked / 26, identity_check_failed / 27; optional matching bodies |
| CheckIdentity | endpoint_id / 1 string, operation_id / 2 string, identity / 3 Identity, timeout_ms / 4 integer |
| IdentityChecked | empty message; correlated success, no secrets or authority token |
| identity_check_failed body | Failure |
| Failure | facts / 3 optional Facts |
| ErrorCode | repository_refused=13 |

CheckIdentity is v2-only, positive unique stream_id, explicit_key identity only,
positive finite timeout_ms bounded by the endpoint's admission ceiling. It uses
normal negotiated metadata/queue/admission caps, consumes a bounded pending
check slot, never a pool lease. It is legal only under a ready binding and live
request. Success/failure makes that check terminal. Cancel cancels the check;
the endpoint returns identity_check_failed(Cancelled, None) when delivery is
still possible. Disconnect/expiry locally terminates waiters and contains any
late supervised completion; no terminal response can resurrect a retired check.
Check failures always have Effect::None and no authentication facts. The check
identity mode/base/path rules are identical to Open's explicit selection rules.

Failure.facts carries current-attempt evidence on OpenFailed/Failed, bounded by
existing metadata caps. Closed.facts is the sole authority for every Closed,
including failed Closed: Closed.failure.facts MUST be absent/null. A nested
Failure.facts value is rejected before dispatch even if equal to Closed.facts;
it never supplies a second authority. Bind/check failures also forbid facts.
The typed terminal adapter combines Closed.failure's code/effect with Closed.facts
when producing one internal failure receipt. The public observation and failure
projection use that same receipt, not independently chosen copies. Late facts cannot change the selected failure code: a
prior rejected key must not turn a later Timeout into Authentication. Reuse
reports credential_offered=false and independently proven authentication facts.

RepositoryRefused is endpoint-generated only for the N3 canonical refusal rule:
no stdout bytes, complete/untruncated recognized repository-refusal stderr and
completed command status. No raw stderr travels in Failure. Send Failed with
that code before exposing clean EOF; loss of the terminal report is CarrierLost
or Io, never inferred repository refusal. Malformed/truncated/unrecognized
messages stay generic failures. Authentication, trust, timeout, cancellation
and possible publication retain their own classifications. Effect is None only
when absence of Git request/publication effects is proved; otherwise Possible.
No failed exchange is automatically retried.

Provide a typed terminal-failure path through MessageEndpoint and the blocking
stream adapter, including retained facts. Do not encode classification as data
bytes, private magic error text received from peers, or a process-local flag.
Core may map a verified typed disposition to its existing internal git2 error
bridge; only that verified disposition can trigger private-member suppression.
Clear the corresponding observation using the existing private-member policy.
Closed with a failure uses the same outcome; conflicting terminal outcomes are
protocol errors, never a way to change an earlier result to success.

The owner codec must validate the negotiated profile, one matching body, limits,
version legality and context before dispatch. Bootstrap v1 projection omits the
new optional slots when talking to retained readers. New readers accept missing
new slots/facts; old v1 serialized fixtures and retained Rust/Python decoders
are part of the gate. Existing v1 kinds/fields/codes never change semantics.

## 7. Observations, errors and authority

Construct each observation from its current operation selection plus that
stream's verified Opened/terminal facts, using §6's single Closed facts authority. endpoint/connection/stream/reused are
optional when not yet known. Do not copy another operation's row, infer offered
from authenticated, or retain private-member details after suppression. Preserve
facts on ordinary results, early errors, events and nested operation drivers.
CLI rendering remains a projection of core results, not independent Git policy.

At the public GWZ boundary, missing/closed binding maps to IoError (28),
unsupported placement/scheme/version to UnsupportedOperation (14), malformed
placement/path context to InvalidRequest (1), and unavailable selected files to
PermissionDenied (27), before effects. RepositoryRefused maps to RemoteRejected
(22) under existing private-member policy. Other active Git exchange failures
retain the existing GitCommandFailed (23) projection and bounded diagnostic, with
typed transport cause/facts retained inside the adapter. No public numeric error
code is added. Diagnostics name endpoint placement and failure category without
key contents, resolved credential paths or raw remote stderr.
After possible remote publication, return the specific failure with uncertainty;
neither carrier loss nor pool cleanup proves a push did not happen.

## 8. Implementation batches and evidence

A. Shared schema + bounded mux/check/failure lifecycle + generated Rust/Python
compatibility. Include deterministic in-memory bidirectional tests, random chunk
and read-size replayable seeds, saturated queues/control progress, stale sessions,
request isolation, cancellation during checks and terminal-facts ordering.
Use existing transport/consumer harnesses, extending rather than replacing them.
Public schema/API changes take aggregate Code/State and Surface closure review.

B. Reuse SSH endpoint behind local and CLI host facades; integrate request context,
endpoint identity preflight and all N3 network funnels. One aggregate Code/State
review over this complete integration: clone/materialize, fetch, push/post-push,
pull preflight/verification, remote reads/management, nested drivers and private
members. Test two logical hosts with distinct fake filesystem/credential contexts,
no core credential or Git-host access for cli placement, same-key pool reuse,
last-target preflight failure, timeout vs prior rejection, canonical refusal,
and selected-file changes after check. Existing direct local callers stay valid.
No separate micro-review is required for each command funnel.

C. Qualify a real supplied host connection when available: separate processes,
distinct credential/trust environments, kills during allocation/connect/I/O/close,
bounded cleanup and no fallback. Absence of a supplied connection leaves this
exit evidence outstanding; it neither authorizes building one nor blocks A/B
in-memory implementation. Do not claim Phase 4 complete or advertise production
cli placement until this and applicable activation gates pass. Keep platform/
selected-source qualification in the later operator-requested single batch.

Test retained old/new core/driver and gwz-py ordinary-local combinations; new
explicit-cli/old-core must be rejected before sending the operation. Test missing,
null, malformed and unknown fields, negotiated v1/v2, unchanged old slot semantics,
bounded outer attachments, endpoint teardown, cleanup after user-visible return,
and explicit cli with unsupported/mixed routes. Add deterministic bootstrap
barriers immediately before/after endpoint readiness and mux installation, with
owner cancellation, multiple waiters, delayed/lost/duplicate Bound and shutdown.
Require bounded waiter release, no stale authority and recovery only through the
permitted established or fresh binding. Probe new-core capabilities, replace or
reroute the receiver to old core, then request cli: assert zero operation sends
and zero local/remote effects. Cover channel replacement, unsupported affinity
and retained local requests. Decode Closed with nested equal/conflicting facts
(both rejected) and absent/null nested facts (one authoritative Closed.facts).
Compile the guide example once the proposed API exists; design review traces it
against declared signatures only. No passing test claim is made
by this document. Production activation, HTTPS and final performance/platform
qualification remain separate later work.

## 9. Exact authority changes and acceptance

On acceptance, this amendment refines Design §§3/3.0/3.1, 4.1.1 and 10 and
Plan Phase 4 with exact attachment tags, endpoint checks, failure facts and v2
negotiation. It supersedes only v1-only implementation assumptions and the local
N3 refusal receipt as a sufficient cross-placement mechanism. It preserves v1
wire meanings, all hard caps, pooling, timeout, authority, gh-only HTTPS and
no-carrier rules. Local N3 remains valid evidence for its accepted local scope.
The owner schema header's historical draft label is not permission to bypass
this interface gate. The new fields/kinds must not be shipped under v1 semantics.

Required design gate: retained Consistency and Safety reviewers plus a Surface
review of the proposed [embedding guide](../docs/TransportPlacement.md), all on
one committed tuple. Two merged remediation rounds maximum, same reviewers.
Acceptance admits A/B implementation only; code, carrier and platform proofs are
not supplied by a design GO. A changed public field/tag, path authority, failure
meaning, cap, or host delivery assumption requires amendment before implementation.

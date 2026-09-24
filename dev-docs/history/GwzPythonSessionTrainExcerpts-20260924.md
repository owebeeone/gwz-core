# Retired Python session train text from gwz-core documents

Retired on 2026-09-24 with the Python concurrency design train (the concurrency NO-GO finding through the v4 foundation draft), when the operator called a clean-slate redesign of the client, core and transport boundary. Nothing here is normative. Each block is the verbatim text removed from the named gwz-core document; relative links inside it resolve from that document's original location.

# From GWZDesign.md

## Python shared transport session design (2026-09-24; implementation pending)

The accepted [Python session v2 design](../../dev-docs/GwzPyTransportSessionV2Design.md) governs the native Python Client's shared endpoint. One serialized capacity-installation leader publishes paired SSH/HTTPS pools and reservation authority atomically. A request with the same installed four-field capacity joins; a request with different capacity cannot replace pools until all operations, leases and cleanup retire. Every accepted operation receives its own core registration, cancellation generation, worker slot and bounded ledger reservation before its worker gate opens. Python issues the public handle synchronously, validates the caller request ID against core's existing identifier grammar, and refuses unsupported custom-bridge session hooks before issuance. One session-owned ledger holds results, events and cancellation evidence across native-host shutdown, with 64 records and a 64 MiB total charge. An unstarted record reserves 4 KiB; admission atomically upgrades it to 8 MiB total, including a primary event reader and 4 KiB for required discovery and possible close summaries. Close transfers its pre-reserved summary charge into retained Client reporting. At most eight accepted operations therefore fit within the ledger without post-effect recovery allocation. Runtime rollover after 256 lifetime IDs requires quiescence and new Port identity from captured endpoint configuration; cancellation remains bound to public operation ID and original generation. The GWZ Taut error enum gains `cancelled=73` and `transport_record_limit=74`, while the request/response method set and transport Envelope remain unchanged. Native Python exposes synchronous `start_*` factories, explicit `accepted()`, retained event/result/cancel/release methods, `OperationStream` with `aclose()`, `recent_operations()` and post-close lookup. This is a design contract, not a claim of implemented behavior or release readiness.

## Python shared transport session v4 foundation (2026-09-24; DRAFT, design review pending)

The draft [Python session v4 foundation design](../../dev-docs/GwzPyTransportSessionV4FoundationDesign.md) replaces the withdrawn v3 draft and proposes a **local-only, Python-session-owned** candidate core admission API beneath the accepted v2 contract; it is not authority until its Consistency/Safety review reports GO. `TransportRuntime::from_environment_for_session()` builds a runtime whose endpoint and driver sessions allow one registration beyond the 256 caller registrations, reserved for bootstrap; legacy constructors keep exactly 256 and refuse bootstrap with `UnsupportedOperation`. Bootstrap is a caller-driven step at generation construction, in two parts. `bootstrap_ready()` registers the reserved request on both sessions, sends the Bind offer and awaits Ready; its ID is `bootstrap-` followed by a 128-bit random token and is never reported. It returns a `BootstrapLease` whose `finish()` cancels, seals and finishes that registration. Dropping `bootstrap_ready()` before Ready disconnects the pending bootstrap and leaves the runtime unusable. `generation_open()` reports synchronously whether both sessions are open and the driver mux is Ready. Phase 1, `admit_local(meta, operation_id) -> Result<Admission, AdmitRefusal>`, refuses CLI placement before mutation, performs validation and local endpoint capacity installation under the existing shared local admission leader and arrival deadline, then read-only endpoint/driver ID and mux pre-checks and a driver-mux Ready check. It registers nothing and is cancellable by dropping its future; the existing `CapacityMutation` guard closes a generation dropped after pool mutation. `AdmitRefusal` distinguishes `Refused`, `AlreadyRegistered` (a duplicate in this generation) and `GenerationClosed` (a closed endpoint session, a driver mux that is not Ready, or a capacity installation that closed the generation). Phase 2, `Admission::register_and_open() -> Admitted`, is **synchronous**: endpoint registration, driver registration, `begin()` on the ready mux and backend attachment, with no await, timer or watchdog. `Admitted::Refused` proves no insertion; `Consumed(error, RegisteredCleanup)` means at least one insertion happened and the caller awaits `RegisteredCleanup::finish()` under its own clock; `Ready(TransportRequest)` means registered and opened. A `ProgressHandle` (`NotEntered`, `MayHaveRegistered`, `Registered`) obtained before phase 2 is written in program order on the caller's thread and read only if `register_and_open` unwinds. No error code or text determines provenance. Core keeps its existing supervisor-driven deadlines, the mux bootstrap and route deadlines and the 5-second cleanup-expiry close; they fire before any caller clock and reach callers as errors. Core gains no new timer, callback or thread with authority over the caller's operation records, and every core future the native owner awaits is bounded by that owner's own clock. The placement supervisor wraps `drive()` in `catch_unwind`; on an unwind, or on loop exit while its session is open, it closes the session, completes the result of every sealed registration conservatively and signals its waiters. The existing `TransportRuntime::request()` keeps its implementation, CLI branch, first-request bootstrap, droppable future and error/cleanup timing; the only change legacy callers can observe is that their waiters wake after a supervisor exit instead of hanging. `TransportRequest::generation()` pins native cancellation to the driver generation. The v2 physical capacity rules are unchanged.

# From GWZRequirements.md

## Python shared transport session amendment (2026-09-24; implementation pending)

The accepted [Python session v2 contract](../../dev-docs/GwzPyTransportSessionV2Design.md) extends the remote-transport programme for the native Python Client only. A Client MAY admit up to eight concurrent network operations on one local endpoint host when their resolved four-field physical pool capacities agree. Equal capacity MUST join without reinstalling pools, including under a held lease; differing capacity MUST refuse before credentials or Git effects while an operation, non-idle lease or cleanup remains. Admission MUST reserve bounded worker, event-reader, result and recovery-metadata capacity before work starts. It MUST expose a public operation ID before admission, keep each operation's result and events isolated by Client, and retain possible-effect outcomes through close and early stream abandonment. Caller request IDs MUST obey core's existing nonempty, at-most-128-byte, no-control-character grammar. Old cancellation authority MUST NOT target a request in a later runtime generation. At the 256-lifetime-registration limit, a quiescent native session MAY replace its runtime using captured endpoint configuration after tickets and physical work have retired; otherwise it MUST refuse with a typed pre-effect outcome. Append-only GWZ Taut error codes `cancelled=73` and `transport_record_limit=74` represent admitted cancellation and own-record overflow; they do not change the transport Envelope. Custom Python bridges preserve their existing `close() -> TransportCleanup | None` behavior. This amendment accepts requirements, not implementation, platform proof or activation.

## Python shared transport session v4 foundation amendment (2026-09-24; DRAFT, design review pending)

The draft [Python session v4 foundation design](../../dev-docs/GwzPyTransportSessionV4FoundationDesign.md) replaces the withdrawn v3 draft and proposes **local-only, Python-session-owned** candidate admission requirements beneath the accepted v2 contract; they are not baseline until its Consistency/Safety review reports GO. A runtime built for the Python session MUST reserve exactly one registration beyond the 256 caller registrations for bootstrap; legacy runtimes MUST keep 256 and MUST refuse bootstrap. Bootstrap MUST be caller-driven in two steps, readiness and then finish of the reserved registration. The reserved registration ID MUST carry a random token and MUST NOT be reported to callers. Dropping bootstrap before Ready MUST leave the runtime unusable rather than partially bound. Admission MUST have two phases separated exactly at request-ID registration. `admit_local` MUST refuse CLI placement before mutation, validate local placement and capacity, and pre-check endpoint and driver ID/mux limits and driver-mux readiness under the shared local admission leader. It MUST register nothing, remain cancellable by dropping its future, and fail the generation closed if dropped after pool mutation. Its refusals MUST distinguish, by type, a duplicate request ID and a closed generation from all other refusals. `register_and_open` MUST be synchronous, with no await, timer or cancellation point, and MUST return `Refused` (proved no insertion), `Consumed` with a cleanup handle the caller finishes (one or more insertions, then failure; no Git work ran), or `Ready`. A caller-held progress handle MUST be written in program order before and after each mux registration, so an unwind inside registration is classified `MayHaveRegistered` or `Registered`. Such an unwind MUST close the generation and forbid same-generation reuse, and MUST NOT be reported as a proved no-insert refusal. Core MUST NOT add any new timer, callback or thread with authority over the caller's operation records. The existing supervisor-driven mux bootstrap and route deadlines and the cleanup-expiry close remain; they MAY close a generation and MUST reach callers as errors, observable through a synchronous generation-state query. Every core wait awaited by the Python-session candidate path MUST be bounded by its caller's own clock. The placement supervisor MUST close its session, complete sealed registration results conservatively and signal its waiters if `drive()` unwinds or its loop exits while the session is open. Callers MUST NOT derive provenance from `ModelError.code` or message text. The existing `TransportRuntime::request()` path, including CLI placement, first-request bootstrap, future-drop behavior and error/cleanup timing, MUST remain unchanged, except that its waiters wake after a supervisor exit instead of hanging. The v2 capacity, generation and identifier requirements are unchanged.

# From GwzRemoteTransportPlan.md

Candidate Phase 2 exit-row correction (2026-09-24, pending Python concurrency
design review): the [Python session v2 design](../../dev-docs/GwzPyTransportSessionV2Design.md)
replaces the unconditional different-per-host-limit overlap assertion above.
Different **per-operation fan-out** may overlap when the resolved four-field
physical capacity is identical; combined leases respect that capacity and
lower fan-out does not evict a peer's connection. Different physical capacities
do not overlap: the second operation is refused before endpoint effects and
may install its limits only after the first operation and cleanup retire. The
old Phase 2 row remains historical until review GO updates core authority.

# From GwzV110Plan.md

Candidate S6.3 clarification (2026-09-24, pending Python concurrency design
review): the [Python session v2 design](../../dev-docs/GwzPyTransportSessionV2Design.md)
requires two genuinely overlapping Python operations on **one** Client and
pool, with independent results and cancellation. The current Python 1.1
session has only a local endpoint; explicit CLI placement must fail with a
typed error before credential access until a separate CLI capacity-owner
handshake is designed. This candidate does not close S6.3 or lift the Phase 6/7
NO-GO by itself.

# From GwzRemoteTransportRetryPlan.md

## Candidate Python concurrency amendment (2026-09-24; pending review)

The [Python session v2 design](../../dev-docs/GwzPyTransportSessionV2Design.md) proposes a replacement for **both** §3 item 7/§6/S1.4's blanket non-idle refusal **and** their no-non-idle-lease installation condition, for a shared Python endpoint generation only. An operation whose **resolved four-field physical capacity exactly equals the installed capacity** may join without reinstalling or resizing the pool, even while another operation has a non-idle lease. Installing **different** capacity requires zero live operations, zero non-idle leases and completed physical cleanup; a differing-capacity request while any of these remains is refused before Git and credential effects, including while a live operation is between leases. Every initial and later installation is serialized and published atomically with its shared reservation authority. The single-operation candidate CLI rule remains as written. The previous Python concurrency draft was rejected; this candidate text is not implementation authority until the new design receives review GO and the core requirements/design are updated.

# From docs/TransportPlacement.md (lines added by e06c752d and 045eb277; the accepted text from 2954e0b8 was restored)

The accepted candidate has optional `RequestMeta.transport_message` and
`ResponseMeta.transport_message` fields, correlated with the existing request_id.
They remain compatible attachment slots and must never invoke the operation
again or invent partial/final results. They are not a sufficient delivery
schedule when application requests/responses are idle.

The proposed [independent-delivery amendment](../dev-docs/GwzIndependentTransportDeliveryAmendment.md)
changes the candidate host schedule: a generated `GwzTransportDeliveryV1`
event carries `(registered transport request_id, Envelope)` even when no
application request or response is moving. The optional metadata fields above
remain additive compatibility slots; they cannot supply the pump's progress.
For a ticketed Python route the Client-owned event-loop host runs separate
bidirectional urgent-control and per-stream ordered delivery. Bulk backpressure
must not stop eligible Window/Cancel/Failed delivery; opening transitions
remain ordered before their dependent controls, and a delivery stall fails the
binding after the amendment's deadline. Core and endpoint register request
context during charged commit admission; the Python host forwards only. The
accepted placement gate remains authoritative until that amendment receives GO.
This interface remains a candidate. Under the proposed amendment, the next
integration gate proves bidirectional `GwzTransportDeliveryV1` events through
CLI/core and gwz-py/core bindings in the same process, including while
application dispatch is blocked or idle. Optional metadata attachments have
separate old-reader compatibility fixtures; they do not prove delivery progress.
For the **proposed** independent-delivery host, this one-loop forwarding example
is replaced by the amendment's ready-selective urgent and per-key ordered
dispatchers with stream-opening barriers and bounded admission deadlines. The
current Rust port exposes only the original FIFO method, so the new host must
not be advertised until the selective API and its saturation tests exist.

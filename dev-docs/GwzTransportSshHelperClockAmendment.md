# SSH password-helper setup clock — DRAFT, remediation round 2

2026-10-03. This corrects the complete draft reviewed at core `eb06fac24` under
GwzTransportSshHelperClock-RemPlan.md and the bounded second correction in
GwzTransportSshHelperClock-RemPlan-2.md. Round1 closed the original findings; Consistency returned GO and Safety
retained the bounded P2-3 refusal branch. This correction awaits both original
reviewers at a newly settled tuple; implementation remains NO-GO meanwhile.
The following interfaces and tests are proposals, not executed implementation.
Do not implement this clock mechanism before root-relayed review GO.

TR1.6 §3.3/OQ6(a) and the accepted helper timing amendment §1 already require
full effective interaction after admission and exclusion of local helper work
from network clocks. Authentication policy, public application schemas and
wire fields are unchanged. The current pool deadline, Control aggregate/stall
state and NativeResource post-result check need one connection-scoped authority.

## Generic authority and concrete same-build interfaces

Add a transport-neutral `pool::SetupClock`, an Arc-owned handle with ONE Mutex
protecting its complete state. Its identity is the full `ConnectionId`; each
accepted transition increments a checked, nonwrapping `PhaseId`. Its phases
are Network, LocalAdmission, LocalInteraction and Terminal. It stores network
aggregate remaining time, stall state, active phase deadline, at most ONE
unacknowledged transition receipt, and one immutable terminal record. No
secrets, helper policy, URLs, configuration or FailureDetail live in transport.

The handle uses one supplied monotonic endpoint-epoch millisecond clock
(`Arc<dyn Fn() -> u64 + Send + Sync>`). PoolHost supplies its existing origin;
Control uses this same source rather than its separate wall-clock deadlines.
Production timestamps are sampled INSIDE the authority lock, never accepted
from a caller's pre-lock sample. Backward samples are clamped to the last sample;
checked-add overflow refuses rather than disabling timing. Deterministic tests
supply the same source. Existing non-shared callers retain existing timing.

`PoolDriver::install_setup_clock(connection, source, stall_ms)` creates and
installs the handle against the connection's existing connect allowance before
`Connector::start_reported` launches setup. It checks current expiry under the
pool runtime lock; an already expired/cancelled entry cannot be installed.
The installed handle replaces that entry's old ConnectClock deadline. An
uninstalled connector keeps the existing contract. Core's private Opening
carries the handle and core context described below; NativeResource and Control
retain it. PoolMachine advance/next_deadline/connected, PoolDriver cancellation,
Control update/check/quantum/network progress and post-result classification
all consult this authority. No original-deadline copy remains authoritative on
this shared path. The pool retains its ordinary non-setup allocation clocks.

The proposed generic methods are `prepare_local(kind, until)`,
`publish_local(prepared)`, `publish_network()`, `acknowledge(receipt)`, `observe()`,
`begin_network_wait()`, `network_progress()`, and `terminate(reason)`.
`kind` is Admission or Interaction, not a helper classification. Observe returns
an immutable Alive snapshot or Terminal record with connection, phase and a
neutral cause: NetworkAggregate, NetworkStall, LocalDeadline, PreparationDeadline, Cancelled,
DriverLost, ResourceFailure, or Completed. A resource failure also retains its
existing scalar code/effect/setup-cause, without secret or dynamic detail.
Snapshots are advisory: irreversible expiry, cancellation, success admission
or failure publication must run through a method under the authority lock.

## Linearization, acknowledgement and bounded ownership

Each operation acquires the authority lock, samples now, and first settles any
active deadline at or before now. It then applies its requested change only
if still live. Publication is the linearization point: a valid local entry at
99 against Network expiry 100 immediately installs LocalAdmission and pauses
network clocks under that same lock. It does NOT leave the authority in Network
while waiting for PoolHost. Thus Control observing at 101 before PoolHost sees
the local deadline, never the displaced Network deadline. A publication at 100
first settles Network expiry and is refused. This rule covers every consumer,
not just the ordering of PoolHost ticks.

Preparing a local transition reserves the sole slot as Prepared and returns a
connection/generation-bound token. It does not pause Network, acknowledge work
or protect the caller from expiry. Core installs its timing witness against
that token before publishing. Publication validates the token and current
phase, then converts the same slot to PendingAcknowledgement, installs the
target phase/deadline and notifies PoolHost AFTER unlocking. Dropping a prepared
token releases its reservation; expiry/cancellation/disposal invalidates it.
Its local deadline is finite. Observe discards its work reservation on expiry
but retains the exact issued connection/generation/kind/deadline in that SAME
bounded slot as ExpiredPrepared until token consumption/drop or active Terminal.
It neither pauses nor extends the current active clock. No next preparation is
issued while that token is still owned, so an expired token cannot be confused
with a later generation. This replaces round1's unqualified discard wording. PoolHost acknowledges that exact connection/
phase receipt through the authority. Before acknowledgement it again settles
expiry/cancellation, and refuses an expired local phase. Core awaits this
receipt outside locks; it starts no admission work, child or password offer
before acknowledgement and a fresh authority check. No second ordinary
transition can publish while a receipt is pending; cancellation/driver loss/
expiry can always settle it. Admission-to-Interaction and Network resume use
the same publication/acknowledgement rule, including delayed resume acknowledgements.
A stale receipt cannot acknowledge another generation.

Waiting is bounded by the newly installed phase deadline, or by cancellation/
driver loss/disposal. Local phases always have finite deadlines. A pending
Network resume is bounded by any LIVE retained aggregate/stall deadline.
With no live deadline, including Disabled aggregate plus enabled-but-Inactive
stall, it is cancellation/driver-lifetime bounded like existing disabled waits.
Acknowledgement alone must not start an Inactive stall clock. Authority observation itself completes expired
receipts; it does not require a functioning driver to notice expiry. The clock
retains one waiter registration per pending receipt and one driver registration,
replacing prior registrations rather than growing a queue. Notifications,
callbacks, waits, Git/child work and filesystem work occur outside all locks.

Expiry, explicit cancellation and resource terminal publication are serialized
by this lock. The first terminal record is immutable; later cancellation cannot
replace a prior timeout, and later success cannot revive cancellation/expiry.
Cancellation observed before launch defeats launch, including cancellation
between publication and acknowledgement. If expiry is already due when
cancellation acquires the lock, expiry remains the first cause. Driver drop,
shutdown and physical disposal call terminate and settle/wake pending receipts
before removing ownership. Dropping a waiting core owner cancels the connection.
Neither a receipt nor logical failure acknowledges physical disposal.

Lock order is pool Runtime, then SetupClock, then Control's bookkeeping state
if needed. Control may acquire SetupClock then its own state, but never acquires
pool Runtime while holding either. A Control caller must not hold its state
while entering SetupClock. No witness lock is nested with these locks. Methods
return wake notifications to their caller for delivery after unlocking. Pool
advance and connected admission settle the authority while holding Runtime;
Control cannot independently latch an obsolete deadline after a snapshot.
Post-result classification uses observe, and final pool connected admission
checks/commits Completed atomically through the same authority; a result that
expires between those checks is rejected. Core's terminal reporting reads the
already committed first terminal record, never writes a competing cause.

## Network aggregate and stall state

The shared authority owns both network clocks. Aggregate is Disabled or Live
with an absolute endpoint deadline in Network and a retained duration locally.
Stall is Disabled, Inactive (no network wait yet), or Live with an absolute
deadline in Network and a retained duration locally. A valid departure from
Network captures each live deadline minus publication time, once for THAT
departure. Pre-entry aggregate or stall expiry, including equality, settles
Terminal before the departure can pause anything. Ties prefer aggregate as in
the existing Control contract.

LocalAdmission-to-LocalInteraction preserves these exact paused states and
remainders. Local work and completed helper activity cannot call network
progress or manufacture a wait. A network begin/progress call while local
returns WrongState without mutation; actual core call sites must stop such
calls during local work. On valid Network publication, live clocks are rebased
to publication time plus their retained remainder BEFORE any Control check;
Disabled and Inactive remain distinct. Network time awaiting resume acknowledgement
counts against those rebased clocks, so acknowledgement grants no extra time.
No local duration is subtracted. A later departure captures the THEN remaining
network time after intervening Network work, never the first departure's value.

In Network, begin_network_wait changes Inactive to a full configured stall
interval; repeated begin while Live preserves its deadline. Actual network
progress performs the existing complete_wait reset to the configured interval.
A resumed live stall therefore expires at its retained remainder unless genuine
network progress occurs. Helper completion is not such progress. On this path
Control's wait_started/aggregate copies are removed or unused; its checks and
quantum derive from this authority, so stale timestamps cannot reappear.
Non-shared Control behavior is expressly unchanged.

## Core timing witness and typed terminal bridge

Core owns a private `SetupContext` Arc per connecting resource/attempt, shared
by Opening, Control/setup, NativeResource, PoolHost's connection entry and the
OpenRequest's checkout completion owner. It contains no credential material:
full connection ID, a bounded current/prior phase witness, and first sanitized
terminal Failure. One configured helper lookup is allowed per setup; witnesses
are therefore at most one Allocation and one Interaction record. Entries are
not process-global, reused across attempts or inferred from generic error text.

Before publishing LocalAdmission, core captures the retained allocation integer
milliseconds once using the accepted timing amendment's capture rule. The same
capture fixes admission expiry and its witness (`setup_cause=Allocation`, exact
`helper_budget_ms`, Effect None). Zero/sub-millisecond allocation records zero
and refuses immediately with that typed failure, launching no child or lookup
and setting no missing-Git latch. It does not pass through pool allocation's
ordinary free-resource or busy-resource inference.

After both endpoint and host permits, core captures exactly
min(configured_interaction_ms, positive_Open_interaction_ms,120000). That same
positive integer fixes the Interaction deadline and witness. Admission waits
and paused network deadlines do not reduce it. Discovery, reads, parsing,
verification and fill share it without resets. Core obtains a prepared token, installs the witness for its exact connection/
phase ID BEFORE publication, then publishes and awaits acknowledgement. There
is no visible local phase whose witness is still unassociated and no reporting
wait for a core thread to finish association. Publication has three complete typed refusal branches, settled under the
existing authority/token protocol:

1. `ActiveTerminal(record)`: active expiry/cancellation/resource failure already
   won, including during the initial deadline settlement. Consume/invalidate
   the preparation and return that immutable record. Core uses its admitted
   phase witness only; the rejected preparation cannot relabel it.
2. `PreparationExpired(record)`: the authority remains Alive, the token matches
   its exact issued Prepared/ExpiredPrepared slot, and its captured preparation
   deadline is due. Atomically consume that slot and commit Terminal with neutral
   PreparationDeadline, the exact prepared connection/phase identity and no
   active-phase timeout assertion. This is a failure to start the proposed phase,
   not a claim that live Network/Admission expired. Core matches ONLY this
   authority-validated identity to its originally captured witness, reporting
   Timeout/Effect None/Allocation or Interaction and the original helper_budget_ms.
   It releases any already-owned admissions under existing physical cleanup.
   There is no admission/child work, missing-Git latch, renewed capture, retry or
   wait for a previously nonexistent Terminal. For Interaction preparation this
   applies equally while the old Admission remains live. Another consumer's
   earlier active expiry/cancel still takes branch1, never this branch.
3. `InvalidToken`: stale, foreign, forged or otherwise nonmatching token while
   Alive. This refuses without assigning helper detail or altering a foreign
   clock. Core's setup owner terminates/disposes ONLY its own connection with
   scalar InvalidRequest/Effect None and reports the returned first terminal
   record; if active expiry/cancellation wins that arbitration it is preserved.
   No arbitrary unadmitted witness can supply timing provenance. The operation
   settles boundedly without launch, latch, budget reset or a terminal wait.

Preparation consumes no pause or allowance extension. No core context lock is held while calling the authority or awaiting
acknowledgement.

Extend generic pool Error with a neutral `SetupEnded` record containing full
connection ID, PhaseId and the first authority terminal cause/scalar resource
failure. Pool-first expiry carries this record instead of discarding local-phase
identity into InteractionTimeout/ConnectTimeout. Pool connected failure uses
the same identity-bearing record on the installed-clock path. Existing generic
pool errors and uninstalled users remain unchanged. This is a same-build library
interface change, not a transport wire or GWZ application schema change.

Core `EndpointOpenFailure::capture` receives the connection's SetupContext.
For SetupEnded it matches the exact connection/phase record, obtains the first
core failure (including the validated PreparationDeadline refusal above), and emits the existing Failure envelope. A LocalDeadline with a
matching Allocation/Interaction witness yields Timeout, Effect None, its exact
setup cause and helper_budget_ms. The ordinary network Aggregate/Stall path has
NO helper detail. Helper-first failure publishes the same terminal phase through
Control/NativeResource before pool completion; Control-first and pool-first
local expiry resolve that same witness, not independent timeout renderers.
Other helper failures retain existing fixed cause/outcome rules in core. Missing
Git, malformed output, cancellation and generic network expiry cannot be
relabeled helper timeout from absent detail. Facts keep their existing progress
ownership, with no secret-bearing dynamic diagnostics.

SetupContext and clock live through checkout logical reporting AND any retained
physical setup/child/file-worker cleanup. OpenRequest retains its Arc after a
logical checkout failure until capture finishes; PoolHost/NativeResource and
retained Job keep theirs until physical disposal. The first core failure is
immutable and later results cannot overwrite it. Context storage is released
only when these bounded owners drop; cleanup does not require the reporter to
stay alive. Existing cleanup retains both permits and sensitive buffers. No
secret is offered after cancellation or an unverified host key.

## Authentication transitions and precise supersession

Connecting starts Network for trust and server-method discovery. Only the
accepted ambient password-only route may enter local helper phases. Opening
privately carries repository path, captured config/shared HelperSlots, helper
enablement and the existing retained Open budgets. Explicit key, publickey-only
or combined offers and disabled helpers retain key/agent selection without
helper work. URL-password precedence and pool/account identity remain unchanged.
After successful lookup AND completed child/file-worker cleanup, publish/ack
Network; check cancellation/expiry before offering the password. A failed lookup
terminates, without new helper retries or broader fallback policy. Each started
lookup receives its full independent effective interaction allowance.

For ONLY the installed shared-clock setup path, this supersedes the original
ssh_setup NativeResource deadline/classify_setup_result authority, Control's
aggregate/wait_started expiry in agent_job, and pool ConnectClock connecting
expiry and connected post-result admission. The rules above replace the first
draft's PoolHost-only pending drain and unspecified stall suppression. They also
supersede local helper expiry's detail-dropping pool Error conversion and
ssh_worker EndpointOpenFailure::capture detail=None construction for the exact
identity-bearing shared setup record. Generic/non-helper timeouts retain absent
helper detail. TR1.6 §3.3/OQ6(a), timing amendment §1, configuration-view preparation
and cleanup remain controlling; no policy or accepted numeric bound changes.

Affected owners: transport pool mod/machine/clock/allocation/lifecycle/asynchronous
and generic deterministic tests; core ssh_pool Opening/PoolHost/Connector,
ssh_setup SetupConnector/NativeResource/result classification, agent_job Control/
Job reporting, ssh_local/ssh_password, ssh_worker OpenRequest/capture and typed
retry/error projection. New cohesive leaves keep modified files bounded. No
protected dependency, CLI/settings, handwritten Python, public schema or wire
change. Existing process-global Job debt and native-copy limitations are not
claimed closed by this amendment.

## Required executable regression boundary

- Deterministic Network expiry100: publish Admission99, delay PoolHost, run the
  actual Control supervisor101 first; no stale failure, no launch before ack.
  Repeat Interaction and Network resume ordering; publication100 refuses.
- Cancel between publish/ack, ack after local expiry, driver loss, waiter drop,
  disposal and late success settle once, wake once and cannot launch/resurrect.
  Exercise lock ordering with concurrent pool/control/result consumers.
- Discovery leaves a live stall remainder; local work exceeds its original
  wall-clock deadline, then successful resume/offer occurs. Inactivity expires
  at retained remainder; genuine network progress alone resets it. Include
  inactive/disabled stall, exact-boundary refusal and repeated local departures.
- Force pool-first, Control-first and helper-first Allocation/Interaction expiry:
  identical Timeout/Effect None/setup cause/exact helper_budget_ms, including
  1250ms and zero allocation; generic network timeout has no helper detail.
  With aggregate/stall Disabled, prepare Allocation1ms, install witness, delay,
  observe first, then publish: bounded validated preparation expiry reports that
  original 1ms, no work/latch/reset or nonexistent-terminal wait. Repeat with a
  later live Network deadline and with Interaction preparation expiring before
  still-live Admission. Active expiry/cancel controls preserve their cause and
  reject prepared helper detail. Stale/foreign tokens have no helper detail.
- Disabled aggregate plus configured-but-Inactive stall: delay Network resume
  acknowledgement beyond the configured interval, preserve Inactive, then start
  stall timing only on the subsequent real network wait.
- Trust consumes network; admission consumes allocation only; zero/free and
  saturated admission launch nothing. Helper outlasts original connect deadline
  but succeeds inside full allowance. Disabled network still bounds helper.
- Post-result race rejects expiry without stale original deadline; cancellation
  kills/reaps and offers no secret; retained child/file worker owns both permits.
  Password-only calls helper; publickey/combined, explicit key and disabled do not.

These are TDD obligations at the document gate, not claimed execution. Final
secret Code/State, combined Surface and normal integration gates remain owed.

# SSH password-helper setup clock — DRAFT

2026-10-03. Proposed ownership mechanism for the already required TR2.23
password-only server route. This is not implementation acceptance. TR1.6 §3.3,
OQ6(a), and the accepted helper timing amendment §1 require a full effective
interaction allowance after admission, with network clocks excluded during
helper work. No SSH authentication-policy choice is reopened.

The current `ssh_setup::SetupConnector` passes the pool network deadline to
`agent_job::Control`, and `NativeResource` independently retains/checks it after
job completion. The generic pool also expires the connecting resource. Pausing
only Control would still reject a valid helper result at the stale deadline.
All three consumers must use one connection-scoped clock authority.

## Proposed private/shared interfaces and ownership

Add a transport-neutral `pool::SetupClock` shared handle, created by the pool
for each connecting resource and destroyed with its physical owner. Its typed
phases are Network, LocalAdmission, LocalInteraction, and Terminal. It stores
one retained network remainder (None means disabled), the current phase's
absolute deadline, and monotonic transition receipts. It contains no secrets,
URLs, configuration, helper policy or SSH-specific values. No process global.
The pool's endpoint epoch supplies timestamps; wall-clock consumers use the
same epoch to derive their Instant, rather than inventing another deadline.

Extend the same-build `Connector::start_reported`/Opening seam to carry this
handle; other connectors keep existing Network behavior. Core's Control and
NativeResource borrow the handle. Control retains cancellation/failure and
stall-progress ownership; its aggregate expiry consults SetupClock. During a
local phase it neither starts nor expires a network stall timer. NativeResource
post-result admission consults this same authority, never its former separate
original-deadline copy. PoolDriver timer scheduling/expiry consults the handle.

The clock transition API is generic: request LocalAdmission(until), request
LocalInteraction(until), resume Network, and terminate. A transition returns an
acknowledged receipt or a typed expired/cancelled result. Network remainder is
captured exactly once at the transition timestamp. Transition timestamps must
be monotonic and cannot resurrect a phase already expired at that timestamp.
The physical owner retains the transition request until the pool acknowledges
it; no helper child starts before acknowledgement. PoolHost drains pending
transition receipts before advancing timer expiry, in timestamp order, then
advances to its current time. This prevents a stale supervisor tick from
expiring the original network deadline after a timely pause, without allowing
a late pause to revive an expired connection. Cancellation always wins before
acknowledgement or launch. Terminal rejects every later transition.

Library impact: gwz-transport pool phase/clock/driver and tests; core ssh_pool
Connector/Opening bridge, ssh_setup setup/result owner, agent_job Control's
optional shared-clock path, and ssh_local/ssh_password helper integration.
Existing non-shared Control users and old pool interaction APIs retain their
contracts. No protected git2-rs/gwz-git change, wire field, public GWZ request/
response schema, CLI driver/settings or Python handwritten source change.

## Phase and admission transitions

1. Connecting begins in Network with the retained positive Open/configured
   connect allowance, or None for disabled network timing. Trust and server
   authentication-method discovery consume this network/stall allowance.
2. Only the accepted ambient password-only route enters LocalAdmission.
   Opening privately carries the original repository path, helper enablement,
   and retained allocation allowance/deadline; it does not replace an exhausted
   value with a default. The clock pauses network timing before helper-slot
   admission. Endpoint and shared host HelperSlots remain distinct. Zero or
   expired allocation refuses without launch or missing-Git latch.
3. Once both permits are owned, capture integer milliseconds exactly once:
   min(configured interaction, positive Open interaction, 120000). A positive
   effective interaction enters LocalInteraction with its own deadline. Slot
   waits do not consume it. A zero/expired effective interaction cannot launch.
   Configuration discovery/read/parse/walk/view verification and credential
   fill share this single captured interaction deadline.
4. On a successful lookup and completed child/worker cleanup, resume Network
   with the retained remainder. The password is offered only after a fresh
   cancellation/expiry check and host-key verification. The pool identity and
   requested SSH username keep their existing meanings. Helpers are not run
   when publickey is offered or an explicit key/disabled policy owns the route.
5. Refusal, missing Git, malformed answer, timeout or cancellation terminates
   the local attempt with the existing typed outcome. A helper timeout is its
   independent interaction expiry; a slot timeout is allocation expiry. Neither
   is misreported as network aggregate expiry. Do not resume a failed attempt
   to run another helper or broaden key/agent fallback policy.
6. Kill/reap and retained cleanup continue under the existing cleanup owner.
   The helper's endpoint/host permits and sensitive buffers survive an
   unfinished child or blocking file worker. A late result after cancellation
   is disposed, never admitted. Terminal/cancelled clock authority remains
   live with retained physical cleanup and cannot be reused for a new setup.

No arithmetic adds helper time independently to three deadline copies. Local
phase expiry is bounded even when network timing is disabled. Repeated local
phases cannot manufacture a new network allowance; each genuinely started
lookup gets the contract's fresh effective interaction allowance, not a shared
interaction remainder shortened by earlier slot waits.

## Exact supersession and regression boundary

For the TR2.23 configured-password path only, replace the current ssh_setup
original-deadline post-result check and Control aggregate/network timing during
helper admission/work with the shared phase authority above. Replace generic
pool connecting expiry for an acknowledged local phase with that phase's
bounded deadline; resume the original retained network remainder afterward.
This implements, rather than changes, TR1.6 §3.3 and timing amendment §1.
Existing SSH trust, key/agent precedence, URL-password handling, disabled
network semantics and cleanup contracts remain controlling. The accepted
configuration-view mechanism remains the only helper configuration parser.

Required TDD rows: trust delay consumes network; admission consumes allocation
only; zero/free and saturated admission launch nothing; a started helper
outlasts the original connect deadline yet succeeds within its full independent
bound; resumed network expires at its retained remainder; helper expiry retains
its captured timing detail; exact-boundary/stale-tick transition cannot revive
expired Network; post-result classification uses the acknowledged authority;
disabled network still has a bounded helper; cancellation before/after phase
acknowledgement and during lookup kills/reaps and offers no secret; retained
child/file-worker cleanup owns both permits. Password-only offers call helpers;
publickey-only/combined offers, explicit key and disabled helpers do not.

Status: DRAFT. Do not implement the shared clock mechanism before root-relayed
review GO. Remaining HTTPS route/pool work may proceed independently; the final
credential implementation package still requires complete Code/State and
Surface acceptance and normal gates.

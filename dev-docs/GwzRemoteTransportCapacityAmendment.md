# Concurrent operation capacity amendment (draft)

Date: 2026-09-23. Status: **DRAFT correction 2 for operation-session review; existing accepted
plans remain authoritative until this amendment receives GO**. This is a
bounded textual replacement for the dynamic-capacity rules, not an
implementation claim or a change to the physical transport messages.

## Exact supersession

The accepted [retry plan](GwzRemoteTransportRetryPlan.md) §3 item 7 says
"At operation start, when no lease is non-idle, the operation installs its
resolved `--max-per-host` and `--jobs`, which can raise those ceilings. While
a lease is non-idle, a request cannot raise them; that operation is refused.
A lower limit installs at the next idle operation start and does not evict
another operation's non-idle connections." Replace that **whole** three-
sentence installation rule with: **At the start of the first operation and
at a later truly quiescent boundary, install that operation's exact resolved
physical capacity. Quiescent means no admitted operation scope, no non-idle
lease and no unfinished physical cleanup. During a live capacity epoch, an
incoming request whose resolved capacity is componentwise no greater than
the installed capacity may run without physical resize, while enforcing its
own `jobs` and per-host member-work limits. A requested raise refuses before
Open or helper access. A lower request never evicts the active operation's
connections; at a later quiescent start it may install its own lower caps.**
The shared physical caps are ceilings on the combined pool, not per-operation
guarantees or an excuse to discard the lower request's work limit.

Retry §6's earlier bullet also says:

> If the pool has no non-idle lease, idle connections above the new `total`
> or the new per-host cap are closed, and the four fields are set to this
> operation's resolved values. A sequential later operation therefore gets
> its own caps. The first operation does not freeze them.

Replace that **whole** bullet with: **Only a quiescent capacity transition
may close excess idle connections and set the four physical fields to a new
operation's resolved values. An admitted operation that has not opened its
first lease still pins the current epoch; absence of non-idle leases alone
does not permit a resize. A sequential later operation after all scopes and
cleanup retire may install its own exact caps.**

Replace retry plan §6's bullet:

> If any lease is non-idle, the new operation is refused with a typed error.
> It does not run on the other operation's caps. The CLI does not start a
> second operation. The host path that already rejects a second client
> endpoint keeps that rejection.

with: **When a physical capacity is installed and any operation or lease is
live, an incoming operation whose resolved capacity is componentwise no
greater than the installed capacity is admitted without changing that
capacity, idle timer or existing leases. It still enforces its own resolved
`jobs` and per-host member-work limits. If any requested component is greater,
the new operation is refused with typed `CapacityConflict` before a handler,
Open or credential/helper access. Existing operations continue. An explicit
CLI placement must use the bound endpoint's pre-Open capacity admission or
refuse the placement before dispatch.** The candidate CLI remains a
single-operation process and installs its requested caps at host creation.

Replace S1.4's entire first three sentences — "Install the resolved caps at
operation start, as §6 and §3.7–§3.8 describe, on the candidate CLI and on
the host pool. Close idle connections that exceed the new caps before
accepting the operation. Refuse the operation when a non-idle lease exists."
— with: **The candidate CLI installs its sole operation's exact caps at host
creation. The long-lived host installs exact resolved caps at first operation
start and at later quiescent transitions as this amendment defines; it
retires excess idle connections only during those transitions. While any
operation or lease is live, it admits a componentwise equal/lower request
under the installed epoch and refuses a request needing a raise before
Open.** Keep S1.4's upper-bound removal, defaults, no-clamp and no-freeze
requirements. This changes S1.4's test oracle; it does not claim S1.4 has
already been reimplemented.

The [remote transport plan](GwzRemoteTransportPlan.md) Phase 2 exit sentence
"Overlapping operations with different per-host policy limits respect both
physical ceilings; lowering one operation's fan-out does not evict the
other's connections" is replaced with: **Start an operation whose resolved
capacity is high enough for both, hold one lease, and overlap another
operation with a lower per-host work limit. The lower operation must run
without resizing the shared physical pool and must respect its own lower
fan-out. A concurrent request that needs to raise any physical cap must
refuse before Open, without evicting or changing the first. After all scopes
and physical cleanup retire, a later higher-limit operation may install its
requested cap.** This makes the order and expected outcome testable. Phase 2's
other pool/stream exit cases remain.

## Atomic transition and proof

The first install and every later capacity change use one transition owner,
target and cancellable waiters. **Quiescent** means no admitted operation
scope, no non-idle physical lease and no unfinished physical cleanup; an
idle-only pool with a live operation that has not opened yet is not
quiescent. At a quiescent boundary, one owner retires excess idle capacity and
installs the exact new value. Requests with the same target wait for atomic
publication; a different target during that transition refuses without
mutation. Cancellation, install failure and host close wake all waiters.
Before an operation is accepted, the physical owner atomically compares its
resolved capacity with the published value and reserves its place in that
capacity epoch. The epoch cannot be changed until those operations and their
cleanup retire.

The requested capacity is the existing retry-plan tuple:
`per_user_host = per_host = resolved max_per_host`,
`total = max(256, resolved jobs)`, and
`max_requests = max(1024, resolved jobs)`. Componentwise comparison uses all
four fields. Pool `max_requests` is a checkout/request budget, not the count
of top-level operations or workers. Existing nonzero validation and the
60-second idle timeout remain unchanged.

Closure cases: equal and lower policies overlap under a held lease; a higher
policy refuses before remote Open; two same-target requests during a later
capacity change both proceed; a different target during transition refuses;
close/cancel/failure wakes waiters; after true quiescence a larger capacity
installs. Include an explicit bound-CLI placement or prove its typed early
refusal. Neither this draft nor a Python-only admission guard closes those
core tests.

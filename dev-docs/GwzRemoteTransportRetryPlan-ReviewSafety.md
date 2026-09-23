# GwzRemoteTransportRetryPlan — SAFETY-AXIS REVIEW

**Review object:** `gwz-core/dev-docs/GwzRemoteTransportRetryPlan.md`, SHA-256 `623511dd9508bc0a8c44ca14e576eec51923e041ed7ad6bea46a0f2db862948e`; uncommitted draft, not implementation authority, dated 2026-09-23
**Baseline:** gwz-dev `9c0008870ecd8f209611ebd99fb1598a14021da9`; gwz-core `102015a5ff3abc059be699f801d1412b0fd01c8a`; gwz-cli `ab59011db0ee00ab0c032fc23fc06b2578bf7b68`; gwz-transport `aa40936d0805e8cb60f8027615abe20d4f2045e4`. Committed allocation, scheduler, pool, and factory sources were read with `git show HEAD:path`; the three authorized gwz-core exceptions were read from the working tree.
**Date:** 2026-09-23
**Axis:** SAFETY — degraded and mixed-version paths, irreversible preconditions, disclosure scale, stuck states, adverse interleavings, and blast-radius expansion. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on it. Filed verbatim by the lane owner.

**Verdict: NO-GO** — P0: 0, P1: 0, P2: 5, P3: 1. I pre-commit to GO on a revision that resolves P2-1 through P2-5 as specified; P3-1 should be corrected in the same revision.

---

## 0. Evidence base

The tuple and both document hashes were verified unchanged at review start and end. `GwzV110Plan.md` remained at working-tree SHA-256 `f0c57611cc7c03a102602983a08231aeb666d7369e94b9476234b1e88e568fe8`; its recorded difference from accepted hash `9d49af85…` was treated as authorized.

Read:

- The review object and all named controlling documents.
- `AgentProcessRules.md`, as amended by `GwzProcessOptimization.md`.
- The accepted two-clock timeout plan.
- Working-tree exceptions `ssh_setup.rs`, `transport_host/mod.rs`, and `GwzV110Plan.md`.
- Committed `gwz-transport` pool allocation, configuration, clock, machine, and asynchronous ownership code.
- Committed `par_map_per_host.rs` and candidate transport-factory code.
- Current SSH and HTTPS pool-error mappings.

No files were modified, no builds or tests ran, and no peer current-round report was read.

## 1. Findings

### [P2-1] Unbounded values drive eager OS-thread creation outside the jobs permit

- **Root cause:** The plan removes the `4096` cap while expressly retaining the current scheduler shape, which creates `min(--max-per-host, members-on-host)` scoped threads before any thread acquires a `--jobs` permit.
- **Where:** Plan §1 lines 29–37, §6 lines 200–205, and S1.4; committed `gwz-core/src/operation/par_map_per_host.rs` lines 71–104.
- **Violated invariant:** A concurrency permit must bound execution resources; accepting a large logical value must not turn a low-`--jobs` operation into an eager, fallible allocation of unrelated OS threads.
- **Reproduction/state sequence:** Use `--jobs 1 --max-per-host 100000` on 100,000 members sharing one parsed host. The scheduler attempts 100,000 `scope.spawn` calls. At most one thread passes the semaphore, while the rest consume thread stacks and kernel thread slots. Thread creation eventually panics or exhausts process/host resources.
- **Impact:** An explicit but accepted value can abort the entire coordinated operation rather than return member results. Raising the default and removing the only upper bound widens this failure beyond the pool itself.
- **Required correction:** Decouple logical per-host concurrency from eager thread creation. Use bounded/lazy scheduling with fallible resource acquisition and a typed operation failure; do not silently clamp the requested policy.
- **Closure/regression test:** Exercise a very large per-host value with `jobs=1` and many synthetic members. Assert bounded live worker count, no panic, preserved result order, and cancellation of queued work.

### [P2-2] The unchanged 1,024-request pool limit contradicts unbounded operation concurrency

- **Root cause:** The plan scales `total` from `--jobs` but leaves `max_requests` at 1,024 and retains its existing bound.
- **Where:** Plan §6 lines 179–185 and 200–205, S1.3/S1.4; committed `gwz-transport/src/pool/mod.rs` lines 20, 33, and 43–54; `pool/machine.rs` lines 114–133.
- **Violated invariant:** Every operation admitted by the advertised concurrency policy must either queue or execute; it must not fail through an unrelated hidden capacity ceiling below the accepted policy.
- **Reproduction/state sequence:** Resolve `--jobs 5000 --max-per-host 5000` for at least 1,025 simultaneous members. The plan creates pool `total=5000`, but request 1,025 reaches `requests.len() >= max_requests` and receives `Error::Capacity`.
- **Impact:** Valid larger values do not work as documented. Members fail immediately even though the configured connection capacity remains available.
- **Required correction:** Define and implement a coherent request-capacity rule derived from admitted outstanding operations, or queue outside the bounded pool without converting excess members to failures. Preserve a finite allocation strategy without reintroducing a hidden clamp.
- **Closure/regression test:** Submit more than 1,024 simultaneous checkouts under matching larger resolved limits; all must queue or receive leases, with no `Capacity` result caused solely by `max_requests`.

### [P2-3] First-operation cap capture makes later explicit policies race-dependent

- **Root cause:** The endpoint-owned pool is reusable across operations, but §6 freezes its caps from whichever operation first opens a connection and forbids resizing.
- **Where:** Plan §1 lines 23–31, §6 lines 187–211, and S1.4; `GwzRemoteTransportRequirements.md` D1 establishes endpoint ownership and cross-operation reuse.
- **Violated invariant:** An operation’s resolved `--jobs` and `--max-per-host` values must have the advertised effect independent of prior or concurrent operations.
- **Reproduction/state sequence:** In one long-lived process, operation A first opens a connection with defaults, constructing caps 32/32/256. Operation B then resolves `--max-per-host 5000 --jobs 5000`; §6 requires B to use A’s pool caps. Reverse arrival order and the shared pool instead retains B’s huge caps. Concurrent arrival makes the effective process policy scheduling-dependent.
- **Impact:** Explicit settings are silently ignored, performance and connection blast radius depend on operation order, and tests of isolated operations do not predict long-lived host behavior.
- **Required correction:** Give endpoint construction a stable process-level policy independent of operation arrival, support a defined safe reconfiguration protocol, or partition pools by compatible policy. A later operation’s explicit value must not be silently replaced by first-writer state.
- **Closure/regression test:** Run two sequential and two concurrently starting operations with conflicting low/high policies in both orders. Assert deterministic documented behavior and that each operation’s effective limit is enforced.

### [P2-4] The degraded-key machine does not prevent or account for the initial connection stampede

- **Root cause:** Backoff serializes only after failures have been observed; the plan leaves normal pool allocation free to open up to 32 connections for a cold key before the per-key machine has established health.
- **Where:** Plan §5 lines 130–153 and S3.1/S3.3; committed `gwz-transport/src/pool/allocation.rs` lines 47–93 opens one connection for every eligible waiter up to all caps.
- **Violated invariant:** Members on one unproven or degraded key must share one probe and one attempt-generation decision; retry accounting must not multiply network attempts.
- **Reproduction/state sequence:** Queue 32 members for a cold dead host. The pool starts 32 handshakes before any fails. When failures arrive, either each consumes the shared four-attempt budget—exhausting it in one wave—or they are coalesced as one attempt while the remote still received 32 simultaneous probes. The new default makes this first degraded wave four times the old default, followed by up to three more probes.
- **Impact:** The text’s “queued behind the single probe rather than multiplied” safety claim is false for the first wave. Dead or refusing hosts receive a burst, and the four-attempt contract has no defined answer for concurrent failures.
- **Required correction:** Define explicit per-key health/generation states covering the initial open, concurrent in-flight siblings, transition to backoff, stale completions, and reopening after a successful probe. Gate cold/degraded keys so one health probe precedes bulk normal allocation, or specify another bounded mechanism with equivalent safety.
- **Closure/regression test:** With 32 cold members and a dead key, assert exact handshake concurrency and attempt-generation counts from first admission through final failure. Repeat with delayed sibling failures and a late success; no stale success may reset or escape the degraded generation.

### [P2-5] Interaction and allocation timeouts are indistinguishable from retriable setup timeouts

- **Root cause:** The plan classifies `Timeout` as retriable but defines only stall and aggregate as retriable timeout origins. Current SSH and HTTPS mappings collapse `AllocationTimeout`, `ConnectTimeout`, and `InteractionTimeout` into the same `ErrorCode::Timeout`, and the plan adds no typed internal cause or exclusion test.
- **Where:** Plan §4 lines 84–118, §5 line 143, and S3.1/S3.2; `gwz-core/src/git/endpoint/ssh_worker.rs` lines 170–181; `https_pool.rs` lines 221–230.
- **Violated invariant:** Interaction, queue capacity, authentication, and the two network clocks must remain distinct; only stall/aggregate expiry during non-reusable setup may trigger backoff.
- **Reproduction/state sequence:** A setup enters an interactive helper and consumes the existing 120-second interaction allowance. The pool returns `InteractionTimeout`; both current paths expose `ErrorCode::Timeout`. The proposed common classifier sees a setup-stage timeout and retries it, starting backoff and potentially another interaction. The same ambiguity exists for `AllocationTimeout`.
- **Impact:** User interaction or capacity waiting can be repeated up to four times despite §4’s closed set, extending latency and prompting behavior while misreporting the cause as network retry.
- **Required correction:** Preserve a typed internal timeout origin through the common classifier. Explicitly make interaction and allocation timeout non-retriable; only setup stall and aggregate expiry may enter backoff.
- **Closure/regression test:** On both SSH and HTTPS, inject stall, aggregate, interaction, and allocation expiry. Assert exactly one attempt for interaction/allocation, four only for stall/aggregate, and distinct retained diagnostics.

### [P3-1] The stated worst-case duration omits paused interaction time

- **Root cause:** §5 computes the bound as four aggregate clocks plus three waits while §4 and the accepted timeout plan pause both network clocks during up to 120 seconds of interaction on every fresh attempt.
- **Where:** Plan §4 lines 117–118, §5 lines 147–153, and §7 line 231.
- **Violated invariant:** A published worst-case bound must include every bounded clock that can extend the same operation.
- **Reproduction/state sequence:** On each of four attempts, spend nearly 120 seconds in successful interaction, resume setup, then fail retriably near the 30-second aggregate limit. Three maximum-jitter waits add 7.75 seconds. The sequence approaches 607.75 seconds before any additional disposal accounting, not approximately 127.75 seconds.
- **Impact:** Operators and cancellation tests can assume a bound almost eight minutes too short.
- **Required correction:** State separate network-only and full wall-clock bounds, including interaction and any serialized cleanup that implementation requires.
- **Closure/regression test:** Inject four near-limit interactions followed by retriable setup failures and maximum jitter; assert the documented full bound and prompt cancellation during each phase.

## 2. Invariant analysis

- `--ssh-timeout 0` correctly remains explicit opt-in to unbounded network waiting. The retry loop advances only after an attempt returns; the plan does not falsely claim a finite bound in that mode.
- Positive stall values remain independent of the 30-second aggregate. The candidate factory correction no longer copies stall into aggregate.
- Authentication, trust, host-key mismatch, `gh` rejection, protocol failure, repository refusal, and post-reusable fetch/push failures are textually excluded from retries.
- A timeout during an active push remains outside setup and is not retried.
- Late setup success after aggregate expiry remains non-reusable under the accepted timeout rule and current `classify_setup_result`. P2-4 requires that the retry generation also reject stale completions.
- The retry text does not add new secret-bearing diagnostics. Intermediate attempts do not complete the member, and final output adds only an attempt count behind existing verbosity/JSON gates. No widening of token, agent-socket, known_hosts, or credential logging was found.
- Cancellation during a backoff is normatively required to suppress another attempt. The closure suite should additionally cover cancellation of every waiter on a shared key and prove that an orphaned wake cannot launch a probe.
- Defaults 100, 32, 9 seconds, 30 seconds, four attempts, absence of clamping, and absence of a product ceiling were treated as deferred outcomes rather than findings. P2-1 and P2-2 concern unsafe and contradictory implementation consequences of those outcomes, not the choices themselves.
- Channel multiplexing, learned refusal caps, body retries, idle/cleanup/interaction retuning, and live measurement were not treated as required scope.

## 3. Risks and next action

The plan is not safe to freeze. The blocking defects are bounded and text/code-fixable:

1. Redesign scheduling so accepted large values do not eagerly allocate one OS thread per member.
2. Reconcile pool request capacity with larger operation limits.
3. Replace first-operation-wins pool sizing with deterministic cross-operation policy.
4. Specify the complete per-key degraded-state and attempt-generation machine from the first cold open.
5. Preserve timeout origin so interaction and allocation waits cannot enter network retry.
6. Correct the wall-clock bound and add the corresponding deterministic-clock tests.

After amendment, rerun the independent Safety review against the new exact hash.

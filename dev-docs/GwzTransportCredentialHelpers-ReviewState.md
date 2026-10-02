# Credential helpers, typed SSH/HTTPS consumers and shared SetupClock — STATE-AXIS REVIEW

**Review object:** Final implementation awaiting acceptance at core `f97ff21fa56c2fe4abec6a73e90871dbe2d9d185` and transport `41a16b2713b302c3675f081584d392afeae26ad5`. Controlling document: `gwz-core/dev-docs/GwzTransportCredentialHelpersDesign.md`, revision 4, with its accepted amendments.

**Baseline:**

| Member | Settled SHA |
|---|---|
| root | `dc2b4af36e6ad6eca4a781611fcf76f6d9e76226` |
| gwz-core | `f97ff21fa56c2fe4abec6a73e90871dbe2d9d185` |
| gwz-transport | `41a16b2713b302c3675f081584d392afeae26ad5` |
| gwz-py | `947ed579abec292a23db3db7394923e70ee4363e` |
| gwz-cli | `1543a3bec00cda913a02c266e89d841eeeafb55b` |
| gwz-core-evidence | `662d89828b478a2acce8c0308834db7d17c872f7` |

Committed sources were inspected using `git show HEAD:`, range diffs and numbered reads. All six heads matched the required tuple at both the beginning and end.

**Date:** 2026-10-03

**Axis:** State machines, terminal arbitration, races, ownership, cleanup and fail-closed behavior. Independent, adversarial, read-only. The other axes run in parallel; nothing here relies on their reports. Filed verbatim by the lane owner.

**Verdict: NO-GO** — five P2 findings block acceptance. No P0, P1 or P3 finding is asserted. I pre-commit to GO on a revision that resolves P2-1 through P2-5 as specified, with the closure regressions passing against the revised settled tuple.

---

## 0. Evidence base

I read the canonical `PromptState.txt` in full, root `AGENTS_GWZ.md`, applicable `AGENTS.md` files, relevant process clauses in `AgentProcessRules.md`, `GwzProcessOptimization.md`, the checkpoint’s credential acceptance requirements, and the library-boundary policy. Excluded drafts, private bug/proposal documents, old untracked campaign evidence and current peer reports were not used.

Authority inspected at the settled core included:

- `GWZDesign.md` and `GWZRequirements.md`: adopted credential clock, context and configuration-view requirements.
- `GwzTransportCredentialHelpersDesign.md`: revision 4 operator rulings, helper lifecycle, buffers, parser, outcomes and messages.
- `GwzTransportCredentialHelperTimingAmendment.md`: retained admission allowance, zero/submillisecond refusal, typed helper timing detail and validation across Failure carriers.
- `GwzTransportCredentialHelperConfigurationViewAmendment.md`: native Git discovery, controlled configuration encoding, controlled-empty correction, preparation bounds, one interaction deadline, worker ownership and whole-capacity wiping.
- `GwzTransportSshHelperClockAmendment.md`: authority serialization, phase witnesses, retained network clocks, first terminal failure, private helper enablement and authentication transitions.
- `GwzRemoteTransportRetryPlan.md` §4 and relevant release-plan/amendment clauses: retry eligibility and explicit deferrals.
- `GwzTransportCredentialHelpers-ImplementationRecord.md`: evidence and disclosed limits, without treating it as design authority.

The source examination covered these ownership paths:

| Area | Files and portions inspected |
|---|---|
| Shared authority | Transport `pool/setup_clock.rs` through line 293; its `state.rs` and `transitions.rs`; modified setup, allocation, admission, asynchronous, lifecycle and clock paths |
| SSH context and consumers | Core `ssh_setup_context.rs` and tests; `ssh_password_helpers.rs`; `ssh_local.rs`; `ssh_password.rs`; `ssh_setup.rs`; `ssh_pool.rs`; worker `open_request.rs` and `runner.rs` |
| Physical cleanup | Core `agent_job.rs`, `agent_job/control.rs`, shared control state; helper `owner.rs` and `file_worker.rs` |
| Helper preparation and secrets | Core `https_auth.rs` and its lookup, runner, view, framing, executable, secret and owner modules |
| HTTPS scope and budgets | Core HTTPS worker credentials, preparation, challenges and budget paths; operation route ownership and missing-Git latch |
| Terminal transport details | Transport `stream/incoming.rs` lines 1–168; stream state, machine and asynchronous accessors; Failure-detail and HTTPS-shape validators |
| Public consumers | Core Git backend transport binding and native credential callback; endpoint configuration; session opening and failure rendering; request completion; HTTPS RPC adapter and blocking stream |
| Existing regressions | SetupClock tests and custom-waker tests; Failure-detail carrier tests; helper runner tests; SSH password-helper and typed-projection tests |

These were source inspections, not new test executions.

I also read the allowed final-owned receipt and implementation source receipt. The final-owned receipt’s SHA-256 was:

`bb64ed2d9a1ad3a18422cf342f1a3733e3f654bc756bbb769485a83f3c0412aa`

Read-only hashing compared receipt entries with committed `git show HEAD:` bytes: 121 core, 35 transport, three Py and one CLI files, 160 total, with zero mismatches.

Recorded final gates inspected included:

- `gwz-tr222-ssh-consumer-green-v7.log`: focused consumer result, two passed, zero failed.
- `gwz-tr222-ssh-consumer-affected-v8.log`: 442 passed, zero failed, four ignored.
- `gwz-tr222-clock-transport-final-v8.log`: 17 SetupClock tests passed, plus the recorded transport suite and doctest results.
- `gwz-tr222-ssh-consumer-clippy-v9.log`: completed; the receipt records 50 retained warnings, so this is not strict-core Clippy green.

The earlier full ordinary and candidate suites predate the final consumer patch. I did not promote them to final consumer proof. The record’s unpublished failed `5e` gate and subsequent staged checker corrections were treated as disclosed historical evidence.

Commands were read-only source, Git, status and hash inspections. No builds, tests, probes, source writes or Git mutations were performed. The findings below are reproducible source-supported sequences; they were not executed during this review.

## 1. Findings

### [P2-1] SSH publishes a resource terminal before associating its detailed first Failure

**Location:** Core `src/git/endpoint/ssh_setup_context.rs:129–140`, especially line 135; fallback caching at lines 50–109. Zero-allocation producer: lines 149–159. Consumer: `src/transport_host/session/driver.rs:44–61`.

**Violated invariant:** The SSH clock amendment requires pool-first, Control-first and helper-first reporting to obtain the same first core failure, with that failure immutable. Publication of a scalar terminal must not expose a window in which its associated helper detail is unavailable.

`terminate_failure` first executes:

```rust
let record = self.clock.terminate(cause).deliver();
```

Only afterward does it lock context state and install the supplied `Failure`. The terminal is therefore observable, and notifications can be delivered, before the detailed failure is installed.

**Reproduction/state sequence:**

1. Enter helper Admission with retained allowance zero.
2. `terminate_failure` creates a resource cause carrying Timeout, Effect None and Allocation, then publishes it.
3. Before lines 137–139 run, a pool observer completes the checkout and `EndpointOpenFailure::capture` calls `SetupContext::failure(record)`.
4. `failure` reconstructs only the resource cause’s scalar fields. It has no `helper_budget_ms`, and caches this fallback in `state.first`.
5. The producer resumes, sees `state.first` already occupied, and discards its supplied Failure containing `helper_budget_ms: 0`.

A synchronous safe custom waker that observes/captures the terminal during delivery can expose the ordering deterministically. A separate pool thread can expose it without relying on notification execution.

**Impact:** The first immutable result permanently loses helper provenance. In the zero-allocation case, the SSH typed renderer cannot recognize the helper timeout, so the operation loses the intended CredentialHelperTimeout/code75 and M10 recovery message. Other resource failures can lose their fixed detail or facts through the same window.

**Required correction:** Establish a bounded association between the candidate detailed Failure and the exact terminal identity before another observer can materialize and cache a fallback. Preserve authority arbitration: earlier expiry/cancellation must still win. Do not solve this by merely postponing wake delivery; an independent observer can still see the terminal. Do not introduce nested authority/context locks or require the logical reporter to wait for physical cleanup.

**Closure regression:** Force an observer between terminal commitment and the current detail-store point. Assert that both pool-first and producer-first capture return the same complete Failure, including zero helper allowance, and that later cancellation or results cannot replace it. Exercise an earlier competing terminal so an unadmitted candidate cannot supply its detail.

**Classification:** **Non-architectural.** The accepted design already specifies the authority and core failure association. The implementation orders their publication incorrectly.

### [P2-2] SSH username conversion can release an unwiped credential allocation

**Location:** Core `src/git/endpoint/https_auth/secret.rs:41–43`, parser construction at line 110, and drop at lines 63–67. Call site: `src/git/endpoint/ssh_password.rs:132`.

**Violated invariant:** Endpoint-owned credential allocations must be wiped when their ownership ends. Growing a populated sensitive vector must not free its previous allocation with credential bytes still present.

The parser constructs the username using `username.to_vec()`. `ssh_parts` then appends a NUL byte using `push(0)`. A username vector with capacity equal to its length must grow. If reallocation moves it, the old allocation is released without passing through `Secret::drop`. The eventual drop wipes only the replacement allocation.

**Reproduction/state sequence:**

1. A valid helper returns a nonempty username and password.
2. Parsing creates a tightly sized username vector.
3. Trusted password-only SSH authentication calls `ssh_parts`.
4. Appending the NUL requires growth.
5. An allocator that moves the allocation releases the original username bytes without overwriting them.

The defect does not require malformed helper output or an unverified host. It occurs on the successful helper path.

**Impact:** Endpoint-owned credential bytes survive in a freed allocation. This contradicts the claimed credential-buffer lifetime guarantee. This review does not demonstrate an external disclosure, so it is classified as P2 rather than active exposure/P0.

**Required correction:** Allocate the final required SSH representation before copying sensitive bytes, or use a separate correctly sized zeroizing representation. Keep HTTPS username semantics intact. Any replacement strategy must wipe the old populated allocation before releasing it and must not introduce another sensitive growth copy.

**Closure regression:** Use a tightly sized nonempty username and an allocation-observation seam that forces movement or records deallocation contents safely. Assert that no populated username allocation is released unwiped. Also verify repeated SSH conversion and unchanged HTTPS Basic construction.

**Classification:** **Non-architectural.** This is a bounded buffer-sizing and conversion defect.

### [P2-3] A ready helper result can be accepted after the HTTPS interaction deadline

**Location:** Core `src/git/endpoint/https_auth/runner.rs:122–151`; lookup completion at `lookup.rs:124–135`; answer commitment at `src/git/endpoint/https_worker/credentials.rs:55–68`.

**Violated invariant:** One interaction deadline bounds discovery, preparation, parsing and final fill. A successful result must be checked against cancellation and that deadline before admission. The configuration-view amendment expressly requires checks before and after bounded operations.

The runner checks before spawning, but its final `tokio::select!` can choose the work branch when both work and the deadline are ready. A successful branch has no subsequent `Runner::check`. The final lookup parses the output and returns it without a final check. The HTTPS consumer then creates and caches the credential.

**Reproduction/state sequence:**

1. Final fill launches before deadline D.
2. The lookup future is not polled while the child completes successfully and stdout reaches EOF.
3. Polling resumes at or after D. Both completed work and `sleep_until(D)` are ready.
4. The select chooses completed work.
5. The runner returns successful output, lookup parses it, and the HTTPS consumer caches the answer.

There is a second boundary within the same root cause: successful child completion can occur just before D, while final credential parsing crosses D. Neither path has a final successful-result admission check.

**Impact:** HTTPS can accept and subsequently offer a helper answer beyond its captured interaction allowance, producing success where the accepted contract requires a helper timeout. The ready-work race also permits a successful lookup result when cancellation is already ready; the deadline case alone establishes the blocking defect.

SSH’s later shared-clock checks do not repair the HTTPS path, which does not install that SSH setup context.

**Required correction:** Check cancellation and the unchanged interaction deadline after child completion and after final parsing, before successful answer admission. Refused output and parsed secrets must retain their wiping behavior; child/group and permit ownership must still reach physical cleanup. A biased select alone is insufficient because final parsing can cross the boundary.

**Closure regression:** Withhold polling until both successful child completion and the timer are ready, then test both branch orders. Test equality at D and final parsing across D. Assert Timeout and no cached credential or Authorization offer. Add the equivalent already-ready cancellation case and confirm cleanup ownership.

**Classification:** **Non-architectural.** The existing deadline policy is correct; successful-result admission omits its final enforcement.

### [P2-4] The local SSH route loses the backend’s helper-disabled policy

**Location:** Core `src/git/gitbackend/transport_binding.rs:150–213`; SSH opening at `src/transport_host/session/driver/opening.rs:21–25`; endpoint helper construction at `src/transport_host/session.rs:363–372`; helper selection at `src/git/endpoint/ssh_local.rs:116–124`.

**Violated invariant:** The SSH clock amendment specifies that Opening privately carries helper enablement and that disabled helpers retain key/agent selection without helper work. The existing backend helper policy must survive selection of the local transport.

The HTTPS branch translates `backend.credential_helpers` into its route policy at line 164. The SSH branch carries context and selected identity but no helper policy. SSH session opening distinguishes only explicit-key versus ambient identity. At the endpoint, helpers are enabled whenever its HTTPS configuration contains auth configuration; ordinary endpoint environment construction supplies that configuration.

Consequently, the ambient password-only SSH route runs the endpoint’s helpers regardless of the requesting backend’s `CredentialHelperPolicy::Disabled`.

**Reproduction/state sequence:**

1. Construct a helper-enabled local endpoint from the ordinary environment configuration.
2. Bind a `Git2Backend::without_credential_helpers()` operation to its host context.
3. Open a trusted SSH server offering password-only authentication, without an explicit selected key.
4. The SSH route supplies no disabled bit.
5. `ssh_local` finds endpoint helpers and invokes lookup, then can offer the returned password.

The existing disabled fixture that omits helpers from the entire endpoint does not test this operation-specific opt-out.

**Impact:** An operation explicitly disabling credential helpers can launch configured helpers and offer their credentials. The native callback and local HTTPS route respect the policy; local SSH does not.

**Required correction:** Carry backend/operation helper enablement through the private SSH opening and admission path and enforce it before helper lookup. Preserve isolation when enabled and disabled operations share an endpoint. Review pool selection/reuse so the private policy cannot be silently lost there. Do not globally disable helpers for unrelated operations.

**Closure regression:** Use a helper-enabled endpoint, a recording helper and a trusted password-only SSH fixture. A backend with helpers disabled must invoke no helper and offer no helper password; its existing key/agent outcome must remain. An enabled operation through the same endpoint must still work. Exercise enabled/disabled ordering and pooled reuse.

**Classification:** **Non-architectural.** The accepted design already calls for this private enablement value. Its propagation is missing.

### [P2-5] `Closed.failure` discards an admitted terminal Failure

**Location:** Transport `src/stream/incoming.rs:115–129`; retention contract at `src/stream/machine.rs:272–275`; contrasting `Failed` handling at `incoming.rs:149–159`. Core consumer: `src/git/endpoint/https_remote.rs:63–71`.

**Violated invariant:** The first admitted incoming terminal Failure must remain available through the stream’s lifetime. Valid detail must survive every admitted terminal carrier, and later ignored terminals must not replace it.

The `Closed` branch admits the envelope, extracts its Failure, and uses only its code and effect to fail the stream. It never assigns `retained_failure`. The `Failed` branch does assign it.

This is a supported carrier, not an invented envelope: `tests/failure_detail.rs:90–158` explicitly admits and round-trips helper timing and retry detail through `Closed`. The endpoint close-construction API also accepts a validated Failure.

**Reproduction/state sequence:**

1. Complete a valid initiator close handshake: local EndWrite, peer EndWrite and local Close.
2. Receive a valid `Closed` with a Failure containing Timeout, Interaction and `helper_budget_ms: 1250`, optionally with retry provenance.
3. Admission succeeds and the stream records `PeerFailed`.
4. `retained_failure()` returns `None`.
5. A later `Failed` is ignored because the stream is already terminal, so it cannot restore the lost Failure.

**Impact:** The stream exposes the scalar failure but loses its typed detail, setup cause and Failure-owned facts. The core RPC adapter returns the original generic I/O error when retained Failure is absent, bypassing its typed report callback and recovery renderer. This loses diagnosability for a valid terminal carrier; it does not turn the failure into clean EOF.

**Required correction:** Retain the admitted `Closed.failure` as the first terminal Failure, preserving the existing `Closed.facts` handling. Keep validation before retention and preserve late/duplicate immutability.

**Closure regression:** Run a real `StreamMachine` close handshake with detailed `Closed.failure`. Assert exact retained Failure and unchanged close error/facts. Trace it through the blocking/RPC consumer to the typed report. Invalid envelopes must retain nothing, and late or duplicate terminals must not replace the first Failure.

**Classification:** **Non-architectural.** One supported terminal branch omits the newly established retention behavior.

## 2. Invariant analysis

The following attacks did not establish additional findings:

- **Shared-clock ordering and equality:** The authority’s change path samples time under its mutex, clamps backward samples and settles active expiry before applying the requested mutation. Equality expires rather than granting a new phase. Prepared expiry preserves the issued identity needed to reject an expired publication even while the active phase remains Alive.
- **Foreign and stale tokens:** The examined token checks bind reservations to their authority and exact phase identity. Invalid acknowledgements and publications do not grant time or transfer a reservation.
- **Network pause/resume:** Departure captures live aggregate and stall remainders. Resume rebases those remainders; local helper activity does not reset network stall. Disabled and Inactive states remain distinct. Repeated local phases capture the current remainder rather than recovering the original allowance.
- **Driver/waiter lifetime:** Registration is bounded. Waiter cancellation and driver loss reach terminal ownership. The reviewed notification collection and custom-waker test support dropping/waking registrations after authority and pool runtime locks are released.
- **Late SSH native success:** The pool’s final installed-clock admission rejects success after the authority has ended. Native completion is not sufficient by itself to reopen the setup. P2-1 concerns the associated Failure’s content, not successful connection admission.
- **Trust and authentication selection:** Host-key trust precedes helper password authentication. Explicit key and publickey/combined server-method paths bypass the password-only helper path. The helper password offer checks Control and the trusted key. P2-4 is the missing disabled-policy input to this selection.
- **Admission ownership:** Endpoint and host helper admissions share retained allocation allowance. SSH captures zero/submillisecond refusal without inventing occupancy. Interaction starts after admission. Pending child and file-worker cleanup retain permit ownership; a cleanup grace miss is not a disposal acknowledgement.
- **Filesystem/crash grammar:** The reviewed configuration-view mechanism creates no named sensitive source copy or durable marker. It therefore adds no filesystem recovery-state grammar requiring directory repair after process death. Discovered source handles are opened nonblocking and checked for regular-file suitability. Initial native Git discovery’s own file reads are expressly subject to the bounded interaction/kill behavior rather than the later preread limits.
- **Configuration bounds and parsing:** Source/count/depth/output ceilings are explicit. Native Git remains the configuration grammar parser. Controlled-empty handling removes inherited parser parameters where required, while ordered filtering and round-trip verification guard the final view.
- **Cleanup and wiping:** Bounded output and scratch buffers avoid ordinary growth, and drop paths wipe owned allocations. Headers use sized zeroizing buffers and redacted Debug. P2-2 is a separate growth hole in the new SSH username conversion.
- **Answer isolation:** HTTPS route ownership separates operation and destination answers. Credential drop retires the opaque pool scope; rejected answers are not reused. Missing-Git latching is operation-local. Controlled-view E2BIG maps to configuration refusal rather than creating the missing-Git latch.
- **Failure admission and duplicates:** Validators enforce the helper timing/detail shape on admitted carriers. The stream ignores messages after its terminal state, and the `Failed` path retains its admitted Failure. P2-5 is the unsupported retention gap within the otherwise admitted `Closed` path.
- **Retry direction:** The examined retry policy excludes helper Allocation/Interaction timeout and authentication failures. No additional retry path was found that grants a fresh helper allowance or converts those failures into success.

These conclusions are bounded source-analysis results. The recorded green gates support the covered paths but do not close the five sequences above.

## 3. Risks and next action

All five blocking causes have bounded implementation remedies within the accepted design. None requires reopening the operator’s helper policy, adding a new clock policy or expanding the public transport schema.

The recorded limitations remain: earlier full suites predate the final consumer patch, retained Clippy warnings are not strict-core green, and native Git/OS copies are outside Rust’s physical-zeroization claim. Windows provider/trust/parity, platform/performance/package/release outcomes, supplied-carrier/iroh expansion and 1.2 sessions remain explicitly deferred; this report makes no acceptance claim for them.

The next action is one bounded remediation revision resolving P2-1 through P2-5, with the specified regressions and refreshed affected gates, followed by a newly frozen tuple for acceptance. The current tuple must not be merged as accepted credential implementation. Exact source acceptance still requires all three independent axes to return GO.

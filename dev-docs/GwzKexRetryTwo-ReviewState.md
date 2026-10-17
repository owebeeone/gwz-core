# gwz-core `c86ef11d` (Windows key-exchange retry raised to two, D32): adversarial review on the STATE axis

- **Object:** gwz-core `c86ef11d`, one commit over `4f4b7e01`, lane `/Volumes/projects/limbo/gwz-dev-d14two`. 8 files, +301/−144.
- **Date:** 2026-10-18
- **Axis:** STATE (retry state machines, clocks and budgets, fail-closed direction, credential-offer boundary, test validity)
- **Requirement:** D14, D22 and D32 in `gwz-core/dev-docs/GwzTransportTR18-OperatorDecisions.md`
- **Verdict: GO.** P0: 0, P1: 0, P2: 0, P3: 6.

## §0 Evidence base

**Tuple.** I checked it at the start and again at the end, and it was the same both times:

| Member | Commit |
|---|---|
| lane root | `bbb39271` |
| gwz-core | `c86ef11d` |
| gwz-transport | `a27817b` |
| gwz-core-evidence | `89c6d0b0` |
| gwz-sspi | `75c1b03` |

The untracked files were also the same both times: the root dev-docs prompts, gwz-core `dev-docs/GwzRemoteTransportBugReport.md`, and the evidence run `2026-09-22-alpha-setup-timeout/`. I modified nothing.

**Code read:**
- `ssh_network.rs` (`establish_inner`, `handshake`, `connect_addresses`, `clean`)
- `ssh_network/kex_retry.rs`, `kex_retry_tests.rs`, `observed.rs`
- `agent_job/control.rs` and `control/{room,shared}.rs` (`mark` and `has_room_since`)
- gwz-transport `pool/setup_clock.rs` and `setup_clock/{state,transitions}.rs`, `pool/setup.rs`, `pool/allocation.rs`
- `ssh_connection.rs` (Drop order)
- `ssh_local.rs` (the only production caller, and where `credential_offered` comes from)
- `setup_retry.rs` (`classify`), `transport_host/session/driver.rs` (`open_backstop_ms`) and `driver/backstop.rs`
- `ssh_tests/{network,host_case,max_startups}.rs`
- libssh2 1.11.1_DEV from libssh2-sys 0.3.3: the `kex.c` and `session.c` paths that return -8

**Evidence checked:**
- Run `2026-10-17-d14two`. The tree digest `07d614e8…` recomputes from `git diff 4f4b7e01 c86ef11d -- . ':(exclude)dev-docs'` to the same value, so the receipt and the loop cover the committed code exactly.
- Receipt `d14two-20261017-a` is PASS. All 14 `kex_retry` rows are `ok` in `hosted-like.log` on Windows.
- In `red1.log`, 8 of 14 rows fail; in `green1.log`, 14 of 14 pass.

**Built and ran locally (macOS):**
- Built with `CARGO_TARGET_DIR=/Volumes/projects/limbo/build-targets/reviewd32-state` and `RUSTFLAGS='--cfg gwz_transport_candidate'`.
- `kex_retry_tests`, `carriage_return_data_matches_native_trust`, `ssh_tests::host_case` and `max_startups`: 20 of 20 pass.
- `kex_retry_tests` run 20 times in a row: 0 failures.
- The target directory is deleted. I did not run `run_r4bg_aggregate_gates.py`.

## Findings

### F1 (P3): rows that end on a real handshake are not deterministic on Windows, and this commit roughly triples that exposure

- **Root cause.** Several rows inject only the first `n` handshake failures and leave the next handshake to the real fixture `sshd`. They then assert exact counts or classes. On WinCNG a real handshake itself ends with -8 about 0.8% of the time (233 of 29,056 in this commit's own loop).
- **Location.** `src/git/endpoint/ssh_network/kex_retry_tests.rs`. Each of these rows has one real handshake with an exact expectation:
  - `a_key_exchange_failure_is_retried_once_on_a_new_connection_and_the_setup_succeeds` (`Times(1)` is used up)
  - `two_key_exchange_failures_then_a_completing_handshake_connect_on_the_third_try`, first half (`Times(2)` is used up)
  - `a_triple_key_exchange_failure_then_a_connection_that_completes_is_one_retry_by_the_row` (expects `tries[1].handshakes == [None]`)
  - `the_classes_of_a_try_count_the_retries_that_cured_a_first_key_exchange_failure`, which has four such handshakes, for `clean`, `recovered`, `recovered-twice` and the trailing `clean`

  That is 7 in total. Before the commit there were 2: the retried-once row and the old double-then-complete row.
- **Violated invariant.** A scripted row must be red or green by the code, not by chance on the platform the rule exists for.
- **Sequence.** On the Windows receipt, the real handshake in `the_classes…` with `failures = 0` returns -8. `Times(2)` retries it, so the class becomes `recovered` where the row asserts `clean`.
- **Impact.** About 1 − 0.992⁷ ≈ 5.5% of Windows receipts show a false red, against about 1.6% before. It is never a false green, the product is not affected, and macOS and Linux are deterministic (OpenSSL backend; 0 of 20 locally). It is bounded, so P3.
- **Correction.** Make the real part deterministic. Either have `Scripted` record each real handshake's answer and rerun the row, up to a bound, only when a real handshake returned -8 beyond the injected ones (the same class-gated approach as `observed.rs`). Or give these rows a policy with one more retry than they inject, and assert the injected count rather than the total.
- **Regression test.** A unit row that injects a "real" -8 after the scripted ones, through a second `Steps` layer. It should show the row reruns rather than fails, and that the rerun happens only on -8.

### F2 (P3): the implementer's per-try budget rule is written into the operator-decision row as if it were D32's

- **Root cause.** The D14 row now reads "(D32: **twice only**; each retry needs more budget left than the try it follows spent)". The D32 row says only "retry twice". The per-try reading of "room" is the implementer's choice; the brief says so. It is a reasonable reading of D14 applied to each retry, but the operator did not decide it.
- **Location.** `dev-docs/GwzTransportTR18-OperatorDecisions.md:57`. The same claim appears in the commit message and in `GwzTransportWindowsParityDesign.md:245`.
- **Violated invariant.** Requirement provenance: an agent's choice must not be attributed to the operator (global rules, "Task scope").
- **Impact.** A later reader cannot tell the operator's rule from the implementer's interpretation. The alternative reading, measuring room against everything spent since the setup began, would make fewer retries. Both readings stay within the attempt's bounds.
- **Correction.** Mark the per-try clause as the implementation's reading of "the rules below hold for each retry" and point to the D32 lane. Or ask the operator to confirm it, and record that in the D32 row.
- **Regression test.** None needed; this is a documentation fix. The behaviour itself is already pinned by the `(2, [3000], 3)` row and by row 1 of `the_retries_together_stay_inside_the_attempts_bounds`. With a setup-start mark, both would give 2 connects.

### F3 (P3): the total handshake count across the retry machine is not stated

- **Root cause.** Inner retries now multiply with outer attempts. A persistent -8 becomes `Io` → `Verdict::Retry`, so it costs (R+1) × 3 handshakes. With `DEFAULT_MAX_RETRIES = 3` that is 12 on Windows, against 4 on Unix (8 under D14). The two inner retries are immediate, with no backoff, and they come before the limit machine sees `Signal::SetupDrop`.
  - libssh2 also returns -8 for deterministic causes: a failed algorithm agreement (`kex.c`, `kex_agree_methods`) and any `exchange_keys` failure, including a host-key signature that does not verify.
- **Location.** The D14 row and `GwzTransportWindowsParityDesign.md:245`. Both state the cost only per attempt ("tried three times … before it is reported").
- **Violated invariant.** "Is the total bounded and stated?" It is bounded (2 × (R+1) extra connections, all inside each attempt's aggregate), but it is not stated.
- **Impact.** A server shedding under MaxStartups sees three times as many immediate re-connections per Windows open before the governor learns of the drop. The operator accepted the per-attempt cost; the product has not been stated.
- **Correction.** Add one sentence: at most 3 × (R+1) handshakes per open on Windows (12 by default), with retries immediate and the SetupDrop signal arriving after the third.
- **Regression test.** Optional. A row that counts connects across the outer machine for a persistent -8 under the platform policy, expecting 3 × (R+1) on Windows and R+1 elsewhere.

### F4 (P3): the evidence README overstates what the budget rule guarantees

- **Root cause.** `2026-10-17-d14two/README.md` §4 says "a retry that cannot finish ends the setup with the original -8 rather than a timeout". The rule only predicts, from the previous try's spend. A retry that takes longer than the previous try still ends as the timeout. The commit's own test row `[3000, 3000, 20000] → TimedOut` shows exactly this.
- **Location.** That README sentence. `kex_retry.rs:92` ("so that a repeat could only run the deadline out") is accurate as a statement of intent.
- **Impact.** It misreads the diagnostics. In that case the user's error says stall or aggregate timeout, and the earlier -8s are not recorded anywhere.
- **Correction.** Reword to "a retry is not started when the last try's spend says it cannot fit; one that runs the bound out anyway ends as that timeout."
- **Regression test.** None needed; the existing row already pins the behaviour.

### F5 (P3, pre-existing): the stall part of the room comparison is untested, and it is a per-wait bound compared with a per-try spend

- **Root cause.** `has_room_since` (`agent_job/control/room.rs:23`) takes the nearer of the aggregate and stall bounds. D14 decided it that way.
  - The stall bound resets on every network progress. The value compared against it is a whole try (connect plus every handshake wait).
  - So with a small `--ssh-timeout` on a high-latency link, every retry is refused even though the stall clock would never end the next try.
  - This fails closed: the -8 is reported and nothing is exceeded.
  - Neither clock's stall branch is exercised. `timed()` uses stall `ZERO`. In `the_shared_setup_clock_gives_room_by_what_the_failed_try_spent`, the stall timer is still `Inactive` because `begin_network_wait` is never called.
- **Impact.** D32 recovers less than it could in that configuration, and the stall branch has no pin.
- **Correction.** Two parts:
  - **Owed now:** rows that pin the stall branch for both the local and the shared clock (a stall-bounded control with progress, then a -8 after a try longer than the stall).
  - **Question for the operator:** keep the stall bound in the comparison, or use only the aggregate. D14 decided it, so this is not a defect of this commit.
- **Regression test.** The two rows named above.

### F6 (P3): doc and checkpoint follow-ups outside the object

- **Stale root checkpoint.** The root `dev-docs/CurrentProgramCheckpoint.md:57` still says "D14 (retry WinCNG's -8 key exchange once on Windows…)". Line 15 still lists the D14 residual as open for the operator, although D32 decided it. This is outside the commit (it is the gwz-dev root), but it is the last text still promising one retry.
- **Broad absorption in `host_case.rs`.** It still reruns on any `ErrorKind::Other`, which hides every libssh2 failure class, not just a triple -8. Only the comment changed in this commit. Moving it to the `observed.rs` triple gate would match the carriage-return row.

## Attacks that failed

- **Aggregate, stall, setup and backstop bounds.**
  - Every try uses the attempt's one `Control`. `connect_addresses`, `connect`, the handshake loop and every `wait_step` call `check()`, so no sequence runs past the aggregate.
  - A failed handshake does not restart the stall clock. In the shared clock, `begin_network_wait` only starts it from `Inactive`, and `network_progress` is the only reset. The local `begin_wait` and `complete_wait` behave the same way.
  - The production network aggregate is the pool's `connect_timeout_ms` (30 s; `session.rs:475`), well inside `OPEN_ATTEMPT_MS = 150 s` in `open_backstop_ms`. The backstop is not affected.
  - Within the limits of F4 and F5, the per-try rule cannot widen any bound.
- **A misleading timeout instead of the -8.** A retry is refused when the room left is no more than the last try's spend. An expiry seen at the `check()` right after `steps.handshake` reports the timeout, which matches D14's "an expired setup reports that". The case where the next try runs out the bound is a true timeout (F4). It is not a corruption of the -8.
- **Cancellation.**
  - Local clock: `has_room_since` ignores `state.failure`. The next try's first `check()` then returns the cancellation, so the cancellation is reported rather than the -8.
  - Shared clock: a terminal clock makes `mark` fall back to 0 and `has_room_since` return false. The pool's arbitration through the clock's terminal record decides the reported cause.
  - Neither path changed in this commit.
- **Credential boundary.**
  - In `ssh_local.rs`, `establish_written` runs before every authentication step: the URL password (`ssh_password::authenticate`), the selected key (`ssh_key_auth`), the helper (`password_only` and `helpers.lookup`), the Pageant mark and the agent (`agent_authenticate`).
  - `registry.lookup` before it only pins an already admitted snapshot.
  - `session.handshake()` is libssh2's banner, kex and service request. It sends no user name and no credential, and -8 comes only from kex (`session.c:785–792`).
  - The `credential_offered` closure reads the open's `progress`. If an earlier attempt left it true, the retry is refused, which fails closed.
  - Even a -8 caused by a host-key signature that does not verify, which a hostile server could produce, gets only a new KEXINIT. Nothing is exposed.
- **Only on -8.** The `KEX_FAILURE == Session(-8)` match is exact. `only_a_key_exchange_failure_is_retried` covers -2, -5, -7, -9, -13 and -43 under both `Times(1)` and `Times(2)`.
- **Resource lifetimes.**
  - `handshake` owns the socket; `known` is declared after `connection`, so it drops first.
  - `SshConnection::drop` shuts down both socket handles before `Session`'s destructor runs `libssh2_session_free`. A session with no channels has nothing to send, so it does not stall in nonblocking mode.
  - All of this happens synchronously before the loop's next `connect_addresses`. Each try therefore has its own TCP connection and its own session, including the libssh2 and WinCNG state.
  - Per-failure native cleanup is the same as under D14; this commit only adds tries.
- **The retry machine sees one attempt.** `a_retried_key_exchange_failure_is_one_setup_attempt` passes with `Times(2)` and 3 connects. `record_tcp_connect` keeps the last try's connect time, which matches the governor's single settle reading. The `(N of M)` count is still the endpoint's own outer count, so it is correct.
- **Counter arithmetic.** `retried: u8 < allowed: u8` before each increment cannot overflow. `Times(0)` behaves like `Never`.
- **The carriage-return residual class.** Absorbing only `[-8, -8, -8]` is right. A double, which can only be a retry the budget refused or a policy below 2, and any mixed sequence both end the row with the codes printed. This is exactly the class D32 leaves behind. The loop shows 0 triple, 0 other and 0 refused in 29,056 establishes; 3 were cured by the second retry. Under independence about 0.015 triples would be expected, so their absence is consistent with D32 but does not by itself prove it.
- **Red before and green after.** `red1.log` has 8 of 14 failing on the intermediate state; green is 14 of 14 on macOS (also reproduced locally) and on Windows. The per-try mark is not shown red on its own in `red1.log` (the loop stopped at an earlier row), but rows `(2, [3000], 3)` and `[3000] → 3` pin it, as worked out in F2.
- **`observed.rs` is test-only.** It is declared `pub(crate) mod observed` inside `cfg_if! { if #[cfg(test)] }` at `ssh_network.rs:567–571`, and `ssh_tests` is `cfg(test)` (`mod.rs:79`). There are no production references.
- **Repository rules.**
  - Every new control-flow body is braced.
  - Conditional code uses only `cfg_if` (`PLATFORM`, `PLATFORM_CONNECTS`); there is no `#[cfg]` or `cfg_attr` on items in the diff.
  - The diff adds no statics, thread-locals or `Once*`.
  - Largest touched files: `kex_retry_tests.rs` is 643 lines and `ssh_network.rs` is 596, both under 1000.
  - The Windows warning that `Never` is never constructed was already there, since `Never` is only constructed in the Unix arm.
- **Doc consistency inside the object.** Every touched "once" text in the D14 row, the design §9 paragraph, the `kex_retry.rs` and `observed.rs` module docs, and the `host_case` and `network.rs` comments now cites D32 and says two or three. The one remaining "once" is in the root checkpoint (F6).

## Follow-ups (none blocks this object)

1. F1: make the real-handshake rows deterministic on Windows.
2. F5: rows for the stall branch of `has_room_since` on both clocks, and the operator question about the stall bound.
3. F3: state 3 × (R+1) and the immediate retries in the D14 row and the design §9.
4. F2: provenance wording for the per-try rule, or operator confirmation.
5. F4: README §4 wording.
6. F6: root checkpoint refresh; move `host_case.rs` to the triple gate.
7. A diagnosability follow-up, not a defect of this commit. `clean()` reduces the -8 to a bare `ErrorKind::Other`, so the user's final error names neither the -8 nor the inner tries. There is no production record that inner retries happened. Consider a note through the handoff, like `EndNote`, so a field report of "attempt 4 of 4" on Windows can show the 12 handshakes.

## Next action

GO on the STATE axis. Merge the verdict with the peer reviewer's. F1 and F2 are the cheapest to close in a follow-up commit on this lane; the rest can be tracked for the D32 lane.

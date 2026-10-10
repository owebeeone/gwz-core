# Limit wiring review (Code+State), round 2

Verified tuple (start and end): gwz-transport `77f89cdf20da6e69ab5bc91fd609918dbdcb6f04`, clean; gwz-core HEAD `fb5ad0b563cead3e6ebd8f2385bb112e2b6800b9` with `git -C gwz-core diff` sha256 `32b5639b6863c080ff21520858950c21e9b8bc253b10c1ba9bae4db2d2eed916` (matches the snapshot), plus the out-of-scope untracked `dev-docs/GwzRemoteTransportBugReport.md`.

Tests run on a `cp -RL` copy of the patched candidate tree in scratch, built with `--cfg gwz_transport_candidate`, filters `governor`, `limit_tests`, `throttle`, `events_tests`, `control_pool`, `states_tests`, `placement_endpoint::retry_tests`, `https_endpoint::retry_tests`, `setup_slot_tests`: 80 of the patch's tests passed; one added probe failed (P2-3). `check_windows_parity.py`: unported 210, nothing new.

## Closure table

| Finding | Status | Evidence |
| --- | --- | --- |
| P2-1 | Closed | `placement_endpoint.rs:182-187`: ceiling = `min(per_host, per_user_host)`; `open_ceiling` only in `admits_open`. Test `in_saturated_the_pool_site_limit_is_the_pools_own_cap_not_the_open_ceiling` passes. |
| P2-2 | Closed (new defect P2-3) | `governor.rs:154-158` returns early while another operation is live. Tests `a_second_live_operation_does_not_erase_a_hold` and endpoint test `a_second_request_does_not_erase_the_hold_a_live_request_set` pass. |
| P3-1 | Closed | Year bounded to 1970..=9999, checked arithmetic (`throttle.rs:85-107`); huge year and `u64::MAX` give `None`. |
| P3-2 | Closed | `control.rs:452-460` sets the barrier only when adaptive; `without_adaptation_an_inconclusive_refusal_sets_no_barrier` passes. |
| P3-3 | Closed | `states.rs:127-130` (SettingUp, ServerClosed) → Gone; `answered` always promotes; `exchange_begins` first ends the connection's own unanswered setup attempt. Two new governor tests pass. |
| P3-4 | Closed | `prepare.rs:322-350` returns the lease, waits in `wait_for_hold`, then `continue`; `a_hold_set_between_admission_and_lease_keeps_the_exchange_from_the_server` passes. |
| Lane gate | Closed | Parity guard: unported 210, nothing new. |

### Checks on the fixes

- **`end_operation` coverage.** It is called only from the two `cancel_request` paths (`placement_endpoint/cancel.rs:29`, `https_endpoint.rs:420`). Every request exit reaches them through the endpoint session's `Session::cancel`: `ClientRequest::finish` and `Session::finish` → `seal` → `cancel`; `ClientRequest::Drop`, including an early `?` after `set_max_retries` at `mod.rs:418`, which drops `client_guard`; `TransportRequest::finish` and `Drop` via `local_registration`; `HttpsEndpoint::shutdown`. Request ids are never reused within a session (`state.used`), and a repeated `end_operation` does nothing. No stale live operation remains.
- **`wait_for_hold` (`throttle.rs:122-139`).** Termination: each poll calls `exchange_may_begin`, which runs `sync`, which performs the long-hold discard itself, so the wait ends at the hold's expiry (at most 30 s per server hold). Cancellation is handled in the `select!`. No spin: `exchange_begins` refuses only for a hold, and `result(Ended)` closes the connection's earlier window before `begin`, so there is no tight re-lease loop. Minor: the native `logical_deadline` is not checked during the wait, so it can be overrun by up to one hold and is then reported as `Timeout` at the loop top. Bounded; not raised as a finding.
- **Carried-challenge exemption.** A carried lease is the credentialed retry of a discovery the server already answered with 401, so the member is past its first exchange (§4.5). It proceeds without a window, and a 429 on it still sets the hold through the no-attempt path. Safe.

## New findings

### P2-3 — A slot kept across the reset keeps the previous operation's ceiling and stale pool numbers for the whole next operation
- **Root cause:** `begin_operation` (`governor.rs:160-172`) keeps a holding slot whole: its `Limit` was built with the old ceiling, and its N, table, windows and `applied` cache are all kept. The new `ceiling` reaches only new slots through `book.ceiling`. The slot is never rebuilt after its hold lapses, and because `applied` still matches what the slot wants, `sync` never re-sends numbers to the pool, even after the pool's `install_capacity` cleared them. The test `a_hold_still_in_force_outlives_its_operation_but_the_rest_of_its_state_does_not` claims the opposite and passes only because both operations use ceiling 8.
- **Violated:** §4.1 (C is "the pool's `min(per_host, per_user_host)` for the operation"); the shipped claim that behaviour is unchanged except for Retry-After holds; P2-2's plan line ("only the hold survives").
- **Reproduction** (added to a scratch copy, run on the patched tree):
  1. `Rig::new(32, false)`, then `begin_operation("a", 4, …)`.
  2. Open a connection; `refused(…, Some(20_000), …)` sets a hold; `end_operation("a")`.
  3. `begin_operation("b", 32, …)` at t=200, then `admission(key, 25_000)`.
  4. Result: target = 4 and pool limit = Some(4), where 32 was expected. The test fails.

  In production: a request with `--max-per-host 4` gets a 429 with `Retry-After` on H; within 30 s a request with per_host 32 starts on the same session (e.g. a long-lived gwz-py runtime); `install_capacity` clears the pool limit, the governor keeps `applied = (4, 0)`, `admission.target` stays 4, and `admits_attempt`/`admits_open` cap that whole operation on H at 4 opens in flight long after the hold ends.
- **Impact:** a parity regression. One request's limit leaks into a later request's for its entire duration, and the pool's numbers and the governor's view disagree (diagnosability).
- **Correction:** on reset keep only the `Hold`: move it into a fresh `Limit::new(new ceiling, new adaptive, …)` with `applied = None`, or drop the kept slot once its hold lapses. Either way set `applied = None` on any kept slot so the pool's numbers are re-sent after `install_capacity`.
- **Test:** the probe above. Hold under ceiling 4, end, begin with 32; assert `admission.target == 32` after the hold, `pool.limit(site)` is 32 or `None`, and the gate stays closed until the hold ends. Rename or strengthen the existing test to use different ceilings.

### Note: the shared-operation path
While another operation is live, `begin_operation` does not update `book.ceiling` or `book.adaptive`. Overlapping requests always have the same capacity (a different one is refused with `TransportCapacityConflict`, `session/capacity.rs:78-95`) and adaptive is always false in this change, so nothing goes wrong today. Per-operation scoping, deferred to the adaptive step, must settle this.

Verdict: NO-GO

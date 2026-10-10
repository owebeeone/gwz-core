# Adaptive step review (Code+State), round 2

**Verified tuple (start and end):** gwz-core `0d7b3588a220dde73c458956e791356976886cc6` on top of `e043d952`, CI pin `.github/gwz-transport.commit` at `6146bd5`; gwz-transport `6146bd581fe3dbb8ba6a4716611c1dc8bf4246ad`. Both trees clean apart from the out-of-scope untracked `dev-docs/GwzRemoteTransportBugReport.md`.

**How it was tested.** A fresh candidate tree, copied with `cp -RL` to `/Volumes/projects/limbo/build-targets/review-adaptive-2/cand`, with build directories under that path. The eight `review_probe_*` tests were ported to the new interfaces (`start_test` returns a token; `setup_failed` takes the member; the rig's requests are tagged). All 8 probes pass (`probes.log`). Focused filters: `setup_retry` 172; `placement_endpoint` 32; `https_endpoint` 33; `https_pool` 8; `ssh_worker` 4; `ssh_pool` 7; `background_close` 20; `https_worker` 112 with the 2 known SSPI environment failures (the target directory sits beside the source); gwz-transport `pool_tag`, `pool_settle`, `pool_limit` 28.

**The gwz-transport API change.** Every in-tree destructure of `Action::Connect` already uses `..` (`tests/transport_consumer/tests/pool_host.rs`, `ssh_setup_context/tests.rs`, gwz-transport's own tests); the one exhaustive match, `ssh_pool.rs:466`, is updated. No in-tree code builds a `pool::Request` as a struct literal.

## Closure table

| Finding | Status | Evidence |
| --- | --- | --- |
| P2-1 | Closed | A new operation starts from a copy of the governor's per-site table (`governor.rs:256-287`). PROBE1: `b` sees 22 connected; its Overload is to 22, not 2. Copy semantics: after seeding, every event reaches both the site table and every slot; a slot seeded inside an event loop gets that event a second time, harmless because repeated transitions change nothing. No path updates one table and not the other. |
| P2-2 | Closed | `started` gives the armed test only to a connection tagged with the arming operation (`governor.rs:555-560`). PROBE2: `b`'s connection does not take the test, and the operation's gate reopens once its own carrier is answered. Tag lifetimes: an untagged connection (including `None` for a cancelled request) belongs to no operation, takes no test, and its unreported failure is swept after 1 s with no verdict; the test is given back when the carrier's token drops. |
| P2-3 | Closed | A fresh HTTPS answer counts as every live operation's success (`scoped.rs:123-136`). PROBE7: n = 7, and a test is armable at 8. |
| P2-4 | Closed | A failed setup is judged per connection through its tag (`scoped.rs:203-237`, stages Live, Ended, Judged, Up); the FIFO queue is gone. A report arriving before the host sees the connect end is judged at once and its window frozen. PROBE3 gives `Overload{n:3}`. |
| P2-5 | Closed | `TestToken` gives the test back only if the same arming is still pending (it compares the arming id), so a late drop is a no-op once the test was taken or re-armed. Tokens are dropped outside the governor lock on every exit (SSH `start_attempt` errors, HTTPS allocation expiry, a cancelled entry). PROBE8: the gate reopens and the test can be armed again. |
| P2-6 | Closed | HTTPS `Outcome::Retry` applies the member's budget (`retry.rs:161-171`). PROBE4: no member exceeds 3 of 3. |
| P2-7 | Closed | `Book::slot` returns `None` for an operation that is not live, so a late report revives nothing. PROBE6: no zombie scope; the pool limit stays at 3. |
| P2-8 | Closed | A lone throttle without `Retry-After` holds the site for T0, for its own operation only. PROBE5: 4 requests over 1.59 s. Starvation check: the hold fires only at hi = 0 (no other connection in the table, or no attempt in flight) and lasts 500 ms per throttle, so it is bounded backoff, not starvation. |
| P3-1 | Closed (new P3-B) | All three hooks are wired: `set_no_evict` from `closes_suppressed`; the `carrier` flag that skips the claim-closing deferral (`runner.rs:197-203`); connect time feeding Ts. Stranding check: `no_evict` is recomputed on every sync, cleared in `reconcile` when the last operation on the site ends, and cleared by `install_capacity`. Across operations it is ORed, but that is self-limiting: a blocked eviction means a full site, so no new setups start and the key becomes quiet. |
| P3-2 | Closed | With `--max-retries 0` a Suspect goes to the retry machine (`filter.rs:77-81`); covered by `p3_2_at_max_retries_zero_a_dead_host_costs_one_wave_not_one_timeout_per_member`. |

## New findings

**P3-A — Ts is measured from the whole setup, not from the TCP connect (not architectural).**
- **Root cause:** `connect_ms` runs from `Started` to setup complete (`governor.rs:601-603`, `scoped.rs:114-117`): for SSH after authentication, for HTTPS after the first exchange is answered.
- **Violated:** §4.1 defines Ts as `max(250 ms, 2 x SRTT + 100 ms)`, with SRTT "the smoothed TCP connect time".
- **Sequence:** an SSH login of about 0.7 s gives Ts ≈ 1.5 s; an HTTPS discovery answered in 2 s gives Ts ≈ 4.1 s. The implementer's own test `p3_1_the_settle_time_follows_the_connect_times_the_machine_measured` pins this (an answer 400 ms after the connect gives 900 ms).
- **Impact:** after any refusal, settle holds, inconclusive-refusal barriers and quiet waits are 3 to 10 times the design's intent, slowing STABLE operation and delaying tests. Bounded, and only after a refusal.
- **Correction:** measure TCP connect completion (a host event), or take SRTT only from that stage.
- **Test:** an SSH fixture with a slow login; Ts must track the TCP connect time, not the login time.

**P3-B — Two closure tests the plan promised are missing (not architectural).**
- **Root cause:** no endpoint-level test for each carrier exit that drops the token (P2-5's plan row), and no test that a carrier is never deferred by the background close (P3-1's row); `ssh_worker_tests.rs` only adds `carrier: false`.
- **Impact:** removing the `!request.carrier &&` guard at `runner.rs:200`, or a future exit that leaks the token, would pass the suite.
- **Correction and test:** an SSH worker test where a carrier open meets a closing connection of the same key and identity and must check out at once, not be deferred; placement and HTTPS endpoint tests driving each named exit (`selected_path` error, `start_endpoint_open` error, allocation expiry), asserting the gate reopens.

Verdict: GO

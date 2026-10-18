# Limit wiring review (Code+State)

Verified tuple (start and end): gwz-transport HEAD `77f89cdf20da6e69ab5bc91fd609918dbdcb6f04`, clean; gwz-core HEAD `fb5ad0b563cead3e6ebd8f2385bb112e2b6800b9`, clean apart from the out-of-scope untracked `dev-docs/GwzRemoteTransportBugReport.md`.

Tests run: gwz-transport `pool_settle`, `pool_limit`, `pool_discard_idle` (30 passed); gwz-core candidate lib (`--cfg gwz_transport_candidate`) filtered to `governor`, `limit_tests`, `throttle`, `events_tests`, `control_pool` (37 passed); a standalone build of `http_date` to reproduce the overflow. Build output in scratch.

What holds: lock order host → governor → pool (the pool wakes wakers outside its lock, `asynchronous.rs:25-52`; the governor never calls back into a host; the session-state → host/governor edges add no cycle), so no deadlock was found. F10 holds within one pool: `Started` precedes any lease; the worker calls `answered`/`refused` before releasing the lease, so `Closing` follows them; the host reports `Closing` for Close/Abort only, and cancelled connects become `Retired`. The pool's evictor rule, hold lapse via `next_deadline`/`advance`, `discard_idle` against leases (atomic under the pool lock), and `stop`/`install_capacity` clearing holds are all legal; no stranded hold or orphaned evictor. With `adaptive=false`, N never leaves SATURATED, so settle is 0 and production holds are unreachable for now. The gwz-transport API additions are additive.

## Findings

### P2-1 — In SATURATED the SSH site limit is clamped to `open_ceiling` (32), capping connections tighter than 1.0.17
- **Root cause:** `PlacementEndpoint::set_max_retries` builds the ceiling as `per_host.min(per_user_host).min(open_ceiling(capacity))` (`gwz-core/src/git/endpoint/placement_endpoint.rs:182-186`), and the governor sends it to the pool as `set_limit(site, C)` (`governor.rs` `sync`). `open_ceiling` bounds opens in flight: opens leave `self.opens` at `Opened` (`completion.rs:135`) while their streams still hold leases. The pool's site limit counts every connection (opening, idle, leased, closing).
- **Violated:** §4.5, SATURATED admits "below C (the pool's own rule today)"; the change ships as "behaviour unchanged except Retry-After holds"; §11 parity with 1.0.17.
- **Sequence:** `--max-per-host 64 -j 64` (`resolve_per_host` does not clamp) gives `per_host = per_user_host = 64` and `open_ceiling = min(256, 1024, 64, 32) = 32`, so C = 32 and the pool gets `set_limit(ssh github.com:22, 32)`. 32 opens complete and stream on 32 leases. The 33rd open passes `admits_open` (no opens in flight) but waits in the pool because `site_full` (32 ≥ 32); before this change it opened, since per_host was 64. Closing connections now also block new opens.
- **Impact:** SSH per-host concurrency silently halved for any `per_host > 32`, with more evictions and idle churn. A behavioural regression with adaptive off.
- **Correction:** set the pool site limit to the pool's own cap, `min(per_host, per_user_host)`, and keep `open_ceiling` only in the endpoint's opens-in-flight rule. Alternatively, clear the site limit while SATURATED. Either way, record in the design that the §4.1 C used for the pool excludes `open_ceiling`.
- **Test:** placement endpoint with `per_host = 64`; after `set_max_retries`, assert `pool.limit(site)` is 64 or `None`; hold 33 leases on one site and assert a 34th checkout opens.

### P2-2 — A concurrent operation's `begin_operation` erases a live Retry-After hold
- **Root cause:** `Governor::begin_operation` (`governor.rs:139-149`) drains every slot (holds, windows, `leased`, table, N). It runs on every request's `set_max_retries`: `HttpsEndpoint::set_max_retries` (`https_endpoint.rs:180-190`) and `placement_endpoint.rs:174-187`. Same-capacity requests may overlap on one session's pools (`session/capacity.rs:78-95`, reused path `:141-143`).
- **Violated:** §4.5 ("any 429 or 503 carrying `Retry-After` sets the key's hold" until it expires); §4.1 (machines are per operation and dropped with that operation's retry machines, not with another operation's start).
- **Sequence:**
  1. Request A is live on HTTPS host H. Member a1's discovery gets 429 with `Retry-After: 20`, so slot(H) holds until t+20 s and A's held opens are gated.
  2. Request B, same capacity, is admitted. `admit_client_request` finds the capacity reused, then `set_max_retries(B)` calls `begin_operation`, which drains the slots.
  3. On the next `step`, `start_held` → `admission(H)` creates a fresh SATURATED slot with the gate open, and A's opens start inside the server's Retry-After.
- **Impact:** the one behaviour this change ships is lost under overlapping requests. A's in-flight attempts are forgotten, and in the adaptive step N would reset mid-operation.
- **Correction:** do not reset slots for other live operations: either scope machines per operation as designed (with one pool-number reconciler), or make `begin_operation` reset only when no other operation is registered and keep any hold still in force.
- **Test:** governor; set a hold via `refused(..., Some(20_000), ...)`; call `begin_operation` as a second live operation would; assert `admission(key).gate_open == false` until 20 s. Add an endpoint-level variant with two overlapping requests.

### P3-1 — A hostile HTTP-date `Retry-After` overflows (panic in debug/test, wraps in release)
- **Location:** `https_worker/throttle.rs` `http_date`: the year is an unbounded `u64`; `era * 146_097` and `days * 86_400` overflow (`:96-99`).
- **Reproduction:** `Retry-After: Sun, 06 Nov 1000000000000 08:49:37 GMT`. With overflow checks on it panics ("attempt to multiply with overflow", reproduced) in the prepare task, which becomes a JoinError → `EndpointError::Protocol` (`poll.rs:26`) and closes the session. In release it wraps (13110207864150044961) and the 30 s cap bounds the hold.
- **Correction:** bound the year (e.g. `1970..=9999`) or use checked arithmetic returning `None`.
- **Test:** add a huge year and `u64::MAX` to `an_unreadable_retry_after_is_none...`, expecting `None`.

### P3-2 — A refusal without Retry-After can now delay starts (the barrier); the "unchanged except holds" claim omits it
- **Location:** `control.rs:449-451` with `filter.rs` `evidence`.
- **Mechanism:** with `adaptive=false`, an ordinary refusal with `hi ≥ C` is still `Inconclusive`, so it sets a barrier and `gate_open` stays false until the window's winding-down connections settle (Closing time + ≥250 ms).
- **Sequence:** HTTPS with `--max-per-host 8` (the CLI default in one mode, `globalargs/transport.rs:183`); a bare 429 arrives while 8 others were possible; the next opens wait out the closes plus `Ts`.
- **Impact:** a bounded new delay that the commit's claim omits. Design-sanctioned (§4.4 "in SATURATED too").
- **Correction:** state it in the commit or release notes, or skip the barrier when `!adaptive`.
- **Test:** non-adaptive governor, bare 429 with `hi ≥ C`, assert the gate state.

### P3-3 — HTTPS connections can stay "Setting up" in the governor table forever (latent for the adaptive step)
- **Root cause:** HTTPS reaches Connected only through `answered()` on a discovery in `prepare.rs:421-426`. Gaps:
  - a fresh connection whose exchanges are POST-only (`serve`) is never answered;
  - `answered` on a leased exchange does not promote a connection still in SettingUp (`governor.rs:204-208`, `fresh=false`);
  - the table has no `(SettingUp, ServerClosed)` transition (`states.rs:126`), so keep-alive idle loss (reported at `https_connection.rs:508`) leaves the entry SettingUp with its Started window open.
- **Impact:** inflated `hi` (more Inconclusive rulings and barriers); `is_quiet()` and `setups_in_flight` never resolve. Once adaptive is on, probes never become ready and overload notes never flush.
- **Correction:** promote on any answered exchange; treat `(SettingUp, ServerClosed)` as Gone.
- **Test:** HTTPS Started → lease returned without an answer → `idle_closed`; assert `view.possible == 0` and the key is quiet.

### P3-4 — `exchange_begins`'s refusal is ignored
- **Location:** `prepare.rs:327-331` discards the bool.
- **Sequence:** an attempt is admitted at t0; another member's 429 sets a hold at t1; the attempt leases an idle connection at t2; `exchange_begins` returns false; the discovery is sent anyway.
- **Violated:** §4.5, "no open begins its first exchange on a leased connection" during a hold. The governor test at `governor_tests.rs:306` asserts the rule, but production does not honour it.
- **Impact:** a bounded race that risks a further 429 extending the hold.
- **Correction:** when the call returns false, wait on the hold, or release the lease and requeue.
- **Test:** set a hold between admission and lease; assert no request reaches the fixture until the hold ends.

## Implementer readings

Safe as written:
- a Retry-After with nothing in flight sets the hold without judging;
- an HTTP-date without `Date` takes the 30 s cap;
- a bare 503 is unanswered (the dropped lease is Discarded, which closes the window);
- a cancelled connect leaves a settle hold, a refusal or idle loss none;
- `per_host` still counts across ports while the limit counts per site (F16);
- a throttled discovery's connection is discarded with no verdict.

Exception: "limits and holds belong to one operation" is not safe, because concurrent operations exist (P2-2).

Verdict: NO-GO

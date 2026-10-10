# Limit wiring: remediation plan (round 1)

Object: gwz-transport `77f89cd`, gwz-core `fb5ad0b5`. Review: `GwzTransportLimitWiring-ReviewCodeState.md` (NO-GO). One merged patch; the same reviewer re-verdicts.

| Finding | Disposition | Closure test |
| --- | --- | --- |
| P2-1 | The pool's site limit in SATURATED is the pool's own cap, `min(per_host, per_user_host)`. `open_ceiling` stays only in the endpoint's opens-in-flight rule. The design's §4.1 C for the pool excludes `open_ceiling` (recorded in the design's changelog). | Placement endpoint with `per_host = 64`: `pool.limit(site)` is 64 (or `None`), and with 33 leases held on one site a 34th checkout opens. |
| P2-2 | `begin_operation` resets the slots only when no other operation is live on the governor, and never drops a hold still in force. Scoping machines per operation is left to the adaptive step. | Governor: set a hold of 20 s, then `begin_operation` as a second live operation; the gate stays closed until 20 s. Plus an endpoint variant with two overlapping requests. |
| P3-1 | The HTTP-date parser bounds the year to 1970..=9999 and uses checked arithmetic, returning `None` on overflow. | Huge year and `u64::MAX` give `None`. |
| P3-2 | No barrier while `!adaptive`, so the shipped claim "unchanged except Retry-After holds" is exact. The barrier returns with the adaptive switch. | Non-adaptive governor, bare 429 with `hi ≥ C`: the gate stays open. |
| P3-3 | Any answered exchange promotes a connection still in SettingUp to Connected. `(SettingUp, ServerClosed)` is Gone. | HTTPS Started, lease returned unanswered, `idle_closed`: `possible == 0` and the key is quiet. |
| P3-4 | When `exchange_begins` refuses, the lease goes back unused and the open waits for the hold to end before it leases again. | Hold set between admission and lease: no request reaches the fixture until the hold ends. |
| Lane gate | The Windows-parity count must not rise above origin/main's 210. The throttle parse tests become portable. Tests that need the TLS fixture join an existing 0.5b-owned gated module rather than adding new gates. | `check_lane_commits.sh` parity step and `check_windows_parity.py --shrink-from` against origin/main pass. |

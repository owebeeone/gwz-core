# Limit wiring: remediation plan (round 2)

Review: `GwzTransportLimitWiring-ReviewCodeState-2.md` (NO-GO, one new P2; round-1 findings all closed). This is the last remediation round under the two-round cap. P2-3 is not architectural: it is a correction to the P2-2 fix.

| Finding | Disposition | Closure test |
| --- | --- | --- |
| P2-3 | On reset, a slot whose hold is still in force keeps only its `Hold`. The hold moves into a fresh `Limit` built with the new operation's ceiling and adaptive flag, and the slot's `applied` is cleared so the pool's numbers are re-sent after `install_capacity`. The existing test is strengthened to use different ceilings. | The reviewer's probe: hold set under ceiling 4, end, begin with 32. After the hold, `admission.target == 32` and `pool.limit(site)` is 32 (or `None`); the gate stays closed until the hold ends. |
| Note: shared-operation path | Recorded in the design's changelog as a constraint for the adaptive step's per-operation scoping. No code change now: overlapping requests share one capacity, and adaptive is off. | None. |

# Limit wiring review (Code+State), round 3

Verified tuple (start and end): gwz-transport `77f89cdf20da6e69ab5bc91fd609918dbdcb6f04`, clean; gwz-core HEAD `fb5ad0b563cead3e6ebd8f2385bb112e2b6800b9` with `git -C gwz-core diff` sha256 `b6a312b3446baaa6bac2e1d5cc3a8191489f7312f7c14814bede709e374c1dca` (matches the snapshot). Compared with the round-2 patch, the code changes are only in `governor.rs`, `control.rs` (`take_hold`, `restore_hold`) and the tests, plus a design changelog line.

**P2-3: closed.** `begin_operation` (`governor.rs`) now rebuilds a held slot: a fresh `Limit::new` at the new ceiling and adaptive flag, the `Hold` moved across, an empty `leased` set, and `applied = None`. The original probe, run unchanged on a `cp -RL` copy of the patched candidate tree, now gives target 32, pool limit `Some(32)` and the gate still closed at 20 099 after `begin_operation("b", 32)`; round 2 gave target 4 and `Some(4)`. The implementer's two strengthened tests (ceilings 4 then 32, with an `install_capacity` between) pass. Focused filters: 83 passed, 0 failed.

**Regression check on the fix: no new findings.**
- A long hold kept across a reset still discards idle connections and lifts. Probe added: 5 s Retry-After, the operation ends, a new one begins, and the gate opens at 5 200. Passes.
- The hold move is complete: `Hold` is a `#[derive(Default)]` struct and the move carries its state, including `long` and `discard_asked`, so `take_hold` leaves nothing behind.
- Events for the old connections arriving at the rebuilt slot are harmless: the table does not know them and ignores them, as it already did for reset slots.
- No double application: the pool's numbers are sent again on the next `sync`.

The shared-operation constraint is now in the design's changelog: while another operation is live, the ceiling and the adaptive flag are left as they are. That is safe today (overlapping requests share one capacity; adaptive is off). Not a finding and not architectural; the adaptive step must settle it.

Verdict: GO

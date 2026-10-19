# Push producer-busy follow-ups (gwz-core f69b1112) — STATE-AXIS RE-VERDICT (round 2)

**Review object:** gwz-core `f69b1112` (full sha `f69b111275526c5a8a372a08b4a06a7c6bf06123`), one commit over `86bb198c`: 7 files, +295/−17. Evidence: run `2026-10-18-blobfix` at gwz-core-evidence `c98e6276` ("Run 2026-10-18-blobfix addendum…"). Lane `/Volumes/projects/limbo/gwz-dev-blobfix`.
**Baseline:** `86bb198c` (implementation, State GO, round 1, `GwzPushProducerBusy-ReviewState.md`).
**Date:** 2026-10-19
**Axis:** STATE (round-1 closures, new-row validity and determinism, the fail-closed direction, evidence integrity).

**Verdict: GO** — 0 P0, 0 P1, 0 P2, 3 P3 (new). Round 1's findings: P3-1, P3-2, P3-3 and P3-5 are closed. P3-4 is closed for the items it named, and the §7 drift left over is filed as N1.

## 0. Evidence base

**Tuple at start:**
- gwz-core HEAD is `f69b1112…06123`, parent `86bb198c`. Its only untracked file is `dev-docs/GwzRemoteTransportBugReport.md`, which is out of scope.
- gwz-core-evidence HEAD is `c98e627659691014256d0947a8c3e6233d2864b7`, the addendum commit. Its only untracked item is `2026-09-22-alpha-setup-timeout/`, which is out of scope.

I modified nothing and ran no cargo.

**Read:**
- `git diff 86bb198c f69b1112` in full.
- `ssh_pump.rs:207-208` (where `pack_sent` is set) and `:535-543` (`producing()`).
- `stream_io.rs:18`, `:57-62` and `:112-161`: `PRODUCER_CEILING_MS`, `busy`, `enter`, and the `_call` guard on `end_write`, `close` and the I/O methods.
- `session.rs:180`, `:357-375` and `:502`, and `driver/opening.rs:256`. The default flows `PRODUCER_CEILING_MS` → `Session::new` → `ProducerGate::new(state.producer_ceiling_ms)`.
- `ssh_handoff.rs:303-333`.
- `ssh_pump_clock_tests/producer.rs`: the helpers and the two tracker rows.
- `stream_io_tests.rs:87-107` (`gated`, `blocked_in`) and the two new gate rows.
- `producer_busy_tests.rs` in full.
- `blob_push_tests.rs:1-24` and `:82-98`.
- §10.1 of `GwzRemoteTransportDesign.md:698-730`.
- The design at `f69b1112`.
- D42 (`GwzTransportTR18-OperatorDecisions.md:85`).
- D45, which is not in the lane: see §3.

**Evidence run:**
- README diff.
- `raw/addendum/{addendum-mutations.log, build-digest.txt, gate-*.log}`.
- `raw/trigger/final/*.{env,log}`.
- `raw/blobpush-2026-10-17-ssh-3072-default.red.{env,log}`.
- `raw/windows/addendum-b/*`.
- `runner/round2/*`.

**Commands run:**
- `python3.13 -B tools/verify_archive.py` from the evidence root: "Verified 5553 migration records; no build/cache directories in archive".
- An independent manifest check. Every file under the run, except `manifest.json` itself, is in `manifest.json`; every entry exists; every sha256 matches. No mismatches.

**Digests recomputed** from the commits with the format of `runner/round2/digest.sh`:
- `git diff 86bb198c f69b1112 -- . ':!dev-docs' | shasum -a 256` gives `624191ac270d7569d9f1274d37fd10552ef2988529c0fdb5f64e0b89b3bd98ac`.
- The same diff with the three excluded docs left out gives `db27572f1cc17ca7fa6dc66bc5a6042f04fdb814ca090198a36dad704d227ced`.
- Both equal `raw/windows/addendum-b/digest-at-staging.txt` and `digest-after.txt` (base `86bb198c`). Receipt `blobfix-20261019-b` therefore tested exactly `f69b1112`'s tree.
- The README's Windows bullet states these same two digests against `86bb198c`, which is correct.

**Trigger provenance.**
- Both final trigger `.env` files and `build-digest.txt` record code-only `624191ac…`, equal to `f69b1112`'s code.
- They record full `d1cf372c…`, not `db27572f…`. The docs were edited after the trigger: the §0 stock-server text cannot predate the stock run. Nothing compiles dev-docs (`rg 'include_(str|bytes)!.*dev-docs'` finds nothing), so the binary was built from `f69b1112`'s code. The README does not explain the full-digest difference; this is noted under N1's correction and is not a separate finding.

## Problem, requirement provenance, and scope

**Problem.** D42 requires that a multi-GB SSH push no longer stall at the 9 s limit while libgit2 reads a blob, and that a ceiling stop a hung producer from holding the clock open. D45 sets that ceiling at 600 s. Round 1 found the implementation GO, with five P3s. This commit claims to fold them in.

**No production behaviour changes.**
- `git diff --stat 86bb198c f69b1112 -- src ':!src/**/*tests*'` lists only `session.rs` (+6/−1).
- That hunk sits inside the existing `cfg_if::cfg_if! { if #[cfg(test)] { … } }` (`session.rs:359`). It moves one doc line and adds `producer_ceilings_for_test`, which is test-only.
- The other three source files are test modules declared under `cfg(test)`:
  - `stream_io_tests` and `ssh_pump_clock_tests` at `git/endpoint/mod.rs:110-118`;
  - `producer_busy_tests` at `transport_host/mod.rs:74-79`;
  - `producer` is a child of `ssh_pump_clock_tests`.
- The source has no leftover mutation switches: `rg PUMP_MUT|GUARD_MUT|CEIL_MUT|GWZR_MUT src` finds nothing.
- Repository rules hold: the new function is in an existing `cfg_if` block, all bodies are braced, and there are no conditional imports.

**Scope.** Everything touched (tests, design and transport docs, the evidence run) is inside the editable boundary. The operator's untracked files are untouched.

## 1. Prior-finding closure table

**P3-1: the stopped-tracker mutant survived; the tracker row did not assert its own precondition.** **Closed.**
- Counterexample re-checked: pack start read as `!fetch && turns.stopped`.
- New row `producer.rs:161` `a_stopped_tracker_does_not_start_the_pack_before_the_clients_first_byte`: stderr line, busy gate, no client byte, then `tick(START) == Network`, not terminal at `START-1+STALL_MS-1`, timed out at `+1`.
- Added precondition in `the_pack_having_begun_does_not_depend_on_the_turn_tracker`: `tick(START) == Network` before `send_head`.
- `addendum-mutations.log`: `PUMP_MUT=turns_stopped` turns both rows red; unmutated, 36/36 pass.
- Deterministic: a simulated clock, no wall time.

**P3-2: the shipped 600 s ceiling was pinned by no test; the `close` and `end_write` guards were untested.** **Closed.**
- New row `producer_busy_tests.rs:279` `a_push_open_on_a_runtime_nothing_configured_carries_the_shipped_ceiling`. `Rig::new(.., None)` calls no setter. It asserts `producer_ceilings_for_test() == vec![600_000]` and `PRODUCER_CEILING_MS == 600_000`. `CEIL_MUT=1` turns it red.
- Gate rows `stream_io_tests.rs:164` (`close`) and `:182` (`end_write`). `GUARD_MUT=close` and `GUARD_MUT=end_write` each turn their own row red.
- Determinism caveat: see N3.

**P3-3: the trigger evidence was not durable and had no build provenance.** **Closed.**
- The red log is now held in this run: `raw/blobpush-2026-10-17-ssh-3072-default.red.log` (`no progress for 9 seconds`, exit 101). The blobpush lane's own run is still untracked there, which no longer matters.
- Final trigger `.env` files record the commit, the tree's digests and `binary_sha256`.
- `runner/round2/run-trigger.sh` refuses to run on a digest mismatch.
- Windows per-row extracts are archived (`producer-extract.txt`: 36 ok, 2 ignored; plus the round-a extract).
- The binary itself is not archived, so its sha256 is a claim tied to the build-time digest. That is acceptable.

**P3-4: §7's 300 ms and 1 s figures, §10.1's "first pack byte", and the misplaced `session.rs` comment.** **Closed for the named items.**
- §7 now says `--ssh-timeout` 1 s and test ceiling 2 s, equal to `STALL_MS = 1_000` and `CEILING_MS = 2_000` (`producer_busy_tests.rs:23-25`).
- §10.1 (`GwzRemoteTransportDesign.md:701` and `:720`) and design §8 now say "the client's first byte (its commands)". That equals `ssh_pump.rs:207` (first non-empty client `Data`, `!fetch`) and `push_started` (commands first).
- The `session.rs` comment is moved.
- Left over: §7's window bounds still disagree with the code. See N1.

**P3-5 (operator): the stock-server claim was unmeasured.** **Closed (measured).**
- `raw/trigger/final/ssh-3072-stock-server.log` shows `GWZ_BLOBPUSH_SERVER_DEFAULTS=1`, which skips `unpackLimit=1` (`blob_push_tests.rs:89-91`), `mib=3072`, `limit_ms=9000`, `longest_uplink_pause_ms=30790`, `result=ok`, exit 0.
- §0 now claims exactly that, as one run at one size on one host under load, with the right caveats.
- §0 cites a path that does not exist. See N1.

## 2. New findings

### N1 (P3): the design's evidence pointers and §7 figures were not reconciled against the landed artifacts

- **Root cause.** The fold-in edited the design's figures and the stock-server claim without re-checking every pointer and number against the files that landed.
- **Locations.** `dev-docs/GwzPushProducerBusyDesign-DRAFT.md`:
  - (a) Line 9 (§0) cites `raw/trigger/ssh-3072-stock-server.log`. The file is `raw/trigger/final/ssh-3072-stock-server.log`; `raw/trigger/` holds only `final/` and the round-1 `ssh-3072-default.*`.
  - (b) Lines 112-113 (§7, frozen and hung rows) give the window as "between `ceiling + stall - 100 ms` and `ceiling + stall + 2 s`". The code asserts `found + 150 ms >= limit` and `found <= limit + 3 s` (`producer_busy_tests.rs:225,229,251,255`).
  - (c) Line 115 (§7, trigger row) still points the red log at the blobpush run's `raw/final/ssh-3072-default.log`. That file is untracked in another lane. The durable copy is `2026-10-18-blobfix/raw/blobpush-2026-10-17-ssh-3072-default.red.log`.
- **Invariant broken.** Design status line 3 says "The rows below are the ones that landed"; the deliverable cites its evidence.
- **Reproduction.**
  - `ls gwz-core-evidence/.../2026-10-18-blobfix/raw/trigger/ssh-3072-stock-server.log` reports no such file.
  - Compare §7:112 with `producer_busy_tests.rs:225-229`.
- **Impact.** A reader auditing §0 cannot find the cited log at that path. A reviewer re-deriving the e2e bound from §7 works from tighter numbers than the code enforces. There is no behavioural effect.
- **Correction.**
  - Point §0 at `raw/trigger/final/ssh-3072-stock-server.log`.
  - Write §7's window as −150 ms / +3 s.
  - Point §7's red log at the blobfix copy.
  - Optionally, have the README state that the trigger ran on full digest `d1cf372c…`: the same code (`624191ac…`) with earlier docs.
- **Regression test.** A link check over the design's backticked `raw/…` paths against the run's `manifest.json` keys. None is needed for the figures beyond the correction.

### N2 (P3): the trigger row's doc comments still state the stock-server silence that the stock run measured against

- **Root cause.** §0 was corrected after the stock-server run, but the same claim in the test file's comments was not.
- **Location.** `src/transport_host/blob_push_tests.rs:17-19` (module doc: a stock server "would keep it silent for the stall limit by itself") and `:82-87` (`quiet_server` doc: unpacking "would deflate the whole blob again after the last byte and keep the server silent for longer than the stall limit").
- **Invariant broken.** Code comments agree with the recorded evidence and with design §0.
- **Reproduction.** `raw/trigger/final/ssh-3072-stock-server.log`: `last_data_ms=843336`, `push_ended_ms=843544`. The stock server answered 208 ms after the last byte at 3 GiB, with `result=ok`.
- **Impact.** A future maintainer reads as fact that `unpackLimit=1` is required, and may carry that into release notes. That is the scope-wording risk P3-5 was raised to prevent.
- **Correction.** Reword both comments to match §0. `unpackLimit=1` keeps the server's post-pack work out of what the row measures. One stock-server run at 3 GiB did not show that silence. The earlier failure of that kind was a 64 MiB HTTPS row.
- **Regression test.** None. This is a comment.

### N3 (P3): the new `close`/`end_write` gate rows depend on a 300 ms wall-clock sleep

- **Root cause.** `blocked_in` (`stream_io_tests.rs:95-107`) assumes the spawned thread is inside its call after `thread::sleep(300 ms)`. Both new rows (`:164`, `:182`) then assert `!gate.busy()` once.
- **Invariant broken.** The new rows are meant to be deterministic, with no wall-clock races.
- **Reproduction (reasoned).**
  - Before the spawned thread's `close` takes its `_call` guard (`stream_io.rs:139`), the gate is still busy from the earlier `write`.
  - If scheduling delays the thread by more than 300 ms, the assertion fails on correct code.
  - The addendum gates ran at load 45 to 100 on this host.
- **Direction.** It can only fail red. Under the mutants the gate stays busy whatever the timing, so the rows cannot go falsely green. The fail-closed direction is unaffected. The same helper already serves the round-1 `read` and `write` rows.
- **Impact.** Possible spurious red on a loaded host or a CI runner. Bounded: no false GO.
- **Correction.** In `blocked_in`'s callers, poll `!gate.busy()` until a generous deadline (for example 10 s) instead of sleeping, then assert. Under the mutant the poll times out, so the row stays red. Apply it to the `read` and `write` rows as well.
- **Regression test.** Re-run `GUARD_MUT=close` and `GUARD_MUT=end_write` after the change: both rows must still go red.

**Area 2: the other new rows.**
- The stopped-tracker and amended tracker rows are deterministic (simulated clock). They assert the state §7 row 6 names (`Network`, then a stall at `STALL_MS`), and the mutation log shows them red.
- The default-ceiling row is deterministic: no timing. The `Entry` exists once `push_started` returns from `open`. It asserts D45's value through the shipped path, and the mutation log shows it red.

**Area 5: fail-closed direction. No defect.** The shipped 600 s bound is now pinned end to end:
- the default-ceiling row puts 600 000 on the `Entry` gate;
- `ssh_handoff.rs:303-318` shows the pump receives that same gate;
- the hung-producer and frozen-server rows show that the `Entry` ceiling governs the pump in a live push;
- the pump's ceiling-edge rows pin `now - last_byte < ceiling`.

`last_byte` is no later than D45's "first eligible tick after the last byte", so the charge is at most 600 s, then `Network` with at most the kept stall budget. The chatty-server and full-`R` cases are pre-existing and documented in design §4, as round 1 noted.

## 3. Out-of-scope observations

- **D45 is not recorded in the lane.** The lane's `gwz-core/dev-docs/GwzTransportTR18-OperatorDecisions.md` ends at D44 (last commit `accb1b04`). D45 is at line 88 of the main workspace's `/Volumes/projects/limbo/gwz-dev/gwz-core/dev-docs/GwzTransportTR18-OperatorDecisions.md`, and its text matches the implementation: 600 s from the first eligible tick after the last byte, then `Network`. The lane's design and §10.1 cite D45, which resolves only after the merge. The merge should bring it in; no action is needed in this object.
- **The fmt gate's file list omits `session.rs`.** `gate-fmt.log` covers 10 files; `ssh_pump_clock_tests/producer.rs` is covered through its parent. The `session.rs` hunk is inside `cfg_if!`, which rustfmt does not format, so nothing is lost.
- **The blobpush lane's run is still untracked** in `/Volumes/projects/limbo/gwz-dev-blobpush/gwz-core-evidence`. It is no longer load-bearing, since the red log is copied here.
- **HTTPS.** HTTPS's `Backpressure` with no ceiling (design §0, Reference only) is unchanged and still the operator's note.

## 4. Tuple check at end

Unchanged from the start:
- gwz-core HEAD `f69b111275526c5a8a372a08b4a06a7c6bf06123`; untracked `dev-docs/GwzRemoteTransportBugReport.md` only.
- gwz-core-evidence HEAD `c98e627659691014256d0947a8c3e6233d2864b7`; untracked `campaigns/transport-qualification/runs/2026-09-22-alpha-setup-timeout/` only.

Nothing in the lane was modified.

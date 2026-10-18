**Re-verdict: GO.** P0 0, P1 0, P2 0 open (two were raised in round 1; one is closed and the other is now a push-time step whose plan is correct, with one precision below). Two round-1 P3s are closed. Five P3s are open or recorded: P3-4 (the lane owner's), new P3-5, the architectural item P3-6, and two items moved to the coordinator's list. None of them blocks.

# Code re-review, round 2: gwz-sspi 0.1.2 launch guard (D31) and the D47 seam

- **Object:** lane `sspi012`, root `3c24748c`
  - gwz-sspi `87946fd0` over `bd684977`
  - gwz-core `300cf64f` over `02d73b40`
  - gwz-cli `8340ac9f` (unchanged)
  - gwz-py `b3e9505c` over `7185f22e`
  - evidence `77edc72e`, run `2026-10-18-sspi012-rem`, receipt `sspi012-20261018-k`
  - plan `gwz-core/dev-docs/GwzSspi012LaunchGuard-RemPlan.md`
- **Date:** 2026-10-19
- **Axis:** CODE

## §0 Evidence base

- **Tuple.** Verified at start and end; identical. The only change in untracked files is a new `gwz-core/dev-docs/GwzSspi012LaunchGuard-ReviewState-2.md`, which is the State reviewer's report. I did not open it.
- **Read:**
  - all three remediation diffs in full;
  - the plan, the evidence README and `tools/digest.sh`;
  - the Windows mutation log;
  - `Git2Backend`'s constructors and derivations;
  - every production `Command::new` in gwz-core;
  - every backend construction site in gwz-py's `native/src`.
- **Ran.** External target directories, all deleted afterwards.
  - gwz-sspi `87946fd0`:
    - clippy `-D warnings` on the host and on `x86_64-pc-windows-msvc`: clean;
    - fmt: clean;
    - `cargo test --all-features --locked`: 85 lib tests pass, plus the integration tests and 21 doctests, including the `#[must_use]` compile-fail doctest.
  - gwz-core:
    - transport-shape tests filtered on `spawn_gate`: 16 pass, including `a_native_hosts_own_backend_holds_the_hosts_gate_and_no_other`;
    - ordinary-shape `clippy --lib -D warnings`: clean (the `same_gate` allow works).
  - gwz-py:
    - `check --all-targets` in the ordinary, transport and session shapes: no errors;
    - the D47 rows `a_requests_own_backend_is_on_its_clients_gate` and `with_the_off_switch_on_an_operation_runs_on_the_client_hosts_gate`: pass.
  - **Code-only digests recomputed from the commits.** For each repo I hashed: the base SHA, then `git diff base..remediation --binary`, with added files left out of the diff and listed as `sha256  path` lines, all excluding `dev-docs`. All four match `k-at-staging`, `k-after-run` and `after-all-gates` exactly:
    - gwz-sspi `0678cc28…`
    - gwz-core `cb2ad2c4…`
    - gwz-cli `8dfff588…`
    - gwz-py `afb0eaaf…`

    So receipt `-k` binds to the committed code.
  - Not done: anything on Windows. The Windows rows and mutations come from the evidence (`windows-final.log` §1 and §3).

## Closure table (my round-1 findings)

| Finding | Status | Basis |
|---|---|---|
| **P2-1**: recheck before an unbounded guard wait | **Closed** | `launch.rs` `start`: `*held = guard.map(hold); if held.is_some() { origin.verify()?; if stop… { Cancelled } }`, before `InheritWindow::open`. A failure leaves the hold in `start_and_close`'s `held`, which is dropped after `drop(ends)`. The pre-guard verify is kept, so a refused launch still takes no guard. Without a guard, `held` is `None` and the extra checks are skipped, so `Supervisor::new` remains exactly 0.1.1. The counterexample I described in round 1 now exists as `a_caller_that_changes_while_the_guard_is_held_creates_no_worker`: a `Changing` origin that verifies once and then fails gives `IdentityMismatch`, Job count 0, and the guard taken and then released after the ends are closed. Its "no second verify" mutant goes red on Windows (`windows-final.log:79-82`). The docs are fixed in `Architecture.md:59-61`, the CHANGELOG and `LaunchGuard`'s rustdoc. |
| **P2-2**: the lock moved without the pin | **Plan correct, with one precision**; execution belongs to the coordinator | The order (gwz-sspi to main, then gwz-core with the pin, then a stock lane check, then gwz-cli and gwz-py, then root) matches the bump rule and CI's "pinned on main" step. **Precision:** gwz-core now has two lane commits. The lock moves in `02d73b40`, not in `300cf64f`, so amending HEAD alone would leave `02d73b40` permanently red. The pin has to land in the commit that moves the lock: either rewrite `02d73b40` and replay `300cf64f`, or squash the two. Then run the per-commit lane gate and the stock `windows_lane_check.py` on the rewritten head. The pin must name gwz-sspi `87946fd0`, not `bd684977`, and the `v0.1.2` tag goes there too. The rewrite changes SHAs, so the stock rerun is what binds the receipt. gwz-cli `8340ac9f` needs no change. |
| **P3-1**: C9 overclaimed for gwz-py | **Closed by D47** (beyond what round 1 asked) | Every gwz-py backend site is now on the `Client`'s gate: `client_host.rs:119` (sync call), `dispatch/mod.rs:550` (submit), `dispatch/merge.rs:45`, `local_family` via `Scope`, and the off switch (`route/transport.rs:195` via `attach_own`). The only remaining `Git2Backend::new()` are `route/native.rs:45` (the non-candidate build, which has no supervisor and so no window) and the two fallbacks in P3-5. C9's new text ("all of its operations run on its host's gate", with the exceptions named) is now accurate. In gwz-core, every production std spawn reached from gwz-py either goes through the backend's gate (commit, tag, tag -d, credential helper, WH2 launch) or is recorded (`gwz log`); the other `Command::new` sites are all `#[cfg(test)]`. |
| **P3-2**: no test went through `NativeHost::new` | **Closed** | `gated_by_the_constructor` uses `NativeHost::new` with nothing wrapped around the guard. The "ctor drops the guard" mutant goes red on Windows (`windows-final.log:136-164`: 330 probes, 2 worker pipes leaked). The detection is statistical, but the control leaks within 1 launch and this phase makes 100, so the bound is comfortable. As before, the mutant survives on the Mac, where the gate doesn't exclude. |
| **P3-3**: gwz-py's unused `Supervisor` | **Closed** | The non-candidate `NativeRoute` is a unit struct and `own_backend` there is `Git2Backend::new()`. No supervisor and no threads are created. |
| **P3-4**: D31 as-built not recorded | **Open** (lane owner, per the plan) | `GwzTransportTR18-OperatorDecisions.md:74` still says "Options hook". **D47 itself is not in the ledger at all:** the D-table has no D47 row, though C9 and the code cite it. Add both. |

## The D47 seam on the code axis

- **Public API and semver.**
  - `NativeHost` gains `Clone`, `backend() -> Git2Backend` and `gates(&Git2Backend) -> bool`. `NativeHost` is new this cycle, exists only under `gwz_transport_candidate`, and has never been released, so there is no semver exposure.
  - A clone is the same host (it shares the `Arc<Supervisor>` and the gate's `Arc`). `same_gate` is `Arc::ptr_eq` on the same state that `hold` uses, so identity is exactly what decides exclusion.
  - Every `Git2Backend` derivation keeps the gate: `with_route_notes` and `with_transport` clone it, and `with_host_context` replaces it with the request's gate, which is the host's.
  - gwz-sspi's remediation adds no public surface; `#[must_use]` on `hold` is lint-only. The `Platform` trait that gained `stop` is private.
- **Does gwz-py use it everywhere?** Yes; the sites are listed under P3-1 above.
- **`gates()` is public only so gwz-py's tests can use it.** This is a follow-up, not a finding: consider `#[doc(hidden)]`, or keep it, given the narrow-API policy.

## The implementer's question: on the Mac, gate identity instead of holding the gate

**Acceptable.** On the Mac, `SpawnGate::new()` doesn't exclude, and gwz-py can't build an excluding gate because `SpawnGate` is crate-private. So holding the gate would prove nothing there, and identity is the property gwz-py actually controls. The composition is complete without it:

- gwz-core proves that `backend()` holds an *excluding* host gate (`with_exclusion(true)` in `a_native_hosts_own_backend_holds_the_hosts_gate_and_no_other`, with a mutant that goes red);
- `gitbackend::spawn_gate_tests` prove that commit, tag and the credential helper wait for the backend's gate;
- the Windows row proves the end-to-end exclusion.

Both gwz-py identity rows have mutants that go red (`gwz-py-d47-mutations-red.log`).

## New findings

### P3-5: gwz-py keeps two production-unreachable fallbacks to a backend with no host gate

- **Location.**
  - `shims.rs:62-66`: `None => Git2Backend::new()`. `Backend::own` always sets `route: Some`, and `given` never reaches this arm.
  - `route/transport.rs:193-195`: `map_or_else(Git2Backend::new, …)` when `own` is `None`. Production always calls `attach_own` in `network()`; only tests leave it unset.
- **Root cause.** The host is required, but it is modelled as `Option`.
- **Impact.** Nothing is wrong today. But a future caller that forgets `attach_own` silently falls back to a private gate, which is exactly the D47 failure, with no error and no test catching it. This is the "unreachable fallback" pattern the remove-dead-code rule names.
- **Correction.** Make the host non-optional: `Backend::Own { route, cell }` as its own variant, and `Inner::Native { own: NativeHost }` passed into `capture`/`capture_with` instead of being attached afterwards. Then the type system enforces it.
- **Regression test.** The existing D47 rows, with the off-switch test built on a host.

### P3-6 (architectural, record): being on the gate is opt-in, and the default fails open

- **Observation.** `Git2Backend::new()` (and `without_credential_helpers()`) default to a fresh private gate. Every host-level guarantee (D25, D31, D47) therefore depends on each construction site remembering to use the host's backend.
  - The round-1 P3-1 gap was one instance of this.
  - D47 fixes gwz-py's current instances.
  - gwz-core still builds its own `Git2Backend::new()` internally (`diff/*`, `status/*`, `handle_remote_identity`). None of those spawns today, so nothing is exposed now; but the next spawn added on such a path won't be gated, and nothing will flag it.
- **Is it new?** Not new code: the default exists since D25. But D47 makes it the load-bearing mechanism for a whole `Client`.
- **Suggested record (for the operator, not a prerequisite):** the CS3.6 host context owns the gate, and in the transport build a backend comes from a host (or a gate is a required constructor argument). Alternatively, a source check that flags `Git2Backend::new()` on paths that can spawn.

## Attacks that failed

- The second verify and the stop check run only when a guard is held, so `Supervisor::new` remains exactly 0.1.1.
- A failed reset of the window can't release the gate early: the hold is now dropped after `drop(ends)`, and the "guard before ends closed" mutant goes red.
- The stop flag is set by every terminal transition (`Context::update`) and by `close`, and the platform is handed the record's own flag (`the_platform_is_handed_the_records_stop_flag`, with a mutant on the Mac).
- Cloning `NativeHost` creates no second supervisor or gate.
- The off-switch backend and the own backends keep `GitEnvironment::process()` captured when they're built, as before.
- All four gwz-py shapes and both gwz-sspi targets compile clean.

## Follow-ups (for the coordinator)

- **P2-2's precision:** the pin goes into the lock-moving commit (`02d73b40`), naming `87946fd0`; then the per-commit lane gate and the stock lane check run on the rewritten head.
- **P3-4:** annotate the D31 row and add a D47 row to the decisions ledger.
- **P3-5:** before S7.1 or folded into the next gwz-py change.
- **P3-6:** to the operator, next to CS3.6.
- **Evidence (State's call):** `raw/digests/staging-k.digests` disagrees with `k-at-staging.digests` for gwz-core and gwz-py, so it looks stale. The README still says "Uncommitted" and lists the pre-remediation HEADs. The authoritative trio matches the commits, as recomputed above.

## Next action

GO for D31 and D47 on the code axis. Carry out P2-2's push order with the precision above. P3-4 is a ledger edit for the lane owner; P3-5 and P3-6 are tracked and don't gate the tag.

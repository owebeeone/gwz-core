# D31 launch-guard re-verdict, State axis, round 2 (gwz-sspi 0.1.2, NativeHost, D47)

- **Object:** lane `/Volumes/projects/limbo/gwz-dev-sspi012`

  | Repo | Head | Over |
  |---|---|---|
  | gwz-sspi | `87946fd0` | `bd684977` |
  | gwz-core | `300cf64f` | `02d73b40` |
  | gwz-cli | `8340ac9f` | unchanged |
  | gwz-py | `b3e9505c` | `7185f22e` |

- **Plan:** `gwz-core/dev-docs/GwzSspi012LaunchGuard-RemPlan.md`
- **Date:** 2026-10-19
- **Axis:** State
- **Verdict:** **GO.** P0 0, P1 0, P2 0 open. Every round-1 finding is closed or recorded as an operator item. There are 3 new P3s, one of which is an architectural root cause. Nothing blocks.

## §0 Evidence base

- **Tuple:** verified at start and end against the heads above. No tracked files dirty. Nothing was built this round, and no review build directory remains.
- **Code read:**
  - gwz-sspi `87946fd0`: `windows/launch.rs` (`start`, `start_and_close`), `monitor.rs`, `ports.rs`, `values.rs`, `windows/inheritance_tests.rs`.
  - gwz-core `300cf64f`: `native/sspi.rs` (`NativeHost: Clone`, `backend()`, `gates()`), `spawn_gate.rs` (`same_gate`), the `sspi_window.rs` diff, the C9 and OQ-H diff.
  - gwz-py `b3e9505c`: `client_host.rs` (`Scope`, `own_backend`), `shims.rs`, `route/transport.rs` (off-switch `Inner::Native { own }`), the `dispatch/*` changes, `route.rs` (cfg selection).
  - A sweep of every remaining `Git2Backend::new()` in gwz-py and gwz-core.
- **Evidence:** `2026-10-18-sspi012-rem/`.
  - `manifest.json`: 48 files, 0 bad.
  - The binding is `tools/digest.sh`.
  - Receipt k started at 16:31:06, inside `windows-final.log`'s locked run (lock taken at 16:30:27), in the fresh directory `/e/gwz-tests/sspi012-20261018-k`. Ordinary and transport exited 0; hosted-like passed 3971 with the 4 named gaps.

## Closure table

| Finding | Status | Re-verification |
|---|---|---|
| **P2-1**: gwz-py non-network ops off the Client gate; C9 overclaims | **Closed** (D47) | Covered below under "P2-1". |
| **P2-2**: receipt not bound to gwz-core | **Closed** | Covered below under "P2-2". |
| **P3-1**: identity recheck before an unbounded wait | **Closed** | `launch.rs`: `*held = guard.map(hold)`, then `origin.verify()?` again before `InheritWindow::open`. A failure returns with the window unopened, and the ends are non-inheritable when the guard is released. Row `a_caller_that_changes_while_the_guard_is_held_creates_no_worker` (Job count 0); its mutant is red (`windows-final.log` §no-second-verify). |
| **P3-2**: a launch after its record ended still creates a worker | **Closed** | After the hold, `stop.load(Acquire)` returns `Cancelled` before the window. Interleaving re-checked: `stop` is set by `Context::update` when the kernel goes terminal (the deadline driver runs every 10 ms, so at most about 10 ms late) and directly by `close()`. `no_child` keeps the record's own terminal fault (Timeout). Row `a_record_that_ends_while_the_guard_is_held_creates_no_worker` (Job count 0); its mutant is red. |
| **P3-3**: wording of the SSPI Timeout under the gate | **Closed** (doc) | The C9 and `LaunchGuard` docs now say that `start` ends at its own deadline and that the record stays charged until the hold ends. This matches my round-1 experiment (302 ms against a 300 ms deadline). |
| **P3-4**: guard released before the ends close | **Closed** | `start_and_close` runs `start(…, &mut held)`, then `drop(ends)`, then `drop(held)` on every return path. Row `the_guard_is_released_only_after_the_child_ends_are_closed_when_creation_fails` sees `output_closed: true` at release; its mutant is red. On the panic path, unwinding drops the local `held` before the by-value `ends`. No panic source exists after the window opens (only FFI and infallible drops), and before it the ends are not inheritable. No finding. |
| **P3-5**: gated phase does not assert its windows | **Closed** | `gated.windows == GATED_LAUNCHES` and `inside >= SPAWNERS` are now asserted. A new phase runs through `NativeHost::new` unwrapped. Mutants "ctor", "count" and "inside" are all red. |
| **P3-6**: per-Client coupling | **Recorded** | The OQ-H bullet states it: a credential prompt in one operation blocks the Client's other operations' backend spawns (an uncancellable wait), its worker launches and its helper launches (`GateTimeout`). D47 widens this to non-network operations. That is the stated price of one gate per Client, left to the operator. |

### P2-1: re-checking the gwz-py race

- `ClientHost::scope()` carries `own: self.native.clone()` into every path:
  - `call`;
  - `submit` via `spawn_call`;
  - the `merge` and `local_family` submit threads;
  - the off-switch route, through `attach_own`.
- `shims::Backend::get` lazily runs `own_backend(route)`, which is `NativeHost::backend()`: `Git2Backend::new()` with `spawn_gate = self.gate.clone()`. Making the backend lazily keeps the old `ensure_server_timeout` ordering.
- Re-running the round-1 interleaving: T2's `client.commit()` takes `commit_cli` and calls `backend.spawn_gate.output(...)` on the Client's gate. It now blocks while T1's worker window holds that gate. Closed.
- The one remaining `Git2Backend::new()` in gwz-py is `route/native.rs:45`. It is in the non-candidate arm (`route.rs` `cfg_if`), where gwz-py carries no supervisor (`NativeRoute` is a unit struct), so there is no window.
- The internal gwz-core fresh-backend sites gwz-py reaches (`handle_create_workspace`, `diff`, `status`, `historical_identity`, `remote_identity`) call no CLI-spawn method. `tag_list` is libgit2; `create_workspace` makes no CLI commit. See N3.
- C9 and OQ-H now state the residual precisely: other Clients and hosts, `gwz log`'s `rev-list`, and the embedding process's own spawns.

### P2-2: re-computing the digests

- I recomputed the code-only digests from the commits with the receipt's recipe: base HEAD, then `git diff --binary` with `dev-docs` excluded, then the hashes of added files (there are none).

  | Repo | Code-only digest |
  |---|---|
  | gwz-sspi | `0678cc28…803f1` |
  | gwz-core | `cb2ad2c4…ecc68` |
  | gwz-cli | `8dfff588…1152a` |
  | gwz-py | `afb0eaaf…0e4` |

- These equal `k-at-staging`, `k-after-run` and `after-all-gates` exactly.

## Attacks on D47 that failed

- **Lock identity.**
  - `NativeHost` is `#[derive(Clone)]` over `Result<Arc<Supervisor>, _>` and `SpawnGate` (an `Arc<Shared>`). Every clone shares one supervisor and one gate.
  - `backend()` clones that gate.
  - `on_gate` is `pub(crate)` and `SpawnGate` is crate-private, so no caller can pair one host's supervisor with another gate.
  - `gates()` is `Arc::ptr_eq`. It is an identity oracle that works off Windows too.
  - gwz-cli is unchanged and correct: one invocation per process, and no `NativeHost` for a non-transport or off-switch command, so it has no supervisor and no window.
- **Clone lifetimes.**
  - Clones moved into submit threads keep the supervisor alive until those threads end. `Supervisor::drop` only closes the context.
  - Callers already held an `Arc<Supervisor>` through the port, so this changes no semantics.
- **Deadlock.**
  - No gate holder takes the GIL or awaits an SSPI start. The holders are:
    - backend spawns (held for the spawn only);
    - `helper_credential` (held around an external `git credential` call);
    - `Runner`;
    - the launch thread.
  - Gate waiters wait inside `py.detach`.
  - The GIL-held `scope()` and `caller()` calls take only gwz-sspi's short state lock.
  - The gate is non-reentrant, and no path nests a hold on one thread.
  - The per-Client coupling (P3-6) is a stall bounded by a user's answer, not a cycle.

## New findings

### N1 (P3): gwz-sspi's own suite lost the "guard taken before the window opens" oracle

- **Location:** `gwz-sspi/src/supervisor/windows/inheritance_tests.rs:108–115`. The remediation rewrote `Seen` as `{event, created, output_closed}` and dropped the `inheritable` observation that the round-0 rows made at "taken" and "released".
- **Violated invariant:** D31's ordering, which is gwz-sspi's published contract, is no longer pinned by gwz-sspi itself.
- **Interleaving:** a regression that moves `guard.map(hold)` below `InheritWindow::open` now passes all 12 launch rows.
  - "taken" still sees `created 0` and `output_closed false`.
  - The second-verify and stop rows see the same values either way.
  - The round-0 mutation "taken after the window opens" is not in this round's set (`windows-final.log` §1 has three mutations).
- **Mitigation:** gwz-core's `sspi_window` would very likely catch it. With the mutation, the launch thread waits for a gate held by a spinning spawner while the ends are inheritable. That is undemonstrated, and it is cross-repo.
- **Correction:** restore `inheritable` in `Seen`, expecting `false` at both "taken" and "released", and re-run the four round-0 ordering mutations against gwz-sspi's suite.

### N2 (P3, evidence hygiene): a stale digest file sits under the final run's name

- **Location:** `raw/digests/staging-k.digests`.
- It equals `raw/aborted-before-d47/k-at-staging.digests` (gwz-core code-only `678e0c94…`, gwz-py `81d8b752…`), which is the pre-D47 state. The README doesn't cite it and receipt k is bound by `k-at-staging.digests`, but a reader checking "the staging-k digest" will find a mismatch.
- **Correction:** move it into `aborted-before-d47/` in the next evidence commit.

### N3 (P3, ARCHITECTURAL root cause): the backend gate fails open by default

- **Root cause:** `Git2Backend::new()`, `without_credential_helpers()` and `RequestContext::new` (`SpawnGate::default()`) each mint a private gate. Exclusion therefore holds only where each construction site was rewired by hand, and each round found another gap: D25 (backends), D31 (the supervisor), D47 (gwz-py's own backends).
- **Today:** gwz-core's public entries that build their own backend (`handle_create_workspace`, `diff`, `status`, `historical_identity`, `remote_identity`) reach no CLI spawn. Nothing enforces that, so a future `commit`, `tag` or remote operation inside one would silently leave the gate.
- **Correction, for the operator:** make the gate a required input of the production backend constructors. Alternatively, add a source check that lists every `Git2Backend::new()` and `without_credential_helpers()` site reachable from a host entry, against the CLI-spawn methods (`commit_cli`, `tag_create_cli`, `tag_delete_cli`, `helper_credential`).
- **Status:** not blocking; I found no current reachable path. This is an observation, not authorized work.

## Follow-ups

- N1: restore the `inheritable` observation and re-run the four round-0 ordering mutations in gwz-sspi.
- N2: move `staging-k.digests` into `aborted-before-d47/`.
- N3 and P3-6 go to the operator, beside D25's helper-hold item.

## Next action

GO on the State axis for this tuple. N1 and N2 can be folded into the push-order step without another review round.

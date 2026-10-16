# State review: WH2 spawn gate (gwz-core `7933e9e7`)

- **Object:** gwz-core `7933e9e7`, one commit over `af7e4041`, in lane `/Volumes/projects/limbo/gwz-dev-spawngate`. 25 files, +880/−41.
- **Date:** 2026-10-16
- **Axis:** State (concurrency, lock ordering, deadlock and starvation, cancellation and deadlines, fail-closed direction, handle inheritance, secret exposure)
- **Verdict: GO.** P0: 0, P1: 0, P2: 0, P3: 5.

## §0 Evidence base

**Tuple.** I checked it at the start and the end, and it was the same both times:
- gwz-core `7933e9e72c00…`
- gwz-core-evidence `573be32637b5…`
- lane root `2c399f015d5d…`

The only untracked files were ones that existed before I started: gwz-core `dev-docs/GwzRemoteTransportBugReport.md`, evidence `runs/2026-09-22-alpha-setup-timeout/`, and the root's 5 dev-docs. I wrote nothing in any member.

**Read fully:**
- The whole source diff (`git diff af7e4041 7933e9e7 -- src/`).
- The contract diff: C9, §6 and the header.
- `spawn_gate.rs` and its tests.
- `runner.rs:80-210`, `lookup.rs:60-135`, `ssh_password_helpers.rs:1-80`.
- `transport_host/mod.rs:270-360, 460-530`, `session.rs:520-570`, `cancellable.rs:200-245`, `local_command.rs:50-100`.
- `https_worker.rs:340-410` (how helper failures are classified) and `helper_failure.rs`.
- `refs.rs:160-260`, `repository.rs:290-330`, `commit_log/mod.rs:290-350`.
- gwz-cli `globalargs/dispatch.rs:30-110, 440-458`.
- D25, D19 State P3-2, and `GwzSspi011PipeFix-ReviewState.md` P3-3 (main checkout).
- The evidence README, the Windows lane-check log, and the receipt keys.

**Sampled:**
- gwz-py `native/src/{client_host,shims,route/transport}.rs` (concurrency model, one host per operation).
- `scripts/checks/process_globals_allowlist.json`.
- CI workflows (whether any Windows job denies warnings).
- The Windows receipt `-c` log (gate rows and warnings).
- The Mac red and green logs.
- libgit2 `process.c` and `ssh_exec.c`, git2-rs `cred.rs` (spawn reachability).

**Not read:** gwz-sspi source (I relied on its review for the seam), the gwz-cli pager, forall and hook bodies beyond their spawn sites, and `view::prepare`.

**Commands and results:**
- **Spawn enumeration.** I grepped `src` for `Command::new|process::Command|CreateProcessW(|tokio::process|.spawn()|.output()|.status()|credential_helper|Cred::`, then traced each hit's module cfg (results under "Completeness" below).
- **Test run.** `CARGO_TARGET_DIR=/Volumes/projects/limbo/build-targets/reviewgate-spawngate-state cargo test --locked --lib spawn_gate` in the lane's gwz-core (ordinary build, macOS): **11 passed, 0 failed**. That is the 7 `spawn_gate::tests` rows and the 4 `gitbackend::spawn_gate_tests` rows.
- **Cleanup.** I deleted the target dir afterwards. No `reviewgate-*` dirs remain.
- **Candidate and Windows rows.** I took these from the archived logs:
  - Runner gate rows: red 3/4 (`mac-red-runner-gate.log`), then green.
  - Windows control and gated rows: green in receipt `-c`.

## Findings

### P3-1. D25 names the gwz-sspi worker window, it is not gated, and the contract restates D25 without it as "built"

- **Location:** contract C9 cell ("**The spawn gate closes it** (operator decision D25 …)") and §6 OQ-H ("Decided … built, C9").
- **Violated invariant (requirement trace).** D25 reads: "serializes a std spawn with an inheritance window (WH2 helper launch, gwz-sspi worker launch)". OQ-H restates D25 without that parenthesis and marks it decided and built. C9 then lists gwz-sspi as "not gated, open", and §6 lists it as open.
- **State sequence (Windows):**
  1. gwz-sspi's supervisor thread opens its worker launch window. The child pipe ends and the NUL handle are inheritable; in 0.1.0, which the receipt compiled, they stay so for longer.
  2. On a request thread, `backend.spawn_gate.output(git commit)` takes the host gate. gwz-sspi never takes the gate, so nothing excludes the two.
  3. `git commit` and its hook tree inherit the worker's child ends.
- **Impact:**
  - A stall or forged-response surface on the SSPI worker channel. This is the same class as D19 P3-1 and Sspi P3-3.
  - D25 ("build it before S7.1") reads as discharged in the contract while one of its two named windows is unbuilt and no operator amendment exists.
- **Correction:**
  - Restore D25's wording in OQ-H.
  - Mark OQ-H as partially discharged: the WH2 window is built, the gwz-sspi window is open.
  - Make "**closes it**" in C9 conditional on the open items.
  - Raise the route to the operator: coarse gating around `Supervisor::start`, or an additive gwz-sspi 0.1.2 launch-guard seam and a second pin move. Either way S7.1 waits on that ruling.
- **Regression test** (Sspi P3-3's row): run the contract's PowerShell probe as a gated std spawn concurrently with a gwz-sspi launch, and check that the probe sees none of the worker's pipe names.

### P3-2. A launch that times out waiting for the gate is reported as "no credential helper answered", though none was started

- **Where it happens:**
  - `runner.rs` `hold_spawn_gate` returns `AuthError::Timeout` when `check()` passes the deadline.
  - `https_worker.rs:355` maps that to `SetupFailureCause::Interaction`.
  - `helper_failure.rs:41-44` then renders "No credential helper answered within N seconds, a fixed bound, so gwz gave up on it."
- **Violated invariant (diagnosability).** The reason should name what the lookup waited for. The codebase already has the matching wording for a resource wait: `HELPER_ALLOCATION_PREFIX`, "Other helpers may be busy, waiting for sign-ins or stuck".
- **State sequence (Windows):**
  1. A workspace has an `http://` member on libgit2's native route and an https member on the transport.
  2. `helper_credential` holds the gate whole while GCM waits for the user. D25 accepts this, and C9 (4) states it.
  3. The transport member's lookup enters the Interaction phase (`lookup.rs:93-104`) and polls the gate until its deadline (at most 120 s).
  4. The lookup fails `Timeout`. The user is told no helper answered, while they are looking at another helper's prompt and this member's helper was never launched.
- **Impact:** The user is misdirected in exactly the long-hold case D25 accepted. Other members fail for a cause the message does not name. Recovery still works ("sign in once … then retry"), so this is bounded.
- **Correction.** Give the gate wait its own outcome, for example `AuthError::GateTimeout`, or a flag on the Timeout. Keep the interaction clock, but render the busy-helper wording, or add "another credential helper of this command was waiting for a sign-in".
- **Regression test.** Extend `a_lookup_that_waits_for_the_gate_gives_up_at_its_deadline_and_starts_nothing` to assert the classified failure and its rendered reason.

### P3-3. No test pins that the host chain shares one gate

- **Locations:**
  - `transport_host/mod.rs:309-312` (runtime takes `slots.gate()`) and `:514-516` (`with_spawn_gate`).
  - `request.rs:66` (`RequestContext::new` defaults to a fresh `SpawnGate::default()`).
  - `backend.rs:93` (`with_host_context`) and `transport_support.rs:164` (`remote_callbacks` clones `backend.spawn_gate`).
- **Violated invariant.** C9 (1): the helpers' launches and the backend's spawns "share one lock".
- **Why no current row can see a break:**
  - Every gate row builds the shared gate by hand: `HelperSlots::with_gate(gate.clone())`, `backend.spawn_gate = gate.clone()`, or `helper_credential(&gate)`.
  - On macOS and Linux the production gate never excludes, and no Windows row goes through `TransportRuntime::request`.
- **Reproduction (mutation).** Drop `.with_spawn_gate(spawn_gate)` at `mod.rs:516`, or pass `&SpawnGate::default()` in `remote_callbacks`. Every row on every platform still passes, and on Windows C9 is silently open again: the backend's `git commit` and libgit2 helper spawn hold a private gate.
- **Correction (test only).** Build `TransportRuntime::with_https(…, HelperSlots::with_gate(SpawnGate::with_exclusion(true)))`, as the existing `https_policy_tests` do, and take a request. While holding `slots.gate()`, assert:
  - `request.backend().spawn_gate.try_hold().is_none()`;
  - a derived backend (`gitbackend.rs:272`) is still excluded.

  Add a `remote_callbacks`-level row that shows the callback's helper call blocks on the backend's gate.

### P3-4. A Windows-only unused import sits outside the conditional section it serves

- **Location:** `src/git/endpoint/https_auth/process_tree.rs:25`, `use crate::git::spawn_gate::SpawnHold;`. It is used only by the Unix arm of the `cfg_if!` below it; the Windows arm re-exports `windows::spawn`.
- **Violated invariant.** The repository rule on explicit conditional boundaries: imports belong to the scope that uses them; only unconditional imports stay outside.
- **Evidence:** the receipt `spawngate-20261016-c-hosted-like.log:81-84` shows "warning: unused import: `crate::git::spawn_gate::SpawnHold` --> src\git\endpoint\https_auth\process_tree.rs:25:5", 5 times.
- **Impact:**
  - A Windows warning this commit introduced.
  - It is the same failure class as the `cfa14b8` reassociation hazard: a platform-specific dependency placed unconditionally. It fails any Windows `-D warnings` lint of gwz-core. No Windows job denies warnings today, so this is bounded.
- **Correction.** Move the import into the Unix arm of the `cfg_if!`, or write the path in place in the Unix signature.
- **Regression.** Add a source check, or `cargo check` with `-D unused-imports` on the Windows leg for `https_auth/process_tree.rs`.

### P3-5. The Windows receipt does not bind to the reviewed commit

- **Location:** `windows/spawngate-20261016-c/receipt.json`: `"commit": "af7e40412b16…"`, `"source": "worktree (tracked files, and untracked ones outside dev-docs)"`. The README says "uncommitted when archived".
- **Violated invariant.** The evidence must bind to the object. The Windows control and gated rows are the only evidence that observes inheritance through the OS, and the object of this review is `7933e9e7`.
- **Impact.** Nothing ties the OS-level proof to the reviewed tree except narrative. An edit between the run and the commit would go unseen.
- **Correction.** Either re-run the receipt (or the `gate`-filtered leg) at `7933e9e7`, or record that the worktree diff hash at staging equals `git diff af7e4041 7933e9e7`.
- **Test:** none needed beyond the re-run.

## Invariant analysis (attacks that failed)

### Completeness

These are all the production spawn sites in gwz-core:

| Site | Gated how |
| --- | --- |
| `process_tree.rs:62` (Unix helper launch, a no-op hold) | under the launch's hold |
| `windows/launch.rs:218` (`CreateProcessW`, the WH2 window) | opens only under a hold |
| `refs.rs:174` (`git tag`) | through `backend.spawn_gate.output` |
| `refs.rs:245` (`git tag -d`) | through `backend.spawn_gate.output` |
| `repository.rs:301` (`git commit`) | through `backend.spawn_gate.output` |
| `transport_support.rs:272→297` (libgit2 `Cred::credential_helper`) | held for the whole call |
| `commit_log/mod.rs:320` (`git rev-list`) | exempt |

Everything else is test-only or not a spawn:
- `helper_script`, `ssh_controlled_server`, `pageant_083`, the `*_characterization` modules and in-file `mod tests` are test-only by cfg.
- `session_host/environment.rs` `apply_to` has test callers only.
- `verify_checkout_state.rs:120` is `git2::Status`, not a spawn.

Dependencies do not spawn on these paths either:
- libgit2's `process.c` is reachable only from `ssh_exec.c`, and the build enables `GIT_SSH_LIBSSH2`, not exec.
- git2-rs spawns only in `cred.rs`, which is gated whole.

The exempt `rev-list` cannot run in a gwz-cli process that has a host: `log` is not in `transport_meta`'s scope (`dispatch.rs:442-457`). It can run in gwz-py (`dispatch/log.rs:41`) concurrently with another operation's launch. C9 and §6 state that honestly.

Request backends carry the gate:
- There is exactly one `RequestContext::new` (`mod.rs:514`), and it is followed by `with_spawn_gate`.
- Every `Git2Backend` literal either carries the gate (`gitbackend.rs:272`) or is a fresh backend with no host.
- The production `Git2Backend::new()` callers are read-only (`classify`, `handle_diff`, `tagged`) or not transport (`create_workspace`).
- The SSH password route reuses the same slots, and therefore the same gate (`session.rs:542-549`).

### Scope

The gate is per host. Two concurrent gwz-py operations (`submit` threads, `call` with the GIL released) have separate gates. The C9 "Scope" paragraph and §6 ("A process-wide host") state this explicitly, and D25 itself says "in the host context". I treat it as a stated limitation for the operator, not a defect.

What a stray realistically gets is bounded:
- It holds the helper's stdin read end; the request on it carries `url=` only.
- It holds the stdout and stderr write ends, which stall the lookup until the stray exits, or let a malicious stray forge an answer.
- It cannot read the answer.

This matches the P3 class D19 assigned.

### Deadlock and lock order

- The gate's mutex is a leaf. Nothing under the logical hold takes another gwz lock: the Windows launch path (`ProcessTree::new`, `pipe::create`, attributes) has no Mutex or static, and `output()` takes only std's private `CREATE_PROCESS_LOCK`.
- No nested `hold` exists. `helper_credential` calls into git2-rs, which does not call back into gwz.
- The runner's hold is taken and dropped inside a single poll (`try_hold` → `process_tree::spawn` → `drop`, with no `.await` between). So a current-thread executor whose thread blocks in `hold()` elsewhere cannot deadlock against it.
- Every release notifies one waiter, and waiters re-check `while *held`, so there is no lost wakeup when a poller wins the race.
- Poisoning cannot leave a stale `true`: the flag is only flipped under the lock, and `into_inner` is used.

### Starvation and CPU

- A launch behind a queue of std spawns waits roughly N × CreateProcess time, which is bounded.
- Behind a GCM prompt, it is bounded by its own interaction clock (D25 accepted; P3-2 covers the wording).
- Polling costs at most 8 launches per host × one wake per ms, nearer one per 15.6 ms at the default Windows timer resolution. That is negligible.

### Cancellation and deadlines

- `hold_spawn_gate` runs `check()` (setup, token, owner, deadline) before each `try_hold`. A lookup that gives up starts nothing; the marker rows show this, and they were red first.
- No hold survives an error or panic. A failed spawn drops the hold (`a_spawn_that_fails_releases_the_gate`), and unwinding drops `SpawnHold`.
- `output()` and `spawn()` release before the wait, so a hook or GPG prompt under `git commit`/`git tag -s` does not hold the gate.
- `SpawnGate::output` sets exactly the stdio std's `output()` defaults to. None of the three call sites sets stdio explicitly, so behaviour is unchanged.

### Fail-closed

- `SpawnHold`'s field is module-private, so only `hold()` and `try_hold()` can mint one.
- `InheritWindow::open` and `launch::create` require `&SpawnHold`.
- `with_exclusion` has no production caller other than `new()` (which passes `cfg!(windows)`), and `spawn_for_test` is `cfg(test)`.
- Residual: the type proves *a* hold, not *the host's*. P3-3's test covers the identity.
- On Unix the no-op is harmless. The ends come from std and tokio pipes created with `O_CLOEXEC`/`pipe2`, so a concurrent `posix_spawn` or `fork`+`exec` cannot keep them.

### Tests

- The Windows control row observes inheritance through the OS: a PowerShell handle walk that matches pipe names, not a mirror of the code.
- The gated row was not run red on Windows. The control row is effectively its red form (the same probe spawned inside the window sees all three names).
- The gated row's 750 ms negative check could pass vacuously if the spawner thread were slow, but a broken gate would usually turn it red through the probe.
- The Mac rows with `with_exclusion(true)` are meaningful wiring rows, and red logs exist for each group:
  - gate unit 2/7;
  - runner 3/4 (the fourth is a both-ways row);
  - backend sites 4/4.

### Repository rules

- The diff adds no bare `#[cfg]`. The new test modules are `cfg_if!`-wrapped, and `cfg!(windows)` is an expression.
- All control flow is braced.
- No globals or thread-locals; `GATE_POLL` and `PIPE_NAMES_PROBE` are consts.
- No production code under `tests/`.
- `#[allow(dead_code)]` on `try_hold` is justified: it is used only by the candidate build's runner and by tests, and is dead only in an ordinary non-test build.
- The one exception is the import in P3-4.

## Follow-ups (out of scope)

**Operator decisions:**
- How to cover gwz-sspi's window (P3-1): coarse gating, or a 0.1.2 seam.
- A process-wide host for gwz-py, so that concurrent operations, including `log`'s `rev-list`, serialize. CS3.6 is the stated vehicle.
- Whether to replace the whole-call hold on libgit2's helper with gwz's own `git credential fill` through `SpawnGate::output`. This is the allowlist debt at `process_globals_allowlist.json:22`. It would remove P3-2's scenario entirely.

**Found outside this diff:**
- `commit_log/mod.rs:318`: `#[cfg(test)] let _reader_probe = …` is a bare `#[cfg]` on an unbraced declaration. It predates this commit, so it is a migration item, not a finding here.
- The Windows candidate test build carries several other unused-import warnings (`setup_retry.rs:26`, `ssh_tests/network.rs:3`, among others). They are pre-existing, and no Windows job denies warnings.

## Risks and next action

**Risks:**
- The gate only works on Windows, but its wiring is only exercised by hand-built gates (P3-3).
- On Windows, mixed native-route and transport workspaces now serialize behind a user prompt (accepted in D25, misreported per P3-2).
- S7.1's D25 precondition is ambiguous until the operator rules on gwz-sspi (P3-1).

**Next action:**
1. Correct the C9 and OQ-H wording and raise the gwz-sspi route to the operator (P3-1).
2. Move the import (P3-4).
3. Add the host-chain identity row (P3-3) and the gate-timeout classification row and wording (P3-2).
4. Re-bind the Windows receipt to the landed commit (P3-5).

None of these blocks GO.

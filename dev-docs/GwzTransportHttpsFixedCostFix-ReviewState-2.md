# HTTPS fixed-cost fix: state-axis re-verdict (round 1 remediation)

**Review object:** the round-1 remediation, gwz-core `git diff 1d038c6e..85d7d910`. It is one commit, `85d7d910` ("Close round 1 of the HTTPS fixed-cost fix's review: TLS 1.2 floor, full-duplex stream, no default-paths fallback"), touching 10 files (+480/-78). It is judged on top of the object reviewed in round 1: `d6446749` (reaper join) and `5cea51dc` (OpenSSL TLS client), as merged into `1d038c6e`.

**Baseline (review workspace `/Volumes/projects/limbo/gwz-dev-perf`):**
- gwz-core `85d7d9101ec4aa79c9b74accc914bae82bc607d2`
- gwz-transport `a27817b9c86fd60a07a9e77d62b1f5e94a5cf252` (reference only)
- gwz-core-evidence `db363555a1ed153ad710894a55a75f73f8c9887d` (reference only)
- lane root `7d77e21973bd10c58b74b69e54a51e53409464cf` (out of scope)

I checked all four at the start and again at the end; none moved.

**Date:** 2026-10-14. **Axis:** STATE (durable-state semantics and adversity). Independent, adversarial, read-only; same reviewer as round 1, context intact.

**Verdict: GO.** There are no open P0, P1 or P2 findings. My round-1 findings P2-1 and P3-1 are both closed and verified. One new P3 (P3-2) is not blocking.

This honours my round-1 pre-commitment: P2-1 is resolved as specified.

## Prior-finding closure table

| ID | Disposition claimed | Verified on corrected tree | Status |
|---|---|---|---|
| P2-1: the OpenSSL context dropped native-tls's TLS 1.2 floor and overrode the system policy floor | `base()` sets `set_min_proto_version(Some(SslVersion::TLS1_2))` and `set_max_proto_version(None)`, which are native-tls 0.2.18's `TlsConnectorBuilder` defaults. The module and function comments and the earlier README are corrected. The context row stops normalizing its reference. There is a new negative row, `a_server_capped_at_tls_1_1_is_refused`, run in a child process under a lax `OPENSSL_CONF`. | See "P2-1: how the closure was verified" below. | **Closed, verified** |
| P3-1: `poll_read` waited for unsent ciphertext to drain before reading | `poll_read`'s `WouldBlock` arm makes the send opportunistic: it propagates only `Ready(Err)` and goes on to `poll_fill` when the send is `Pending`. `poll_write` keeps its drain-first gate, and its `WouldBlock` arm only fills. A module note says the stream is polled from one task. | See "P3-1: how the closure was verified" below. | **Closed, verified** |
| §3 follow-up: the ClientHello oracle cannot see protocol-range differences | Doc comment added. | `tests.rs` gained a comment above `the_client_hello_is_what_native_tls_sends`. It says the oracle cannot see protocol-range differences on OpenSSL 3 at security level ≥ 1, and it names the two rows that can. | **Closed** |
| §3 follow-up: S7.1 must carry `prepare.py`'s target tables into the product manifest | Recorded in the RemPlan as a follow-up for a later round. | This is a follow-up, not a finding, so nothing was due this round. | **Recorded (not in this round)** |

**P2-1: how the closure was verified**

- **Code read.** At `85d7d910`, `openssl_stack.rs:146-147` sets `set_min_proto_version(Some(SslVersion::TLS1_2))` and `set_max_proto_version(None)`.
- **The original static counterexample no longer holds.** In round 1, `base().min_proto_version()` was `None` while native-tls's was `Some(TLS1_2)`. The context row now asserts `Some(TLS1_2)` and `max == None` directly. Its reference applies native-tls's floor, with a comment citing `TlsConnectorBuilder`. It also compares the options and the mode mask.
- **The original wire counterexample no longer holds.** In round 1 this was "a server capped at TLS ≤ 1.1, on a host whose policy permits it, completes the handshake". The new row recreates exactly that host state: a child process with `MinProtocol = TLSv1` and `@SECLEVEL=0`.
- **The row discriminates.** It first proves the host can complete TLS 1.1, using a control context with a TLS 1.0 floor, and only then requires the product `Connector` to refuse.
- **Red, then green, on weftpi.** In `raw/tdd/red-1.log`, the child printed "the control (a TLS 1.0 floor) completes TLS 1.1 here" and then failed with "the context completed a TLS 1.1 handshake"; the context row failed with `left: None`. In `green-1.log`, all 21 `openssl_stack` tests pass. The full Linux candidate leg is 3,908 passed, 0 failed.

**P3-1: how the closure was verified**

- **Code read.** `stream.rs:145-156`: on `Pending`, `poll_send` falls through to `poll_fill`.
- **The original counterexample no longer reproduces.** The counterexample was a peer that writes 8 MiB before it reads, against a client that writes and reads concurrently from one task. It is now the row `our_stream_writes_and_reads_8_mib_at_once_against_a_peer_that_writes_first`, bounded at 30 s. A second row covers an early 413 answer while a 16 MiB body is blocked, bounded at 2 s.
- **Both rows have native-tls oracles,** through `tokio-native-tls`.
- **Red, then green.** `red-1.log` has "the 8 MiB exchange stalled: Elapsed" and "the response head did not arrive within 2 s". `green-1.log` passes both rows and both native-tls controls.

## Changed-range analysis

**What changed since `1d038c6e`, mapped to the RemPlan:**

- **`openssl_stack.rs`**
  - TLS 1.2 floor (State P2-1).
  - `Store::add_defaults` removed. `fill` is infallible and adds the loader's roots (none without a loader) plus the endpoint's (Code P3-2).
  - Doc corrections.
- **`stream.rs`:** the opportunistic send in `poll_read`, and the single-task note (State P3-1 and Code P3-1).
- **`openssl_stack/tests.rs`**
  - Context row hardened, including the mode mask (Code §3).
  - Recorder simplified; the two default-path rows removed.
  - New rows: `without_a_loader_the_store_is_empty`, the TLS-floor child-process row, the two stream rows and their native-tls controls.
  - The ClientHello doc note (State §3).
- **`https_fixture.rs` (cfg(test) only, `endpoint/mod.rs:54-60`):** `Server::scripted` and the `Script` type, for the scripted peers.
- **`https_fixture/tests.rs`:** a `cfg_if` helper `platform_roots()`, which gives an explicit empty loader on OpenSSL and `None` on macOS and Windows, plus a cross-platform scripted-server row. That row was added to remove two new dead-code warnings on macOS and Windows; this is outside the RemPlan's wording, but it is test-only and justified.
- **`https_tls.rs`:** header doc (Code doc nit).
- **`native_stack.rs`:** a one-word doc fix.
- **`prepare.py` and `test_prepare.py`:** `tokio-native-tls` added as a dev-dependency of the OpenSSL targets, for the oracles. It is already in the lock.
- **`windows_parity/0.5.json`:** one `paired` row for the new `cfg_if` arm.

Nothing falls outside the dispositions except the warning-fix row, which is test-only, in scope and proportionate. No production behaviour changes beyond the floor, the read path and the no-loader store.

**The implementer's two deviations, judged:**

1. **`poll_write` keeps its drain-first gate. Accepted.**
   - Waiting for socket writability before encrypting more is ordinary write backpressure. It is what `tokio-native-tls` does, since `SSL_write` returns `WANT_WRITE` while a record is pending.
   - It bounds `outbound` to about one record.
   - The defect was the *read* waiting on the write, and that is removed.
   - The `WouldBlock` arm of `poll_write` (OpenSSL needs to read during a write, i.e. renegotiation) only calls `poll_fill`. It never waits on a read in a way native-tls did not.
   - I found no interleaving where a pending write now blocks a read, or a pending read blocks a write that native-tls would have completed.
2. **The single-task polling note. Accepted.**
   - tokio keeps one waker per socket direction. If two *tasks* polled the two halves of a `split` stream, the read task's opportunistic send would replace the writer task's write-readiness waker.
   - The same hazard exists under `tokio-native-tls`, where `SSL_read` can write through `AllowStd`, and it existed equally before this round.
   - The only consumer is hyper's connection task. The inner stream of a proxied connection polls the outer stream from that same task, and the CONNECT exchange is sequential on one task.
   - So the note documents an existing constraint; it is not a regression.

**Regressions hunted, none found:**

- **No-loader store (Code P3-2).** `Connector::build(None, …)` on OpenSSL now trusts only the endpoint's roots. In the product the loader is always present on these targets (`endpoint_environment.rs:209`, unconditional inside the non-Apple, non-Windows `cfg_if` arm). The change can only narrow trust, never widen it.
- **The floor in both directions.** An explicit TLS 1.2 minimum also overrides a *stricter* distribution floor such as `MinProtocol = TLSv1.3`, lowering it to 1.2. That is exactly what native-tls did, so it is parity with the replaced stack and not a regression; see §3.
- **New opportunistic-send states.**
  - `poll_send` `Pending` with `poll_fill` `Pending`: both directions are registered on the one task waker, and a spurious wake is harmless.
  - `poll_fill` reaching EOF with `outbound` unsent: the session is ending, and no progress is invented.
  - Outbound produced during a read, such as a KeyUpdate response, still leaves on the next `WouldBlock` read, write or flush, as before.
  - I found no new stuck state.

**New finding:** P3-2, below.

## 0. Evidence base

**Read in full:**
- The whole `1d038c6e..85d7d910` diff.
- `stream.rs` `poll_read`, `poll_write`, `poll_flush` and `poll_shutdown` at `85d7d910`.
- The RemPlan (`/Volumes/projects/limbo/gwz-dev/gwz-core/dev-docs/GwzTransportHttpsFixedCostFix-RemPlan.md`).
- The rem1 evidence README.
- `endpoint/mod.rs:54-66`, which shows the fixture is cfg(test).

**Sampled:**
- `raw/tdd/red-1.log` and `green-1.log`: the rows, the failure messages and the `tls-floor` lines.
- `raw/gates/pi-rem1-candidate-leg.log`: 15 `openssl_stack` "ok" lines and the result lines.
- `runner/pi-run.sh`: how the rows were run. Without `--nocapture` the parent's printed lines are captured.
- I grepped the evidence tree for `tls-floor`. It occurs only in the README, the patch and `red-1.log`.

**Not read:** the Windows receipts beyond the README summary. `perf-20261014-d` failed on an unrelated SSH key-exchange row in code this round does not touch; `-e` and `-f` passed.

**Commands run** (gwz-core at `85d7d910`):
- `check_windows_parity.py --root .`: nothing new (unported 1, paired 154, platform 39).
- `check_cfg_boundaries.py`: nothing new.
- `check_process_globals.py`: nothing new.
- `check_candidate_switches.py`: 23 sites match.
- `check_checked_artifact_boundaries.py --source src`: ok.
- `prepare.py` into `/Volumes/projects/limbo/build-targets/reviewtls2-state-cand`, then a candidate `cargo +1.95.0 test --lib -- https agent_job` with target `.../reviewtls2-state-target`: **452 passed, 0 failed**.
  - This covers macOS `native_stack`, the fixture's explicit-loader helper, the new scripted-server row and the reaper rows.
  - The `--no-run` warning list: two of its locations are in this round's area (`https_fixture.rs:68`, `leaf_digest` used only by Linux rows, and `https_fixture/tests.rs:52`, a field of `Failed`). Both pieces of code were already present at `1d038c6e`, so neither warning is new.
  - `openssl_stack` is not compiled on this Mac. The Linux rows were judged by reading the code and the weftpi logs.
- No file was written or modified. No git or gwz mutation was run, and nothing was written through the prepared tree's links.

## Problem, requirement provenance, and scope (changed range)

- **Problem and requirement.** The remediation closes review findings on the authorized speed fix. Its one trust-relevant change, the TLS 1.2 floor, *restores* the parity that the directive "trust semantics must not change" required. It needs no operator decision, as the RemPlan says.
- **Code P3-2's removal of the default-paths fallback** is proven dead in the product: the loader is always supplied on the OpenSSL targets. That falls under the standing "remove proven-dead code" rule, and its direction is fail-closed.
- **Scope.** All changes are in the stated files. The test-only additions (scripted server, oracle dev-dependency, parity row) are necessary for the named closure tests or for warning parity. I found no unauthorized expansion and no unsupported prerequisite.

## 1. Findings

### P3-2: the TLS-floor row can pass vacuously, and the archived logs cannot show which branch ran

**Location:** `src/git/endpoint/https_backend/openssl_stack/tests.rs`, `a_server_capped_at_tls_1_1_is_refused`:
- the child's host-conditional early `return` after the `"tls-floor: host-conditional: …"` `println!`;
- the parent's re-print of the `tls-floor:` lines with `println!`.

**Violated invariant:** the RemPlan's closure clause says the negative row is "recorded as host-conditional with the condition printed".

**Root cause.**
- When the control cannot complete TLS 1.1, the child returns early and *passes*.
- The parent then re-prints the condition with `println!`, which libtest captures for passing tests.
- In a normal run (`run_tests.py`, or `pi-run.sh` without `--nocapture`), a vacuous pass is therefore indistinguishable from a live one.

**State sequence / reproduction.**
1. Run the Linux suite on an OpenSSL built without TLS 1.1 (`no-tls1_1`), or under a provider configuration that refuses TLS 1.1 even at `SECLEVEL=0`.
2. The control fails and the child takes the host-conditional branch.
3. The row reports `ok`, and no output appears in the log.

The archived `green-1.log` is an instance of the logging gap: it has no `tls-floor` line. The rem1 README states that the green run printed "the product's connector refuses TLS 1.1", but that text is not in the archived log. For weftpi, the live branch is still established indirectly: `red-1.log` shows the control (identical code in both trees) completing TLS 1.1 on the same host, so the green pass there is live.

**Impact:**
- Coverage and auditability only. The floor itself is also pinned by the context row, which is unconditional.
- On a future host the negative wire row could go silently vacuous.
- The rem1 README's quoted green output cannot be verified from the archive.

**Required correction:**
- Make the host-conditional outcome visible in normal runs. For example, the parent writes the `tls-floor:` lines through `std::io::stderr().write_all`, which is not captured by libtest, or fails unless an explicit opt-out variable names the host as lacking TLS 1.1.
- Correct the README's green-run sentence, or archive a `--nocapture` run of that row.

**Closure / regression test:** a normal (captured) run of the row on weftpi whose log contains "the product's connector refuses TLS 1.1".

## 2. Invariant analysis

**Fail-closed trust**
- The floor is now fixed at TLS 1.2 regardless of `openssl.cnf`. The red and green child-process rows show this under a lax policy.
- The store holds the loader's roots plus the endpoint's and nothing else. With no loader it holds only the endpoint's: 0 or 1 in `without_a_loader_the_store_is_empty`, which was red with 150 before the change.
- `add_cert` failures still only narrow trust.
- Hostname, SNI, verify mode, options, ciphers and now the mode mask all match `SslConnector`. I found no widening.

**Stream state machine**
- **Read path.** It no longer depends on write readiness, so the round-1 deadlock interleaving does not occur. The 8 MiB and early-413 rows pass with native-tls as control.
- **Write path.** Drain-first backpressure is preserved.
- **Shutdown, truncation and handshake classification.** Unchanged since round 1, where they were already at parity with `tokio-native-tls`. The `Watched` wrapper's `Io` vs `Trust` split is untouched.

**Reaper (`d6446749`)**
- Unchanged since round 1, where it was GO.
- Re-run on this tree: the `agent_job` rows pass as part of the 452 tests.

**Repository rules**
- The new conditional code sits in a `cfg_if` (`https_fixture/tests.rs` `platform_roots`).
- The scripted server is cfg(test).
- The child process gets its environment through `Command::env`, so there is no process-global mutation; the globals check is clean.
- No `unsafe`. No file over 1000 lines (`openssl_stack/tests.rs` is 697).

## 3. Follow-ups outside the finding bar

- **Explicit TLS 1.2 floor vs a stricter policy.** The explicit floor also lowers a stricter distribution floor (`MinProtocol = TLSv1.3`) to 1.2. This is native-tls parity, which is what this round required. gwz 1.0.17 (libgit2) left the bounds to the system policy instead. If the operator wants "max(1.2, policy)", that is a separate trust decision. The `base()` comment describes only the downward case and could mention both.
- **RemPlan follow-ups that still stand:**
  - `openssl_probe::probe()` reads the process environment, which leaks past the snapshot in 1.2.0;
  - the vacuous control row `a_host_that_has_default_verify_paths_shows_the_loader_leaves_them_out`;
  - S7.1 manifest tables;
  - x86_64 Linux and OpenSSL 1.1.1 are still unexercised.

## 4. Risks and next action

**Residual risks:**
- The Linux code compiles and runs only on weftpi (aarch64, OpenSSL 3.5.6).
- The negative floor row's live or vacuous status is not visible in captured runs (P3-2).

**Next action:**
- The object, `d6446749` + `5cea51dc` + `85d7d910`, is GO on the state axis.
- Address P3-2 at the lane owner's discretion; it does not block, and it needs no re-review beyond a log showing the live branch.

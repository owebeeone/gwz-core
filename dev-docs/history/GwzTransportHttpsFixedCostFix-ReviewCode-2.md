# HTTPS fixed-cost fix: code-axis re-verdict (round 1 remediation)

**Review object:** gwz-core `85d7d910` ("Close round 1 of the HTTPS fixed-cost fix's review: TLS 1.2 floor, full-duplex stream, no default-paths fallback"). The changed range is `git diff 1d038c6e..85d7d910`: one commit, 10 files, +480/-78. It sits on top of the object reviewed in round 1 (`d6446749` plus `5cea51dc`, merged in `1d038c6e`).
**Baseline:** review workspace `/Volumes/projects/limbo/gwz-dev-perf`:
- gwz-core `85d7d9101ec4aa79c9b74accc914bae82bc607d2`
- gwz-transport `a27817b9c86fd60a07a9e77d62b1f5e94a5cf252` (reference only)
- gwz-core-evidence `db363555a1ed153ad710894a55a75f73f8c9887d` (reference only)
- lane root `7d77e21973bd10c58b74b69e54a51e53409464cf` (out of scope)

The tuple was checked at the start and at the end, and it did not move.
**Date:** 2026-10-14. **Axis:** CODE (architecture, interfaces, call graphs, compatibility). Independent, adversarial, read-only. Same reviewer as round 1, with that context.

**Verdict: GO.** P0 0, P1 0, P2 0, P3 0 open. Round-1 P3-1 and P3-2 are closed, and there are no new findings. One residual case of P3-1 that falls outside the correction I specified, and one test-reporting gap, are recorded in §3.

**Correction to my own round-1 report.** My §2 said `base()` reproduces native-tls's protocol bounds ("`set_min/max_proto_version(None)`, as native-tls `supported_protocols(None, None)` does"). That was wrong. native-tls 0.2.18's `TlsConnectorBuilder` defaults to `min_protocol: Some(Protocol::Tlsv12)` (`src/lib.rs:490`), so the original object had lowered the floor. The State reviewer found this as P2-1. I did not, because I compared against `supported_protocols` but not against the builder's defaults, and the original test normalized the reference to `None`. This round closes that finding too, and I have checked the closure below because it is in the code range I review.

## Prior-finding closure table

| ID | Disposition claimed | Verified on corrected tree | Status |
|---|---|---|---|
| Code P3-1 (a read waited for unsent ciphertext to drain) | `poll_read`'s `WouldBlock` arm sends only if it can: it propagates `Ready(Err)` and goes on to `poll_fill` when the send is `Pending`. `poll_write` keeps its drain-first gate (a stated deviation). The module doc says a stream is polled from one task. Closure rows: 8 MiB both ways against a peer that writes first, and an early 413 while a 16 MiB body is blocked, each with a native-tls control. | **Re-traced my counterexample.** In `stream.rs:150-155`, a `WouldBlock` from `SSL_read` with outbound unsent and the socket not writable now leaves `poll_send` `Pending` (waker registered) and goes on to `poll_fill`, which reads the 413 head. Nothing in the read path waits on the write path any more. **Closure tests exist and assert it:** `our_stream_reads_an_early_answer_while_a_16_mib_body_is_blocked` (`tests.rs:694`, head within 2 s, `starts_with(b"HTTP/1.1 413")`) and `our_stream_writes_and_reads_8_mib_at_once_against_a_peer_that_writes_first` (`tests.rs:632`, 30 s bound). Both drive the split halves from one task with `try_join!`/`select!`, which is the documented polling model. The native-tls controls run the same helpers. **weftpi logs:** in `raw/tdd/red-1.log` both rows fail ("the response head did not arrive within 2 s: Elapsed", "the 8 MiB exchange stalled: Elapsed"); in `green-1.log` and `gates/pi-rem1-candidate-leg.log` both pass, and so do the two native-tls controls. The red-1 results show both rows would have caught the original defect. **Residual:** a `poll_send` *error* still preempts reading bytes that are already buffered (the peer-reset case, which round 1 noted as "a related case"). My required correction kept that propagation, so this is a follow-up and not an open finding (§3). | **Closed** |
| Code P3-2 (product-unreachable default-paths fallback) | The OpenSSL `Connector::build(None, ..)` gets an empty store. `Store::add_defaults`, the `DefaultPaths` event and the two default-path rows are removed, and `fill` cannot fail. The fixture tests take their platform roots from a `cfg_if` helper: an explicit empty loader on OpenSSL, `None` on macOS and Windows. | **Re-traced.** `openssl_stack.rs:166-173` is `platform.map(..).unwrap_or_default()` followed by `roots`. No branch remains. `git grep` at `85d7d910` finds no `set_default_paths`, `set_default_verify_paths`, `add_defaults` or `load_verify_locations` call in `src` (only doc text). The process environment can no longer reach the OpenSSL trust store through this path. **Closure test:** `without_a_loader_the_store_is_empty` (`tests.rs:396`) asserts 0 certificates with no roots and 1 with one CA root. It went red on weftpi before the change (`left: 150, right: 0`) and is green after. `a_client_that_does_not_trust_the_fixture_ca_is_refused` passes with the explicit loader: on weftpi (leg log) and on this Mac (native arm, run below). | **Closed** |
| Round-1 §3: mode mask not compared | The context row compares `set_mode(SslMode::empty())` against `SslConnector`'s. | Present in `the_context_is_the_one_sslconnector_makes_bar_its_trust_roots` (`tests.rs:139`). `set_mode` returns the mask after OR-ing, so OR-ing nothing reads it back correctly. Green on weftpi. | Closed |
| Round-1 §3: `https_tls.rs` header doc nit | The header now says the bundle decode is on the OpenSSL build only. | Diff read; correct. | Closed |
| Round-1 §2 error (not a filed finding; State P2-1): the floor | `base()` sets `set_min_proto_version(Some(TLS1_2))` and `set_max_proto_version(None)`. | `openssl_stack.rs:146` matches native-tls 0.2.18's `TlsConnectorBuilder` defaults (`lib.rs:490`) and the order of `supported_protocols` after `SslConnector::builder`. The context row asserts `Some(TLS1_2)` and `None` without normalizing. `a_server_capped_at_tls_1_1_is_refused` runs a child under a lax `OPENSSL_CONF` (SECLEVEL 0, MinProtocol TLSv1). There, a control context with a TLS 1.0 floor completes TLS 1.1 against a server capped at TLS 1.1, and the product connector must refuse the same server. In red-1 the child printed "the control (a TLS 1.0 floor) completes TLS 1.1 here" and then failed; in green and in the leg it passes. The control's result does not depend on the fix, so the weftpi green is the live branch and not the host-conditional one. | Closed (State's finding; corroborated on this axis) |

## Changed-range analysis

What changed since `1d038c6e`, file by file. The evidence `patch/gwz-core-round1-since-1d038c6e.patch` has the same `+`/`-` lines as the commit.

- **`openssl_stack.rs`:** the TLS 1.2 floor, the fallback removed, and corrected docs. The new doc says an explicit minimum overrides a distribution's `MinProtocol`. That matches native-tls, which also sets the minimum explicitly after `SSL_CTX_new` applies `system_default`, so the behaviour against a host's `openssl.cnf` is now the same as the replaced stack in both directions (floor and ceiling). The `ErrorStack` import is still used (`base`, `context`, `from_der`), and the evidence shows no new warning.
- **`stream.rs`:** the `poll_read` change above, and the module note about polling from one task.
  - **The `poll_write` deviation is acceptable.** The gate at the top of `poll_write` (`:172`) waits only for this stream's own outbound record to drain, which is backpressure that keeps outbound to one record (`ENABLE_PARTIAL_WRITE`). It never waits on a read. Its `WouldBlock` arm (`:182`) fills only when `SSL_write` needs input, which happens only on TLS 1.2 renegotiation (TLS 1.3 writes never read). The finding's counterexample is a read blocked behind a write, and that coupling is gone.
  - **The single-task note is accurate, and the constraint is not new.** The round-1 adapter already called `poll_send` from `poll_read` and registered the reader's waker for write readiness. In gwz the stream is polled only by hyper's connection task and by the sequential CONNECT and handshake code in `https_connection::connect`. `git grep` finds `tokio::io::split` only in the two new tests, and both poll the halves from one task. Inside one task the read and write wakers are the same, so no wakeup is lost.
  - **No new loop hazards.** A `Pending` send followed by a `Ready` fill always makes progress; EOF still ends the loop through `Wire` returning `Ok(0)`.
  - **Unsent output is still sent.** When a read completes with outbound unsent, the write-readiness waker remains registered, and the next `poll_read`/`poll_write`/`poll_flush`/`poll_shutdown` drains it. hyper's `poll_loop` calls `poll_flush` on every pass.
- **`native_stack.rs`, `https_tls.rs`:** doc comments only.
- **`https_fixture.rs`:** adds `Server::scripted` and the `Script` type. The module is compiled only under `cfg(test)` (`endpoint/mod.rs:54-60`, inside `cfg_if`), so no production code is added. The cross-platform test `a_scripted_server_hands_each_connection_to_its_script_after_the_handshake` uses it everywhere (it was added to clear two warnings on macOS and Windows that came from Linux-only use).
- **`https_fixture/tests.rs`:** the `platform_roots()` helper is inside `cfg_if` with both arms. Parity gains one `paired` row (`0.5.json`).
- **`prepare.py`, `test_prepare.py`:** `tokio-native-tls = "=0.3.1"` becomes a dev-dependency on OpenSSL targets for the native-tls controls. It is already in the lock and pinned by the test. Product dependencies are unchanged.

Every change maps to a disposition in the RemPlan. Nothing falls outside them except the fixture extension, which the closure rows need, and the warning-fix test, which only restores the earlier warning count. I found no regression from the remediation. No root cause is architectural.

**Common check (problem, necessity, scope) for the changed range.**
- Each change traces to a filed finding or to the RemPlan's dispositions. The floor restores the native-tls behaviour the lane owner's directive requires ("trust semantics must not change"), so it needs no operator decision.
- The changes stay inside gwz-core in the perf lane, plus evidence. No product dependency, public API or wire format changes.
- The re-time on weftpi (`raw/rem1-row1.txt`, 1 member, paired median −0.005 s) shows the fixed-cost gain survives the remediation, as expected: the floor and the read path change neither the connector build nor the first record.

## 0. Evidence base

**Read in full:**
- The RemPlan (`/Volumes/projects/limbo/gwz-dev/gwz-core/dev-docs/GwzTransportHttpsFixedCostFix-RemPlan.md`).
- The whole changed-range diff.
- The `rem1` evidence README.
- The corrected parts of the earlier run's README (the protocol-bounds and fallback paragraphs).

**Read in part:**
- `stream.rs` at `85d7d910` (lines 136–195).
- `endpoint/mod.rs` lines 52–64.
- The weftpi logs `raw/tdd/red-1.log`, `raw/tdd/green-1.log` and `raw/gates/pi-rem1-candidate-leg.log`, searched for the closure rows and result lines. The leg sums to 3,908 passed, 0 failed.
- native-tls 0.2.18 `src/lib.rs`, `min_protocol` defaults.

**Not read:** the Windows receipts (I relied on the README: `-d` failed on an unrelated SSH `kex_retry` row, `-e` and `-f` pass, and the changed range touches no Windows product code), and the Mac gate logs.

**Commands run (from gwz-core):**
- `check_windows_parity.py --root .`: 1 unported, 154 paired, 39 platform, "nothing new".
- `check_cfg_boundaries.py`, `check_process_globals.py`: "nothing new".
- `check_candidate_switches.py`: 23 sites match.
- `check_checked_artifact_boundaries.py --source src`: ok.
- `git grep` for default-paths calls, as above.
- A patch-against-commit line comparison: identical.
- `prepare.py` into `/Volumes/projects/limbo/build-targets/reviewtls2-code-cand`, then a candidate `--lib` test with target `reviewtls2-code-target`, filters `https agent_job ca_trust ca_bundle endpoint_environment`: **477 passed, 0 failed** on this Mac. This covers the native arm of the fixture helper, the scripted-server test and every `https` row.
- The OpenSSL stack does not compile on this Mac, so its rows were verified by reading the code and the weftpi logs.

## 1. Findings

None open. Round-1 P3-1 and P3-2 are closed (see the table above). No new finding IDs.

## 2. Invariant analysis (attacks that failed)

- **Floor and ceiling against `openssl.cnf`.** A host policy that raises `MinProtocol` above 1.2 is overridden down to 1.2, and one that lowers it is raised to 1.2. A `MaxProtocol` is cleared. This is native-tls's exact behaviour. No certificate or chain handling changed; the store and host-name code are untouched.
- **Waker and progress.** I tried the cases where the send is `Pending` and the fill is `Pending`, `Ready` or EOF, and where the read completes with outbound still unsent. None leaves output stranded or loops without progress when polled from one task.
- **The `poll_write` gate.** It cannot wait on a peer's writes, because it waits only on its own socket's write readiness. The renegotiation-only fill arm cannot leave application ciphertext stranded, because it goes into `Wire.inbound`, which the next `SSL_read` consumes.
- **Fallback removal.** No caller on OpenSSL depended on `None` meaning default paths: `endpoint_environment` always supplies the loader, and the fixture now passes an empty loader. macOS and Windows behaviour is unchanged: `native_stack` with `None` still means the platform's own roots.

## 3. Follow-ups outside the finding bar

- **Residual of round-1 P3-1 (peer reset).** `stream.rs:152-154` still returns a `poll_send` *error* from `poll_read` before reading ciphertext that is already buffered. So a peer that answers and then resets can lose its answer, where tokio-native-tls would have read the answer and reported the write error from `poll_write`. This is the narrow case I noted in round 1. The correction I specified kept the propagation, so I do not reopen it. A possible change: keep the error, keep reading, and raise it on the next `poll_write`/`poll_flush`. A closure test would be a scripted peer that writes a 413 and then closes with SO_LINGER 0 while the client is uploading.
- **The floor row's host-conditional pass is silent in captured logs.** The child's `tls-floor:` lines reach the gate logs only under `--nocapture`. On a host whose OpenSSL cannot complete TLS 1.1 even at SECLEVEL 0, the row passes with no visible trace. weftpi is the live case (shown by red-1). Emitting the condition on stderr, or failing on hosts where the row cannot run, would make it auditable.
- Unchanged from the RemPlan's recorded follow-ups: the process-environment read in `openssl_probe::probe()` for the 1.2.0 session host, the vacuous control row on hosts without a bundle, S7.1 carrying the three target tables, and x86_64 Linux / OpenSSL 1.1.1 not exercised.

## 4. Risks and next action

**Residual risk:**
- Linux behaviour is established on weftpi only (aarch64, OpenSSL 3.5.6).
- The 2 s and 30 s rows depend on timing and could flake on a heavily loaded host. None has failed so far.
- Windows receipt `-d` failed on an SSH key-exchange row outside the changed range; `-e` (same tree) and `-f` (final tree) pass.

**Next action:** GO on the code axis. Nothing in this range blocks merging or advancing the lane. The two follow-ups in §3 are optional hardening.

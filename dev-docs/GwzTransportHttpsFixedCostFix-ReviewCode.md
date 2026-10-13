# HTTPS fixed-cost fix (Linux OpenSSL TLS client, reaper join): code-axis review

**Review object:** two gwz-core commits as merged into `1d038c6e`. They are d6446749 ("Join a setup job whose result is stored…") and 5cea51dc ("Build the OpenSSL TLS client directly on the openssl crate"). The object diff is `git diff 0d501ee6..5cea51dc` (21 files, +1133/-110). The merge `5cea51dc..1d038c6e` leaves `https_backend*`, `agent_job.rs`, `https_tls.rs` and `https_handshake.rs` unchanged and touches no TLS line in `https_connection.rs`.
**Baseline:** review workspace `/Volumes/projects/limbo/gwz-dev-perf`. gwz-core `1d038c6ea0e36500713a0e3f2110caf382778c3c`, gwz-transport `a27817b9c86fd60a07a9e77d62b1f5e94a5cf252` (reference only), gwz-core-evidence `d5aed2facebf7902dc2676055da0f2e129e97896` (reference only), lane root `2f7b2fbe5fc9d4094421be402583e77336f8bc0f`. The tuple was checked at the start and again at the end, and nothing moved.
**Date:** 2026-10-14. **Axis:** CODE (architecture, interfaces, call graphs, compatibility). Independent, adversarial, read-only.

**Verdict: GO.** P0 0, P1 0, P2 0, P3 2.

## 0. Evidence base

**Read in full:**
- The whole object diff.
- `https_backend.rs`, `native_stack.rs`, `openssl_stack.rs`, `openssl_stack/stream.rs`, `openssl_stack/tests.rs`.
- `https_handshake.rs`.
- `agent_job.rs` at 1d038c6e, `agent_job/reap_tests.rs`, and the reaper loop in `agent_job/supervisor.rs`.
- The diffs to `https_tls.rs`, `https_connection.rs`, `ca_bundle.rs`, `verify_paths.rs`, `https_fixture*`, `ca_trust_tests.rs`, `endpoint_environment_tests.rs`, `prepare.py`, `test_prepare.py`, `README.md` and `0.5.json`.

**Read in part:**
- `https_connection.rs` `connect()` (lines 596–700, proxy and CONNECT path) and `HttpConnector::new`.
- `endpoint_environment.rs` `tls_config` and `platform_roots` (lines 130–215).
- `verify_paths.rs` header and `Paths::resolve`.
- TR2.7 erratum: `GwzTransportReleasePlanAmendment-2.md` §3.22 part E.
- Evidence READMEs: `2026-10-13-https-fixed-cost-pi`, `2026-10-14-https-fixed-cost-fix`, `2026-10-14-https-fixed-cost-fix-merged`, and the `https_backend` and `reap_tests` lines plus the result line of `pi-merged-groups.log`.

**Reference sources, read-only, compared line by line:**
- native-tls 0.2.18 `src/imp/openssl.rs` (`TlsConnector::new`/`connect`, `tls_server_end_point`, `shutdown`).
- openssl 0.10.81 `ssl/connector.rs` (`ctx`, `SslConnector::builder`, `configure`, `into_ssl`, `setup_verify`, `setup_verify_hostname`) and `ssl/mod.rs` (`SslContextBuilder::new`, `set_cert_store`, `SslStream` `read_uninit`/`ssl_read_uninit`/`Write::write`/`shutdown`/`make_error`).
- hyper 1.11.1 `proto/h1/conn.rs` `can_read_head` and `dispatch.rs` `poll_loop`.
- openssl-probe 0.1.6 `probe`.

**Not read:** the GwzRemoteTransportHttpsDesign.md §5 text itself (its trust and proxy behaviour was checked against the code, which did not change), the evidence `raw/` measurement files, and the rest of the tree.

**Commands run (from gwz-core unless stated):**
- `check_windows_parity.py --root .`: 1 unported, 153 paired, 39 platform, "nothing new", exit 0.
- `check_cfg_boundaries.py`: "nothing new", exit 0.
- `check_process_globals.py`: "nothing new", exit 0.
- `check_candidate_switches.py`: 23 sites match, exit 0.
- `check_checked_artifact_boundaries.py --source src`: ok, exit 0.
- `test_prepare.py` was not run (pytest is not installed for python3.13). The evidence records 14 passed.
- `prepare.py` into `/Volumes/projects/limbo/build-targets/reviewtls-code-cand`, then a candidate `--lib` test with external target `reviewtls-code-target` on this Mac (native_stack):
  - filters `agent_job https_fixture tls_share ca_trust ca_bundle endpoint_environment https_handshake`: 51 passed, 0 failed, including the 3 `reap_tests` and the 5 rewritten `https_fixture::tests`;
  - filter `https`: 436 passed, 0 failed.
- `rg` for `unsafe|thread_local|static|unwrap()|expect(` in the new non-test files: nothing except doc text and test asserts.
- The OpenSSL stack does not compile on this Mac. I did not run `cargo check --target x86_64-unknown-linux-gnu`, because openssl-sys and vendored libgit2 need a cross toolchain that is not set up here. Linux compile and test results come from evidence: `pi-merged-compile.log`, and `pi-merged-groups.log`, which shows all 12 `https_backend::openssl_stack` tests and 3 `reap_tests` passing on weftpi (1,237 passed, 0 failed), plus `pi-merged-candidate-leg.log`.

## Problem, requirement provenance, and scope

**Observed problem.** The attribution run `2026-10-13-https-fixed-cost-pi` found two costs on the Pi:
- 32.4 ms between "DNS done" and `connect()`. Of the build thread's 38 ms, 24 ms is two whole-bundle parses inside native-tls `TlsConnector::new`: `SslConnector::builder` → `set_default_verify_paths`, then the `openssl-probe` file and directories. Both parses fill a store that `disable_built_in_roots(true)` then replaces. I confirmed this sequence by reading native-tls 0.2.18, lines 286–321.
- A reaper race costing 20 ms in 7–76% of runs.

**Provenance.** OD17, plus the lane owner's directive to remove the attributed fixed cost. Speed is a goal; trust semantics must not change.

**Was the remedy necessary and proportionate?**
- native-tls 0.2.18 has no builder option to skip the default paths. `disable_built_in_roots` acts only after both parses. Its `SslContext` cannot be injected (the fields are private).
- "Build once" is already done once per endpoint (`SharedTls`, `OnceLock`). Each gwz command is its own process, so there is nothing to reuse across commands.
- So a localized fix inside native-tls was not available. The only ways to drop the throwaway parses were to fork or vendor native-tls, or to build the context directly. The lane chose the second, gated to non-Apple, non-Windows targets. macOS and Windows code is moved verbatim (`native_stack.rs`), and the Windows receipt `perf-20261014-c` passed.
- A smaller-code alternative existed for the async adapter: the `tokio-openssl` crate. The lane wrote its own 237-line adapter instead, which adds no package (`openssl` 0.10.81 was already in the lock through native-tls and openssl-sys). That is a design choice, not a scope expansion.
- The reaper change is about 40 lines in `agent_job.rs`. It is necessary because the attribution shows the build fix alone shifts the race to 51–61 of 80 runs.

**Scope.** All changes are in gwz-core, inside the lane. gwz-transport and evidence are untouched. `prepare.py` gets candidate-manifest target tables; the product `Cargo.toml` is unchanged. The evidence correctly flags that S7.1 must carry those tables. I found no unauthorized expansion and no unsupported prerequisite. This is an optimization, not a blocker fix, as the controlling directive frames it.

## 1. Findings

### P3-1: the OpenSSL stream's read path waits for the write path to drain (half-duplex coupling that native-tls did not have)

- **Location:** `src/git/endpoint/https_backend/openssl_stack/stream.rs:141-144`, in `AsyncRead::poll_read`, the `WouldBlock` arm: `ready!(self.poll_send(cx))?;` runs before `ready!(self.poll_fill(cx))?;`. It interacts with `poll_write` at `stream.rs:161-167`, which returns `Ready(Ok(count))` even when the record is still in `wire.outbound` because the socket returned `Pending`.
- **Violated invariant:** "Nothing else differs from what `native-tls` made" (`openssl_stack.rs` module doc), applied to I/O progress. With tokio-native-tls a blocked write left the record in OpenSSL's write buffer, `poll_write` returned `Pending`, and `SSL_read` still progressed on its own. Here, any unsent outbound record blocks every read that needs more ciphertext, even when the socket has inbound data waiting.
- **State sequence:**
  1. hyper streams a large POST body. Our `poll_write` accepts one record and its trailing `poll_send` gets `Pending`, because the socket send buffer is full.
  2. The server has sent an early response head (for example 413 or 401) and has stopped reading, with the connection kept open.
  3. hyper's client dispatcher polls read while writing (hyper 1.11.1 `can_read_head` is true once `writing != Init`; `poll_loop` calls `poll_read` first). `SSL_read` needs ciphertext, so `Wire` returns `WouldBlock` and `poll_send` returns `Pending`.
  4. `poll_read` returns `Pending` without calling `poll_fill`, so the response head is never read.
  5. Old stack: the response head reaches `send_request`. New stack: the request stalls until the connection's I/O timeout, and the failure class is a stall or `Io` instead of the server's HTTP status.
  
  A related case: if the peer resets after answering, the `poll_send` error is returned from `poll_read` before the already-buffered response bytes are read.
- **Impact:** bounded by the I/O timeout. It is a diagnosability and compatibility divergence for servers that answer early without lingering-close (nginx and Apache usually lingering-close, so they are not affected). It is not a trust issue. The code path is confirmed by reading; I have not found a production server flow in gwz's scope that triggers it.
- **Required correction:** in `poll_read`'s `WouldBlock` arm, attempt the send without gating the read on it. Propagate only `Ready(Err)` from `poll_send`, treat `Pending` as "keep going", then `poll_fill`. Alternatively, record the coupling as an accepted difference in the module doc.
- **Closure test (Linux, fixture):** a raw server that reads the request head, writes `HTTP/1.1 413` plus headers, then holds the socket open without reading. The client writes a 16 MiB body through `Stream` while concurrently reading. Assert the response head arrives within 2 s. Run the same with `native_tls` (the existing dev-dependency oracle) as the control.

### P3-2: product-unreachable fallback on the OpenSSL build that reads the process environment's default paths

- **Location:** `src/git/endpoint/https_backend/openssl_stack.rs:167` (`None => store.add_defaults()?` in `fill`) and `openssl_stack.rs:145-147` (`Store::add_defaults` → `X509StoreBuilder::set_default_paths`).
- **Violated invariant:** the operator's standing rule to remove proven-dead code (an unreachable fallback). Also the TR2.7 erratum (amendment-2 §3.22 E): on Linux the platform roots are exactly OpenSSL's default verify paths *read from the command's environment snapshot*.
- **Evidence that it is dead in the product:** on non-Apple, non-Windows targets, `endpoint_environment.rs:209` always sets `config.platform_roots = Some(...)`. `HttpConnector::new` (`https_connection.rs:271-274`) is the only product caller of `SharedTls::new`, and `Connector::build(None, …)` is called only from `https_fixture/tests.rs:27,90`. The evidence README says the same ("only tests build one").
- **State sequence:** any future product path that builds an `https_connection::Config` without `endpoint_config` (`Config::default()` leaves `platform_roots: None`) silently gets OpenSSL's default paths from the *process* `getenv(SSL_CERT_FILE/SSL_CERT_DIR)`. It also loses the `openssl-probe` locations native-tls added. That differs from both the erratum and the replaced stack, and nothing fails.
- **Impact:** a latent trust-semantics trap in new code, kept alive only by tests. There is no current product effect.
- **Required correction:** make the loader mandatory on the OpenSSL stack. For example, `openssl_stack::Connector::build` treats `None` as an empty store, or the OpenSSL `Config` cannot be built without a loader. The fixture tests then pass a loader (`https_fixture/tests.rs` already has the shape). Remove `Store::add_defaults` and its two tests, or record a reasoned exemption with the operator.
- **Closure test:** a unit test that `Connector::build(None, &[])` on the OpenSSL stack holds 0 certificates (or does not compile). The existing `a_client_that_does_not_trust_the_fixture_ca_is_refused` stays green with an explicit loader.

## 2. Invariant analysis (attacks that failed)

**Context equivalence with `SslConnector`/native-tls.** `base()` reproduces openssl 0.10.81 `ctx()` exactly:
- options: `ALL|NO_COMPRESSION|NO_SSLV2|NO_SSLV3|SINGLE_DH_USE|SINGLE_ECDH_USE` minus `DONT_INSERT_EMPTY_FRAGMENTS`;
- mode: `AUTO_RETRY|ACCEPT_MOVING_WRITE_BUFFER|ENABLE_PARTIAL_WRITE|RELEASE_BUFFERS` (the version check is always true on supported OpenSSL);
- the identical cipher string;
- `set_verify(PEER)` (`setup_verify` has no callback);
- `set_min/max_proto_version(None)`, as native-tls `supported_protocols(None, None)` does.

`SslContextBuilder::new` is only `SSL_CTX_new`. Neither stack sets ALPN (native-tls's ALPN list is empty for gwz). `session()` reproduces `ConnectConfiguration::into_ssl` with `sni=true, verify_hostname=true`: SNI only for non-IP hosts, `NO_PARTIAL_WILDCARDS`, `set_ip` for literals, `set_host` otherwise. `use_sni`, `accept_invalid_*` and identity were all defaults in the old code.

I found no certificate, name, IP literal, wildcard or chain that is now accepted and was refused before, or the reverse. Tests pin options, bounds, verify mode, the full ClientHello against native-tls, and accept/refuse for name and IP.

**Store contents.** Old with a loader: `SslConnector::builder` default paths → probe file and dirs → `set_cert_store(X509StoreBuilder::new())` (everything discarded) → `add_cert` of the loader's roots, then the CA-file roots. New: `X509StoreBuilder::new()` → loader roots → CA-file roots. The final store is identical (`add_cert` errors are ignored in both). `verify_paths.rs` and `endpoint_environment.rs` change only the type, so the TR2.7 erratum semantics are untouched: snapshot `SSL_CERT_FILE`/`SSL_CERT_DIR`, and on OpenSSL only `GIT_SSL_CAINFO` as the endpoint CA file. `SSL_CERT_DIR` hashed lookup was already replaced by explicit roots before this change, so no directory-based root is lost now.

**Channel binding.** `stream.rs:202-211` is native-tls's client branch verbatim: the hash follows the signature digest, SHA-256 for MD5/SHA-1, and `None` for unknown. The Linux test compares bytes with native-tls and with `leaf_digest()`. `https_connection.rs` still takes it only from the final origin stream. macOS is unchanged (`get_ref().tls_server_end_point()`).

**Error mapping.** `https_handshake` still classifies only by `Watched` (EOF, read/write error, zero write gives `Io`; otherwise `Trust`). `poll_fill` reads at most `CHUNK` only when OpenSSL wants input, so an alert followed by FIN is still `Trust` and a bare close is still `Io`. Build failures map to `InvalidInput` as before.

**Adapter.**
- Partial reads advance `taken`; `inbound` is cleared only when empty, so it holds at most one `CHUNK`.
- `ENABLE_PARTIAL_WRITE` together with a drain-first `poll_write` bounds `outbound` to one record. The `Wire` never blocks writes, so `SSL_write` never needs the same-buffer retry.
- `WANT_READ` and `WANT_WRITE` are handled. EOF is `Ok(0)` from `Wire`, so there is no busy loop. Truncation behaviour is the openssl crate's `SslStream::read`, which tokio-native-tls also used.
- Shutdown mirrors native-tls (ZERO_RETURN ok, `WouldBlock` tolerated, socket not shut).
- There is no `unsafe` and no stored `Context`. The 1 MiB in/out tests pass on Linux.
- The only divergence found is P3-1.

**Proxy path.** A TLS proxy and the origin TLS share one connector, as before. CONNECT `write_all` followed by `read_u8` works because the read path flushes pending output. The CONNECT reply's own bytes are not over-consumed (OpenSSL buffers decrypted data internally either way).

**Reaper.** Can it ever block on running work?
- `working()` is false only when the thread has exited, or when `cell.result` is `Some`. The job thread stores the result as its last act before `drop(state)` and `unpark()` (`agent_job.rs:269-279`). `work` has already been consumed, and the remaining drops (`control`, `target`, `wake`) are never the last references, because the Entry and Job hold them.
- So `join` waits only for thread exit, which includes thread-local destructors such as OpenSSL's per-thread cleanup. That is bounded.
- The result is never taken before `joined` (`poll_result` requires `state.joined`), so a stored result cannot revert to `None` and re-park.
- Lock order is consistent: worker state→result, reaper result alone, then state→result. `join` runs outside the `entries` lock (`supervisor.rs:87-95`).
- Panic detection is preserved (`Exit::join` returns `is_err`). The `join: None` path is unchanged.
- The `Exit` seam is a two-method trait with a default type parameter: no global and no clock.

**Repository rules.**
- All new conditional compilation is inside `cfg_if` (`https_backend.rs`, `openssl_stack.rs` `to_der` and `tests`, `stream.rs` tests, `agent_job.rs` test modules). Every control body is braced.
- No globals or thread-locals; no production code under `tests/`.
- All files are ≤ 398 lines.
- The two new parity rows are `paired` arms; the checks pass.
- `prepare.py` target tables match `https_backend.rs`'s cfg exactly. native-tls is only a dev-dependency on OpenSSL targets, and no non-test reference to `native_tls`/`tokio_native_tls` remains outside `native_stack.rs`. CI resolves the candidate lock with `cargo metadata` before `--locked` runs, so the extra `openssl` lock line does not break CI.

## 3. Follow-ups outside the finding bar

- **Hypothesis, outside the diff:** `endpoint_environment.rs` `standard_paths()` calls `openssl_probe::probe()`, which starts from the *process* `SSL_CERT_FILE`/`SSL_CERT_DIR` (`probe_from_env`, openssl-probe 0.1.6 lines 155–168). In the 1.1.0 CLI the process environment is the command's, so it is equivalent there. In a 1.2.0 long-lived session host it would mix the host process environment into a snapshot-defined trust decision. It does not block this change.
- **Coverage:** `the_context_is_the_one_sslconnector_makes_bar_its_trust_roots` does not compare the SSL mode. Losing `ENABLE_PARTIAL_WRITE` or `AUTO_RETRY` would silently change the adapter's one-record bound. `SslContextBuilder::set_mode` returns the resulting mask, which a test can compare against a `SslConnector` builder.
- **Coverage:** `a_host_that_has_default_verify_paths_shows_the_loader_leaves_them_out` passes vacuously on a host with no bundle (it prints and returns early).
- **Doc nit:** the `https_tls.rs` header ("Building a connector decodes the CA bundle (about 10 ms…)") describes the OpenSSL build only. On macOS and Windows no bundle is decoded.
- **Maintainability option:** `tokio-openssl` would replace the hand-written `stream.rs`. The trade-off is a new package against 237 lines owned by the lane, and it is not required.

## 4. Risks and next action

**Residual risk:**
- The OpenSSL stack was compiled and exercised only on weftpi (aarch64, OpenSSL 3.5.6). x86_64 Linux and OpenSSL 1.1.1 were not run; the evidence states this.
- I reviewed the Linux code by reading and against the archived logs, not by running it.
- P3-1 is a real behavioural divergence of the new adapter, with no demonstrated production trigger.
- S7.1 must carry `prepare.py`'s three target tables into the product manifest, or the release build will not have `openssl` on Linux. The evidence already records this.

**Next action:** GO. Fix P3-1 with its closure test, and fix P3-2 or record an exemption with the operator. Neither needs another dual round; a lane-owner check of the two closure tests on weftpi is enough.

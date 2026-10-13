# HTTPS fixed-cost fix (Linux OpenSSL TLS client, reaper join): state-axis review

**Review object:** two gwz-core commits, `git diff 0d501ee6..5cea51dc` (21 files, +1133/-110), as merged into `1d038c6e`:
- `d6446749`: the reaper joins a setup job whose result is already stored.
- `5cea51dc`: on non-Apple, non-Windows targets the TLS client is built directly on the `openssl` crate.

**Baseline:**
- gwz-core `1d038c6ea0e36500713a0e3f2110caf382778c3c`
- gwz-transport `a27817b9c86fd60a07a9e77d62b1f5e94a5cf252` (reference only)
- gwz-core-evidence `d5aed2facebf7902dc2676055da0f2e129e97896` (reference only)
- lane root `2f7b2fbe5fc9d4094421be402583e77336f8bc0f` (out of scope)

The review was done in `/Volumes/projects/limbo/gwz-dev-perf`. All four SHAs were checked at the start and again at the end, and none had moved.

**Date:** 2026-10-14. **Axis:** STATE (durable-state semantics and adversity). The review was independent, adversarial and read-only.

**Verdict: NO-GO.** There is 1 blocking finding (P2-1) and 1 non-blocking finding (P3-1); there are no P0 or P1 findings.

I pre-commit to GO on a revision that resolves P2-1 as specified.

## 0. Evidence base

**Read in full:**
- The whole object diff.
- `src/git/endpoint/agent_job.rs` (at `1d038c6e`) and the reaper loop in `agent_job/supervisor.rs` (lines 30–115).
- `https_backend.rs`, `native_stack.rs`, `openssl_stack.rs`, `openssl_stack/stream.rs` and `openssl_stack/tests.rs`.
- `https_handshake.rs`, the `connect` path of `https_connection.rs` (lines 590–720) and `tls_config` / `platform_roots` in `src/transport_host/endpoint_environment.rs`.
- Reference sources:
  - `openssl-0.10.81/src/ssl/connector.rs`: `ctx`, `SslConnector::builder`, `into_ssl`, `setup_verify_hostname`.
  - `openssl-0.10.81/src/ssl/mod.rs`: `SslStream` read, write and shutdown.
  - `native-tls-0.2.18/src/imp/openssl.rs`: `TlsConnector::new`/`connect`, `supported_protocols`.
  - `native-tls-0.2.18/src/lib.rs`: the `TlsConnectorBuilder` defaults.
  - `native-tls-0.2.18/build.rs`.
  - `tokio-native-tls-0.3.1` `AsyncRead`/`AsyncWrite`.
  - libgit2's `streams/openssl.c` context options, for the gwz 1.0.17 reference.
- Evidence READMEs: `2026-10-13-https-fixed-cost-pi` (outcome and attribution table) and `2026-10-14-https-fixed-cost-fix` (all of it).

**Sampled:**
- `raw/tdd/openssl-green.log`: 12 passed, including `the_client_hello_is_what_native_tls_sends` and `the_context_is_the_one_sslconnector_makes_bar_its_trust_roots`.
- The evidence patch, against the commit diff. Both are 1652 lines; a plain diff showed only file-order differences in the part I compared, and I did not check content equality further.
- `GwzRemoteTransportHttpsDesign.md` and the amendments, searched for a TLS protocol floor. None is stated.

**Not read:**
- `ca_bundle.rs` and `verify_paths.rs` beyond their diff hunks (a type rename only).
- The `-merged` evidence run beyond its listing.
- The other reviewer's material, which this process forbids.

**Commands run** (from gwz-core):
- `check_windows_parity.py --root .`: nothing new (unported 1, paired 153, platform 39). Exit 0.
- `check_cfg_boundaries.py`: nothing new. Exit 0.
- `check_process_globals.py`: nothing new. Exit 0.
- `check_candidate_switches.py`: 23 sites match. Exit 0.
- `check_checked_artifact_boundaries.py --source src`: ok. Exit 0.
- `prepare.py` into `/Volumes/projects/limbo/build-targets/reviewtls-state-cand`, then a candidate `cargo +1.95.0 test --lib` with an external target:
  - filter `agent_job`: **15 passed, 0 failed** (includes the 3 new `reap_tests`);
  - filter `https`: **436 passed, 0 failed**. On the Mac this exercises `native_stack`; `openssl_stack` is not compiled here.
- The Linux side was judged by reading the code plus the weftpi logs. I did not attempt a cross `cargo check` because openssl-sys needs target headers.

**Process note:** while comparing the evidence patch with the commit diff, I briefly wrote one temporary diff file into the session scratchpad and then deleted it. No repository, member or evidence file was touched, and no git or gwz mutation was run.

## Problem, requirement provenance, and scope

**Problem.** The attribution run (`2026-10-13-https-fixed-cost-pi`) found two causes of the Linux candidate's fixed cost:
- About 32 ms of TLS-configuration work on the critical path. `native-tls` parses the system bundle twice (10.7 + 13.4 ms) into a store that `disable_built_in_roots(true)` then throws away.
- A 20 ms reaper `park_timeout` race after a setup job stores its result.

**Provenance.** OD17 plus the lane owner's directive to remove the attributed fixed cost. The fix run's README records operator approval of the two fixes on 2026-10-14. The constraint "speed is a goal; trust semantics must not change" is part of the tasking.

**Necessity of replacing native-tls on Linux.**
- `native-tls` 0.2.18 has no builder option to skip `SslConnector::builder`'s `set_default_verify_paths` or its unconditional `PROBE_RESULT` loads (`imp/openssl.rs` `TlsConnector::new`).
- The connector is already built once per endpoint (`SharedTls`). Each gwz command is its own process, so "build once per process" still pays both wasted parses on every command.
- So no localized native-tls alternative existed. A contained replacement of about 500 lines, Linux only, behind a two-arm `cfg_if` surface (macOS and Windows code unchanged, only moved), is proportionate.
- This is an optimization, not a confirmed blocker, and it was authorized as one.

**Scope.** The touched files are all within the stated object. `prepare.py` moves `native-tls` to Apple/Windows targets and adds `openssl = "=0.10.81"`, a version already in the lock, so nothing new is introduced. I found no unauthorized expansion. However, P2-1 is a behaviour change that the authorization does not cover ("trust semantics must not change"), and the commit's own documentation claims it is absent.

## 1. Findings

### P2-1: The OpenSSL context drops native-tls's TLS 1.2 minimum and also overrides the system policy's floor

**Location:**
- `src/git/endpoint/https_backend/openssl_stack.rs:130-131`, which calls `context.set_min_proto_version(None)?;` and `context.set_max_proto_version(None)?;`.
- The rationale for those calls at `openssl_stack.rs:11-17` and `:108-111` ("the protocol bounds `native-tls` clears (to the library's defaults)", "Nothing else differs from what `native-tls` made").
- The test at `openssl_stack/tests.rs:166-173`, which applies `set_min_proto_version(None)` to the *reference* `SslConnector` before comparing it.

**Violated invariant:** the replaced client must accept exactly the handshakes `native-tls` accepted (directive: trust semantics must not change). The commit states this invariant as met.

**Root cause.**
- `native-tls` 0.2.18 does set a minimum protocol by default: `TlsConnector::builder()` has `min_protocol: Some(Protocol::Tlsv12)` (`lib.rs:490`; its doc says "Defaults to `Some(Protocol::Tlsv12)`").
- `TlsConnector::new` applies it through `supported_protocols` → `ctx.set_min_proto_version(Some(SslVersion::TLS1_2))`. That path is compiled on every OpenSSL ≥ 1.1.0 build (`build.rs` `have_min_max_version`).
- The old gwz code (`https_tls.rs` at `0d501ee6`) never changed that default, so the candidate's Linux floor was TLS 1.2.
- The new `base()` sets the minimum to `None`, which means OpenSSL's lowest supported version.
- That explicit call is not neutral. In OpenSSL 1.1.1+, `SSL_CTX_new` applies the `system_default` section of `openssl.cnf`, including `MinProtocol`. A later `SSL_CTX_set_min_proto_version(ctx, 0)` overwrites it.
- So the new context is laxer than `native-tls` (explicit TLS 1.2) and also laxer than plain `SslConnector::builder`, which never touches the bounds and so keeps a distro's `MinProtocol = TLSv1.2`.

**Why the tests do not catch it.**
- The unit test normalizes the reference to the same `None`, so it compares the defect against itself.
- The ClientHello oracle passed on weftpi because OpenSSL 3.5.6 at security level ≥ 1 refuses to offer TLS ≤ 1.1 (`SSL_SECOP_VERSION`) whatever `min_proto_version` says. The two hellos are byte-equal there by accident of the host's library, not by construction.

**Reproduction / state sequence.**
1. Use a Linux host whose OpenSSL permits TLS 1.0/1.1 at the security level, with the floor set by policy. Examples: OpenSSL 1.1.1 with a distro `MinProtocol = TLSv1.2` (Debian 10/11, EL8 crypto-policies), or OpenSSL 3 under a policy at `SECLEVEL=0`.
2. Run an HTTPS server limited to `max_proto_version(TLS1_1)` with a valid certificate chained to a root gwz trusts.
3. Before (native-tls): the handshake is refused, classified `Trust`, and the key is closed.
4. After: `base()` resets the minimum to 0, the ClientHello offers TLS 1.0/1.1, and the handshake completes. Chain and host-name checks still run.

The same comparison shows statically: `SslContextBuilder::min_proto_version()` returns `Some(TLS1_2)` for a native-tls-equivalent context and `None` for `base()`.

**Impact.**
- On the affected hosts, gwz now completes TLS 1.0/1.1 sessions with servers capped at TLS ≤ 1.1, which the pre-change candidate refused. It also defeats an administrator's system-wide TLS floor.
- The certificate, chain and host-name checks are unchanged, and a server that can do TLS 1.2+ cannot be downgraded (Finished/transcript integrity and TLS 1.3 downgrade sentinels).
- That is why this is P2 (a concrete security-relevant parity defect, stated as absent in code, tests and evidence) and not P0/P1 (no certificate, name or chain that was refused is now accepted).
- gwz 1.0.17's libgit2 context set only `NO_SSLv2|NO_SSLv3|NO_COMPRESSION` and left the bounds alone, so it kept the system floor. The new code therefore matches neither the replaced stack nor 1.0.17.

**Required correction:**
- Set `context.set_min_proto_version(Some(SslVersion::TLS1_2))?` (native-tls parity). Keep `set_max_proto_version(None)` only if native-tls parity is the stated target; native-tls does that too.
- Correct the module comment and the evidence README's "no protocol bounds" claim.
- If a different floor is wanted (for example 1.0.17's "library and policy decide"), that is a trust decision for the operator, not a perf-lane default.

**Closure / regression test:**
- `the_context_is_the_one_sslconnector_makes_bar_its_trust_roots` must stop normalizing the reference. It should assert `base()?.min_proto_version() == Some(SslVersion::TLS1_2)`, the value `native-tls` 0.2.18 sets, with a comment citing `TlsConnectorBuilder`'s default.
- Add a negative test: a local OpenSSL `SslAcceptor` restricted to `max_proto_version(TLS1_1)` is refused by `Connector`. It needs a host or library state where ≤ 1.1 is otherwise enabled, so either run it in a child process with a test `OPENSSL_CONF` at `SECLEVEL=0`, or record it as host-conditional like the existing control test.

### P3-1: A read cannot pull ciphertext from the socket while unsent ciphertext is waiting, a stuck state native-tls did not have

**Location:** `src/git/endpoint/https_backend/openssl_stack/stream.rs:142-143`, `poll_read`'s `WouldBlock` arm:

```rust
ready!(self.poll_send(cx))?;
ready!(self.poll_fill(cx))?;
```

**Violated invariant:** read progress must not depend on the socket being writable. Under `tokio-native-tls`, `poll_read` runs `SSL_read` through `AllowStd`, which reads the socket regardless of a pending `SSL_write` record.

**Root cause.**
- `poll_write` returns `Ok(count)` while ciphertext can still sit in `Wire.outbound` (lines 160-168).
- After that, any `poll_read` that needs more ciphertext first waits for `outbound` to drain. If the socket is not writable, `poll_read` returns `Pending` without ever calling `poll_fill`.

**State sequence (one hyper connection task):**
1. Hyper's h1 client polls read while the request body is still being written (`can_read_head` once writing has started).
2. A large POST fills our socket send buffer, so `outbound` stays non-empty and `poll_write` and `poll_read` are both `Pending` on write readiness.
3. The peer writes a response bigger than its send buffer plus our receive buffer *before* it finishes reading our body, so the peer blocks on its write.
4. Neither side reads, and the connection is stuck until the transport's body stall or aggregate deadline fails it.

With native-tls, step 3 drains through `SSL_read`, so the peer's writes unblock, it resumes reading, and the transfer completes.

**Reproduction:** a rustls fixture server that, after the handshake, writes 8 MiB before reading. The client does `tokio::join!` of `write_all(8 MiB)` and `read_to_end` on one `Stream`. It completes over `tokio-native-tls` and stalls over `openssl_stack::Stream`.

**Impact:** limited by the stall and aggregate deadlines (a failure, not a hang). Git smart-HTTP servers normally read the whole request before answering. Early 401/413 replies are small and fit in the buffers. So the trigger needs an unusual server. It is still a new stuck state, introduced on the critical I/O path with no test.

**Required correction:** in `poll_read` (and in `poll_write`'s `WouldBlock` arm), make the send opportunistic. Call `poll_send`, propagate a `Ready(Err)`, but on `Pending` go on to `poll_fill`. A waker is registered for both directions either way.

**Closure / regression test:** the full-duplex test above, with a time bound.

## 2. Invariant analysis

### Reaper (`d6446749`): no defect found

- **What runs after the store.** The job closure (`agent_job.rs:275-284`) stores `Some(result)` under the state lock, drops the lock, and unparks the reaper. All captures of `work` are consumed and dropped before the store (it is `FnOnce`; in `start_setup` the wrapper's `setup` Arc is dropped inside that call). After the store, only the closure's `control`, `target` and `wake` remain. The `Entry` holds `cell` and therefore `control`, so none of those drops is the last reference. Thread-local destructors (OpenSSL and libgit2 per-thread state) are bounded.
- **Blocking the whole host.** A join on a thread whose result is stored therefore waits only for a bounded exit. The "stores, then panics or blocks in a destructor" case cannot happen: the result value (TLS stream, SSH session) is *moved into* the cell and is dropped later by the owner or by the reaper's discard path, never on the job thread. That discard path, and the rule of dropping the result outside the arbitration lock, are unchanged.
- **Panic before the store.** The result stays `None`, so `working()` keeps relying on `exited()`, exactly as before.
- **Lock order.** `working()` takes only the `result` lock. The job thread, the reaper discard path and `poll_result` all take `state` then `result`. There is no inversion. The reaper loop calls `reap()` outside the `entries` lock (`supervisor.rs:86-94`), so a join never blocks `Job::start_control`.
- **Lost wakeups.** If the reaper looks before the store, it parks with a 20 ms timeout; an unpark that lands before the park is kept as a token. Behaviour is unchanged and only becomes faster.
- **Double join and budget permits.** `join.take()` makes a second join impossible; with `join == None`, `working()` is false and `take()` yields nothing. The permit is still released only on `consumed` (reaper) or on a take (owner), once, through `Option::take`.
- **Shutdown ordering.** Unchanged.
- **Tests.** The three stand-in tests cover stored-but-not-exited, exited, and working-never-joined. Local run: 15/15 `agent_job` tests pass.

### Context and trust state (`openssl_stack`)

- **Lifetime.** One `SslContext` per endpoint per command (`SharedTls` `OnceLock`). A build failure is kept and fails every connection with `InvalidInput`, as before. A fresh `Ssl` is made per connection. The context has no verify callback and no captured mutable state. The client session cache keeps OpenSSL's default (server-only), so no resumption state is shared, matching native-tls.
- **Fail-closed roots.**
  - With a loader (always the case in the product on these targets, `endpoint_environment.rs:209`), the store starts empty and holds only the loader's and CA file's roots.
  - `add_cert` errors are ignored, as in native-tls; this can only narrow trust.
  - An empty or unreadable default path or bundle gives an empty store, so the handshake fails with `Trust`.
  - A garbage `GIT_SSL_CAINFO` is still refused at configuration (`ca_bundle` `Refusal` → `invalid(...)`). Only the type changed.
  - The `None`-loader path (tests only) reads the default paths and leaves out openssl-probe's. That is narrower, and the evidence README discloses it.
  - Trust is never widened by any failure.
- **Hostname and SNI.** Byte-for-byte the openssl crate's `into_ssl` and `setup_verify_hostname` (`NO_PARTIAL_WILDCARDS`, IP vs name dispatch, SNI only for names). Options, mode, cipher list and `SslVerifyMode::PEER` equal `SslConnector`'s. The weftpi tests confirm the name/IP refusals. The only gap is the protocol bounds (P2-1).
- **Channel binding.** `tls_server_end_point` is native-tls's computation. A digest error gives `None` in both (`.ok().flatten()` before).

### Stream state machine (`stream.rs`)

- **Handshake.** Retrying `SSL_connect` after `WANT_READ` is legal, and after completion `SSL_connect` returns 1, so re-polling after a `Pending` final send is safe. The `Wire` never returns `WouldBlock` on write, so `WANT_WRITE` never comes up. `poll_fill` sets `ended` on EOF, after which the `Wire` always returns `Ok(0)`, so none of the loops can spin.
- **Failure classification.** `Io` vs `Trust` still comes from the `Watched` wrapper under the TLS layer. Both stacks read and write through it identically, so verify failures stay `Trust` (non-retriable), and EOF or reset stays `Io`. A fatal alert followed by FIN is read as the alert first (fills happen only when the `Wire` is empty), so it stays `Trust`.
- **Truncation.** It uses the same `SslStream::read` as native-tls: `SYSCALL` with no I/O error gives `Ok(0)` on 1.1.1, and an "unexpected eof" error on OpenSSL 3. Parity holds.
- **Post-handshake errors.** Socket errors from `poll_fill` and `poll_send` keep their `io::ErrorKind`, as native-tls returns them via `into_io_error`.
- **Proxy CONNECT reply.** It is read one byte at a time through the proxy `Stream`. Any extra ciphertext stays in that stream's `Wire` or OpenSSL buffers and is handed to the inner handshake. No bytes are lost or leaked.
- **Cancellation.** Dropping mid-handshake or mid-transfer frees both buffers, with no task or thread left behind. No `close_notify` is sent on drop, the same as `tokio-native-tls`.
- **Large transfers.** `poll_write` drains before encrypting more, so `outbound` stays at about one record. Inbound is filled only when empty, so it holds at most one 16.5 KiB chunk. The weftpi tests move 1 MiB each way.
- **Shutdown.** Mirrors `tokio-native-tls`. A repeated `SSL_shutdown` after `Sent` produces `WANT_READ`, which is ignored, and the stream completes after sending.
- **Defect found:** read/write coupling (P3-1).

### Repository rules

- All conditional code is in `cfg_if` (`https_backend.rs`, the test-only `to_der`, the test modules), with no `#[cfg]` on imports.
- No `unsafe`, no globals or thread-locals (process-globals check passes).
- No production code under `tests/`.
- Every file is under 1000 lines.
- Braced control flow throughout.

## 3. Follow-ups outside the finding bar

- The ClientHello oracle depends on the host. On OpenSSL 3 at security level ≥ 1 it cannot see differences in the protocol range. Record this in the test's doc comment once P2-1 is fixed.
- `prepare.py` keeps the per-target TLS tables in the candidate manifest only. The evidence README already notes that S7.1 must carry them into `Cargo.toml`, and that should stay a tracked item.

## 4. Risks and next action

**Risks:**
- Code and evidence both claim native-tls parity for the TLS context, but the protocol floor is lower. It is also lower than the distro policy wherever that policy is the only thing enforcing TLS 1.2.
- On the measured host (OpenSSL 3.5.6) the gap is hidden by the security level, so the green weftpi runs do not show parity.

**Next action:**
1. Fix P2-1: set the minimum to TLS 1.2, correct the comments and the README claim, and replace the self-normalizing test with an assertion of native-tls's actual default, plus the negative TLS 1.1 test.
2. Preferably fix P3-1 in the same revision, with the full-duplex test.
3. Re-run the weftpi `https_backend` group and the HTTPS/CA-trust groups.

The reaper commit `d6446749` is GO on its own.

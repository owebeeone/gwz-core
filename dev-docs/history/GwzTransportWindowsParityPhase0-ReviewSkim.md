# Windows parity Phase 0 skim review

Scope: gwz-core `b9a6595f` (0.3 to 0.5) and `a3ba9ea0` (0.5b), each against its parent. Controlling document: the parity plan, revision 3, §4 and §5 (OQ16: skim). Nothing was modified. The parity checker, `check_cfg_boundaries.py` and the 54 Python unit tests for the checker and `windows_lane_check.py` all pass. No Rust builds. Steps 0.1 and 0.2 got a glance only: the `candidate-windows` workflow hunk is sane and `CARGO_TARGET_DIR` is set for the SSPI validator test.

## Checked, no defect found
- **Test code in product builds:** `rustls` and `tokio-rustls` enter only through `prepare.py`'s `[dev-dependencies]` (`prepare.py:86-97`), and `test_prepare.py:109` asserts they are absent from `[dependencies]` and the root `Cargo.toml`. `https_fixture`, `loopback` and `cut_proxy` sit under `cfg(test)` (`mod.rs:53-65`) inside the candidate-only `endpoint` module. Product TLS is unchanged: `tokio_native_tls` is still used by `https_handshake.rs` and `https_connection.rs`.
- **Client verification:** `grep danger` over `src` finds nothing. The only root injection is `https_tls.rs:61,65` (the CA bundle), and the new fixture tests (`https_fixture/tests.rs:22-27`) build their client with `add_root_certificate(fixture CA)` only.
- **cfg and brace rules:** the new and changed conditionals are `cfg_if!` arms (including the `isolate` and `kill_tree` twins at `native/tests.rs:6-22`). No bare `#[cfg]` on an import in either diff; new Rust bodies are braced.
- **Ratchet on the 0.5b rows:** the 27 removed rows correspond to gates that are now `cfg(test)` or gone (the checker's STALE rule would otherwise fail). `anonymous_failure_crosses_mux_before_distinct_gh_open` was re-inventoried under 4.11 (G18), not dropped.

## Findings

**P2-1 — `loopback.rs:19-26` silently falls back to IPv4-only on any `::1` bind failure, including EADDRINUSE.**
- `bind()` takes `127.0.0.1:0`, then tries `[::1]:<same port>` and discards the error. Ephemeral ports are per address family, so a port free on IPv4 can be busy on `::1`: a `::1` source port or a TIME_WAIT socket from a parallel test's `localhost` client is enough.
- Simulated on macOS with 8,000 live `::1` connections: the v6 bind failed on 8,001 of 20,000 attempts after the IPv4 port was chosen.
- On Windows the fixture is then IPv4-only, `localhost` tries `::1` first, and the 2.01 s refusal returns: the flake 0.5b set out to remove, with nothing in the output to say why. Worse, if another process owns `::1:<port>`, a `localhost` client reaches it instead of the fixture.
- Fix: on `AddrInUse`, retry the pair with a fresh port (about 16 attempts). Fall back to IPv4-only only for `AddrNotAvailable` or `Unsupported` (no IPv6), and say so in a panic or `eprintln`.

**P3-1 — The negative CA test cannot tell why the connection failed.** `https_fixture/tests.rs:81-86` asserts only `refused.is_err()`, and `exchange` (`:36-49`) folds TCP, handshake and read errors into one `io::Error`, so a server crash would also pass. Fix: return the handshake error separately and assert it is a certificate error, not an I/O error.

**P3-2 — The ratchet can be relabelled past.** `shrink()` in `check_windows_parity.py` (about `:430-445`) compares only the `unported` count, the scope roots and `done_steps`, so changing an entry from `unported` to `platform` with any non-empty `reason` passes the lane gate. Fix: have `--shrink-from` print or fail on every entry newly `platform` against the base, so a reviewer sees it. Runtime splits are not scanned either: `cfg!(unix)`, `cfg!(windows)`, `cfg_attr(unix, ..)`, `os::linux`, `os::macos`; `https_transport_binding_tests.rs:38` already has a `cfg!(windows)` expectation split the inventory cannot see. Fix: add these patterns to `os_uses` or the attribute scan.

**P3-3 — The Windows compile gate fires on fewer paths than the inventory scope.**
- `windows_lane_check.py:42` `TRIGGER_PATHS` (mirrored in `AGENTS.md`) omits `src/git/gitbackend/transport_*`, `src/git/gitbackend.rs`, `src/transport_setting*`, `Cargo.lock` and `.github/*.commit`. `b9a6595f` itself changed Windows-conditional cfgs in `transport_binding.rs` and `transport_observations.rs`, so a lane like it would not trigger the gate.
- `check_lane_commits.sh` never calls `--if-triggered` or looks for a receipt, so "cannot merge without a Windows compile" (plan 0.4) is prose only.
- Fix: derive `TRIGGER_PATHS` from the inventory `roots` plus `Cargo.lock`, and make the lane gate fail when a trigger path changed and no receipt label is named.

**P3-4 — Documentation drift.** Plan revision 3 has no step 0.5b (the inventory's `step_notes` says "Added by the lane"); record it in revision 4. The comment at `https_opening_tests.rs:127-128` says step 4.4 gives the gh helper open its owner, but the inventory row owns it under 4.11.

**P3-5 — `Loopback::accept` (`loopback.rs:47-56`) polls the IPv4 listener first every time,** so a constantly ready IPv4 listener starves the `::1` one. Harmless at the fixtures' connection rates. Fix: rotate the starting index, or use `select!`.

Verdict: GO. P2-1 should be fixed in the next lane that touches the fixture, and before Phase 1 relies on those timing tests.

---

Lane owner's note: the severity contract makes this NO-GO while P2-1 is open, whatever the verdict line says. P2-1, P3-1, P3-4 (the comment) and P3-5 are being fixed now in lane `tls05b`. P3-2, P3-3 and the plan's revision 4 are recorded as open items in the program checkpoint.

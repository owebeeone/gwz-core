# HTTPS fixed-cost fix — remediation plan, round 1

Status: **accepted at gwz-core `85d7d910` (with `d6446749` and `5cea51dc`) after `GwzTransportHttpsFixedCostFix-ReviewCode-2.md` and `GwzTransportHttpsFixedCostFix-ReviewState-2.md` reported GO (2026-10-14); this accepts the HTTPS fixed-cost fix only.** Drafted 2026-10-14. The object reviewed is gwz-core `d6446749` (reaper join) and `5cea51dc` (the Linux TLS client on the `openssl` crate), as merged into `1d038c6e` (tuple in both reports; review workspace the `perf` lane). The reports are filed verbatim:
- [Code](GwzTransportHttpsFixedCostFix-ReviewCode.md): GO, 2 P3.
- [State](GwzTransportHttpsFixedCostFix-ReviewState.md): NO-GO, 1 P2 and 1 P3, with a pre-commitment to GO on P2-1 as specified. The reaper commit is GO on its own.

No finding is architectural.

**Blind convergence.** Both axes independently found the same stream defect: `poll_read`'s `WouldBlock` arm waits for unsent ciphertext to drain before it reads (Code P3-1, State P3-1). Code reached it through an early response to a large POST, State through a peer that writes before it reads.

## Dispositions

All in one patch, in the `perf` lane, then one re-verdict by the same two reviewers.

| Finding | Disposition | Closure test |
|---|---|---|
| State P2-1 (protocol floor) | `base()` sets `set_min_proto_version(Some(SslVersion::TLS1_2))`, native-tls 0.2.18's default (`TlsConnectorBuilder::min_protocol`); `set_max_proto_version(None)` as native-tls. Correct the module comments and the evidence README's "no protocol bounds" claim. Keeping native-tls parity needs no operator decision; a different floor would. | `the_context_is_the_one_sslconnector_makes_bar_its_trust_roots` stops normalizing the reference and asserts `min_proto_version() == Some(TLS1_2)` with a comment citing the native-tls default. A negative row: a local `SslAcceptor` capped at `max_proto_version(TLS1_1)` is refused, run where TLS ≤ 1.1 is otherwise enabled (a child process with a test `OPENSSL_CONF` at `SECLEVEL=0`), or recorded as host-conditional with the condition printed. |
| Code P3-1, State P3-1 (read waits on write) | In `poll_read`'s `WouldBlock` arm, and in `poll_write`'s, the send is opportunistic: call `poll_send`, propagate `Ready(Err)`, and on `Pending` go on to `poll_fill`. | Two time-bounded rows on Linux, each with the native-tls oracle as control: a server that writes 8 MiB before reading while the client writes 8 MiB and reads concurrently on one `Stream` (completes); a server that answers `413` after the request head and stops reading while the client streams a 16 MiB body (the response head arrives within 2 s). |
| Code P3-2 (dead default-paths fallback) | The OpenSSL connector needs a loader: `Connector::build(None, …)` on the OpenSSL stack gives an empty store (or `None` is not accepted there). `Store::add_defaults` and its tests are removed; the fixture tests pass an explicit loader. | A row: `Connector::build(None, &[])` on the OpenSSL stack holds 0 certificates (or the call does not compile). `a_client_that_does_not_trust_the_fixture_ca_is_refused` stays green with an explicit loader. |
| Code §3 coverage | The context test also compares the SSL mode mask with `SslConnector`'s. | Part of the context row. |
| Code §3 doc nit | `https_tls.rs` header: the bundle decode happens on the OpenSSL build only. | Review of the diff. |
| State §3 | The ClientHello oracle's doc comment says it cannot see protocol-range differences on OpenSSL 3 at security level ≥ 1. | Review of the diff. |

## Follow-ups recorded, not in this round

- `standard_paths()` calls `openssl_probe::probe()`, which reads the process `SSL_CERT_FILE`/`SSL_CERT_DIR` (Code §3): equivalent in the 1.1.0 CLI, a snapshot leak for the 1.2.0 session host.
- `a_host_that_has_default_verify_paths_shows_the_loader_leaves_them_out` passes vacuously on a host with no bundle (Code §3).
- `tokio-openssl` as a replacement for `stream.rs` (Code §3): not adopted.
- S7.1 carries `prepare.py`'s three target tables into the product manifest (both).
- x86_64 Linux and OpenSSL 1.1.1 were not exercised (Code §4).

## Re-verdict

One revision goes back to the same two reviewers with this plan and the diff since `1d038c6e`. Each reviewer re-checks its own counterexamples and fills a prior-finding closure table. This is round 1 of the two-round cap.

## Outcome (2026-10-14)

Both reviewers re-verdicted GO on `85d7d910`; every finding above is closed, one remediation round of the two-round cap used. The Code reviewer corrected its own round-1 §2, which had read native-tls's protocol bounds as `None`. New observations below the bar:
- Both axes, blind: the TLS-floor row's host-conditional pass is silent in captured logs (State P3-2, Code §3). Small follow-up: print the branch on stderr, and archive a captured run showing the live branch.
- Code §3: a `poll_send` error still preempts reading ciphertext already buffered (a peer that answers then resets).
- State §3: the explicit TLS 1.2 floor also lowers a stricter distribution floor (`MinProtocol = TLSv1.3`) to 1.2, as native-tls did; gwz 1.0.17 (libgit2) left the bounds to the system policy. "max(1.2, policy)" would be an operator trust decision.

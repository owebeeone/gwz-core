# GWZ 1.1.0 route ledger

Status: **DRAFT, 2026-10-17.** This is the route ledger that `GwzV110Plan.md` S7.2 asks for (remaining plan step C5). It is drafted before C4 and S5.6 and is **finalized after both**: the route column is read from the code at this lane's base, and every evidence cell says `S5.6 pending` until S5.6 marks it. It decides nothing, and it authorizes no implementation, commit, push, tag or publish.

Sources read for this draft: `GwzV110Plan.md` S7.1 and S7.2; `GwzV110RemainingPlan.md` (C5, D3); `GwzTransportTR18-OperatorDecisions.md` (D1 to D33); `GwzRemoteTransportDesign.md` §3.2 and §11; `GwzTransportReleasePlanAmendment-2.md` §3.19; the route code in `src/git/gitbackend/transport_binding.rs`, `src/git/endpoint/ssh_destination.rs`, `src/transport_scope.rs` and `src/protocol/transport_capabilities.rs`. Lane `s56` is building the S5.6 cell table in parallel; this ledger's rows are written so that table can fill the evidence column cell by cell.

## 1. The rules this ledger is held to

S7.2 (quoted in substance): every advertised cell is one that S5.6 marked with evidence. An unsupported cell, a native route, or a cell with no S5.6 row fails the step. Windows exact-agent stays false, in capabilities and in the ledger, until a named fixture GO. S7.1 adds: paths that 1.1.0 does not support stay native and are listed in the migration notes.

How this draft reads those rules (an open question, section 7, item 2): a native route is allowed when this ledger lists it with its reason and the migration notes repeat it; what fails S7.2 is a native route that is not listed, a cell advertised without an S5.6 row, or an unsupported cell presented as supported.

Words used below:

- **Transport** is the endpoint-owned `gwz-transport` route: pooled, with the setup clocks and the retry machine of the accepted timeout and retry plans.
- **Native** is libgit2's own transport, as gwz 1.0.17 used. No pool, no setup retry, no 30 second setup budget; `--ssh-timeout` is libgit2's connect and read timeout.
- A **cell** is one row of section 3 on one platform. The evidence column holds the S5.6 row id, or `S5.6 pending`.
- **Informative evidence** (section 4) names runs that exist today. It is not an S5.6 mark and no cell may be advertised on it.

## 2. Platforms

| Platform | 1.1.0 | Evidence host | Note |
|---|---|---|---|
| macOS ARM64 | supported | the operator's Mac | |
| Linux x86-64 | supported | WSL 2 Ubuntu on dabeest (D20) | the Raspberry Pi 5 (aarch64) runs are hardware evidence for a platform 1.1.0 does not list; they are not Linux x86-64 cells |
| Windows x86-64 | supported | dabeest (Windows, MSVC toolchain) | parity is measured against gwz 1.0.17 |
| Linux ARM64 | **unsupported** (recorded, V110 §2) | none | no cell |
| macOS x86-64 | **unsupported** (recorded, V110 §2) | none | no cell |

Narrowing out Linux ARM64 and macOS x86-64 is a scope decision of the plan, not a silent pass of those rows; putting either back is an amendment.

## 3. The ledger

Operations that reach a remote, and so take a route: `clone` (workspace and repo member), `init` from sources, `materialize`, `fetch`, `pull` (head and snapshot), `push`, and `tag` when it pushes, fetches, lists or deletes on a remote (`src/transport_scope.rs`, nine operations). Every other command runs no network operation. `gwz log` runs a local `git rev-list`; it is not a remote operation.

The route is chosen per remote URL, after the effective-URL rewriting, before any network effect (design §3.2). It is chosen by the URL's scheme and by whether the operation has a host context; both gwz-cli and gwz-py give every in-scope operation one.

| Id | Remote kind | Protocol / authentication | Route in the 1.1.0 build | Why (for a native route) | macOS ARM64 | Linux x86-64 | Windows x86-64 |
|---|---|---|---|---|---|---|---|
| L1 | scp-style `user@host:path`; `ssh://`, `ssh+git://`, `git+ssh://` | SSH, key from an agent (ssh-agent on Unix; Pageant, then the `SSH_AUTH_SOCK` pipe, then the default OpenSSH agent pipe on Windows) | transport | | S5.6 pending | S5.6 pending | S5.6 pending |
| L2 | as L1 | SSH, a key file named with `--identity`, `--remote-identity` or a remote's `gwzSshIdentity`. Windows: an unencrypted RSA PEM file only. A passphrase-protected file is refused on every platform | transport | | S5.6 pending | S5.6 pending | S5.6 pending (`file_identity`, D30) |
| L3 | as L1 | SSH, a password (a URL password, or the configured helper's password-only route) | transport | | S5.6 pending | S5.6 pending | S5.6 pending |
| L4 | `https://` | anonymous | transport | | S5.6 pending | S5.6 pending | S5.6 pending |
| L5 | `https://` | Basic, answered by the user's configured git credential helpers (`gh` among them; Git Credential Manager on Windows) | transport | | S5.6 pending | S5.6 pending | S5.6 pending |
| L6 | `https://` | Negotiate or NTLM, answered by the Windows logon session | transport, **Windows only**. macOS and Linux serve neither scheme (as 1.0.17), so a Negotiate-only server is refused there | | not applicable | not applicable | S5.6 pending |
| L7 | `https://` through a proxy | macOS and Linux: `https_proxy`, `HTTPS_PROXY`, `all_proxy`, `ALL_PROXY` and `no_proxy`, an `http://` or `https://` proxy with no credentials and no path. Windows: the one static WinHTTP machine proxy (D1, D6, D8); a 407 is refused as proxy sign-in (measured on Windows; S5.6 to confirm on the others) | transport | | S5.6 pending | S5.6 pending | S5.6 pending |
| L8 | `https://` with a custom CA | macOS: the platform roots plus one CA file (`GIT_SSL_CAINFO`, else `SSL_CERT_FILE`). Linux: OpenSSL's verify paths (`SSL_CERT_FILE` and `SSL_CERT_DIR` as OpenSSL reads them) plus the CA file `GIT_SSL_CAINFO` names. Windows: the Windows store plus one CA file (`GIT_SSL_CAINFO`, else `SSL_CERT_FILE`) (D4) | transport | | S5.6 pending | S5.6 pending | S5.6 pending |
| L9 | `http://` | any | **native**, on every platform. Windows prints a warning before the first request (D2); macOS and Linux print none | the transport serves `https://` only (design §3.2, D2); a plain-HTTP remote is a compatibility exception, not a fallback | S5.6 pending | S5.6 pending | S5.6 pending |
| L10 | `git://` | any | **native** | no endpoint protocol for the git daemon (design §3.2) | S5.6 pending | S5.6 pending | S5.6 pending |
| L11 | `file://` and local paths (including local clone families) | none | **native** (libgit2's local transport; credential-free) | no network | S5.6 pending | S5.6 pending | S5.6 pending |
| L12 | any other scheme | none | no route: the existing typed unsupported outcome, before any effect | | S5.6 pending | S5.6 pending | S5.6 pending |
| L13 | any of L1 to L8 | the user chose native: `--transport native`, `GWZ_TRANSPORT=native` or `gwz.transport=native` in the global git configuration | **native**, by the user's choice, not a fallback. gwz never moves an operation between routes | the escape hatch for 1.0.17 behaviour; also the route for any combination 1.1.0 does not carry | S5.6 pending | S5.6 pending | S5.6 pending |

Reading rules that apply to every row:

- A failed operation is reported. The route is never changed while an operation runs (no fallback in either direction).
- Native-routed remotes (L9, L10, L11 and everything under L13) still run with the process's `--ssh-timeout`. Under the default transport that value is 9 seconds, so an `http://` or `git://` remote gets libgit2's connect and read timeout of 9 seconds, not the 3 seconds of 1.0.17. `--transport native` restores 3 seconds, 50 jobs and 8 connections per host.
- `--max-retries` has no effect on a native route.
- An operation outside the nine in the list above does not use a route, and a request whose backend has no host context takes the native route (amendment 2, TR2.11). Both drivers give all nine a host context, so the second case is reachable only by an embedding of gwz-core that builds its own backend. Check this in S5.6 or S7.4.

### 3.1 Windows rows of the capability answer

| Capability | Windows answer in the 1.1.0 build | Basis | State |
|---|---|---|---|
| `file_identity` | true | D30: GO on the key-file rows `home-unset-absolute-identity` and `home-unset-tilde-identity` of run `2026-10-15-parityrows` | lane `cap72` builds the answer; S5.6 row pending |
| `exact_agent_identity` | true only after one exact-selection row passes on Windows (an agent holding two keys: the selected one authenticates and the other is never offered) | D30: GO on the Pageant evidence (run `2026-10-17-pageant-operator`), subject to that row, which the Pageant and 6.2 rows do not contain | lane `cap72` adds the row; until it passes the ledger and the capability answer say false |

Before D30, S7.2 said Windows exact-agent stays false until a named fixture GO. D30 is that GO, with the one row as its condition. If the row is not in by C5's finalization, `exact_agent_identity` stays false on Windows and this ledger says so.

The answers on macOS and Linux are whatever the 1.1.0 build's capability handler returns; at this lane's base the static handler (`src/protocol/transport_capabilities.rs`) answers `file_identity: true` and `exact_agent_identity: false` with no platform split. S5.6 must record the final answer per platform.

## 4. Informative evidence (not S5.6 cells)

Runs that exist today in `gwz-core-evidence/campaigns/transport-qualification/runs/`. They say what was measured; S5.6 decides which of them mark a cell.

| Rows | Runs |
|---|---|
| Windows baseline on released 1.0.17 (the parity reference) | `2026-10-10-tr18-windows-ssh-rows` (90 SSH rows); `2026-10-10-tr18-windows-auth-rows` (35 plain-HTTP authentication and proxy rows); `2026-10-10-tr18-redirect-parity`; `2026-10-10-tr18-b17-macos-linux` |
| Windows proxy, trust, agent service | `2026-10-10-tr18-td10-machine-proxy`; `2026-10-11-tr18-r13-https-proxy`; `2026-10-10-tr18-td9-user-root-trust`; `2026-10-08-ca-trust-parity`; `2026-10-10-tr18-td12-agent-service` |
| Windows parity rows (candidate against 1.0.17) | `2026-10-15-parityrows` (decision D22 on its F1 finding); `2026-10-17-pageant-operator` (the three Pageant rows pass on 1.0.17 and the candidate) |
| Windows speed (TR8.4 / step 6.1) | `2026-10-16-tr84speed` (D29); `2026-10-17-https611` (D33's profile) |
| Windows key-exchange behaviour | `2026-10-14-idleloss`; `2026-10-16-winflakes` (D32) |
| Setup clocks and live fetches (S3.3) | `2026-10-17-s33` |
| Aggregate operations, coalescing, sustained memory (S5.1 to S5.3), macOS ARM64 and Linux x86-64 | `2026-10-15-s51`; `2026-10-15-s52`; `2026-10-15-s53` |
| macOS and Pi speed history (TR8.1; the Pi is not a supported platform) | `2026-10-04-tr8-1-macos-parity` to `2026-10-13-tr8-1-rerun` |

Gaps visible today, for the S5.6 owner (not decisions): there is no dedicated row for L3 (SSH password) or for L10 (`git://`, only the compatibility test `local_http_and_git_remain_native_compatibility_transports` and run `2026-10-15-gitdaemon`'s fixture); macOS has no trusted-HTTPS row (TD7 recorded the Mac trusted-HTTPS row as unexecuted and disposed by inference); Linux x86-64 speed evidence is the WSL 2 host under the dabeest booking lock.

## 5. What the 1.1.0 notes must say about routes

These go into the gwz-cli migration notes (`gwz-cli/docs/Releases.md`) and the gwz-py notes (`gwz-py/docs/Migration-1.1.0-DRAFT.md`):

1. L9, L10 and L11 stay native, and why; the Windows `http://` warning; the 9 second timeout on them.
2. `--transport native` and its three sources, with the 50 / 8 / 3 defaults.
3. HTTPS is authenticated through the user's configured credential helpers (`gh` among them); an `https://` URL password, query or fragment is refused (a user name alone is allowed); Windows adds the logon session for Negotiate and NTLM and nothing else (section 7, item 1 on the word "gh-only").
4. The Windows list of differences from 1.0.17 (TD13, with D2, D4, D6, D8, D12, D14 and D32).
5. Unsupported: Linux ARM64, macOS x86-64, Bazel's `@gwz_core_crates` (D27) until the Phase 8 registry pins.

## 6. Finalization checklist (after C4 and S5.6)

1. Replace every `S5.6 pending` with the S5.6 row id. A row that S5.6 marks unsupported leaves the advertised set and moves to the notes; it blocks S7.5 unless an amendment removes it.
2. Re-read the route column against the C4 tree: the `cfg_if` arms remain, the native branch stays for unsupported paths and the off switch (C3).
3. Fill section 3.1 from `cap72`'s result.
4. Regenerate `gwz-cli/docs/CLI.md` (`scripts/generate_cli_reference.py --write`) so the reference shows `--transport` and `--max-retries`; today it is generated from the ordinary build (help audit, finding H1).
5. Strike the word DRAFT from this file's name only after S7.5 accepts it.

## 7. Questions for the operator

1. **"gh-only HTTPS."** `GwzV110Plan.md` S7.2 says the notes state the gh-only HTTPS policy. Amendment 2 §3.19 (OD10 and OD11 reversed, 2026-10-02) and TR1.6 replaced it: the transport runs the user's configured credential helpers, `gh` among them, and the plan text for S7.1 and S7.2 was meant to be amended and is not (`GwzV110RemainingPlan.md` §5, item 1). The notes in this draft state the as-built policy (configured helpers; no URL-embedded or signed-token credentials). Confirm that is what S7.2 should say.
2. **A native route and S7.2.** S7.2 reads "an unsupported cell, a native route, or a cell with no S5.6 row fails this step"; S7.1 reads that unsupported paths stay native and are listed. This draft reads the first as "an unlisted native route". Confirm.
3. **The capability answer on Windows.** The remaining plan (C5, open item F1) says the static handler answers `exact_agent_identity` true on Windows. At this lane's base it answers false everywhere. This may be stale text or a change by another lane; `cap72` owns it. The ledger follows the code and D30.
4. **Proxies on macOS and Linux.** The transport reads the proxy environment variables only (`transport_host/endpoint_environment.rs`). It does not read git's `http.proxy`. The Windows guide says Windows ignores `http.proxy`, "the reverse of macOS and Linux", which suggests 1.0.17 honoured it there. Unverified here: S5.6's HTTPS proxy row should settle whether the transport differs from 1.0.17 on `http.proxy`.
5. **Plain `http://` on macOS and Linux** prints no warning and goes native; Windows prints one (D2). Intentional, or should the warning be uniform?

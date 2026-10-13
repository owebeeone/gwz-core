# Windows parity 5.6 — remediation plan, round 1

Status: **accepted at root `8fc5acb1`, gwz-core `45fe6e6a`, gwz-transport `a27817b9`, gwz-cli `cd72b954`, gwz-py `ea94500a` after `GwzTransportWindows-5.6-ReviewCode-2.md` and `GwzTransportWindows-5.6-ReviewState-2.md` reported GO (2026-10-14); this accepts step 5.6 only.** Drafted 2026-10-13. The object reviewed is the tuple in both reports:
root `42feaa9d`, gwz-core `37705303`, gwz-transport `a27817b9`, gwz-cli `220501a5`, gwz-py `28aff11d`.
The reports are filed verbatim:
- [Code (with the Surface section)](GwzTransportWindows-5.6-ReviewCode.md): NO-GO, 2 P2 and 12 P3, with a pre-commitment to GO on P2-1 and P2-2.
- [State](GwzTransportWindows-5.6-ReviewState.md): NO-GO, 7 P2 and 8 P3, with a pre-commitment to GO on P2-1 to P2-7.

No finding is architectural. Every blocking finding has a bounded local remedy.

**Blind convergence.** Both axes independently found the same two defects:
- Native commit/tag is not fail-safe against configuration libgit2 does not read: Code P2-2 and P3-10, State P2-2 and P3-8.
- Native commit takes "now" twice: Code P3-10(d) and State P3-7.

Both axes also attacked the POST-hold rule from different sides: Code P3-8 and State P3-4.

## Operator decisions taken for this round (2026-10-13)

- **OD17: ship 1.1.0 and state the gap.** TR8.1 misses only on Linux (weftpi) over HTTPS, by +0.04 to +0.08 s per command. It is release-built, and the adaptive limit is not the cause (gwz-core-evidence `2026-10-13-tr8-1-rerun`). SSH beats 1.0.17 everywhere, and Mac HTTPS meets the target. The gap goes in the release notes. A bounded profiling task on the fixed per-command HTTPS cost runs in parallel and does not block the release.
- **Native commit/tag is narrowed.** `commit -a` always goes to the git CLI, as in 1.0.17. Plain `commit` and `tag` stay native, with the fail-safe gaps below closed. The native `-a` code is kept in the tree for later, but not routed to. This disposes of State P2-3 and P2-4 by construction.
- **Capacity keeps the last-attempt rule; the wording is fixed.** The docs say "its last attempt was refused". The message stops saying "kept refusing" unless more than one attempt was refused.
- **Down-key texts are built now:** adaptive design §5.3 `not attempted:` and "still failing after a retry about 30 s later". No wire change.
- **gwz-py notes go to `logging.getLogger("gwz")`,** as the adaptive design's gwz-py paragraph says (operator: no preference, so the design stands).

## Dispositions

Lanes: **A** native commit and tag; **B** adaptive limit machine, Capacity, Down and Python notes; **C** user-facing wording, HTTPS texts and docs; **D** Windows SSH state fixes and the helper view.

| Finding | Disposition | Lane | Closure test |
|---|---|---|---|
| Code P2-1 | Build the §5.3 Down texts. A `Finish` with `attempt == 0` from a Down key reads `not attempted: <host:port> failed earlier in this run (<recorded reason>)`, keeping the recorded code and showing no attempt count. A failed retest's member ends `still failing after a retry about 30 s later`. A local marker is carried from the machine to the host's wording, with no wire change. | B | `down_tests`: two failed retests, then an arrival with the exact prefix and no `(attempt`. A retest row asserts the suffix. A gwz-cli Troubleshooting pin against the produced string. |
| Code P2-2 | The system config is decided from the git the CLI route would run. Resolve the first `git` on the snapshot's `PATH`, derive its `<prefix>/etc/gitconfig` (plus `share/git-core/gitconfig`), and go to the CLI unless that file is absent or is one libgit2 reads. Also go to the CLI when a file libgit2 reads exists that the resolved git does not read, or when no prefix can be derived. Keep `GIT_CONFIG_SYSTEM` and `GIT_CONFIG_NOSYSTEM`. On Windows, `NOSYSTEM` is not proof while libgit2 reads `%PROGRAMDATA%\Git\config`. | A | Fake `git` under a temporary prefix whose `etc/gitconfig` sets `commit.gpgSign` gives `CliReason::SystemConfig`. No derivable prefix gives the CLI. A covered file that the resolved git does not read gives the CLI. Windows `NOSYSTEM` plus an existing ProgramData config gives the CLI. |
| State P2-2 | Route to the CLI when any `hook.*` key is present (Git 2.54 config-defined hooks). Add this to §6's list. | A | Route rows: `hook.x.event=pre-commit` in the repository config and in the global config gives `Cli(Hook)` for commit; `reference-transaction` gives the same for tag. |
| State P2-3, P2-4 | Moot under the narrowing decision: `-a` always uses the CLI. | A | A route row: `commit -a` gives `Cli` for every input. The existing `-a` differential rows now assert the CLI route. |
| Code P3-10 (a) | `include.path` forms libgit2 does not expand (anything but absolute, relative or `~/`) go to the CLI. | A | Route row. |
| Code P3-10 (b) | Moot for `-a`. `GIT_ATTR_*` and `GIT_TEST_*` are removed from the harmless environment list anyway. | A | Route row. |
| Code P3-10 (c) | `commit.cleanup` is compared case-sensitively; an unknown value goes to the CLI. | A | Row: `Strip` gives the CLI. |
| Code P3-10 (d), State P3-7 | One "now" is used for both author and committer. | A | An injected clock across a second boundary gives equal times. |
| Code P3-10 (e) | Non-canonical epoch dates (leading zero) go to the CLI. | A | Route row. |
| Code P3-11 (part) | `CliReason` is surfaced as a `--verbose` detail line. | A | A CLI `--verbose` row shows the reason for a commit with a hook present. |
| State P3-6 | Tag creation uses a must-not-exist check under the reference lock (a transaction). | A | A check-then-create race row. |
| State P3-8 | `core.fsync` or `core.fsyncMethod` set to anything native does not honour goes to the CLI. | A | Route row. |
| Code P3-6 | The local Capacity report carries which local wait expired (job budget, setup slot, or shared reservation) and the actual wait. Each is worded on its own. | B | A reservation-wait expiry and an 8-slot expiry each produce their own words, neither saying "64" or "stopped answering". |
| Code P3-7 | The hold note carries the server's uncapped value, and the over-30 s case is worded ("asked gwz to wait 120 s; one command waits at most 30 s"). | B | A notes row with a 120 s hold. |
| Code P3-8, State P3-4 | Discovery and POST hold deadlines are kept apart. A hold seen after checkout returns the lease (Reusable when nothing was sent) and checks out again after the hold. | B | The interleaving (discovery 30 s at t=0, POST 2 s at t=1) releases POSTs at t=3. A hold set between checkout and serve uses a fresh lease. |
| Code P3-9 | gwz-py: a noted cancellable entry, with notes mapped to `logging.getLogger("gwz")` (detail lines at DEBUG). | B | A gwz-py limit-8 fixture row asserts a `gwz` log record. |
| State P2-7 | A requeued Down retest counts as a failed retest (`Machine::retest_refused`): `retest_at += 30 s` and `failed_retests += 1`. | B | A machine row puts the key Down, retests, and abandons the retest as evidence; `decide` waits until `retest_at + 30 s`. An endpoint row counts the handshakes. |
| State P3-2 | A RESTORING step closes after S starts; later starts belong to the next step. | B | A trace row with continuous arrivals during RESTORING asserts step progression. |
| State P3-3 | Ns `started` is reported when the setup holds its job place, not at Connect. | B | A row with the job budget full asserts Ns after a conclusive drop. |
| Capacity wording (decision) | The message says "refused the last attempt"; "kept refusing" only when more than one attempt was refused. Troubleshooting and Releases say "its last attempt was refused". | B | Rows: reset, reset, reset, 429 does not say "kept". Two refusals say "kept". A doc pin. |
| Code P3-1 | `BUDGET_WORDS` names the 30-second setup budget, and says only `--ssh-timeout 0` removes it. T6 and the Windows help sentence say "up to 30 seconds; 0 removes the limit". | C | A `pageant_words` row: the budget text names 30 seconds and `0`, and does not say "raise". |
| Code P3-2, State P3-1 | Pageant `Denied` gets its own text: Pageant refused the request; it may be running as another user or at another integrity level; what to do. `SendMessageTimeoutW` ERROR_ACCESS_DENIED (UIPI), and other codes while `IsWindow` still holds, become that refusal, not "gone". | C | Rows for the Denied text and the error-mapping table. |
| Code P3-3 | D7's predicate excludes `Allocation`, `Interaction`, Capacity and any failure without network facts. | C | Under `Direct`, `allocation_timeout()` produces no D7 words. |
| Code P3-4 | The T5 refusal is kept per open (request and stream, or canonical URL), not per operation. | C | Two members redirected to http in one operation each get their own T5. An unrelated `InvalidRequest` gets none. |
| Code P3-5 | `WWW-Authenticate` is parsed on bytes; only scheme tokens must be ASCII. | C | A Basic-only challenge with a UTF-8 realm asks the helper once. |
| Code P3-11 (rest) | Worker provenance refusals name the worker (missing or mismatched) and the remedy (reinstall, or `--transport native`). A 3xx during an SSPI exchange gets its own text. | C | Text rows. The three 4.10 provenance rows assert the text. |
| Code P3-12 | Docs aligned with the code. The guide: line 3 (the candidate banner), lines 333–336 and 583 (the proxy was tested in 4.7), line 12 ("the one case"; file, local and git remotes also go native). Troubleshooting "HTTPS Credential Failure" (drop "under qualification"; add the declined and configuration-refused cases). The `endpoint_environment.rs:18` comment (HTTPS on Windows needs a resolvable home). The stale docstring in `test_windows_qualification_boundary.py`. | C | Doc pins where they exist; review of the diff. |
| Code P3-11 (U20) | Covered by the worded texts above; no blanket `--verbose` pointer. | C | — |
| State P2-1 | The helper view's `in_effect` follows git's urlmatch: lowercase scheme and host, the default port normalised away, path prefix matching at `/` boundaries, `*` per host label, and the `match_partial_url` fallback. Remove or invert the test that pins the divergence. | D | Table rows for wildcard, case, `:443` and path-prefix patterns (true), and for a different port, a different user and a non-prefix path (false). A real-git row on Windows: a wildcard-scoped helper that declines gives `Declined`. |
| State P2-5 | `force_dispose` drops its owners and sets `Disposed` whatever `terminate` returns. `terminate` treats `ConnectionReset` and `ConnectionAborted` as already terminated. | D | A unit row where `terminate` fails with `ConnectionReset` still reaches `Disposed` and the capacity is acknowledged. A Windows row resets an active exchange with linger 0 and the slot is reused. |
| State P2-6 | `cng_reads` admits only text whose first non-blank line is exactly the RSA header, with a matching footer, the admitted block being that RSA block, and every line under 127 characters. | D | Rows: a trailing-space header over an OpenSSH body is refused; a single-line RSA body is refused; a 64-column RSA PEM is accepted. |
| State P3-5 | `regular_file`'s device-path spelling check treats each separator position as `[\\/]`. | D | Table rows for all eight mixed forms. |

## Follow-ups recorded, not in this round

- Code §3 and State §3 follow-ups. These include:
  - Gh and Anonymous advertised on Windows (1.2.0);
  - the trust-map eviction order;
  - stale long holds in `held_over` (1.2.0);
  - helper-failure caching;
  - the cancel-mid-read test flake;
  - a CapacityDetail refused at encode closing the carrier;
  - the `ssh setup timeout: aggregate` wording;
  - the blanket `#[allow(dead_code)]` on `agent_pageant`;
  - new process-global debt (already accepted by the operator).
- The Pi HTTPS fixed per-command cost: a profiling task, not blocking (OD17).

## Re-verdict

One merged revision of lanes A to D goes back to the same two reviewers, with this plan and the diff since `37705303`. Each reviewer re-checks its own counterexamples and fills a prior-finding closure table. This is round 1 of the two-round cap.

## Outcome (2026-10-14)

Both reviewers re-verdicted GO on the merged revision; every finding above is closed, the -a findings as moot under the narrowing. New observations below the finding bar, recorded as follow-ups: local-wait attribution is matched by site and recency (both axes); a POST re-checkout reuses the allocation taken at prepare time (State); the D7 exclusion is narrower than this plan's "any failure without network facts" (Code). CI on the accepted tuple failed two test rows outside product code, the SSH fixture's bind-and-release port race (gwz-core, Linux) and the gwz-py close timing row (Windows 3.13); both reviewers judged the fixture race a test defect. Lane `ciflake` fixes both after acceptance.

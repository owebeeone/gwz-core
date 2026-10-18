# Re-verdict, round 2: exact SSH agent-key selection design (D38), Consistency + Surface

- **Object:** `gwz-core/dev-docs/GwzExactAgentIdentityDesign-DRAFT.md`, revision 1, at gwz-core `7f206c22a2b8104f5f8ebd1f61fb662f380ebe3f`. The parent is `de8e4788`, and the remediation plan is `GwzExactAgentIdentityDesign-RemPlan.md`. Lane: `/Volumes/projects/limbo/gwz-dev-exactagent`.
- **Date:** 2026-10-18.
- **Axis:** Consistency, with Surface folded in. Read-only. This round is not peer-blind: the plan cites the Safety reviewer's findings, which round 2 allows.
- **Verdict: GO.** All ten round-1 findings are closed. There are no new findings at P0, P1 or P2, and no new ARCHITECTURAL root cause. Seven new P3s are listed below; none blocks.
- **Commit checks:** HEAD was `7f206c22` at the start and at the end. The only untracked file, `dev-docs/GwzRemoteTransportBugReport.md`, was already there. I modified nothing and built nothing.

## §0 Evidence base for this round

These are the code claims new in revision 1, each checked against the source:

| Claim | Where | Result |
| --- | --- | --- |
| The transport route compiles only under `cfg(all(gwz_transport_candidate, any(unix, windows)))`, and a backend with no host context takes the native route | `git/gitbackend/transport_binding.rs:8–9`, `:243–247` | ✓ |
| Facts are merged across attempts, with `key_fingerprint` taking the later value, else the earlier | `setup_retry.rs:140–158`, `placement_endpoint/admission.rs:362–370` | ✓ |
| Retry classification: `Unavailable` with cause `ConnectionRefused` is retried (F6) | `setup_retry.rs:63–77` | ✓ |
| The sshd fixture logs at `VERBOSE` to `sshd.log`, and sshd keeps the first value of a keyword | `ssh_fixture.rs:74–83` | ✓ |
| `KeyType::key` exists but is private, so making it crate-visible is a real change | `agent_keys.rs:134` | ✓ |
| The container's line splitter treats CR and LF as boundaries | `ssh_key_container.rs:60–74` | ✓ |
| An agent that refuses to list (`SSH_AGENT_FAILURE`) gives `PermissionDenied`, which the host reads as `Authentication` | `agent_client.rs:45–47` | ✓ (rule 4 → K1) |
| A Unix agent-socket connect error is passed on as its io kind | `agent_socket.rs:35`, `:53`, `:67` | ✓, but see N3 |
| The in-crate Pageant rows exist as twins of B06 and X9 | `ssh_tests/pageant_endpoint.rs:25`, `:50`, `:65` | ✓, but its header (`:4–5`) says "**No parity claim rests on these**" (see N7) |
| The qualification test asserts false under `gwz_session_candidate` | `transport_host/qualification_tests.rs:107–115` | ✓ |
| The step that removes gwz-core's cfg | `GwzV110RemainingPlan.md:82–83`: C3 (S7.1c) removes the switch in gwz-core; C4 (S7.1d) does gwz-cli and gwz-py | Conflicts with the design's §5 (see N7) |

I also checked the arithmetic in the step table. Production lines total 2,090, or 1,990 without steps 2.4, 4.2 and 5.1. Test lines total 4,200, or 3,300 without them. Both pairs match the text.

## Closure table for the round-1 findings

| Finding | Round-1 problem | Closed in revision 1 | Original counterexample re-run | Status |
| --- | --- | --- | --- | --- |
| **C P2-1** | K1's remedy for Pageant sent the user to the wrong agent | K1 now has one remedy per source (§3.3, line 290). The Pageant remedy says to use Add Key or close Pageant, and that `ssh-add` cannot reach Pageant. Table row 5. Rows X9e and PX. Steps 1.3 and 2.4. | X9e (Pageant running without the key, the pipe agent holding it): the user is now told to load the key into Pageant or close Pageant. Both work. | **Closed.** The same root cause remains in K6 (N1). |
| **C P2-2** | On Unix, a missing or dead agent got the bare `ssh setup failed: Unavailable` | New text K0 (lines 278–283). Rule 5. Table row 2 (`Unavailable`, cause `NotFound` or `ConnectionRefused`, source `None` or `Path`). Row X5u, and X5w split out for Windows. | `SSH_AUTH_SOCK` unset now gives K0 "not set", and a stale socket gives "not answering" with the attempt count. | **Closed.** A residual EACCES case is N3. |
| **C P2-3** | Rule C's near-miss detection was too narrow, and its own remedy failed in Windows PowerShell | Rule C steps 2, 4 and 5 now classify UTF-16, a byte-order mark, a NUL byte, RFC 4716, `.ppk` and an options-prefixed line into K4. The remedy is shell-safe. The PowerShell behaviour was measured on dabeest (§3.1). There is one parser row per case (§6.2). | `ssh-keygen -y … > k.pub` in PowerShell 5.1 now gives K4, which names the cause and gives `Set-Content -Encoding ascii` (measured as accepted). A PuTTYgen file gives K4 with the conversion. | **Closed.** A wording residual is N4. |
| C P3-1 | METHOD_NONE was worded K2 instead of K3 | Rule 6 calls `rejected()`. Table row 7. §4.1. Row X8n. Step 1.2. | Now K3. | Closed (minor note in N7). |
| C P3-2 | K5 was not source-aware; T6 sent exact opens to native | K5 points to the native note, carries a "named by" clause and the `--unset` line. Table row 4 drops the native clause from T6, T6b and the cap text. | In gwz-py, K5 no longer names a `--transport` flag that gwz-py lacks. | Closed |
| C P3-3 | K6 named only `--identity`; the no-`.pub` branch; two owners | K6 says "wherever you named `<file>`" and always prints the `ssh-keygen -y` line. Step 3.2 is the sole owner. | `--remote-identity` case: the remedy now takes effect. | Closed. The `KeyFile` addition regressed (N1). |
| C P3-4 | Capability doc comment; the OQ4 premise | §5 ties the capability to `transport_route_installed()` and rewrites the doc comment. OQ4 is restated with the in-process option and its race. | n/a | Closed. Citation drift is in N7. |
| C P3-5 | Step graph and estimate basis | Explicit edge table. Step 2.4 owns the Pageant rows. The Windows oracle control is proven in Phase B. Reviews are placed. The estimate is restated like for like. | n/a | Closed. New residuals are N5 and N6. |
| C P3-6 | Unlisted user docs; the `private_key_path` name | Step 3.3 lists every file (`auth.md`, `Releases.md`, `Troubleshooting.md`, `MachineOutput.md`, `MessageCatalog.md`, the gwz-py docstring). | n/a | Closed |
| C P3-7 | Drift in claims and citations | Every item is fixed: the HTTPS bullet, the alternate-data-stream note, `client.py:371`, `:75`, F2, the OQ6 citations, and keys A and B. | n/a | Closed |

## New findings (all P3)

### N1 (P3): K6 and the Windows `KeyFile` addition remain source-blind

- **Location:** K6 (lines 325–333).
- **Root cause:** this is the same root cause as C P2-1, now in the revised D17 text. `SshLimit::reason(host, port, known_hosts, key_file)` has no agent source (`ssh_limits.rs:87–93`), so K6 says "load the key with `ssh-add`". With Pageant running, `ssh-add` adds the key to the OpenSSH agent, which gwz does not consult.
- **Second problem:** the `KeyFile` words gain "`--identity <key>.pub`" only. That is the wording C P3-3 closed for K6.
- **Consequence:** a Pageant user is sent to the wrong agent first. The next run's K1 (Pageant variant) puts them right, so the chain converges, but only after an extra step.
- **Correction:**
  - Pass the agent source kind into K6. `open_failure_io` already holds it.
  - Add the Pageant branch: load the key into Pageant with Add Key.
  - Use "wherever you named" in the `KeyFile` addition too.

### N2 (P3): the help text's "gwz's own transport only" reads as if it covers the whole option

- **Location:** §2.4 (lines 171–173).
- **Root cause:** the restriction applies only to the public-key branch, but it is placed after the whole sentence.
- **Consequence:** a user on `--transport native` reads it as "`--identity` does not work with native", which is false for a private key.
- **Correction:** "…or a public-key file (for example ~/.ssh/id_work.pub) to use that key from your SSH agent (gwz's own transport only)."

### N3 (P3): on Unix, an agent socket that refuses permission (EACCES) is worded K1

- **Location:** table row 2 (line 265), whose causes are only `NotFound` and `ConnectionRefused`; rule 5.
- **Root cause:** `agent_socket::connect` passes EACCES on as `PermissionDenied` (`agent_socket.rs:67`). The host reads that as `Authentication` with no fingerprint, which is row 5, K1. This breaks the existing invariant in `agent_source/fault.rs:33–36` ("Never `PermissionDenied`: that is an authentication failure, and no key was offered") and rule 5 ("K1 is only for a reachable agent").
- **Consequence:** after `su` without `-`, the shell keeps the other user's socket path. K1 then says the agent does not hold the key and suggests `ssh-add`, which itself fails with "Permission denied". K0 even names sudo as a cause.
- **Correction:** map a connect-time `PermissionDenied` on the `Path` source to `Unavailable` (an agent-fault marker). Give K0 a third clause: "the agent SSH_AUTH_SOCK names belongs to another user". Add a row to X5u.

### N4 (P3): rule C step 2 sends private-key encoding near-misses to K4, whose words assume the user wanted a public key

- **Location:** §2.2 step 2 (lines 112–114); K4 (line 313).
- **Root cause:** step 1's armor test runs on raw bytes, so a UTF-16 private key, or a DER key with a NUL byte, misses it. Step 2 then gives K4: "cannot select an SSH agent key…".
- **Consequence:** the user named a private key and is told about public-key formats. The "Common causes" clause about PowerShell helps, but the opening sentence misleads.
- **Correction:** either give step 2 a K4 variant worded for either intent ("<file> is saved as UTF-16 / with a byte-order mark / holds binary data; gwz reads key files as plain ASCII or UTF-8 text…"), or run the armor test on UTF-16-decoded text before step 2. Add a unit row for a UTF-16 private key and one for a DER key.

### N5 (P3): the step graph contradicts the placed reviews, and one edge is missing

- **Location:** §7 edges (lines 596–597) against "Reviews, placed" (lines 626–631).
- **Root cause:**
  - The edge table lets 4.1 and 4.2 start after 2.3 and 3.1, so they can run before 3.2, 3.3 and the Phase C Surface review. The review text says that review comes "before the evidence steps 4.1 and 4.2 pin them".
  - Step 2.1 adds the K4 `SshLimit` variant, whose `reason` arm needs the K4 text from 1.3. That edge is not listed.
- **Consequence:** if the Surface review changes K1, K3 or K5, the CLI evidence archived on dabeest goes stale and has to be re-run.
- **Correction:** add 4.1 and 4.2 needing the Phase C review (or 3.3), and 2.1 needing 1.3. Alternatively, state that the K4 arm is filled in 2.3.

### N6 (P3): the restated estimate's sum leaves out the reviews it says are on top

- **Location:** §0 (lines 36–39); OQ9.
- **Root cause:** 6 to 9 days for the like-for-like scope, plus 3 to 5 for Windows, is exactly 9 to 14. The text says "two review rounds are on top" and then gives 9 to 14 "in all".
- **Consequence:** the figure the operator confirms in OQ9 is lower than the text says. D38 itself cites 5 to 10.
- **Correction:** add the reviews (for example, 1 to 2 days, giving 10 to 16), or drop "on top".

### N7 (P3): remaining drift in claims and citations

- **Activation step:** §5 says "the activation step C4 replaces that cfg". gwz-core's switch is removed in C3 (S7.1c), and C4 covers gwz-cli and gwz-py (`GwzV110RemainingPlan.md:82–83`). The remediation plan says "until C3b". The "cannot drift" claim needs the right step named.
- **Pageant evidence:** §4.2 calls the synthetic-Pageant rows "evidence new here". `pageant_endpoint.rs:4–5` says no parity claim rests on them. State that the Pageant cell is claimed by composition (shared filter, plus the 2026-10-17 real-Pageant ambient run, plus D30's row on the pipe agent), and record that basis in S5.6.
- **The table's domain:** §4.1 says the selection table is called first "for an exact open" (method `SshAgent`). Rows 8 (K5, not a wire failure) and 9 (K6, method `SshKey`) can never match under that gate. Either mark them as listed for completeness only, or say where they are selected.
- **Row X8n:** its control expects "`METHOD_NONE` in the ambient step's facts". Facts carry no return code, so that is not observable there. Use the server log, or the `observed_authenticate` return-code hook (`agent_auth.rs:174`).

## Attacks on the revised parts that failed

- **The selection table's order and disjointness.** Rows 1, 2/3, 5, 6 and 7 are disjoint by code, fingerprint and source.
  - The fingerprint bit correctly separates UP (a refused URL password resets `authenticated` through `offered`, `ssh_local.rs:92–97`) from K3.
  - Pageant's T10 (`Unavailable`, no cause) and gone words fall to row 3.
  - Windows `AgentFault`s arrive as `Unavailable` (`ssh_setup.rs:608–611`), so they stay out of K0.
- **Rule C ordering.**
  - Byte-order mark: step 1 ignores a UTF-8 mark for the armor test, so a private key with a mark keeps today's refusal.
  - UTF-16 without a mark: caught by the NUL test.
  - An options-prefixed line: caught by step 5 and refused by step 6. A quoted command containing spaces still puts the type within the first five words.
  - A `known_hosts` line goes to K4, which is reasonable.
  - Armor split by CR inside a comment: the shared splitter makes armor win.
  - A list refusal giving K1 matches rule 4 (`agent_client.rs:46–47`).
- **The capability tie.** It is consistent with S7.2 and S5.6: the answer is true only in a transport build, after D30's row. The `transport_route_installed()` predicate matches the route's cfg.
- **No wire change.** This still holds after the new facts usage: `key_fingerprint` is accepted for non-SSPI v2 facts, and only `method`, `credential_offered`, `authenticated` and `key_fingerprint` are used.
- **The first-day walkthrough, re-run on the revised text.**
  - Set up: `ssh-add`, then `--identity k.pub`, which selects exactly and prints `fingerprint=`.
  - No agent gives K0. Windows with Pageant gives the correct K1. A PuTTYgen file gives K4 with the conversion. PowerShell's `>` gives K4 with the fix.
  - `GWZ_TRANSPORT=native` with a stored setting gives K5 with the `--unset` line.
  - Undo with `gwz auth identity <remote> --all --unset`.
  - The only detour is K6 with Pageant (N1).
- **The estimate's line totals.** Both match the step table.

## Operator questions OQ1–OQ9 (which answer the evidence favours, not a decision)

- **OQ1:** A. Option E's costs are stated correctly: one path would mean two credential sources depending on whether the file is encrypted, it works only for the OpenSSH container, and it needs the private file read.
- **OQ2:** content only. A suffix rule would not fix RFC 4716.
- **OQ3:** defer the fingerprint selector.
- **OQ4:** answer for the build's route. The process-setting option is now stated correctly, with its race. gwz-cli never calls the capability.
- **OQ5:** in-crate rows plus the pipe-agent CLI rows meet D30's wording ("one exact-selection row on Windows", from any agent). Record the Pageant claim as composition (N7), because the synthetic rows carry no parity claim of their own.
- **OQ6:** set the fingerprint at the offer. Table rows 5 to 7 now depend on that bit.
- **OQ7:** approve the revision, after N1 is fixed.
- **OQ8:** exact blob equality. Rows C1 to C4 make it testable.
- **OQ9:** confirm that git2-rs stays untouched. Correct the review arithmetic before confirming the figure (N6).

## Next action

GO on this axis. Fold N1 to N7 into the design text before Phase A starts. They are wording, edge and citation fixes and need no re-review on this axis. N3 and N4 add unit rows to steps 1.1, 1.3 and 2.2.

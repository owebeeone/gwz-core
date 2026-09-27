# GWZ transport release plan amendment — first remediation plan

Date: 2026-09-27. Status: **remediation plan for [the first verdict](GwzTransportReleasePlanAmendment-Verdict.md); applied as one revision of `GwzTransportReleasePlanAmendment.md`.**

Every finding of the two reports gets exactly one disposition. The findings are [Consistency](GwzTransportReleasePlanAmendment-ReviewConsistency.md) C-P2-1 to C-P3-9 and [Safety](GwzTransportReleasePlanAmendment-ReviewSafety.md) S-P2-1 to S-P3-4.
- All findings are accepted.
- Two corrections differ in form from the one the reviewer wrote, with the reason given: C-P3-2's changelog line and S-P3-4's closure test.
- Every correction is to the amendment's text. None changes the operator's decisions: OD11 as adopted, the stdio mode in this release, and the SSH remote form designed in TR1.3 under OD12.
- The revision also takes the reviewers' residual notes where they are cheap (§3).

Section numbers below are revision 1's. Revision 1 adds §3.7 (Phase 8's sign-off), so the former §3.7–§3.13 become §3.8–§3.14.

## 1. Blocking findings

| ID | Disposition | Where | Closure test |
| --- | --- | --- | --- |
| C-P2-1 | OD12's "if yes" list gives exact text for every clause it changes: line 23's replacement, the decisions list, both columns of §2's server row, and §9's first bullet. It states that line 561's replacement then applies as written. "If no" states that the same clauses stand and the decisions list records the answer. | §3.11 (OD12) | Applying the yes-edits to a copy of the amended plan leaves §1, §2 and §9 each naming the SSH remote form as shipping, and no sentence calling it unsupported or out of scope. |
| C-P2-2 | Line 561 is replaced: "…a separate-process wire other than the local server socket, the stdio mode's standard streams and, if OD12 brings it in, the SSH remote form's `ssh` channel." | §3.13 | No §9 sentence excludes a wire that §1 or §2 lists as shipping, under either OD12 answer. |
| S-P2-1 | The must-match floor names `SSL_CERT_FILE` and `SSL_CERT_DIR` where the native HTTPS stream is OpenSSL, which the vendored libgit2 uses on Linux (`libgit2-sys/build.rs:256-268`; `streams/openssl.c:148`). TR1.3 enumerates every environment read of the native path per platform from the vendored sources, and records the list as an amendment to the contract's §5.8 disclosure. Later proxy detection joins it. The routed refusal names the variable, never its value. The §8 risk no longer relies on the `auto` key. Evidence item 5 states the facts. | §2 item 5, §3.4, §3.6, §3.12 | Phase 7 exit: on Linux, at an explicit address, a server and a client with different `SSL_CERT_FILE`, where the client's private HTTPS member takes TR1.6's route. The operation is refused before any connection opens, naming `SSL_CERT_FILE`, and the TLS fixture records no handshake. A source-level test asserts the must-match list holds every name in TR1.3's list. |
| S-P2-2 | Question 2 gains an allowlist that gwz applies itself, whatever `ssh` is installed. The host may hold ASCII letters, digits, `.`, `-` and `_`, or be a bracketed IPv6 literal. The user may hold ASCII letters, digits, `.`, `-` and `_`. Neither may start with `-`. The port is a decimal from 1 to 65535. It cites CVE-2023-51385's expansion path. The address sets no `ssh` option beyond its port. | §3.4, question 2 | OD12's yes rows: a user or host holding a character outside the set, including a backtick and `$(`, is refused before `ssh` runs, and a spawn double records zero spawns. |
| S-P2-3 | Question 2 gains "The program": both CLIs resolve `ssh` to an absolute path through `PATH` alone, never the current directory, the workspace or a repository. On Windows this is an explicit `PATH` search limited to `.exe`. On POSIX, empty and relative `PATH` elements are skipped. | §3.4, question 2 | OD12's yes rows: on dabeest with a planted `ssh.exe` in the caller's directory, and on Linux and macOS with a planted `ssh` and `.` absent from `PATH`, the remote form runs the recording `PATH` program, never the planted one. |

## 2. Nonblocking findings

| ID | Disposition | Where | Closure test |
| --- | --- | --- | --- |
| C-P3-1 | The reference reads §3.14, the section that records agent confirmation. | §3.8 | The reference resolves to the bullet that quotes the retry plan. |
| C-P3-2 | Line 508 becomes "…adopted every recommendation below from OD1 to OD11. OD12 is open." Line 7 gains "and OD11 by the amendment. OD12 is open." **Form differs:** line 589, the 2026-09-27 changelog entry, is history and is not rewritten. The plan's new changelog entry on GO records OD11 as adopted and OD12 as open. | §3.1, §3.11, §5 | No sentence in the status block or §7 claims OD12 decided. The new changelog entry says it is open. |
| C-P3-3 | TR1.3's Surface clause names the SSH remote form "unless the operator accepts TR1.3 without its section", matching the carve-out. | §3.4 (Review) | Designing the form is unconditional everywhere; only shipping depends on OD12. |
| C-P3-4 | Takes S-P3-3's rule: only `--server` carries the SSH remote form, and the parser line says so. Question 1 fixes the rule, and says a design proposing another source amends it. | §3.4 | The parser line, question 1 and §3.12's risk name one rule. |
| C-P3-5 | TR2.8's test asserts the `--verbose` transport row names the route's cause, never the key. S7.2's ledger row stays in §3.8. | §3.5 | TR2.8's tests name only outputs that exist in Phase 2. |
| C-P3-6 | Phase 8's sign-off (line 402) gains an "agent key types" cell that TR2.8's tests evidence. §4's native-path records get owners: the certificate case is a TR2.8 fixture row against a `sshd` that trusts a test user CA; the security-key case is a named manual row in Phase 9, before S7.2. | §3.5, §3.7, §4 | S5.6's table lists the cell, and both native-path records have an owning step. |
| C-P3-7 | §1 also controls the SSH agent design's §5 sentence "Ed25519 and RSA modern-signature fixtures are required; unsupported algorithms fail explicitly." TR2.8 quotes it and gives the replacement. On GO, the agent design's status names the amendment, and A2 gains a changelog note. | §1, §3.5, §5 | The agent design's status names this amendment for §5's sentence. |
| C-P3-8 | Question 9, the off switch. The remote form sends no environment, so TR1.5's snapshot-entry carrier is unavailable. The section says which side's value governs. The recommendation is the client's resolved value, as a `ProcessAttributes` field. `--verbose` says which value applied. | §3.4, question 9 | TR1.3's section answers question 9. |
| C-P3-9 | S7.3 gains a fifth route check: one CLI network operation through the stdio mode's local client form. OD12's yes list adds an S7.3 check through the remote form against a disposable loopback `sshd` on Linux, which Phase 10's post-release check repeats on one host. | §3.8, §3.11 | S7.3 names five route checks, or six under OD12 yes. |
| S-P3-1 | Automounts become a requirement. The client walks an absolute path from `/` and checks each prefix with `statfs`. It refuses a path that enters an automounter's or a network file system's mount before looking up the next component. The same check applies to the path `auto` derives from `XDG_RUNTIME_DIR` or `TMPDIR`, which an unread environment can also set. §2's server row names such addresses as unsupported. | §3.2, §3.4, §3.6 | Phase 7 exit: an address under `/net` on macOS is refused, with no lookup below `/net`, and one under an autofs mount on Linux is refused. |
| S-P3-2 | Question 3's marker is accepted only by the stdio host. A socket host refuses a `SessionOpen` that carries the marker, or carries no snapshot, with `invalid_request` before `SessionOpened`. The local client form never sends the marker. | §3.4 (stdio, question 3) | Phase 7's stdio rows: a socket host refuses a `SessionOpen` without a snapshot. OD12's yes rows: a socket host refuses the marker, and the remote client's `SessionOpen` carries it. |
| S-P3-3 | Question 1: only `--server` carries the form. `GWZ_SERVER`, gwz-py's library and every configuration file never do. Question 7 gains the no-terminal case: whether `ssh` gets a terminal, whether `BatchMode` is set, and that a prompt with no terminal fails within a bound with a named error. | §3.4 | OD12's yes rows: `GWZ_SERVER` holding the form is refused before `ssh` runs. A run with no terminal against an unknown host key fails within a bound and leaves no `ssh` process. |
| S-P3-4 | TR2.7 and §3.14 state that the file's certificates add to the platform's roots, which can be wider than Git gives for the same variables. S7.2's notes must say so. **Form differs:** the reviewer's closure test needs a fixture certificate that chains to a platform root, which a disposable fixture cannot hold. A TR2.7 unit test pins the same property instead: the connector keeps the platform's roots. | §3.5, §3.8, §3.14 | The unit test, and S7.2's notes naming the semantics. |

## 3. Residual notes taken

- **RSA with SHA-1.** §2's SSH row reads "RSA with `rsa-sha2-256` or `rsa-sha2-512`". §3.14 records that a server accepting only `ssh-rsa` SHA-1 signatures fails on the transport, with no native route, and the notes say so (Safety).
- **The stdio child's interrupt, and the client's end.** The descriptor rule also covers the client's end, and the design states how the child treats the terminal's interrupt, so the client's interrupt protocol governs (Safety).
- **Refusals name their cause.** A listener sandbox check that cannot be made refuses, naming the cause (Safety).
- **Confirmation.** The notes name `--ssh-timeout 0` and the repeated prompts. Changing the behaviour amends the retry plan, the agent design's §6 and the timeout plan's interaction accounting (both axes).
- **"Never auto-started".** It becomes "`auto` never selects or starts it", because the local client form does start it (Consistency).
- **Phase 7's native-route row** runs at an explicit address, since the `auto` key covers every must-match value (both axes).
- **"§2" qualified.** References to the plan's §2 say "the plan's §2"; references to this amendment's §2 say "this amendment's §2" (Consistency).
- **`SocketCoreBridge`'s grammar** is named in TR1.3's Surface list, as the address grammar the three entry points accept (Consistency).
- **Both CLIs' local client forms** pass Phase 7's stdio rows, not only gwz-cli's (Consistency).
- **Route granularity.** The route is chosen per SSH remote of an operation; HTTPS remotes are unaffected (Consistency).

Not taken, as below the bar or outside this amendment: the handshake bound for a silent listener, which is server design content; the native branch's pre-existing disregard of `GIT_SSL_CAINFO` and proxies; the tension between Phase 7's debt gate and native routes inside a server, which the accepted plan carries; and a note on the listing race's key choice.

## 4. Re-review

- The same two reviewers give a focused re-verdict on revision 1's SHA-256. Each gets this plan and the diff from revision 0.
- The revision changes no shared interface, architecture or reviewed call graph, so no fresh round is needed (AgentProcessRules as amended by GwzProcessOptimization §4.1).
- If a reviewer classifies a finding in the re-verdict as a new architectural root cause, the lane stops for the operator.

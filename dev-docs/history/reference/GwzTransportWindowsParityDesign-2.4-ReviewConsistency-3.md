# TR1.8 freeze review, final round (Consistency, with the Surface mandate)

Verdict: **GO**. Every round-2 finding of mine and every open Surface finding is closed, and the remediation introduced no new P0, P1 or P2. Six P3s remain, listed below; none blocks.

**Tuple, checked at start and end:**
- root `a7dc0859`, gwz-core `b6ef02af`, gwz-core-evidence `928f6d4`;
- gwz-transport `6146bd58`, gwz-cli `21d62310`, gwz-py `5950ba38`, gwz-sspi `73de8c8e`, git2-rs `d13951f7`.

Untracked files are out of scope. `…-2.4-ReviewSafety-3.md` appeared while I worked.

**Pinned hashes (section 1 of the package, K):** all 11 match, including the design `c24c7033f295c671`, the guide `f5e993852aa084f9` and the round-2 remediation plan `84de2944276b1959`.

Citation keys: D = design, G = user guide, K = review package (Annex S is its second half), P = plan, F = decision list.

## 1. Consistency closure (my round-2 findings)

| ID | Status | Evidence |
|---|---|---|
| P2-1 residual | **Closed** | One rule is stated at D165: (a) a value outside the grammar is refused; (b) a shape inside the grammar with no executed row follows WinHTTP's rule, and goes direct only when a stated bypass rule matches. D161, L8 (D324) and D423 point to it. The original counterexample, "refused before any open" applied to the https class, is gone from D423. The guide and Annex S summaries of this rule are inexact (P3-A). |
| P2-4 nit | Closed | D324 and D351 now list `HTTP_PROXY` and `http_proxy` against an https origin, matching D133. |
| P3-2 | Closed in substance (one nit) | D22, K25 and K27 state the settled tuple, and RX is in `928f6d4`. Nit: K3 and K9 still call the objects "uncommitted… working tree"; they are committed in `b6ef02af`. |
| P3-11 nit | Closed | D65 and D330 name step 3.1. |
| N1 (P2, architectural) | **Closed by D5 (b)** | The Network phase is stated with its cost at D115–117, L20 (D336), register item 14 (D307), D368 and D421. Plan step 3.6 (P423) says "no change to the clock", and D5 is recorded at F39. No LocalInteraction residue remains in the design, plan or guide; the only matches are the withdrawal at D421 and the changelog. The clock amendment is untouched. |
| N2 | Closed | D208: eight rounds per operation. |
| N3 | Closed | P506: the 407 message names the machine proxy setting and has no native pointer. |
| N4 | Closed | D218 |
| N5 | Closed | D304 and G427–435 add the CA-file refusal, quoting the run's README lines 78–79. Two precision problems follow as P3-B. |

## 2. Surface section

**Cold read.** The guide reads as user documentation:
- no process vocabulary and no OPEN ITEM placeholders;
- every refusal is quoted with a next action;
- the `http://` exception is placed inside the setting order (G64–72).

### Closure of Surface's round-2 findings

| ID | Status | Evidence |
|---|---|---|
| S1 remainder | Closed | G288–291 says "measured with gwz 1.0.17… GWZ's own transport has not yet been tested through a proxy" and names step 4.7. The unmeasured list is carried at G314–319 and G493–497, and in Annex S.3 and S.5 (K241, K263). |
| S4 remainder | Closed | G355–368 quotes the rejection message with its next action. |
| S14 | Closed | G377–382 has no native pointer. D192 agrees. The code says "nor did gwz 1.0" (`https_failure.rs:136`). That same message also wrongly says Windows answers Digest, which is P3-D. |
| N1 (D1 refusal not actionable) | Closed | G294–307 quotes text T1 and gives both routes, plus the case where direct connections are blocked. |
| N2 (PEM copy never connected to GWZ) | Closed | G236–255 covers the pin, the undo and the `$HOME` caveat. The syntax matches `gwz-cli/docs/commands/auth.md:19-27` and `parser.rs:56`. |
| N3 (`http://` overrides the order) | Closed | G15–23 and G64–72 cover native under `--transport gwz`, a warning that cannot be turned off, and the native timeouts. These match the CLI help. |
| N4 (`--ssh-timeout` scope and stall message) | Closed in the guide | G79–83. Its owed message T7 is not carried by any plan step (P3-C). |
| N5 (limits not stated) | Closed | G387–405; G140–145. |
| N6 (missing CA file; member URL) | Closed | G427–435 (precision in P3-B); G395–397. |
| P3-a (agent service steps) | Closed | G116–121, G127–129. |
| P3-b (name `manager`) | Closed | G372–374. |
| P3-c (redirect hazard) | Closed | G446–448. |
| P3-d (https→http refusal) | Closed | G452–459. |

### First-day walkthrough, from the guide alone

- **SSH with an ed25519 key: completes.**
  - With an ed25519-only `known_hosts`, the user learns to add an RSA entry (G205–208) with `ssh-keyscan` and a fingerprint check (G186–201).
  - The file key is not used. The user loads it into an agent (G217): starting the agent service (G110–122), or Pageant (G132). A pinned key is covered (G135–138).
  - The converted-PEM route is correctly limited to RSA (G219–234). The copy must then be named (G236–255), and G250–251 explains how to undo the pin.
  - Nit: G188 promises the printed path only "when a host is not in `known_hosts`". D371 promises it for every `known_hosts` message, including the ed25519-only one.
- **HTTPS behind a corporate proxy: completes, or ends in a stated dead end.**
  - "Direct access" with a PAC-only setup: the user sees `netsh winhttp import proxy source=ie` (G271–278).
  - The D1 `http=`-only case: T1 is quoted with two next actions, and the guide says native fails too where direct connections are blocked (G294–307).
  - A 407 is refused on both transports (G332–337).
- **A redirecting server: completes.** At most five hops, `--verbose` shows each one, credentials never cross origins, and the Windows sign-in hazard at the new address is stated (G437–450). The https→http refusal is quoted with its next step (G452–459).
- **A plain `http://` remote: completes.** It is carried by native under every setting, the warning is quoted and cannot be turned off, and native's limits apply (G15–28, G64–72). The NTLM-over-http hazard is stated (G409–410).

## 3. The drafter's three calls

1. **Register item 14 and its guide and Annex rows: correct, and required by TD13.** D5 creates a user-visible difference. The 1.0.17 column is honest ("the length of the wait on 1.0.17 was not timed", D307). The promise that "`--transport native` waits as 1.0.17 did" (G154, T6, D307) rests on libssh2's blocking SendMessage (OQ5, `agent.c`), not on a row. That is acceptable as stated.
2. **"Never direct on a guess": correct in the design, inexact in the user text.** D165 states the qualification precisely. The guide (G317–318, G497) and Annex S.3 and S.5 (K241, K263) shorten it to "the proxy is used or GWZ refuses" for a list that includes "this computer's own address as an HTTPS server". G321–323 and D162 send that case direct through the implicit bypass, which is measured only for http URLs. The same is true of an IP-literal origin that an explicit bypass entry matches. → **P3-A.**
3. **The stall error assigned to step 5.2: the wrong owner, and not carried.** The message lives in gwz-core (`transport_host/session/driver.rs:92`) and is not Windows-specific. Plan step 5.2 (gwz-cli switch removal) does not mention T7, while G81 promises the user "the error names `--ssh-timeout`". → **P3-C.**

## 4. Remaining findings (P3, none blocking, none architectural)

- **P3-A: the one-rule summary misstates the bypass case** (G317–318, G497, K241, K263).
  - **Correction:** "the request goes to the proxy, goes direct only where a bypass rule above matches (this computer's own address included), or GWZ refuses".
- **P3-B: CA-file precision** (D172, D304, G420–433).
  - On Windows, `endpoint_environment.rs:129,90-95,135` uses **one** file: `GIT_SSL_CAINFO` if it is non-empty, otherwise `SSL_CERT_FILE`. So "the files named by both" and "a file named by either variable is refused" overstate it: a bad `SSL_CERT_FILE` is ignored while `GIT_SSL_CAINFO` is set.
  - A file over 1 MiB fails with `endpoint CA file exceeds 1 MiB` (`:143-144`), not `cannot read endpoint CA file`.
  - **Correction:** state the precedence and both messages, or record a multi-file change as owed code.
- **P3-C: the owner of T7** (D379, P560–566).
  - **Correction:** put the help text in step 5.2's goal (it owns `transport_help.rs`). Assign the gwz-core message to a gwz-core step, such as 3.7, or record it as a cross-platform item. Raise this to the operator as structural.
- **P3-D: MAIN's unsupported-scheme message says Digest is answered on Windows.**
  - `https_failure.rs:136` says "on Windows NTLM, Negotiate and Digest too", which contradicts TD2 and D191. D192 calls it "the existing… message" with no change owed.
  - **Correction:** add a row to D356–379 ("drop Digest"), owned by step 4.4.
- **P3-E: K3 and K9 say the objects are uncommitted.** They are committed in `b6ef02af`.
- **P3-F: G188 limits the printed `known_hosts` path to one case.** D371 promises the path for every `known_hosts` message.

## 5. New P0–P2

None. The D5 rewrite, the one-rule text, register items 11 and 14, the owed texts T1–T7 and the plan edits for steps 3.3, 3.6, 4.4, 4.7 and 4.9 are coherent with each other. They also agree with amendment 2 revision 8 and its skim package (both untouched, hashes re-verified), the decision list (D1–D5), RX's results table and the accepted SSH clock amendment. No architectural root cause remains open.

Verdict: GO
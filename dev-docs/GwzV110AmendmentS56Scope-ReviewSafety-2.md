# Safety re-review, round 2: GWZ 1.1.0 amendment removing three claims from the S5.6 column (D41)

- **Object:** `gwz-core/dev-docs/GwzV110AmendmentS56Scope-DRAFT.md`, revision 1, at gwz-core `53dca0265b1056cf019fc73460b06468ab240717` in lane `/Volumes/projects/limbo/gwz-dev-amend41`. This is the lane's HEAD and the file's last commit.
- **Date:** 2026-10-18
- **Axis:** Safety (adversarial, read-only)
- **Requirement:** D41. Also checked against the 2026-10-18 operator ruling on Windows row 9.2 (lane `win92`) and D48.
- **Verdict: NO-GO.** P0: 0, P1: 0, P2: 1 (new), P3: 2 (new). All six round-1 findings are closed.
- **New architectural root cause:** none.

## §0 Evidence base

**Commit checks.** At the start and at the end, `git log -1` and `git log -1 -- <file>` both returned `53dca026`. The file has no working-tree change. Nothing was modified.

**Versions read:**
- Lane gwz-core `53dca026`. Its code is unchanged from `c8097a8c`, which the revision says it checked against.
- gwz-sspi in the lane, gwz-cli `6b4fd2e`, gwz-py (lane).

**Where D48 lives.** The lane's `GwzTransportTR18-OperatorDecisions.md` stops at D44. D48 exists only on main (gwz-core `e7bcf54e`, line 91). The revision was therefore written without D48.

**What I re-verified in the code:**
- The Kerberos path:
  - gwz-sspi `session.rs:29`, `:41-44`, `:51-53`, `:55-59`
  - `conversation.rs:137-139`, `:213-217`
  - `native.rs:263-271`, `native_code`
  - `exchange.rs:165-168`, `:215-219`
  - `https_failure.rs:313`, `:342-344`, `:785`
- The agent path:
  - `agent_keys.rs:24-33`
  - `agent_auth.rs:149-159`, `:199-212`
  - `control.rs:122-139`
  - `driver.rs:155`, `:478`
  - `gwz-cli/src/lib.rs:211-220`
- The tests: `key_types.rs` rows 100-135 and 150-165 (Unix) and 262-301 (Windows).
- gwz-py: `client.py:422-428` (`configure_transport_timeout`) and Migration §3.
- The old texts behind E22, E25 and the general `--transport native` advice exist as quoted (`Releases.md:135`, `:154`; guide line 335).

## Closure of round-1 findings

| # | Round-1 finding | Revised text | Re-verification | Status |
|---|---|---|---|---|
| S1 | "Kerberos not supported" reads as "Kerberos not used" | §3 item 2, E20, E23, E26, E10, E12a, E17 | The text now says Windows may choose Kerberos, gwz does not prevent it, asks for no delegation or mutual authentication, and makes no claim. It points to `--transport native`. This matches `session.rs:41-44`, `native.rs:263-271`, `conversation.rs:213-217` and `exchange.rs:165-168` | **Closed** |
| S2 | Kerberos failures fail closed with misleading or bare words, accepted unread | §5 bullet 2, §3 item 2, §6 item 3 | The words are stated with their code locations and filed as a finding, now ruled as D48. The new text has residual defects: N1 and N2 below | **Closed**; residuals as N1 and N2 |
| S3 | Notes items 1 and 3 not scoped by platform | §1 row 3, §3 items 1 and 3, E12c, E15, E25 | Item 1 now says macOS and Linux, with Windows pending on `win92`. Item 3 gives the Unix types including RSA, and Windows Ed25519 and ECDSA from a pipe agent. Both match `key_types.rs`. The gapsb README pointer is corrected in §4. A wording residual remains: N3 | **Closed**; residual as N3 |
| S4 | Note left out the declined-key fallback and the touch clock | §3 item 1, E23, E24 | The text states the skip to the next key, the `--ssh-timeout` stall and its words, and D38 as the mitigation (marked pending). This matches `agent_auth.rs:149-159`, `:199-212` and `control.rs:122-139` | **Closed** |
| S5 | Existing notes and guide text contradicted the change | E22, E23, E25, E26, E27 | `Releases.md:154` and guide lines 335-338, 583 and 612-613 are replaced, not added beside | **Closed** |
| S6 | Authority and review text predated D41 | Preamble, §5 bullet 4 | The text cites D41 and states the dual review, at most two rounds, with no skim | **Closed** |

## New findings

### N1 (P2): notes item 2 fixes in place a message that D48 replaces, and treats it as complete

**Root cause.** §3 item 2 (line 374, repeated in E23 at line 309 and by reference in E24) quotes today's code words as the release fact. Line 371 then forbids changing facts ("the lane owner may reword it, but not its facts"). There is no "pending D48" marker.

**What D48 changes.** D48 (main, `OperatorDecisions.md:91`) gives a provider-refused Negotiate sign-in its own words: the cause, Kerberos and NTLM policy, and `--transport native`. The "reinstall gwz" text stays only for a real worker mismatch.

**What the sentence also gets wrong.** It says a sign-in that fails after a token was sent is reported as the bare "HTTPS endpoint request failed: Protocol". That is true only of the `Protocol` code. A 401 rejection after a token was sent is "`<host>` rejected your Windows account (Negotiate)" (`https_failure.rs:236-249`). A redirect has its own words (`:320-325`).

**Violated invariant.** The notes state no fact that is invented or that will be stale at release. Concurrent work is marked pending, as the revision itself does for D38 and `win92`.

**Credible sequence.**
1. Lane `words2` lands D48.
2. The lane owner applies E23 verbatim, as line 371 requires.
3. The 1.1.0 notes tell a domain user that a refused Kerberos sign-in reads "Reinstall gwz". gwz no longer prints that.
4. The notes also imply that every failure after a token is the bare `Protocol` text.

**Impact.** The release text gives wrong diagnostic guidance, on the exact path that S2 and D48 exist to fix.

**Correction.**
- Rewrite the message sentence of item 2 and E23, and point E24 at it. Suggested text: "A sign-in that Windows refuses is reported with its own message naming Kerberos and NTLM policy and `--transport native` (D48, *pending its landing*). An exchange that ends without a verdict after a token was sent is reported as 'HTTPS endpoint request failed: Protocol', which does not name Kerberos."
- Update §5 bullet 2 and §6 item 3 to cite D48 and lane `words2`.
- Rebase the lane's decisions file, or cite D48 from main.

### N2 (P3): §5 bullet 2 under-counts how a first-step failure reaches the worker text

**Location.** §5 line 398: "The code shows two ways to reach it on the first step". It cites only `session.rs:52` (mechanism not allowed) and `:58` (token too large).

**What it leaves out.** The most direct Kerberos-failure path:
- `InitializeSecurityContextW` returns an error status (`session.rs:29`, `value => return Err(Error::provider(Some(value)))`).
- `AcquireCredentialsHandleW` fails (`conversation.rs:137-139`).

This is round-1 sequence (a): NTLM blocked by policy, with no SPN or no KDC reachable.

**Impact.** The evidence statement is incomplete and the user text is unaffected, so this is bounded. It matters for words2: D48's new words must cover every `ProviderRejected` arm, not only the two cited.

**Correction.** Change "two ways" to "at least four ways" and list `session.rs:29` and `conversation.rs:137-139`.

### N3 (P3): item 3's certificate sentences can still be read as covering every platform

**Location.** §3 item 3 (line 375), E23, E25.

**The RSA sentence.** "An RSA certificate did not authenticate in the tests … the test servers' default algorithm list refuses `ssh-rsa-cert-v01@openssh.com`" begins a new sentence with no platform. The Unix test servers have the same default list, yet RSA certificates authenticate there: the Unix client signs at `rsa-sha2-512` (`key_types.rs:100-135`, flag 4). The real cause is that libssh2 on Windows offers only `ssh-rsa-cert-v01`. A Linux reader can take the sentence as general.

**The security-key certificate types.** Item 3 says these "authenticate from an SSH agent" on macOS and Linux. That is through the software authenticator only. A physical key's certificate is exactly what item 1 calls unsupported.

**Correction.**
- Start the RSA sentence with "On Windows", and give the cause as "libssh2 on Windows offers only `ssh-rsa-cert-v01@openssh.com`, which the servers' default list refuses".
- After "the two security-key types", add "(through a software authenticator; see item 1)".

## Attacks on the new text that failed

**Kerberos credential exposure through the new wording.** "asks for neither delegation nor mutual authentication" is true (`conversation.rs:213-217`). No TGT is forwarded. The identity is always the logon session (`exchange.rs:168`). The text adds no promise the code does not keep.

**The `--transport native` pointer (E23, E26, E24).** It repeats the existing general advice (`Releases.md:135`) and D5's precedent (Pageant). It adds no new route and no credential path. It is an observation for the operator, not a finding, given the standing position on native routes as cover: D41 asks only for "named unsupported", so naming the fallback per feature is the lane owner's choice.

**The `--identity <certificate>.pub` sentence and D38/D39.** Before D38 lands, a `.pub` file given to `--identity` fails closed at the container check. The text marks D38 as pending. Exact-blob matching (D39 OQ8) is untouched (§5 bullet 3). Nothing is weakened.

**Python remedy (E24).** `configure_transport_timeout` exists (`client.py:422`) and sets the clock that bounds SSH setup, including the agent wait. Its "set before the first operation" constraint is in Migration §3, which E24 cites. `GWZ_TRANSPORT=native` exists (`cli_shared.py:19-24`).

**Hardware-key fallback wording.** "skipped and the next key in the agent is tried" matches `agent_auth.rs:149-159`. Pointing to exact selection is the right mitigation for the case where the next key belongs to a different account.

**Leaving Windows 9.2 in place.** E15 records `win92` as the owner. Item 1 marks Windows as pending. §4 does not count 9.2 as `U`. There is no false composition.

**The "agent on a pipe other than the OpenSSH service" distinction.** It cannot be detected at run time (TD12/OQ4(a) checks no pipe server), but it is a statement about evidence, not a promise of behaviour. It is acceptable under "no claim".

## Next action

This is round 2 of 2, so N1 must be closed without another architectural round. The drafter applies N1's correction to §3 item 2, E23, E24, §5 bullet 2 and §6 item 3, citing D48 as pending `words2`. The N2 and N3 one-line edits can go in the same patch. A skim then confirms the N1 text against D48. If N1 is closed as written, this axis is GO.

# Safety review: GWZ 1.1.0 amendment removing three claims from the S5.6 column (D41)

- **Object:** `gwz-core/dev-docs/GwzV110AmendmentS56Scope-DRAFT.md` at gwz-core `84e2570d4804d7452f5e2af70b25f0f2b1273988`. The file is unchanged at gwz-core HEAD `31d5183b`.
- **Date:** 2026-10-18
- **Axis:** Safety (adversarial, read-only)
- **Requirement:** operator decision D41 (`GwzTransportTR18-OperatorDecisions.md:84`)
- **Verdict: NO-GO.** P0: 0, P1: 0, P2: 3, P3: 3.

## §0 Evidence base

**Commit checks.** At the start and at the end, `git log -1 --format=%H -- dev-docs/GwzV110AmendmentS56Scope-DRAFT.md` returned `84e2570d`. The file has no working-tree change. Nothing was modified.

**Code read** (gwz-core at `31d5183b`, gwz-sspi at `75c1b03`, gwz-cli at `6b4fd2e`, gwz-core-evidence at `767d3297`):
- gwz-core endpoint SSH code:
  - `src/git/endpoint/ssh_tests/key_types.rs` (all)
  - `agent_keys.rs` (all)
  - `agent_auth.rs` (all)
  - `agent_job/control.rs:95-160`
  - `ssh_key_container.rs:200-290`
  - `ssh_limits.rs:26-152` (D17's words)
- gwz-core HTTPS sign-in code:
  - `https_worker/native.rs` (all)
  - `https_worker/native/exchange.rs:82-345`
  - `https_worker/native/sspi.rs`
  - `transport_host/request/https_failure.rs:220-347` and `:776-790`
  - `transport_host/session/driver.rs:130-170`
- gwz-sspi:
  - `src/values.rs`
  - `src/worker/session.rs:1-80`
  - `src/worker/windows/conversation.rs:100-279`
  - `src/worker/windows/observation.rs`
- gwz-transport: `codec/validate.rs:266`

**Documents read:**
- In `GwzTransportTR18-OperatorDecisions.md`: TD1, TD5, TD8, TD11, TD12, TD13, and D3, D5, D17, D30, D38, D39, D41.
- In `GwzTransportWindowsParityDesign.md`: §4 (lines 95-110), §7 and §8 (lines 195-215), §11 (lines 270-292), and §13 L1 to L20.
- `GwzV110S56CellTable.md`: §3, rows 9.1 to 9.7, and row 18.19.
- `GwzExactAgentIdentityDesign-DRAFT.md`: §3.3 (K1 to K4), §3.5, and rows C1 to C4.
- `GwzTransportWindowsUserGuide-DRAFT.md:578-616`
- `gwz-cli/docs/Releases.md:130-170`
- The README of run `2026-10-18-gapsb`.

**Not done:** no test was run. Every Kerberos sequence below is either a deterministic reading of the code or is marked as a hypothesis.

## Findings

### S1 (P2): the notes say "Kerberos is not supported", but the code uses Kerberos whenever Windows' Negotiate picks it

**Root cause.** §3 defines "unsupported" as "no claim is made". That definition (line 36) stays in the amendment and does not reach the user text. The user text then says Kerberos "is not supported" without saying that gwz will use it anyway.

**Location.** Amendment §3 item 2 (line 39); the definition is at line 36. The code that selects and accepts Kerberos:
- gwz-sspi `values.rs:86-87` ("Permit Kerberos or NTLM selection") and `values.rs:161-162`
- gwz-sspi `worker/session.rs:39-48`, which allows `Kerberos | Ntlm` for `Package::Negotiate`
- gwz-sspi `worker/windows/observation.rs:46-47`
- gwz-core `https_worker/native.rs:263-271`, which maps Kerberos to `NativeMechanism::Kerberos`
- gwz-transport `codec/validate.rs:266`
- gwz-core `exchange.rs:159-168`: Negotiate, target `HTTP/<host>`, `Identity::CurrentLogon`

**Violated invariant.** The notes state no invented fact in either direction. The prompt names this case: SSPI Negotiate does do Kerberos on a domain-joined machine. Design L1 (line 324) already says "the OS provider may work when domain-joined".

**Credible sequence.**
1. A user is on a domain-joined Windows host.
2. Their remote is an on-premises server with an `HTTP/<host>` SPN, such as Azure DevOps Server or Bitbucket Data Center. This is the population that used Kerberos through 1.0.17's WinHTTP.
3. Negotiate selects Kerberos, and gwz sends a service ticket.
4. An administrator who reads "Kerberos … is not supported" decides policy on the belief that gwz 1.1.0 never uses Kerberos. For example, they treat NTLM as the only protocol in play when they apply the design's OD16 mitigation, "restrict outgoing NTLM by Windows policy".
5. A user whose sign-in fails is told only "not supported". Nothing tells them to use the 1.0.17 path, though the general hint at `Releases.md:136` exists.

**Impact.** The user text is misleading about what gwz sends. Policy is decided on a false premise.

**Correction.** Reword item 2 roughly as follows: "On a domain-joined host, Windows may choose Kerberos inside Negotiate. gwz 1.1.0 does not prevent that, requests no delegation, and has tested only exchanges where NTLM was chosen, so no claim is made for Kerberos. If a Negotiate sign-in that worked with 1.0.17 fails, `--transport native` answers it as 1.0.17 did."

### S2 (P2): Kerberos-path failures fail closed, but with misleading or bare text, which the amendment accepts without reading it

**Root cause.** §5 bullet 2 (line 51) accepts "whatever the transport reports today" for a Kerberos failure without stating what that is.

**Location.** Amendment §5 line 51. The code:
- `exchange.rs:205-219`: a step error is returned with the facts still at `NotStarted` and `credential_offered: false`.
- `native.rs:333-345` (`native_code`): `ProviderRejected` becomes `Protocol`.
- `https_failure.rs:316-347` (`sign_in_words`): the `Protocol` case gives "GWZ's Windows sign-in worker did not answer as the one installed with gwz does: the file may have been replaced by another program… Reinstall gwz". This is pinned at `:777-779`.
- `exchange.rs:250-262` and `https_failure.rs:783-785`: once a token has been sent, the text is the bare "HTTPS endpoint request failed: Protocol".

**Violated invariant.** A fail-closed path must carry a diagnostic that does not point the user at the wrong cause.

**Credible sequences.**
- **(a) Credible, not run.** NTLM is blocked by domain policy (the design's own OD16 advice), and the host has no SPN or no KDC is reachable. Negotiate then cannot fall back, the first `InitializeSecurityContextW` fails, and session.rs returns `Error::provider` (`ProviderRejected`). The user reads that the worker may have been replaced by another program, and that they should reinstall gwz.
- **(b) Deterministic once the token is too large.** A user in many AD groups produces a token larger than the token limit, which is about 49 KB from the 64 KiB header budget (`exchange.rs:116-121`, `native.rs:95-101`). session.rs:52-56 returns `Error::provider(None)`, and the user gets the same "reinstall" text.
- **(c) Hypothesis: depends on SPNEGO negotiation state without mutual authentication.** The server answers 200 with no final `Negotiate` token while the context is incomplete. The result is `Protocol` and the bare "HTTPS endpoint request failed: Protocol", after the server has already accepted the user.

**Impact.** On the path most likely to be Kerberos, the diagnosis is wrong or empty.

**Correction.** §5 must name the words that are actually produced (the `https_failure.rs:342-344` text and the bare `Protocol` text) instead of "whatever". Either notes item 2 says that on a domain host those words can mean a Kerberos failure and points to `--transport native`, or the lane owner files the wording as a separate finding. The amendment itself does not need to change code.

### S3 (P2): notes items 1 and 3 state platform-specific test evidence as if it held on every platform

**Root cause.** §3 restates Unix rows and pre-gapsb Windows rows without naming a platform.

**Item 1 (line 38)** says 1.1.0 "qualifies" `sk-` keys "only through a software authenticator". On Windows, no security-key row exists:
- `key_types.rs:190-192` says security keys "stay with the Unix rows above (not qualified on Windows)".
- Cell 9.2 for Windows is still `G`.

Yet the transport does offer `sk-` agent keys on Windows: `agent_keys.rs:24-33` has no platform condition, and `agent_auth.rs:111-114` offers every listed type. The prompt or touch is then a Windows dialog that has never run.

There is also a gap between documents. The `2026-10-18-gapsb` README says that, for Windows security keys and DSA, "lane `gapsa`'s amendment covers it". Amendment §6 item 4 (line 60) says this document does not cover it. Windows cell 9.2 is therefore covered by neither.

**Item 3 (line 40)** has three problems:
- "an RSA certificate does not [authenticate]" is false on macOS and Linux. `key_types.rs:100-135` authenticates `RSA-CERT` at `rsa-sha2-512` (flag 4).
- It leaves out ECDSA on Windows. ECDSA P-256, P-384 and P-521 certificates authenticate there (`key_types.rs:262-270`, gapsb verdict 3.4).
- "an agent the test owns" is not a configuration a user has. The Windows service uses the same pipe client as the owned pipe (design §4: "The owned-pipe rows cover the same code path"), so a user cannot act on the distinction.

**Violated invariant.** The notes state no invented fact. No claim is widened or narrowed beyond what the evidence shows.

**Credible sequence.**
1. A Linux user with an RSA user certificate reads item 3.
2. They conclude it will not work, though it is tested and works.
3. A Windows user concludes that sk keys are qualified, though they have never run there.

**Correction.**
- Item 1: "On macOS and Linux, through a software authenticator; on Windows not qualified at all."
- Item 3: "On Windows, as in 1.0.17, an Ed25519 certificate from a pipe agent authenticates, ECDSA certificates do too (run `2026-10-18-gapsb`, with no 1.0.17 baseline), and an RSA certificate does not. On macOS and Linux, certificates of every listed type except DSA authenticate from an agent."
- Resolve which document owns Windows cell 9.2.

### S4 (P3): hardware-key note leaves out the executed fallback and the clock that the touch wait runs on

**Root cause.** Item 1 (line 38) drops TD8 B's support statement: "with the executed behaviours (skip a declining identity…) as the support statement".

**Code.**
- `agent_auth.rs:149-159`: when the agent declines, the next agent key is tried. This is pinned by `key_types.rs:284-301` and by `x4-pipe-security-key-then-rsa`.
- The touch wait sits inside `agent.sign` between `begin_wait` and `complete_wait` (`agent_auth.rs:199-212`). That is a Network wait (`control.rs:122-139`), so the default 9 s stall applies.

**Sequence.**
1. A hardware key for one GitHub account and a plain key for another are in the same agent.
2. The hardware key is unplugged, and the agent declines.
3. gwz authenticates silently with the other account's key.
4. Separately, a touch slower than `--ssh-timeout` fails the attempt with "no progress for 9 seconds". Each retry asks for a touch again.

**Why only P3.** This is parity with OpenSSH and with 1.0.17, and D38's exact selection mitigates it. The clock is the one D5 chose for the Pageant prompt and for `ssh-add -c`.

**Correction.** Add two sentences to item 1:
- A key the agent declines is skipped and the next key is used. Select the `.pub` file to prevent this (D38).
- The touch wait counts against `--ssh-timeout`, as Pageant confirmation does.

### S5 (P3): adding the new entries leaves existing contradictory notes text in place

**Location.** §2 row 8 (line 32) says "Add the three entries". `Releases.md:154` already says "Not promised: … certificates held by an agent", and `GwzTransportWindowsUserGuide-DRAFT.md:612` says the same. Neither is listed for change.

**Impact.** The notes would say both that certificates from an agent are not promised and that some are qualified.

**Correction.** Replace those sentences instead of adding beside them, and add the guide to §2.

### S6 (P3): the amendment's account of its own authority and review is stale against D41

**Location.**
- Line 5 and §6 item 1 (line 57) say the ruling has no D-number. D41 (`OperatorDecisions.md:84`, commit `accb1b04`) now exists.
- §5 bullet 4 (line 53) offers the lane owner "a skim". D41 says the amendment "takes its own Consistency and Safety review now".

**Why only P3.** The mandated review is in fact happening.

**Correction.** Cite D41, and state that the route is the dual review.

## Attacks that failed

**Kerberos credential forwarding.** `InitializeSecurityContextW` asks only for `ISC_REQ_CONNECTION | ISC_REQ_ALLOCATE_MEMORY` (`conversation.rs:213-217`). Without delegation no TGT is forwarded, and no mutual authentication is demanded. The identity is always `CurrentLogon` (`exchange.rs:168`). No helper credential goes into SSPI (D3, TD1).

**Kerberos/NTLM switch during an exchange, or success without completion.**
- `History::observe` refuses a change of mechanism or of authority during an exchange (`native.rs:263-280`).
- An NTLM offer cannot report Kerberos (`native.rs:244-256`).
- `accept` requires a complete, authoritative exchange (`native.rs:288-300`), so there is no false success.
- SSPI's own fallback before it selects a mechanism matches 1.0.17 (OD16).

**Failing open through an unintended key.**
- `KeyType::of` skips every unlisted type and DSA certificates (`agent_keys.rs:55-66`).
- Signatures are checked for shape, including the trailing 5 bytes of an `sk-` signature (`agent_keys.rs:96-137`).
- An RSA certificate on Windows fails with `PermissionDenied` and no sign request is sent (`key_types.rs:273-282`).

**Agent-held certificates (Pageant or the Windows service).** They take the same `KeyType` and signing path as the qualified pipe agent, and one agent source is used per session, with no fallthrough to another source (`pageant-empty-no-fallthrough`). Such a path either works or fails closed. Marking it "unsupported" adds no refusal (§5), and none is needed.

**Interaction with D38 (certificates matched by exact blob).** §5 line 52 leaves `exact_agent_identity` untouched. D38's design §3.5 already says that real security-key and Pageant-held-certificate signatures remain unqualified and that exact selection does not widen that claim. Rows C1 to C4 (a certificate does not match its plain key, and the reverse) are unaffected. Nothing is contradicted or weakened.

**D5 clock.** The touch or prompt wait is charged to the Network wait (above), which is the D5 ruling. It fails closed with text that names `--ssh-timeout`.

**D17 (passphrase-protected key files).**
- An encrypted key file is refused before libssh2 sees it (`ssh_key_container.rs:226-228`, `:272-276`), and no agent is tried after an explicit key.
- D17's remedy, "load into an agent" (`ssh_limits.rs:143-152`), steers Windows users with a `-cert.pub` file into a configuration the notes now call unsupported. That route works, or fails closed, so it is not a fail-open. It does sharpen S3's wording correction.

## Next action

The drafter revises §3 items 1 to 3 (S1, S3, S4) and §5 bullet 2 (S2). The lane owner decides whether the misleading texts in `https_failure.rs` become a separate finding, resolves who owns Windows cell 9.2 (S3), reconciles `Releases.md:154` and guide line 612 (S5), and cites D41 (S6). Then this reviewer re-reviews the revised text against the original sequences. That is round 1 of at most two.

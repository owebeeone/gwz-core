# TR1.8 freeze review (Safety)

**Tuple.** gwz-core `e5e8e804` and gwz-core-evidence `46b770f`. Both were verified at the start and again at the end. The object hashes match package §1: design `8d34c609196f3211`, guide `6752211da4bdc1e1`, amendment r8 `5d462c6902e7c26c`, decisions `61e787a87c9e5a26`. I modified nothing. I read MAIN source only to check what the text composes with.

## Findings

**S1 (P0, false composition; not architectural). The plain-HTTP boundary is unstated, and the text claims the reverse.**
- **Where:**
  - `GwzTransportWindowsParityDesign.md:159`: "The transport follows all of them … difference … for plain HTTP".
  - `:271`: register item 4.
  - `:312`.
  - Guide `:146`: "any HTTPS host".
  - Package Annex S row 4 (`:222`).
- **Violated clause:**
  - TD3 says no credentials cross origins.
  - OD16 says the hazard must be stated plainly.
  - Mandate item: Basic over plain HTTP.
- **What MAIN does:**
  - The transport serves `https` only (`transport_binding.rs:202`).
  - A redirect from HTTPS to HTTP is refused (`https_destination.rs:209`; test `redirect_rejects_…_downgrade`).
  - An `http://` remote falls through to libgit2/WinHTTP. That is the 1.0.17 behaviour RA measured: a helper's Basic credential in cleartext, and NTLM from the logon session with no channel binding. The WH1 qualification cfg instead refuses `http:` (`:228-233`).
- **Sequence:**
  1. An implementer follows §7 and register item 4, so the transport admits and follows plain-HTTP and HTTPS→HTTP redirects.
  2. The redirect target challenges Basic.
  3. §7 step 2 asks the helper "for U" and sends the credential over cleartext.
  - Alternatively the code ships as MAIN has it. The migration notes then promise that plain-HTTP redirects are followed. The guide limits the logon-credential warning to HTTPS while `http://` remotes still send NTLM and Basic in clear.
- **Impact:**
  - A false product statement in a credential path.
  - The safety property that bounds Basic and unbound NTLM (HTTPS only, no downgrade) is nowhere in the design.
- **Correction:**
  - State in §7 that the transport serves only `https://` and refuses a redirect from HTTPS to HTTP.
  - Decide what an `http://` remote does on Windows in 1.1.0: refused, as WH1 does, or the native route with 1.0.17's cleartext Basic and unbound NTLM. Register and warn either way.
  - Fix register item 4 and §14 item 7 to "relative `Location` over HTTPS".
  - Fix the guide `:146` and Annex S.

**S2 (P2; not architectural unless the operator changes behaviour). Windows CA-trust widening is unstated.**
- **Where:**
  - Design `:151` ("Certificate trust is validated…") and `:176`.
  - The register (`:262-277`) and §13 are silent.
  - Guide `:151`.
- **Fact:**
  - MAIN on Windows adds every certificate in `GIT_SSL_CAINFO` or `SSL_CERT_FILE` as a root beside Schannel's (`endpoint_environment.rs:128-147`; test `ssl_cert_file_is_a_ca_file_of_the_endpoint_beside_the_backends_own_roots`).
  - 1.0.17 on Windows ignores both (run `2026-10-08-ca-trust-parity`: Windows 4 of 4 cells differ, and TR2.7 is flagged as the operator's).
- **Sequence:**
  1. The environment names a file containing an interception CA.
  2. The transport accepts a MITM certificate that 1.0.17 refuses (RH/c00 shows 1.0.17 refusing at verification).
  3. The helper's Basic token, or the GitHub token, goes to the MITM.
  - Channel binding binds the NTLM response to the MITM's certificate, so it does not help here.
- **Impact:** a trust difference sits in the path TD1 routes helper credentials through, and it is invisible to TR1.8's reviewers and to users.
- **Correction:**
  - State the Windows trust sources in §7.
  - Add the difference to §12 and the guide.
  - Alternatively, ask the operator whether to drop both variables on Windows (the run names a one-line change).

**S3 (P2, architectural). An HTTPS origin through a named machine proxy is both admitted and refused.**
- **Where:**
  - `:122`: "this design takes that reading".
  - `:137`.
  - `:143`: "that class includes the selection of the `https=` entry and the CONNECT leg" (refused before any Open).
  - `:317`, `:320`.
- **Violated clause:** the GO rule (§11) says one design, no provisional clause.
- **Sequence, either reading:**
  - Admitted: an unmeasured selection ships. A mapping with only `http=` sends an HTTPS URL **direct**, by analogy with g05, which bypasses a proxy the administrator required.
  - Refused: every HTTPS fetch behind a corporate proxy fails. TR4.9's test ("an HTTPS fetch … tunnels through it", amendment `:224`) cannot pass, and parity with OD15 breaks.
- **Correction:** an operator or lane-owner decision is needed (R13 below).

**S4 (P2; not architectural). The Pageant confirmation wait has no SetupClock phase.**
- **Where:** `:110`; guide `:57-61` (OPEN ITEM).
- **Sequence:**
  1. A PuTTY `--encrypted` key or a confirmation dialog waits for the user.
  2. The text bounds the wait by the connection's "aggregate and stall bounds", that is, the Network phase.
  3. The stall bound expires in seconds.
  4. There is one send and no resend. The guide's "confirm promptly" cannot succeed against a confirm-per-use agent, so every retry fails.
- **Impact:**
  - A stuck state.
  - A regression from 1.0.17, whose libssh2 `SendMessage` waits.
- **Correction:** classify the wait as the clock amendment's `LocalInteraction`: network stall and aggregate paused, finite deadline at the amendment's interaction allowance. That allowance is not a new 120 s budget. Close the guide's OPEN ITEM.

**S5 (P3). TD12: the pipe-squatting hazard is not written as a limitation.**
- **Where:** `:85`, `:277`; §13 has no row; guide `:50-55`.
- **Fact:** the service's DACL withholds create-instance from Authenticated Users (`0x12019b` = generic read and write minus `0x4`). So a squatter can take `\\.\pipe\openssh-ssh-agent` only while the service is not running, which is the default (disabled on dabeest).
- **Sequence:**
  1. Any local account creates the pipe first.
  2. A user with no Pageant and no `SSH_AUTH_SOCK` connects to it.
  3. The squatter learns the user's identity (identification level), the sign-request metadata, and which keys are offered.
  4. It can make GWZ authenticate with keys of its choosing, or stall up to the clock.
  - It cannot obtain a signature from the user's keys.
- **Correction:**
  - Add a limitation row and a sentence in the guide.
  - Pin the admission grammar to exactly `\\.\pipe\<name>`. Any other server component, including `localhost`, an IP or `?`, is refused; "remote" is ambiguous.
  - Optionally, record for the operator that "server SID is LocalSystem or the caller" would admit both measured cases.

**S6 (P3). TD11: the barrier that makes "no promise" safe is not stated.**
- **Where:** `:83`, `:104`, L9.
- **The argument the text omits:** the per-request mapping DACL (caller SID and SYSTEM) is the cross-SID barrier. A Pageant under another SID cannot open the mapping, so it can neither read the request nor reply, and GWZ refuses without falling through.
- **Residual:** a lower-integrity receiver with the same SID. Same-desktop code under another SID can already inject input, since UIPI is based on integrity level.
- **Correction:**
  - State the property removed with P01 as a retained property.
  - Set the mapping's owner explicitly to the caller's user SID, not the token's default owner. An elevated gwz has Administrators as the default owner, and Pageant checks the owner, which would give a misleading "no key".

**S7 (P3). R3: the boundary of one exchange is underspecified.**
- **Where:** `:170`, `:182`.
- **Gap:** the text does not say what happens on a mid-exchange 401 that carries no continuation token for the selected scheme, or a 3xx. MAIN ends with Authentication (`native/exchange.rs`, after the 200 branch).
- **Correction:**
  - Say that such a 401 is rejection.
  - Say that eight rounds is per operation.
  - Say that no new exchange starts in the operation except TR2.1's retry on network loss before effect.

**S8 (P3). R8: redirect cap and tests are unnamed.**
- **Where:** `:151` ("redirects", plural), `:159`; amendment `:165` ("one validated discovery redirect").
- **What MAIN does:**
  - Caps at 5 hops and refuses any redirect after a native exchange (`https_worker/prepare.rs:501`).
  - Already tests cross-origin non-forwarding: `production_configured_basic_redirect_requeries_without_forwarding_credentials` and `gh_redirect_looks_up_each_origin_and_never_reuses_auth_header`.
- **Correction:** state the cap and reconcile it with the amendment's "one". Name these tests, plus a native redirect-after-exchange test, as the product proof.

**S9 (P3). R14: the "own address" rule is open.**
- **Where:** `:140`; guide `:120`.
- **Impact:** bounded. Traffic sent direct to this machine stays on the host.
- **Correction:** name the rule now:
  - an IP-literal URL host equal to an interface address enumerated once at capture;
  - never resolve names (b04);
  - otherwise send through the proxy.

**S10 (P3). Repeated logon rejection across members.**
- **Where:** `:170`.
- **Gap:** rejection is terminal per operation. A 32-member workspace can still produce up to N rejected NTLM attempts against one host. AD's rule that a match against one of the last two passwords is not counted as a bad attempt limits lockout from a stale credential, but other causes are not bounded.
- **Correction:** record the rejection per runtime for each (host, scheme), so later operations refuse without starting an exchange. Otherwise state it as a limitation.

**On the removed claims.** The GO-rule reading is acceptable to Safety, but two removals dropped safety properties silently:
- P01's removal hid the DACL barrier (S6).
- P03's removal hid the squatting hazard (S5).

S8 covers item 12's rule, which is not measured on 1.0.17 but already tested in MAIN.

**OD16.** The guide `:146-150` states it plainly: crack or relay. The defect is its "HTTPS only" scope (S1). The design `:174` should use the amendment's terms: a NetNTLMv2 response, crackable offline, and relayable to services that do not enforce EPA or signing. It could also mention that administrators can restrict outgoing NTLM by Windows policy.

## R answers (Safety)

- **R3.** The boundary is right. Abuse is bounded: one Type 3 per exchange and eight rounds. The text needs the S7 additions.
- **R4.** Keep fail-closed. Never fall through to Basic. MAIN already refuses (`UnsupportedOperation`). The message should name the cause.
- **R8.**
  - The reading is right: the target's own challenge falls under OD16, and nothing is forwarded from the first origin.
  - The discovery validation that requires `/info/refs` plus the exact service query limits abuse of open redirects.
  - Cap: name it (MAIN uses 5).
  - The rule being unmeasured on 1.0.17 is acceptable at GO only if the existing MAIN tests are named (S8).
- **R10.** Classify the wait as `LocalInteraction` (S4). As written it fails early, and can be stuck against a confirm-per-use agent.
- **R12.**
  - (a) and (b): fail-closed, acceptable.
  - (c): acceptable.
  - (d): accepting `<-loopback>` and ignoring it is acceptable, because the traffic it sends direct stays on the host. Keep the guide's sentence.
- **R13.** Not enough as written, because it contradicts itself (S3). Credentials are safe by construction on the CONNECT leg: no credential on CONNECT, every 407 refused, and the binding taken from the origin handshake. The hazard is only proxy selection. Safety's preference:
  - Admit a bare entry or an explicit `https=` entry for an HTTPS URL.
  - Refuse a mapping with no `https=` entry instead of going direct, until step 4.6 measures it.
  - That needs the operator's approval, since it differs from 1.0.17.
- **R14.** Name the rule now (S9). P3.
- **R15.** "E (loopback)" is honest. The `0x80090346` controls show the binding is checked. L7 stays a limitation, not a condition of GO, provided the guide keeps "tested only against this computer".
- **Checklist 1 (TD12).** Acceptable as parity with 1.0.17, with S5 stated. The identification-level connect prevents impersonation, and no key material is reachable.
- **Checklist 2 (TD11).** Acceptable as B, because the mapping DACL rather than a refusal is the barrier. That must be stated (S6).

Verdict: NO-GO
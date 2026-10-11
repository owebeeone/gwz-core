# TR1.8 freeze review, round 2 (Consistency)

Two of my round-1 findings are still open, and the remediation introduced one new P2, which is architectural. Verdict: NO-GO.

**Tuple, checked at start and end:** gwz-core `17ec75f1`, gwz-core-evidence `928f6d4`, gwz-transport `6146bd58`, gwz-cli `21d62310`, gwz-py `5950ba38`, gwz-sspi `73de8c8e`.

- **Root:** HEAD is `f16d9fb0`, not `b3e6de03`. It is a gwz merge on top of `b3e6de03` that changes only the lock entries and markers under `gwz.conf/` (the dirty flags of `17ec75f1` and `928f6d4` go from true to false). `b3e6de03` is an ancestor, and that commit carries the D3 annotation in `GwzSspiDesign.md`.
- **Untracked, out of scope:** the listed files, plus the round-1 and round-2 review reports, which appeared while I worked.
- **Pinned hashes:** all ten SHA-256 values in section 1 of the review package match.

## Closure table

| ID | Disposition claimed | Status | Evidence |
|---|---|---|---|
| P2-1 | D1: measure, then admit; remove "refused" at the old `:143` and `:320` | **Partially closed (P2 remains)** | `:150-153` and `:160` are fixed, and RX supports them. But `:373` still reads "the https-origin proxy selection and the CONNECT leg are listed above (11) and refused before any open (OQ11)". That is the original counterexample, left in place, and it contradicts D1 (decision list `:36`), `:150-153` and §14 item 11 (`:347`). Also, `:160` says the forms RX did not run "are refused or taken by WinHTTP's rule as stated there", yet `:156` states neither. L8 (`:318`) says "a form without an executed row is refused", which would refuse an IP-literal https origin and a bare entry with no port. `:136` and guide `:222-224` admit a bare host with no port, and the guide refuses no IP-literal origin. |
| P2-2 | Cite the HTTPS §5 and TR1.6 C2 bound; D2; reconcile the amendment | Closed | Design `:168`, `:180`, register item 4 (`:292`); amendment `:165`, `:470`, `:179`, `:227`; amendment §3.19 (`:458`) already routes `http://` to libgit2. |
| P2-3 | D3: keep the code, revise plan steps 4.4, 4.8 and 6.2, annotate the SSPI design | Closed | Design `:201`, `:219`, `:341`; plan `:482`, `:484`, `:514-518`, `:616`; `GwzSspiDesign.md` annotation (`b3e6de03`). No helper-identity row and no Digest row remain in the plan. |
| P2-4 | Narrow "measured"; add RX's environment rows | Closed | `:124-128` matches RX he01–he12: `second_proxy_requests` is empty in he07 and he12. NO_PROXY and no_proxy (he05) match the runner at `:350`. Nit: L8 (`:318`) and §14 item 10 drop `http_proxy` and `HTTP_PROXY` against an https origin, which `:128` lists. |
| P3-1 | `:18` section list | Closed | `:19` |
| P3-2 | Where RN landed; package tuple restated | **Partially closed (P3)** | `:21` gets `12b46d9` and `e07f516` right, but still says RX "is uncommitted at writing"; it is in `928f6d4`. Package `:3`, `:9`, `:24` and `:26` still describe the uncommitted lane `gwz-dev-tr18fix` (root `abf76ab7`, core `3b8e0d70`) rather than the settled tuple. |
| P3-3 | Restate the byte figures | Closed | `:215` and `:317` match RH: `auth_header_bytes` is 88 or 103 for NTLM, 119 or 121 for Negotiate, and 88 for Negotiate by numeric address. |
| P3-4 | The four-keys row described correctly | Closed | `:94`; RN README `:18`. |
| P3-5 | P01 marked P | Closed | `:269` |
| P3-6 | The "HTTPS confirmation owed" text | Closed | Amendment `:167`; skim package `:7`, `:125`, `:136`. The skim package's quoted superseded and replacement texts still match revision 7 and the current amendment verbatim. |
| P3-7 | Register items added | Closed | Register items 9–13 (`:297-301`). |
| P3-8 | §14 item 10 reconciled | Closed | `:345` (see the P2-4 nit). |
| P3-9 | TD12 in-session approval recorded | Closed | Decision list `:28` |
| P3-10 | TD12 supersedes OQ4(b) | Closed | `:87` |
| P3-11 | Name step 3.1 for TD6's Unix review | Closed in the plan; nit in the design | Plan step 3.1 (`:368`) owns it, but design `:64` and L14 (`:324`) still say "the implementation steps" without naming 3.1. |
| P3-12 | The encrypted RSA PEM remedy | Closed | `:72` |
| P3-13 | Guide's retired SSPI-only pointer and "120-second" figure | Closed | Neither string is left in the guide, and no OPEN ITEM placeholder remains. |
| P3-14 | Define "origin" | Closed | `:180` |

## New defects from the remediation

**N1, P2, architectural: the R10 bound rests on a clock extension the accepted amendment forbids.**
- **What the design now says:** `:114`, L20 (`:330`) and guide `:119-123` put the Pageant confirmation wait in the SSH clock amendment's LocalInteraction phase. In that phase the network stall and aggregate clocks pause, and the wait is bounded by the amendment's interaction allowance.
- **What the accepted amendment says:** "Only the accepted ambient password-only route may enter local helper phases" (`GwzTransportSshHelperClockAmendment.md`, section on authentication transitions). Design §1 (`:19`) says that amendment controls its domain and that "this document cannot reverse" it.
- **How the design handles the gap:** it admits the conflict ("is not accepted text", owed to step 3.6) but keeps the clause. That makes it a provisional clause, which §11's GO rule forbids. The guide already states the behaviour to users as fact.
- **Fix, either way:**
  - Amend the SSH clock amendment so Pageant is an admitted route, under its own review, before GO.
  - Or state the Network-phase reading the accepted text gives, record in §13 that the stall bound can expire before the user confirms, and correct the guide.

**N2, P3: §8 still says the eight-round bound is per exchange.**
- `:203` reads "bound the exchange to eight request rounds".
- `:191` and plan 4.8 (`:514`, `:516`) say the eight rounds are "per operation, not per exchange".
- Fix: change `:203` to say per operation.

**N3, P3: plan step 4.7's goal still names the off switch.**
- Plan `:506` says a 407 is refused "naming the setting and the off switch (B10)".
- Design `:162`, register item 13 (`:301`) and 4.7's own tests at `:508` say the message has no native pointer.

**N4, P3: the design still quotes the old README count.**
- `:213` says "the README's text says five".
- The corrected RH README (`928f6d4`, `:23`) now says 4 rows.

**N5, P3: register item 11 leaves out the CA-file refusal.**
- `:299` and guide `:360` record only that roots are added.
- The cited run, `2026-10-08-ca-trust-parity` (README `:78-79`), also shows the transport **refusing** an empty, garbage, malformed or `TRUSTED CERTIFICATE`-format file named by `GIT_SSL_CAINFO` or `SSL_CERT_FILE`, before connecting.
- 1.0.17 ignores those variables and clones anyway, so a user whose stale variable worked on 1.0.17 will now fail.
- Fix: add the refusal to register item 11 and to the guide.

## Other checks

- RX's results table matches every RX citation in `:126-127`, `:150-155` and `:162`, plus register item 9.
- D2 agrees with amendment §3.19 (`:458`).
- The D4 citation "TR2.7 (amendment 1 §3.5)" resolves to `GwzTransportReleasePlanAmendment.md:206-210`.

## Verdict

**Open P0–P2:**
- P2-1 is only partially closed. Its original counterexample survives at `:373`, and L8 contradicts `:160` and the guide on whether unmeasured https forms are refused.
- N1 is new and architectural: the Pageant clock bound contradicts the accepted SSH clock amendment.

This is round 2 of 2 under the cap. N1 has an architectural root cause: a mechanism that crosses a contract and needs either an amendment or a revert.

Verdict: NO-GO
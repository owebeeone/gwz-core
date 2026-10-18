# GWZ transport credential helpers design (TR1.6) — SURFACE-AXIS REVIEW, round 3

**Review object:** `/private/tmp/claude-501/-Users-owebeeone-limbo-gwz-dev/351b18f9-4ec1-4306-ac0a-299e9bded6dd/scratchpad/designs/GwzTransportCredentialHelpersDesign.md`, revision 3, SHA-256 `760f7ad4911bba53d9b7fa019be5225084236f1ab3cfd1a225cc8fceeeaeb025`, **§11 "Surface" only** (file lines 278–384), read 2026-10-02. Hash verified 17:32 and 17:34 AEST; unchanged.
**Baseline:** installed `gwz 1.0.17`; gwz-cli docs at `236f753` (= HEAD, no drift since round 2); gwz-core docs at `ab48966f` (checkout `ee06d16`, `docs/` unchanged since the pin); `gwz-py/README.md` at `2e0509f` (HEAD `43a07a2`, README unchanged); remediation plan 2 (`reviews/GwzTransportCredentialHelpersDesign-RemPlan-2.md`, F1–F7); my round-1 and round-2 reports. Other axes' reports and the design diffs were not opened.
**Date:** 2026-10-02
**Axis:** SURFACE — a focused confirmation: P3-11 to P3-14 against revision 3's §11, the round-2 GO re-checked under F1's codes, and what C10 means for the messages as a user meets them. Independent, adversarial, read-only. Filed verbatim by the lane owner.

**Verdict: GO** — 0 P0, 0 P1, 0 P2 open, 1 new P3 (P3-15, conditional on OQ6 (b)). P3-11 to P3-14 are confirmed closed. The round-2 GO holds under F1's codes with the same single dependency: it assumes **OQ5 (a)**; under OQ5 (b) P2-3 reopens as a P2 and the verdict on this axis is NO-GO. Resolving C10 by merging texts does not change the verdict; it reopens two round-1 P3s (recorded below) that the operator should see before choosing.

---

## Closure table

| Finding | Status | Evidence in §11 revision 3 |
|---|---|---|
| **P3-11** nothing said where the member's URL is | **Confirmed closed** | The `<url>` clause follows "this member's URL" in M2–M5 and M8–M10: "(`git -C <path> remote get-url origin` prints it; before the member is cloned, use the manifest's URL, or under `--url-scheme https` the one `gwz --verbose materialize --lock` prints after `->`)", with `<path>` taken from the member prefix. The Troubleshooting section states the same. The `--url-scheme https` walkthrough now reaches the `https://` URL from the text alone. |
| **P3-12** M3 swallowed by clone's private-member skip | **Confirmed closed** | Table row M3: clone "Fails, `git_command_failed`"; N14: "A rejected credential (HTTP 401) fails the repository on clone with `GitCommandFailed`, as in gwz 1.0". The expired-token clone walkthrough now shows M3. The quiet skips that remain (M2, M6, M7, M8, M9, M11) get a `--verbose` line "its path and the reason" (R-D), so even 1.0.17's documented silence is traceable. |
| **P3-13** M5/M6 stretched `unsupported_operation` | **Confirmed closed** | No message carries it: M6 and M8 are `remote_rejected` on fetch and push; M5 is `remote_rejected` / `git_command_failed`. "Pages that change": "No message carries `unsupported_operation`, so row `:22` and its 'not built yet' sentence stand." Verified against ErrorCatalog `ab48966f` rows 22, 30, 31 and line 94. |
| **P3-14** N3 named only `gitdir:` includes | **Confirmed closed** | N3: "under any `includeIf`, whether `gitdir:` or `hasconfig:remote.*.url:`"; M2 matches word for word. |
| **P2-3** (round 1) codes | **Closed under OQ5 (a); reopens under (b)** | See "The round-2 GO under F1" below. |

## 0. Evidence base

- **Object.** Headings listed to locate §11 (278–384); only those lines read, plus remediation plan 2 whole, as permitted.
- **Docs.** `git diff --stat` against the round-2 pins is empty for gwz-cli `docs/` and `README.md`, gwz-core `docs/`, and gwz-py `README.md`; every line §11 cites was verified in round 2 and is unchanged (ErrorCatalog rows 22/30/31/32 and line 94; MachineOutput 72–80 and 201–215; clone.md 63–71; MessageCatalog 694–702; TransportPlacement 103–109; GitBackend 23–29).
- **git, as a user knows it.** Round 1's `git credential fill` probe (exit 128, "could not read Username", with no helper and no terminal) and git's documented reauth (a rejected helper credential is handed to `erase`; the next run asks afresh) remain the basis for M2's definition and M3's instruction. git has no 16 KiB bound on a helper's answer, accepts non-UTF-8 bytes and a missing final newline, and sends a value containing `\r` as-is — relevant to C10 below.
- **No network operation, no file written, no build.**

## 1. Findings

### The round-2 GO under F1

F1 fixes every code to 1.0.17's where 1.0.17 had the case, and keeps OQ5 for M4 and M10 only. Re-checked against the table:

- **Fetch and push.** Every outcome but M1 and M4/M10 is `remote_rejected`, as 1.0.17's are, so no script keyed on `RemoteRejected` for an HTTPS failure changes behaviour. The documented `meta.transport` facts separate the recoveries that matter to a driver: M2 (`helper`, false, null) "store or sign in"; M3 (`helper`, true, false) "renew"; M9 (`helper`, true, null) "fix access". The pairs that share code and facts — M5/M9, M6/M7/M11, M2/M8 — share a recovery class ("a human acts on the remote, account or helper") or are confined to one caller (M7, embedding only; M11, the push/fetch request). Acceptable.
- **Clone.** Codes follow 1.0.17's `clone_error`: M3 and M5 fail with `git_command_failed`, the rest skip quietly for `private: true` and fail with `remote_rejected` otherwise. The verb-dependent code for a rejected credential (`RemoteRejected` on fetch/push, `GitCommandFailed` on clone) is 1.0.17's own inconsistency, preserved by the operator's parity rule and stated in N14 and the catalog's row 31. A driver must key on both; the text says so. Not graded.
- **OQ5.** Under (a), M4/M10 carry `CredentialHelperTimeout` on every verb, and P2-3 is closed in full. Under (b), they carry `remote_rejected` on fetch and push with facts (`helper`, false, null) — identical to M2 and M8 — while their recovery ("answer or finish the waiting helper, then retry") differs from M2's; a later allocation would change their code, the compatibility break P2-3 named. §11's own "Why one allocation" paragraph argues for (a). **The GO assumes (a).**

### [P3-15] Under OQ6 (b), M4 prints a remainder and calls it "a fixed bound"

- **Location.** M4: "No credential helper answered within <n> seconds, a fixed bound, so gwz gave up on it." `<n>` is "120, or the Open's interaction deadline when shorter, and under OQ6 (b) what the slot wait left of it." Compare M10, which already has the right shape: "in the <m> seconds left of gwz's fixed 30-second wait for resources".
- **Violated expectation.** A number called "a fixed bound" must be the bound. Under OQ6 (b) the slot wait consumes part of the 120 s, so a user sees "within 37 seconds, a fixed bound" on one run and "within 120 seconds, a fixed bound" on another, with no way to tell that eight busy helpers ate the difference — the one fact that explains it.
- **Scenario.** Nine private members on one host under OQ3 (a) with GCM: eight sign-in windows open, the ninth waits for a slot, then its helper gets the remainder; its M4 names a smaller "fixed bound" than the N6 text promised.
- **Required correction.** Under OQ6 (b), M4 reads "within the <n> seconds left of gwz's fixed 120-second allowance" (M10's form); under OQ6 (a) the current text stands. The N6 option text already distinguishes the two.
- **Check.** Under OQ6 (b), M4's text names the allowance and the remainder separately.

### C10 resolved by merging texts: what the user meets

C10 says the wire carries no failure detail, so M8's cause, M2's and M8's distinct texts, and M6's scheme names may not reach the member error. If the operator resolves it by merging rather than by a detail field:

1. **M8's eight outcomes print M2.** For the common cases nothing changes. For M8's own causes the text is wrong in a specific way: it says "no credential helper gave one" when a helper did answer and gwz refused the answer. M2's next step, `git ls-remote` with the URL, then **succeeds** — git has no 16 KiB bound, accepts non-UTF-8 bytes and a missing final newline, and sends a `\r`-tainted value (the realistic case: a CRLF-edited CI secret or helper script) which the server rejects as a wrong token, erasing and re-prompting a storing helper. So a storing-helper user's stale stored value self-heals by accident, while a CI token-helper user sees "git works, gwz says no credential" and nothing further: a dead end that reopens **P3-7** (round 1) as a P3. The `--verbose` skip line already merges them ("no usable credential" for M2 and M8), so clone stays consistent. The bounded mitigation without a wire change: a Troubleshooting subsection "git succeeds, gwz reports no credential" listing the eight causes and a check that inspects the helper's answer for `\r`, size and encoding without printing the secret, and M2's page pointer carrying the reader there.
2. **M6 names no scheme.** "A scheme gwz does not answer here" without the name leaves the user unable to tell the administrator what the server offers, or to pick git's matching mechanism (Kerberos for Negotiate, NTLM, a Bearer-only front). `git ls-remote` does not print the challenge either. That reopens the scheme-naming half of **P3-3** as a P3; the bounded mitigation is a page check that shows the `WWW-Authenticate` header (for example `curl -sI` on `<url>/info/refs?service=git-upload-pack`).
3. **M9 without the reason phrase.** No loss: HTTP/2 has no reason phrase, and M9 already sends the user to `git ls-remote` for the server's explanation. F4's drop is the right call regardless of C10.

Neither reopened item is a P2: the affected users are uncommon, each has a text-only mitigation, and no code or field changes. The verdict is GO under either C10 resolution; the detail field is the better one for the user, and the two mitigations above are the price of the other.

## 2. First-day walkthrough (revision 3)

Unchanged from round 2 except where noted; every case now reaches a command without a guess.

1. **`gh auth setup-git` user, private fetch.** Succeeds. No guess.
2. **Same user, expired token.** Fetch: M3 → `gh auth refresh`. Clone/materialize: M3 fails loudly with `GitCommandFailed` and the same text (P3-12 closed). No guess.
3. **GCM user needing a sign-in.** Per OQ3's chosen N5 text; M4 or M10 carry the `git ls-remote` step and, with F5, where the URL is. Under OQ6 (b), M4's number needs P3-15's wording. Remaining guess: which OQ3/OQ6 texts ship (the operator's).
4. **CI, no helper.** M2 ("With no helper, set one up first"), `RemoteRejected`, facts (`helper`, false, null); the page promises the CI token-helper recipe; the token-in-URL case follows the OQ1 text. Under C10-merged, a CI helper emitting `\r` lands on M2 with a passing `git ls-remote` (item 1 above). Remaining guess: which N12 text ships.
5. **No `git`.** M1 with the hint; `ExternalToolMissing`; public repositories need no git (N1). No guess, provided `--transport native` lands with TR1.5.

**gwz-py.** As round 2: `GwzOperationError.member_errors` on partial/failed/rejected; codes per the table, verb-dependent on clone for M3/M5 as N14 states; the hint reads `GWZ_TRANSPORT=native`.

## 3. Risks and next action

- **Operator decisions that touch this axis.** OQ5: choose (a); (b) reopens P2-3. OQ6: (b) needs P3-15's M4 wording. C10: a detail field keeps M8 and M6 as written; merged texts reopen P3-7 and half of P3-3 as P3s with the mitigations above.
- **Skip line and URL.** The `--verbose` skip line prints the path and reason, not the URL; MachineOutput promises the `manifest-url -> effective-url` line for "each converted member", and clone.md says a skipped member produces "no member progress, response row or transport diagnostic". §11 should confirm the `->` line prints for a member that is then skipped, or the skip line should carry the effective URL; otherwise a skipped private member under `--url-scheme https` has no visible HTTPS URL to feed `git ls-remote`. A sentence, not a finding.
- **Nits, not graded.** M2 with its `<url>` clause runs past a hundred words per failed member. M6's "and nor did gwz 1.0" reads awkwardly before the parenthesis. `Install.md:48-49` still anchors the runtime `git` requirement to the source-build paragraph.
- **Next action.** Record OQ5 (a) and the C10 choice with the operator; apply P3-15 only if OQ6 (b) is chosen; no further Surface round is needed for the P3s.

Object SHA-256 at end of review: `760f7ad4911bba53d9b7fa019be5225084236f1ab3cfd1a225cc8fceeeaeb025` (unchanged).

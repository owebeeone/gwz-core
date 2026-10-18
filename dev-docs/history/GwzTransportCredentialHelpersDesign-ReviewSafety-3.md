# GWZ transport credential helpers design (TR1.6) — SAFETY-AXIS REVIEW, ROUND 3

**Review object:** `reviews/GwzTransportCredentialHelpersDesign-rev3.md`, revision 3 (DRAFT, 2026-10-02, 420 lines), SHA-256 `760f7ad4911bba53d9b7fa019be5225084236f1ab3cfd1a225cc8fceeeaeb025`; read with the exact rev2→rev3 diff (389 lines, 8 hunks) and remediation plan 2 (F1–F7). Revision 2 (`db8009e8…`) was my round-2 object.
**Baseline:** unchanged from round 2: code at gwz-core `9d4dd92f`; user documents at gwz-core `ab48966f`, gwz-cli `236f753`, gwz-py `2e0509f`; libgit2 `b172e3d1`; git2-rs `d13951f7`; gwz-transport `6910ba66`. The object hashed `760f7ad4…` at the start and the end of the round, and every cited SHA resolves (`cat-file -t` → `commit`). Read-only; the peer reports were not opened.
**Date:** 2026-10-02
**Axis:** Safety. Short confirmation round, as asked: P3-6's closure, F1's wider quiet skips, and the other changes for new hazards.

**Verdict: GO** — P0 0, P1 0, P2 0, P3 1 (new, non-blocking). P3-6 closed. No new architectural root cause.

---

## 0. Closure

| Finding | Counterexample re-checked | Revision 3 text | Status |
|---|---|---|---|
| **P3-6** | A 403 whose reason phrase is an instruction; a `WWW-Authenticate` token `Visit-evil.example-now` | M9 (diff line 288) shows no reason phrase, and says why. M6 (line 280) carries the names only as ` (the schemes the server named: "<token>", "<token>")`, at most four, cut at 32, and an HTTP token cannot hold `"` (RFC 9110 tchar), so the quoting cannot be broken; nothing without C10's detail. The messages' rule (line 266) names that one attributed fragment as its only exception. The fixture gains both shapes (line 185); T8(b) asserts neither phrase nor body, T10(b) asserts each token only inside its quotes, cut at 32 | **Closed** |
| R-A, R-B, R-E (F7) | — | OQ1 (b) names the Open field (`Destination.ssh_username`, percent-encoded, re-validated at the endpoint with §2.2's decoded check); §3.3 "Completion" says stderr's read end closes at completion, a later writer gets a broken pipe, and nothing kills the child on success (T16 asserts it); §2.2 places D18's check in `validate_url_shape`, which `parse` and `redirect` both reach | Applied |

## 1. F1's wider quiet skips

**No credential problem is concealed without trace.** On clone, quiet for a `private: true` member: M2, M6, M7, M8, M9, M11; loud: M1, M3, M4, M5, M10. The two outcomes in which a secret was actually sent and something went wrong stay loud or traced: M3 (the stored credential is wrong) fails the member with `git_command_failed`, as 1.0.17's unrecognized replay-limit error did; M5 (a redirect after the credential) fails too. M9 (the credential was accepted and the account refused) is quiet, exactly as 1.0.17's recognized "unexpected http status code: 403" was (`transport.rs:742-751`), and is traced. The quiet classes are otherwise "nothing was sent": no credential (M2), no answerable scheme (M6), helpers off (M7), an unusable answer (M8), a POST challenged after an anonymous discovery (M11). The skip line under `--verbose` (line 261) gives the member's path and a reason class, never a URL, host or helper output; without `--verbose` the skip is as quiet as `clone.md:65-70` documents, and gwz-py and JSON are unchanged, which is the existing contract.

**Deviation 1** (a `Negotiate`-offering challenge, or `NTLM`-only with a helper able to answer, on macOS and Linux; M8's control-character and colon causes): in each, 1.0.17 failed the member on clone and the transport skips it quietly. None was a setup that worked on 1.0.17, fetch and push codes are 1.0.17's, and §11 lists each with its evidence and TR1.8's row for the Negotiate case. The only visibility lost is a loud "'Negotiate' authentication is not supported" on clone, replaced by the `--verbose` line "challenge gwz cannot answer". That is a bounded, stated loss, not a concealed credential problem. **Deviation 3** (a pipe failure crosses as `Authentication`) keeps `Io` for network loss alone, so HTTPS:332-334's rule that loss is never suppressed holds.

## 2. New findings

### [P3-7] A `git` that is present but cannot start takes M8's quiet clone skip, though its remedy is M1's

- **Root cause.** §4's "Unusable" row (line 66) puts "`git` was found but could not start" under M8, which F1 now makes `remote_rejected` and a quiet skip on clone (lines 247, 254). F1's parity rule decides M8 by 1.0.17's drop of an unusable *answer*; a spawn failure of `git` has no 1.0.17 case (1.0.17 ran no `git`), so, like M1, the catalog's rule should decide it, and its remedy is M1's: fix the git installation, not store a credential.
- **Reproduction.** `git` on the snapshot's `PATH` is a non-executable file, or a binary the host cannot exec (an architecture mismatch after a failed upgrade): the spawn fails with a kind other than `NotFound` (today's `https_auth.rs:303-309` maps only `NotFound` to the missing-executable class). `gwz clone` of a workspace with `private: true` members skips every one of them quietly, the `--verbose` line reading "no usable credential"; `gwz fetch` reports M8 "git could not be started (…)" with C10's detail, or M2's "no credential helper gave one" without it, and `git ls-remote` then succeeds with the user's shell `git`, which contradicts the message.
- **Impact.** A broken git installation hidden on clone and misdescribed on fetch; bounded, rare, no exposure.
- **Required correction.** Give the spawn-failure cause M1's class and loud clone treatment (`external_tool_missing`, with its catalog row widened to "was not found or could not be started"), keeping M8 for output problems; one line each in §4, §11's table and the skip-line reasons.
- **Closure test.** T17 gains a non-executable `git`: `external_tool_missing` on fetch, a failed member on clone, no spawn of any helper.

## 3. Notes, no finding

- **R-F (C10).** The detail-field option should restate §5's rule for the field: a cause from M8's fixed set and M6's bounded tokens, never helper output or stderr. The shared-text option's cost is stated (M8 prints M2; M6 names no scheme) and is diagnosability only.
- **R-G (OQ6 (b) with OQ3 (c)).** Under (b) a helper started with the remainder of a consumed allowance that then times out is M4, and under OQ3 (c) an M4 latches the host, though it may mean a starved helper, not a hung one. One clause in OQ6 (b)'s hazard; (a) is recommended and has no such case.
- **R-H.** The `--verbose` skip line serves the CLI only; gwz-py and `--json` keep the documented silence for quiet skips. If the operator wants a trace there, a response-level list of skipped private members (path and reason class, no URL) would serve both without changing the quiet contract.
- **Windows POST replay** (§2.1, new): 1.0.17's WinHTTP replays a challenged POST without its body; the transport's M11 on every platform means no replay and no second publication attempt, which is the safer side. TR1.8 records the 1.0.17 case.

**Next action.** File; P3-7 can be folded into the GO revision's application or TR2.22's text. The operator answers OQ1–OQ6, and the lane owner C10.

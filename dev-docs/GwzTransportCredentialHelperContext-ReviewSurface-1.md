# GWZ Transport Credential Helper Context — SURFACE-AXIS REVIEW

**Review object:** Committed caller Surface note, `gwz-core/dev-docs/GwzTransportCredentialHelpersSurface.md`, at core `ec1b95831582651953bb0bd3be3c4f88985aba6b`. Remediation round 1 of the helper context amendment draft; this is document review, not implementation acceptance.

**Baseline:** Root `745c37398ebe3da9037dffff4020cebd49bc7c89`; gwz-core `ec1b95831582651953bb0bd3be3c4f88985aba6b`; gwz-transport `9f9f0dc4dd82e6329d6ce53e102a231e214ff673`. The Surface note was read using `git show` with the exact committed core SHA. The tuple matched at both start and end.

**Date:** 2026-10-03

**Axis:** Surface: caller-message clarity, timing defaults, HTTPS account selection, secret-safe recovery, and reversible native bypass. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on it. Filed verbatim by the lane owner.

**Verdict: GO** — Zero open findings. Original P2-1 is closed by the revised configuration and recovery guidance. The whole revised Surface note produced no additional findings.

---

## 0. Evidence base

The review ran in `/Volumes/projects/limbo/gwz-dev-tr2-22`.

At the start and end, the following commands returned the same exact tuple:

- `git rev-parse HEAD` → `745c37398ebe3da9037dffff4020cebd49bc7c89`
- `git -C gwz-core rev-parse HEAD` → `ec1b95831582651953bb0bd3be3c4f88985aba6b`
- `git -C gwz-transport rev-parse HEAD` → `9f9f0dc4dd82e6329d6ce53e102a231e214ff673`

The entire committed Surface note, lines 1–166, was read with:

```sh
git -C gwz-core show ec1b95831582651953bb0bd3be3c4f88985aba6b:dev-docs/GwzTransportCredentialHelpersSurface.md | nl -ba
```

The substantive review covered:

- Introduction and URL discovery, lines 1–12.
- M4 interaction timeout and repair, lines 14–34.
- M8 unusable answer, helper identification, and repair, lines 36–72.
- M10 resource timeout and repair, lines 74–97.
- HTTPS URL account selection and refusals, lines 99–112.
- Configuration scope, helper selection, counterexamples, undo, and native bypass, lines 114–166.

The original Surface report supplied the P2-1 counterexamples and closure criteria. No code, design, remediation plan, implementation record, private evidence, or peer report was read. No builds, tests, writes, or history mutations were performed. Counterexample checks below are static walkthroughs of the revised documented behavior, not executed integration tests.

## 2. Invariant analysis

### Original P2-1: configuration-scope mismatch is closed

The original defect was that ordinary Git commands were presented as identifying or repairing GWZ's helper even though those commands could use repository settings or conditional includes that GWZ ignores.

The revised M8 message explicitly warns that `git config --get-urlmatch credential.helper` can select a different helper through repository settings or conditional includes, and directs the user to GWZ's helper chain, lines 38–44. The surrounding instructions repeat that limitation, lines 58–60, and state that a successful Git command may have repaired a different helper, lines 68–72.

The configuration section now gives an actionable route to the relevant helper:

- Lines 116–123 distinguish ordinary Git's configuration context from GWZ's context and explain why native Git success can leave GWZ broken.
- Lines 125–140 identify system, global, XDG, unconditional-include, and captured environment sources; describe path overrides and disabling the system file; cover URL-specific sections and empty-helper resets; and direct the user to repair or sign into the helper selected by that chain.
- Lines 142–152 work through the A/B mismatch explicitly, preserve legitimate unconditional includes, and explain both repairing A and deliberately making B available to GWZ.
- Lines 154–160 require preserving prior entries and order, document restoration, and warn that success through a repository-only helper does not repair another store.

I rechecked the original counterexamples:

1. **Unconditional global A plus conditional B:** A needs repair. A matching `includeIf` resets the native Git chain to working B. Native Git succeeds, while GWZ still uses A. The revised message warns about this mismatch, and the configuration walkthrough directs the user to the unconditional A entry and its repair. Native success through B is no longer presented as proof that A was repaired.
2. **Unconditional global A plus repository-local B:** A needs repair. Local configuration resets the native Git chain to working B. Running the supplied Git command inside the member succeeds through B. Lines 116–123 explain the discrepancy, and lines 142–160 provide the same route to A.
3. **A supplied by an unconditional include:** The revised instructions follow that include rather than disabling all includes. Lines 134–140 and 142–149 preserve A's legitimate configuration source.
4. **Only A configured:** Line 150 directs the user to repair or sign into A directly. The added mismatch guidance does not obstruct the ordinary recovery case.
5. **XDG or environment sources alter the chain:** Lines 127–133 and 150–152 require checking those sources instead of assuming the main global file is definitive.

These walkthroughs satisfy the original closure criteria. The diagnostic limitation is explicit, and the note supplies a route to the helper GWZ actually needs.

### Whole-note attacks that did not produce findings

- **Interaction versus resource waits:** M4 describes a fixed helper-answer allowance and excludes helper-availability waiting from it, lines 23–27. M10 describes resource waiting and states that no helper started, lines 84–91.
- **Actual seconds and defaults:** M4 states the normal 120-second allowance and exact fractional representation. M10 states the normal 30-second starting resource allowance, earlier consumption, and the actual remaining allowance. It expressly rejects a fresh 30-second wait.
- **Zero resource allowance:** Lines 89–91 explain that an earlier wait can leave zero seconds even when helper slots are free. The revised heading and message no longer assert that every slot was busy.
- **M8 failure and credential disposition:** The caller message identifies an unusable `git credential fill` answer and says no credential was sent. The fixed causes and format checklist give concrete repair criteria without including credential values, lines 38–66.
- **URL discovery:** The note identifies the cloned-member command, manifest URL before cloning, and rewritten URL under `--url-scheme https`, lines 9–12 and the repeated caller messages.
- **HTTPS account selection:** Lines 101–106 state the role of a URL username, where it is passed, its absence from HTTP userinfo and helper failure messages, and the behavior when no username is supplied.
- **Refusal of password and control data:** Lines 108–112 explain percent-encoded input, refusal of decoded control characters before requests or helpers start, and refusal of URL passwords, queries, and fragments.
- **Secret-safe diagnosis:** The helper-output checks avoid printing credential values, lines 61–66. Configuration and environment inspection is explicitly private because values can contain secrets, lines 133–134. Lines 154–156 prohibit pasting credentials into configuration or diagnostic output.
- **Undo and native bypass:** Lines 154–155 require recording and restoring helper entries and their order. Lines 162–166 pair native selection with removing the operation's flag or environment setting and state native transport's timing limitation.

Command-family placement and installation/removal lifecycle were not inferred from this note: the authorized object consists of caller messages and URL-username behavior, and the command whitelist did not permit obtaining CLI help.

## 3. Risks and next action

This GO applies to the committed Surface draft. It does not establish that implementation behavior matches the documented configuration scope, credential handling, or timing bounds.

The next action is to accept the revised Surface note as closing P2-1 and carry its configuration-scope counterexamples into implementation validation.

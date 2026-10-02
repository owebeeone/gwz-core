# GWZ Transport Credential Helper Context — SURFACE-AXIS REVIEW

**Review object:** Committed caller Surface note, `gwz-core/dev-docs/GwzTransportCredentialHelpersSurface.md`, at core `0adb093a99f4eb8e9b8818b767514a842533a193`. Helper context amendment draft; this is document review, not implementation acceptance.

**Baseline:** Root `bb44a7214eac779095352224e8940df268fd3f96`; gwz-core `0adb093a99f4eb8e9b8818b767514a842533a193`; gwz-transport `9f9f0dc4dd82e6329d6ce53e102a231e214ff673`. The Surface note was read using `git show` with the exact committed core SHA. The tuple matched at both start and end.

**Date:** 2026-10-03

**Axis:** Surface: caller-message clarity, timing defaults, HTTPS account selection, secret-safe recovery, and reversible native bypass. Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on it. Filed verbatim by the lane owner.

**Verdict: NO-GO** — One P2 diagnosability and recovery finding blocks. I pre-commit to GO on a revision that resolves P2-1 as specified.

---

## 0. Evidence base

The review ran in `/Volumes/projects/limbo/gwz-dev-tr2-22`.

At the start and end, the following commands returned the same exact tuple:

- `git rev-parse HEAD` → `bb44a7214eac779095352224e8940df268fd3f96`
- `git -C gwz-core rev-parse HEAD` → `0adb093a99f4eb8e9b8818b767514a842533a193`
- `git -C gwz-transport rev-parse HEAD` → `9f9f0dc4dd82e6329d6ce53e102a231e214ff673`

The entire committed Surface note, lines 1–121, was read with:

```sh
git -C gwz-core show 0adb093a99f4eb8e9b8818b767514a842533a193:dev-docs/GwzTransportCredentialHelpersSurface.md | nl -ba
```

The substantive review covered:

- Introduction and URL discovery, lines 1–10.
- M4 interaction timeout and repair, lines 12–30.
- M8 unusable answer, helper identification, and repair, lines 32–64.
- M10 resource timeout and repair, lines 66–86.
- HTTPS URL account selection and refusals, lines 88–101.
- Configuration scope, undo, and native bypass, lines 103–121.

No code, design, plan, implementation record, private evidence, or peer report was read. No builds, tests, writes, or history mutations were performed. The finding below is a static interface analysis with a reproducible configuration state; it is not a claim that its closure test was executed.

## 1. Findings

### [P2-1] Helper identification and sign-in recipes can select configuration that GWZ ignores

**Location:** M8, lines 35–39 and 53; helper repair, lines 61–64; M4/M10 sign-in recipes, lines 14–19, 27–29, 68–74, and 83–85; configuration-scope declaration, lines 105–108.

**Violated invariant:** A diagnostic advertised as naming the helper to check, and a sign-in advertised as repairing that helper, must address the configuration sources GWZ actually uses.

The note states that GWZ ignores member repository credential settings and every conditional include. Its diagnostic nevertheless uses ordinary:

```sh
git config --get-urlmatch credential.helper <url>
```

That command can read repository configuration and conditional includes. Likewise, ordinary `git ls-remote <url>` can use helpers supplied by those excluded sources. The note neither constrains these commands to GWZ's configuration scope nor warns that they can identify or repair a different helper chain.

**Credible reproduction:**

1. Configure helper A in unconditional global configuration. A needs repair or sign-in before it can supply a usable credential.
2. Configure helper B for the same URL through a matching global `includeIf` or the cloned member's repository configuration. B already has a usable credential.
3. GWZ ignores B, uses A, and reports a helper failure.
4. Run the prescribed diagnostic and sign-in commands in a context where B's configuration applies.
5. The diagnostic includes configuration GWZ ignores, and native Git can authenticate through B.
6. Retry GWZ. It still uses A and fails.

The conditional-include case requires no unusual working directory. The repository-local case also makes the outcome depend on where the user runs the supplied commands.

**Impact:** The user can inspect the wrong helper and obtain a successful sign-in check without repairing GWZ. M8's assertion that the command “names the helper to check” is therefore unreliable. The first-day recovery walkthrough reaches an unexplained repeated failure.

**Required correction:** Make helper identification and sign-in recovery consistent with the documented GWZ configuration scope. Supply a concrete recipe that identifies and repairs the helpers GWZ uses. If ordinary Git commands remain useful as preliminary checks, explicitly explain their different configuration scope and provide the next action when they succeed but GWZ still fails. Apply this correction to the caller wording and all repeated recovery instructions.

**Closure/regression test:** Review the revised instructions against both fixtures:

- Unconditional global helper A plus a matching `includeIf` helper B.
- Unconditional global helper A plus a repository-local helper B.

In each fixture, A requires repair while B can authenticate. Following the documented steps must identify A and lead to repairing A, or explicitly explain the mismatch and give an actionable route to A. Success through B alone must not be presented as proof that GWZ's helper has been repaired. Repeat the fixture with only A configured to ensure the ordinary recovery path remains clear.

## 2. Invariant analysis

The following attacks did not produce findings:

- **Interaction versus resource waits:** M4 identifies a fixed helper-answer bound, normally 120 seconds, and explicitly excludes helper-availability waiting from that allowance. M10 identifies resource waiting, explains the normal 30-second starting allowance and earlier consumption, and says no helper started for that lookup. These descriptions distinguish the two waits.
- **Actual seconds and defaults:** Both timing sections explain that messages show the actual allowance, including exact fractional seconds. M10 expressly rejects the interpretation that each lookup receives a fresh 30 seconds.
- **M8 failure and credential disposition:** The message identifies an unusable `git credential fill` answer and explicitly says no credential was sent. Its fixed causes exclude helper answers and credential values. The format checklist provides concrete repair criteria.
- **URL discovery before and after cloning:** The note supplies a cloned-member command, the manifest source before cloning, and the rewritten URL source under `--url-scheme https`.
- **HTTPS account selection:** The note explains that the URL username selects an account for credential helpers, is not sent as HTTP userinfo, and is absent from helper failure messages. It states the behavior when no username is supplied.
- **Refusal of password and control data:** The note prohibits URL passwords, queries, and fragments, and states that decoded control characters in usernames or paths are refused before requests or helpers start.
- **Secret handling during repair:** The note warns against putting credentials in URLs, shell commands, configuration, or diagnostic output. It describes validating helper-output structure without printing credential values.
- **Reversibility:** The native retry has an explicit undo: omit `--transport native` or remove the operation's `GWZ_TRANSPORT=native` setting. The note also requires recording and restoring prior helper settings and identifies sign-in tools' own account controls.
- **Native timing limitation:** The bypass section states that native transport does not supply the configured-helper time bound and can still require the same repair.

Command-family placement and installation/removal lifecycle were not inferred from this note: the authorized object consists of caller messages and URL-username behavior, and the command whitelist did not permit obtaining CLI help.

## 3. Risks and next action

This verdict covers the committed Surface draft only. It does not establish that an implementation emits these messages, enforces the stated timing bounds, or preserves the documented URL behavior.

The next action is to revise the diagnostic and recovery instructions to resolve P2-1, then perform a read-only Surface re-review at the new exact tuple.

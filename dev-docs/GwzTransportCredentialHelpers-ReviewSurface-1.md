# Credential helpers and recovery — SURFACE-AXIS REVIEW-1

**Review object:** Remediation round 1 of credential helpers, SSH/HTTPS consumers, shared SetupClock, public error code 75 and user recovery. Settled implementation candidate reviewed on 2026-10-03; this is Surface acceptance, not runtime, platform or release acceptance.

**Baseline:**

| Repository | Exact reviewed commit |
| --- | --- |
| root | `1de9e7fcbd17e5159ebb71ef3b683c2656a1cf8a` |
| gwz-core | `64ec039089b6e217625d7784b106d917452963cb` |
| gwz-transport | `1aab733783e06b25cb5d2321d71ec0b34417a29c` |
| gwz-py | `a0d4350f31069362b3cfbeae668666ab47567264` |
| gwz-cli | `f925e1165c2b2d368a00277594450b010a95867a` |
| gwz-core-evidence | `662d89828b478a2acce8c0308834db7d17c872f7` |

Committed guides were read using `git show HEAD:<path>`. The composed lane Python README at `a0d4350f` is the current authority; auxiliary MAIN Python `a0773afa` was used only as the prior documentation baseline.

**Date:** 2026-10-03

**Axis:** Surface: discovery, names, placement, defaults, setting/override/removal lifecycle, and credential-failure recovery. Independent, adversarial, read-only. The other axes run in parallel; nothing here relies on their current reports. Filed verbatim by the lane owner.

**Verdict: GO** — zero open P0, P1, P2 or P3 findings. Both original credential Surface P3 findings are closed by independently retracing their original counterexamples. The earlier Python help findings remain closed.

---

## 0. Evidence base

Read the complete revised canonical prompt:

`/Volumes/projects/limbo/gwz-lanes-prewarm-20261003/credential-implementation-review/PromptSurface-1.txt`

Retained the standing instructions and process rules read during the original review: workspace `AGENTS_GWZ.md`, root `AGENTS.md`, core and CLI `AGENTS.md`, `AgentProcessRules.md` and its controlling `GwzProcessOptimization.md` amendment. Read the newest credential checkpoint sections to establish the current tuple, review tier and exclusions.

Used the original committed Surface report to retrace P3-1 and P3-2. Used the Surface disposition rows, lines 34–35, of core `dev-docs/GwzTransportCredentialHelpers-RemPlan.md` as claimed corrections, not as closure evidence. No current peer report was read.

Current product evidence:

| Source | Scope |
| --- | --- |
| Python `README.md` | Entire committed page, including corrected transport-setting recipe at lines 182–203 and defaults/lifecycle at lines 205–219 |
| Core `docs/CredentialHelpers.md` | Entire guide, lines 1–81 |
| Core `docs/README.md` | Entire index and helper-guide discovery |
| Core `docs/ErrorCatalog.md` | Main error table and projection explanation, lines 1–58 |
| Core `docs/MessageCatalog.md` | `GwzErrorCode` listing, lines 672–751 |
| CLI `docs/Troubleshooting.md` | HTTPS/common-helper and SSH recovery, lines 130–217 |
| CLI `docs/commands/auth.md` | Entire guide |
| Previously read CLI user and command guides | Verified unchanged from `1543a3be`: Install, QuickStart, README, and fetch/push/pull/clone/materialize/repo command pages |

Read all seven current Python help surfaces and all six current CLI network help surfaces in full, plus the supplemental CLI root help:

| Driver | Surfaces |
| --- | --- |
| Python | root, fetch, push, pull, clone, materialize, repo clone |
| CLI | root, fetch, push, pull, clone, materialize, repo clone |

Help artifacts and provenance came from:

- `/Volumes/projects/limbo/gwz-tr222-remediation-surface-20261003/final/cli-artifacts.json`
- `/Volumes/projects/limbo/gwz-tr222-remediation-surface-20261003/final/py-artifacts.json`
- `/Volumes/projects/limbo/gwz-lanes-prewarm-20261003/credential-implementation-review/round1-cli-root-help.json`

All fourteen captured help files matched their recorded SHA-256 values at both hash-check boundaries. The candidate CLI executable matched `8d5bfa1649ab6f75e7d95207d45bab9961807c667c5a922eaab371bdbac8d3bf`. This verifies artifact identity only; no executable or network operation was run.

Verified every tuple member with `git rev-parse HEAD` and recorded worktree status at both review boundaries. All six HEADs remained unchanged. Root retained the same five excluded drafts, core the same two excluded private drafts, and evidence the same excluded old alpha-run directory. Transport, Python and CLI reported no dirt. Excluded contents were not read.

Inspection used permitted read-only Git commands, text inspection, and Python file/hash/difference inspection. No product implementation, product design or implementation record was inspected. No builds, tests, probes, edits, Git mutations or report serialization were performed.

## Prior-finding closure table

Finding IDs below retain their original report identity.

| ID | Disposition claimed | Verified on corrected tree | Status |
| --- | --- | --- | --- |
| Credential Surface P3-1: Python persistent selector recipe addresses the wrong file when `GIT_CONFIG_GLOBAL` is set | Replace global setter/remover with explicit conventional-file commands and explain the mismatch | README lines 186–197 and all seven Python help epilogs use paired `git config --file "$HOME/.gitconfig"` commands. With `GIT_CONFIG_GLOBAL` naming a different file, both setting and removal still address the documented file GWZ reads. The help explicitly explains why `--global` would address the ignored file. | **Closed** |
| Credential Surface P3-2: CLI elapsed-time examples omit independent credential-helper waiting | Qualify network-clock examples, identify helper admission/interaction phases, and point to code-75 recovery | All six network help surfaces explicitly say the examples are not total elapsed-time bounds, name separate allocation waiting and interaction allowance, describe SSH clock suspension, and identify code 75 and the recovery section. The original post-host-verification password-helper wait beyond 44 seconds is now explained by help alone. | **Closed** |
| Earlier Python Surface P3-1: concurrency defaults absent | State both transport-dependent defaults and explicit-value behavior | All seven Python help surfaces retain omitted jobs defaults of 100/50 and host defaults of 32/8 for gwz/native, with explicit positive values overriding them. README lines 205–210 retain omission versus explicit-value semantics. | **Remains closed** |
| Earlier Python Surface P3-2: transport-setting lifecycle absent | Expose candidate availability, choices, default, precedence, temporary/persistent selection, override and removal | Every Python help surface retains the complete lifecycle epilog. The corrected file recipe also resolves the follow-on credential Surface P3-1. | **Closed in full** |

These closures are textual retraces of the original documentation counterexamples. They do not assert that configuration or credential runtime behavior was executed.

## Changed-range analysis

The Surface changes match the two recorded dispositions.

Compared with the original captured help:

| Surface | Changed current lines |
| --- | --- |
| CLI fetch | 153–166 |
| CLI push | 136–149 |
| CLI pull | 125–138 |
| CLI clone | 144–157 |
| CLI materialize | 146–159 |
| CLI repo clone | 128–141 |
| Python root | 90–96 |
| Python fetch | 58–64 |
| Python push | 62–68 |
| Python pull | 61–67 |
| Python clone | 67–73 |
| Python materialize | 68–74 |
| Python repo clone | 69–75 |

Each CLI change replaces the elapsed-time paragraph with the qualified network-clock explanation and helper recovery reference. Each Python change replaces the persistent-setting/removal recipe and adds the `GIT_CONFIG_GLOBAL` explanation. The supplemental CLI root help is unchanged.

The composed Python README includes the accepted transport-selection documentation absent from the original generated-only lane baseline. Compared specifically with prior auxiliary MAIN `a0773afa`, its documentation change is the explicit-file recipe and explanation at lines 186–197.

The reviewed core helper guide, index, error catalog and relevant message listing are unchanged from the original credential Surface tuple. The reviewed CLI troubleshooting, authentication and command/user guides are also unchanged.

No changed Surface declaration introduces a new command, flag, setting name, default or lifecycle boundary. I found no new architectural or non-architectural root cause in the reviewed Surface changes. Product implementation changes are outside this axis and were not assessed.

## 2. Invariant analysis

The first-day help walkthrough now holds across both drivers.

CLI root help exposes fetch and the route to detailed command help. Detailed network help states the gwz/native choices, gwz default, flag-over-environment-over-global-file precedence, temporary selection, persistent setting and paired removal. A user can select native once, persist it, override with `--transport gwz`, remove the file setting, and unset an exported environment override.

Python root and all six network-command help pages identify the 1.1.0 candidate, gwz/native choices and gwz default. They provide a temporary native invocation, persistent setting, explicit gwz override, and removal of both persistent and exported settings. The original alternate-`GIT_CONFIG_GLOBAL` sequence now reaches the effective conventional file for both writes. No new transport constructor parameter is advertised.

The relevant defaults remain visible and consistent:

| Setting | CLI help | Python help/README |
| --- | --- | --- |
| Transport | gwz | gwz |
| Jobs, gwz/native | 100/50 | 100/50 |
| Host limit, gwz/native | 32/8 | 32/8 |
| Network timeout, gwz/native | 9/3 seconds | 9/9 seconds |
| Progress interval | 100 ms | 100 ms |
| CLI setup retries | Three extra attempts; no effect on native | README states native has no setup retries |

Explicit concurrency values override omitted defaults. Python’s `Client(max_connections_per_host=None)` remains documented as unset; an explicit value, including 32, remains explicit.

The corrected CLI timing paragraph distinguishes normally 30-second helper allocation waiting from interaction allowance of up to 120 seconds, and explains that `--ssh-timeout` does not shorten those helper allowances. It labels the 44/128-second examples as network-clock examples rather than total elapsed-time bounds. The original password-helper scenario therefore no longer requires the user to infer an unstated exception.

Recovery guidance continues to withstand the scope and lifecycle attacks:

- Code 75 is identified as an error code, not a duration. The core guide explains exact allowance reporting, including fractions, and distinguishes failure to start from failure to answer.
- The helper view includes captured system/global/XDG/session configuration and unconditional includes, while excluding repository-local configuration and `includeIf`. This is distinct from the narrower transport-selector file view.
- Native Git success is expressly insufficient proof of GWZ repair when a different local or conditional helper supplied it.
- Repair preserves legitimate unconditional includes, helper order and empty resets. Undo removes introduced entries and restores the privately recorded original chain.
- Missing Git directs the user to repair PATH/installation and start a new operation, including private-member clone failures.
- Rejected credentials are renewed in the owning provider/store and retried in a new operation. GWZ does not promise store/erase or automatic fallback.
- SSH helper scope remains ambient password-only authentication after host verification and method discovery. The guide does not present helpers as a host-trust bypass.
- The user is not instructed to print or save a credential answer.

The message catalog retains `credential_helper_timeout` wire value 75. The error catalog’s row retains HTTPS wording; common SSH applicability is explicit in the helper guide and in the troubleshooting section now referenced by CLI help.

Command placement and names remain coherent with the established families: workspace clone, member repo-clone, materialize, fetch, pull and push. No new credential-helper command family or installation action is introduced.

Candidate and platform boundaries remain explicit in the dedicated guides and Python selector documentation. Pending Windows qualification is not presented there as completed support. The captured candidate help is not evidence that the candidate is released, distributable or qualified on a particular platform.

## 3. Risks and next action

This GO establishes the reviewed public Surface and closes its original documentation findings. It does not establish credential secrecy in execution, error projection, atomic clock behavior, cancellation, cleanup, retries, selected-source provenance, package qualification or platform parity.

The next action is for the lane owner to combine this Surface GO with the required independent source-axis verdicts before accepting and merging the revised tuple. All six repository HEADs and all captured help hashes remained unchanged through the final checks.

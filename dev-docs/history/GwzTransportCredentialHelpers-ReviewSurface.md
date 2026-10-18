# Credential helpers and recovery — SURFACE-AXIS REVIEW

**Review object:** Credential helpers, typed SSH/HTTPS consumers, shared SetupClock, public error code 75 and user recovery at the settled implementation tuple below. Review candidate, not release or platform acceptance.

**Baseline:**

| Repository | Exact commit |
| --- | --- |
| root | `dc2b4af36e6ad6eca4a781611fcf76f6d9e76226` |
| gwz-core | `f97ff21fa56c2fe4abec6a73e90871dbe2d9d185` |
| gwz-transport | `41a16b2713b302c3675f081584d392afeae26ad5` |
| gwz-py | `947ed579abec292a23db3db7394923e70ee4363e` |
| gwz-cli | `1543a3bec00cda913a02c266e89d841eeeafb55b` |
| gwz-core-evidence | `662d89828b478a2acce8c0308834db7d17c872f7` |
| Auxiliary MAIN Python help/README | `a0773afa0cb3ae09be742dadd216dfd13527e692` |

Committed documentation was read using `git show HEAD:<path>`. Auxiliary Python README was read using `git show a0773afa0cb3ae09be742dadd216dfd13527e692:README.md`.

**Date:** 2026-10-03

**Axis:** Surface: command discovery, names, placement, defaults, setting/override/removal lifecycle, and credential-failure recovery. Independent, adversarial, read-only. The other axes run in parallel; nothing here relies on them. Filed verbatim by the lane owner.

**Verdict: GO** — zero P0, P1 or P2 findings; two bounded P3 documentation findings remain. Neither requires a compatibility break.

---

## 0. Evidence base

Read the complete canonical prompt:

`/Volumes/projects/limbo/gwz-lanes-prewarm-20261003/credential-implementation-review/PromptSurface.txt`

Read workspace `AGENTS_GWZ.md`, root `AGENTS.md`, core and CLI `AGENTS.md`, committed `dev-docs/AgentProcessRules.md` and `dev-docs/GwzProcessOptimization.md`. No `gwz-py/AGENTS.md` exists. Read the current credential entry in `dev-docs/CurrentProgramCheckpoint.md` to establish the recorded review tier and exclusions.

The product evidence consisted of:

| Source | Scope read |
| --- | --- |
| core `docs/CredentialHelpers.md` | Entire guide, lines 1–81 |
| core `docs/README.md` | Entire index, including helper-guide discovery |
| core `docs/ErrorCatalog.md` | Entire page; credential rows at lines 32–33 |
| core `docs/MessageCatalog.md` | `GwzErrorCode` listing, lines 672–751 |
| CLI `docs/Troubleshooting.md` | Entire page; HTTPS/common-helper recovery at lines 130–172 and SSH recovery at lines 174–216 |
| CLI `docs/commands/auth.md` | Entire page, lines 1–75 |
| CLI user guides | Entire `docs/Install.md`, `QuickStart.md` and `README.md` |
| CLI command pages | Entire `fetch.md`, `push.md`, `pull.md`, `clone.md`, `materialize.md` and `repo.md` under `docs/commands/` |
| Auxiliary MAIN Python `README.md` | Entire page at exact `a0773afa`; transport lifecycle and defaults at lines 180–217 |
| Prior Python Surface report | Only its Surface findings and their help-only counterexamples, from `GwzPyTransportSetting-ReviewSurface.md` |

Read all fourteen frozen help files named by `surface-artifacts.json`:

| Driver | Surfaces |
| --- | --- |
| CLI | root, fetch, push, pull, clone, materialize, repo clone |
| Python | root, fetch, push, pull, clone, materialize, repo clone |

SHA-256 verification matched every help file against the supplied manifest at both hash-check boundaries. The retained CLI executable also matched its recorded SHA-256, `1268bc7574282a8b0cf9061d97dcbe4c87b7708334bf7a12dba1fc09e94b2e61`. No binary was executed.

All six tuple members matched at the beginning and end. The root retained the same five excluded untracked drafts. At the final check, core retained the two excluded private drafts, evidence retained the excluded old alpha-run directory, and transport, Python and CLI had no reported dirt. No excluded contents were read. Documentation was taken from commits, not working-tree versions.

Inspection used only permitted read-only commands: `git rev-parse`, `git status`, `git show`, `git ls-tree`, `rg`, `sed`, `cat`, `nl`, and Python file/hash inspection. No implementation, product design, product plan, receipt narrative or current peer report was read. No builds, tests, probes, mutations or report serialization were performed.

## 1. Findings

### [P3-1] Python’s persistent selector recipe can edit a configuration file the resolver ignores

**Location:** Auxiliary Python `README.md` at `a0773afa`, lines 186–195; shared epilog in all seven Python help surfaces. Representative locations are `py-root.txt` lines 88–93 and `py-fetch.txt` lines 56–61.

**Violated invariant:** A documented persistent setting and its paired removal must address a configuration source the application actually reads.

**Reproduction/state sequence:**

1. The process has `GIT_CONFIG_GLOBAL` set to a custom file, and `GWZ_TRANSPORT` is unset.
2. Follow the help recipe: `git config --global gwz.transport native`.
3. Git’s global setter writes the custom file. The Python README explicitly states that GWZ reads the conventional global/XDG files and does not read a file named by `GIT_CONFIG_GLOBAL`.
4. The next GWZ operation therefore need not select native.
5. Conversely, if the conventional `~/.gitconfig` already selects native, the advertised `git config --global --unset-all gwz.transport` addresses the custom file and can leave the effective native setting in place.

This is a documentation-derived counterexample; no configuration mutation or execution probe was permitted or performed. The reviewed CLI authentication guide expressly recognizes the same issue and supplies paired `--file "$HOME/.gitconfig"` commands at lines 64–71.

**Impact:** A user following Python help can fail to persist or remove the selector and must discover the resolver’s different scope independently. Environment overrides remain available, so the consequence is bounded.

**Required correction:** Use the paired explicit-file commands already documented for CLI, or otherwise make the recipe target a file GWZ reads. Explain why Git’s `--global` form is unsafe when `GIT_CONFIG_GLOBAL` is set.

**Closure/regression test:** Inspect README and all seven revised Python help surfaces. Trace both setting and removal with `GIT_CONFIG_GLOBAL` pointing elsewhere; the documented commands must still address the effective GWZ configuration file.

**Classification:** Non-architectural documentation correction.

### [P3-2] CLI help’s elapsed-time examples omit the new independent credential-helper wait

**Location:** The shared `--ssh-timeout` text in the six CLI network help surfaces. Representative location: `cli-fetch.txt` lines 154–158. Corresponding text appears in push, pull, clone, materialize and repo-clone help.

**Violated invariant:** Help describing an elapsed-time upper bound must identify independently allowed local interaction time that can exceed it.

**Reproduction/state sequence:**

1. Read fetch help alone. It says an SSH setup making no progress is reported after “at most” approximately 44 seconds at defaults. Its stated local-interaction exception is a host-key prompt.
2. Consider the SSH password-only helper route documented in core `CredentialHelpers.md` lines 26–29.
3. Host-key verification is already complete. The admitted helper waits for a sign-in or unlock.
4. The same guide gives that helper an interaction allowance of up to 120 seconds and states that SSH aggregate/stall clocks pause during admitted helper phases, lines 33–39.
5. A helper waiting beyond 44 seconds is therefore compatible with the documented credential contract, although the help-only timing explanation provides no credential-helper exception.

The contradiction follows from the reviewed public text; it is not a claim that final runtime behavior was executed.

**Impact:** A help-only user can mistake an allowed helper interaction for a broken timeout or unexpectedly hanging process. The dedicated recovery guides explain the separate allowance, limiting the consequence.

**Required correction:** Qualify the elapsed-time examples as network-clock examples and explicitly identify admitted credential-helper interaction and separate allocation waiting as additional local phases. Keep the distinction between `--ssh-timeout` and code 75 visible; do not invent a total wall-clock bound without accounting for all admitted waits.

**Closure/regression test:** Inspect all six revised CLI network help surfaces. A reader must be able to explain why an SSH password helper can legitimately remain pending longer than the displayed network-clock example, and locate the code-75 recovery guidance.

**Classification:** Non-architectural documentation correction.

## 2. Invariant analysis

### Prior Python Surface finding closure

| Prior finding | Result | Independent evidence |
| --- | --- | --- |
| Prior P3-1: transport-dependent concurrency defaults omitted | **Closed** | All seven Python help surfaces state omitted jobs defaults of 100/50 and host defaults of 32/8 for gwz/native, and that explicit positive values override them. README lines 203–208 preserve omission versus an explicit value, including explicit 32. |
| Prior P3-2: transport-setting lifecycle absent from help | **Original absence closed; follow-on defect recorded as this report’s P3-1** | Every Python help surface now names the 1.1.0 candidate, gwz/native choices, gwz default, environment precedence, a temporary invocation, persistent setting, explicit override, and both removal forms. The original help-only discovery counterexample no longer holds. The supplied persistent recipe still fails the `GIT_CONFIG_GLOBAL` case described above. |

### First-day walkthrough

From CLI root help, a user can find `fetch`, inspect its detailed help, select `--transport native` for one invocation, persist the setting with the explicit-file command, override it with `--transport gwz`, and remove the persistent setting with the paired explicit-file remover. The global flag’s placement fits the existing network-command family. No new credential-helper command family is advertised: helpers are selected through Git configuration.

From Python root or network help, a user can identify the candidate, default and temporary environment selection, then use `GWZ_TRANSPORT=gwz` and `unset GWZ_TRANSPORT` to override and remove the environment choice. The persistent-file edge case is P3-1. Concurrency defaults are now available from help alone.

These were textual walkthroughs. Installation and network execution were not performed, and the retained help artifacts do not prove final credential execution.

### Recovery and configuration attacks that held

The dedicated helper guide and CLI troubleshooting section distinguish code 75 from a duration, state the maximum interaction allowance and possible shortening, distinguish allocation waiting from interaction, and separate helper slots from jobs/connections/network timeouts. The core guide explicitly allows fractional seconds in the reported allowance and explains that zero allocation does not establish contention.

The attempted “native Git succeeded, therefore GWZ is repaired” inference is expressly rejected. Both guides explain that GWZ’s captured system/global/XDG/session view includes unconditional includes but excludes repository-local configuration and every `includeIf`. A helper selected by native Git can therefore differ.

Repair guidance preserves legitimate unconditional includes, helper order and empty resets. It instructs the user to privately record original entries and sources, remove only introduced entries, and restore the original chain. Neither guide tells the user to print or save a credential answer, nor to erase the credential through GWZ.

Missing Git has a separate recovery path: repair the captured PATH/installation and begin another operation. The core guide expressly states that a private clone does not quietly suppress this failure. Rejected credentials are renewed in the owning provider/store and retried in a new operation; GWZ does not promise automatic store/erase or transport fallback.

SSH helper scope is explicit: ambient password-only authentication after host verification and method discovery. Explicit keys, other advertised method combinations and disabled helpers retain key/agent selection. The surface does not present helpers as a host-trust bypass.

The code listing assigns `credential_helper_timeout` wire value 75. The error catalog includes code 75 and recovery guidance, though its row uses HTTPS terminology; the common SSH applicability is explicit in the helper guide and CLI troubleshooting section.

### Availability and naming

The core helper guide and new troubleshooting text identify the planned 1.1.0 transport candidate and pending Windows qualification. Python’s selector documentation likewise labels candidate availability. These statements do not establish a released candidate, distributable package, selected-source provenance or Windows credential support.

The reviewed command names and summaries preserve established placement: workspace clone, member repo-clone, materialize, fetch, pull and push. I found no interface-shape defect requiring a compatibility break.

## 3. Risks and next action

This verdict establishes the reviewed public surface only. It does not establish credential secrecy in execution, typed-error projection, shared-clock arbitration, cancellation, cleanup, retry behavior, final binary provenance or platform parity. Those remain outside this Surface review.

The next action is a bounded documentation/help correction for P3-1 and P3-2, followed by inspection of the regenerated frozen help and the two original counterexamples. The settled source tuple and help hashes remained unchanged through the final check.

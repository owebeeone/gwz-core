# GwzRemoteTransportRetryPlan — SURFACE-AXIS REVIEW

**Review object:** proposed help for --jobs, --max-per-host, and --ssh-timeout, plan hash 623511dd9508bc0a8c44ca14e576eec51923e041ed7ad6bea46a0f2db862948e, 2026-09-23
**Baseline:** live `gwz --help` / `gwz-alpha --help` as run
**Date:** 2026-09-23
**Axis:** Surface: help text read cold, no design document. Independent, adversarial, read-only. The other axes run in parallel; nothing here relies on them. Filed verbatim by the lane owner.

**Verdict: NO-GO** — 0 P0, 0 P1, 1 P2, 4 P3.

---

## 0. Evidence base

- **Tuple:** I ran `shasum -a 256 /Users/owebeeone/limbo/gwz-dev/gwz-core/dev-docs/GwzRemoteTransportRetryPlan.md` at the start and again at the end. Both runs gave `623511dd9508bc0a8c44ca14e576eec51923e041ed7ad6bea46a0f2db862948e`, which matches the tuple. I did not open the plan, any dev-docs file, or any source file.
- **Binaries:**
  - `/Users/owebeeone/.cargo/bin/gwz` reports `gwz 1.0.17`.
  - `/Users/owebeeone/.cargo/bin/gwz-alpha` reports `gwz 0.2.0-alpha.transport`.
- **Help commands run, against both binaries:** `--help`, `fetch --help`, `push --help`, `pull --help`, `fetch -h`, `status -h`, `clone -h`, and `help fetch`.
- **Where the three flags live today:**
  - None of the three flags appears in top-level `gwz --help`. That page points to `gwz help COMMAND` for all options.
  - All three appear under **"Global Options"** in every subcommand's help, including local-only commands such as `status`. For fetch, push and pull, that is where a user already finds them. The proposal changes no placement.
- **Live one-line text (`-h`), identical in both binaries:**
  - `--jobs <n>`: "Global ceiling on concurrent member operations (default 50)"
  - `--max-per-host <n>`: "Max concurrent connections to any one host (default 8)"
  - `--ssh-timeout <secs>`: "Abort a stalled SSH/network read after N seconds (0 = no timeout, default 3)"
  - Sibling `--progress-interval <ms>`: "… (0 = every update)". The sibling flags therefore already use the convention that 0 has a special meaning.
- **Live long text (`--help`) for `--ssh-timeout`:** "Maximum seconds to wait on a stalled SSH/network read **before failing**. … 0 disables the timeout. Defaults to 3." No retry is mentioned anywhere in live help. I grepped `push --help` for retr, backoff and attempt; the only hit is `--verbose`'s "every remote authentication attempt".
- **Live long text for `--max-per-host`:** "Maximum concurrent network operations against a single remote host, so a host is not overloaded. … (e.g. local paths) … Defaults to 8."
- **Fetch exit codes (live `fetch --help`):** "0 when every selected repository answered, 1 when some answered and some failed … 2 when every selected repository was refused before the network." Push and pull defer to the same model. None of it mentions retries.
- **What the proposal changes:**
  - The three defaults (deferred; not re-litigated).
  - The `--jobs` long text gains "not extra processes" and "100 is not a maximum".
  - The `--max-per-host` long text drops "so a host is not overloaded" and gains "32 is not a maximum".
  - The `--ssh-timeout` long text changes "before failing" to "before that attempt fails" and adds the retry sentence.
  - The `--ssh-timeout` short line keeps the live wording and changes only the default.

## 1. Findings

### P2-1 — The `--ssh-timeout` short line still promises failure after N seconds, but N now bounds only one attempt

- **Root cause:** The long help changes the unit of `--ssh-timeout` from the operation to a single attempt: "before that attempt fails … is retried with exponential backoff, up to 4 attempts". The one-line help, which is what `-h` and `gwz <cmd> -h` show, keeps the live wording: "Abort a stalled SSH/network read after N seconds". It says nothing about attempts or retries.
- **Location:** The proposed `--ssh-timeout` short help line.
- **Violated invariant:**
  - A flag's one-line help must not describe observable behaviour that differs from its long help.
  - When a released flag (present in 1.0.17) changes meaning, the change must be visible where users meet the flag.
- **Cold-reading reproduction:**
  1. A user reads `gwz fetch -h` and sees "Abort a stalled SSH/network read after N seconds (0 = no timeout, default 9)".
  2. They conclude that a dead host costs about 9 seconds, or 5 seconds with `--ssh-timeout 5`.
  3. Under the long text, a stalled host costs up to 4 × N seconds plus an unstated backoff. That is at least 36 seconds at the default and at least 20 seconds with `--ssh-timeout 5`.
  4. Nothing in `-h` explains the gap.
  5. A 1.0.17 user whose scripts pass `--ssh-timeout 3` to fail fast reads the same short line they already know and gets no signal that time-to-failure has roughly quadrupled.
  6. Even the long text never gives a worst-case wall-clock bound, because the backoff base and cap are unstated. A reader cannot work out how long a dead host takes.
- **Is it a contradiction?** Only partly. "Abort … read" and "that attempt fails" can be reconciled if the reader already knows retries exist. The short line alone gives no such hint, so a cold `-h` reader cannot resolve it.
- **Impact:**
  - **Diagnosability:** "Why did fetch hang for 40 s when I set a 9 s timeout?" has no answer in `-h`.
  - **Compatibility:** The per-attempt meaning of N ships as the contract of an existing flag. If the short text stays operation-shaped, users will build expectations the implementation does not meet. Reconciling the two after release means changing either the text or the behaviour that users have come to rely on.
- **Required correction:**
  - The short line must name the per-attempt unit and the existence of retries. For example: "Per-attempt stall timeout in seconds; stalled connects are retried (0 = no timeout, default 9)".
  - The long text must state the worst-case wall-clock time to failure at the default, or give the backoff base and cap so it can be computed.
- **Closure test:**
  - From `gwz fetch -h` alone, a reader can say that N applies per attempt and that retries exist.
  - From `gwz fetch --help` alone, a reader can bound the total time before a dead host is reported at `--ssh-timeout 9`.

### P3-1 — Retry behaviour sits in the `--ssh-timeout` help, but it also covers failures that have nothing to do with the timeout, and its scope is undefined

- **Root cause:** The only place retries are described is the last sentence of the `--ssh-timeout` long help. That sentence also covers "refused or reset before authentication", which has nothing to do with a timeout. It leaves the scope of retries undefined.
- **Location:** The last sentence of the proposed `--ssh-timeout` long help. Retries are absent from the short help, from the fetch, push and pull descriptions, from their exit-code paragraphs, and from `--verbose`.
- **Violated invariant:** Behaviour must be findable from the help of the thing it affects, and a behaviour with no off switch must say that it has none.
- **Cold-reading reproduction:** A user sees "connection refused" reported only after a delay on `gwz push`. They check `push --help`: nothing about retries in the description or the exit codes. `--verbose` promises diagnostics for each "authentication attempt", not connection retries. They would have to guess that the explanation lives under a timeout flag. Once they find it, they still have to guess:
  - **With `--ssh-timeout 0`:** does "0 disables the timeout" also disable retries? The text only implies that refused or reset connections are still retried.
  - **Which stalls are retried:** "A connection attempt that fails because it stalled" suggests the connect phase only. The first sentence covers any "stalled SSH/network read", including a stall in the middle of a push transfer. Whether a stalled push is retried is a question users will ask.
  - **Transport:** "SSH/network" together with "libssh2" leaves it unclear whether HTTPS remotes are covered.
  - **Visibility:** Do retries appear in `--verbose`, `--json` or `--jsonl` output?
  - **Opting out:** Can retries be turned off at all?
- **Impact:** Users misattribute delays, especially in CI where fast failure is wanted, and cannot learn from help that retries cannot be disabled.
- **Required correction:**
  - State in the retry sentence:
    - which phases are retried (connect or pre-authentication only, versus mid-transfer);
    - which transports are covered;
    - that `--ssh-timeout 0` leaves refused and reset retries in place (or disables them, whichever is true);
    - that the retry budget is not configurable.
  - Add one line in the fetch, push and pull descriptions, or next to their exit codes, saying that failed connection attempts are retried before a repository counts as failed, with a pointer to `--ssh-timeout`.
- **Closure test:** A reader of `gwz push --help` alone can answer these five questions without guessing:
  1. Are refused connections retried?
  2. Is a stall in the middle of a transfer retried?
  3. Does `--ssh-timeout 0` stop retries?
  4. Can retries be disabled?
  5. Does a member that succeeded after a retry count as having answered?

### P3-2 — "100/32 is not a maximum" contradicts itself on a cold read

- **Root cause:** "Not a maximum" refers to the accepted range of values. It appears directly after text that defines the flag as a ceiling or maximum.
- **Location:**
  - The last sentence of the proposed `--jobs` long help: "A larger value is accepted; 100 is not a maximum."
  - The last sentence of the proposed `--max-per-host` long help: "A larger value is accepted; 32 is not a maximum."
- **Violated invariant:** The long help must not undercut the flag's own definition. `--max-per-host` must read as a hard per-host bound if it is one.
- **Cold-reading reproduction:** The reader sees "Maximum concurrent network operations against a single remote host … Defaults to 32. … 32 is not a maximum." A plausible reading is that per-host concurrency can exceed 32 at the default, so the limit is soft. The same happens with `--jobs`: "Global ceiling … Defaults to 100 … 100 is not a maximum."
- **Impact:** Users may distrust the per-host limit, for example when protecting a rate-limited host, or may set values they believe are enforced when the text says they are not.
- **Required correction:** Replace the sentence with range wording that does not reuse "maximum". For example: "Values above 32 are accepted." or "The limit is enforced; you may set it higher than the default."
- **Closure test:** No sentence in either long help contains "maximum" or "ceiling" in a sense opposite to the flag's definition. A cold reader, asked whether the default limit can be exceeded at run time, answers "no".

### P3-3 — What `--max-per-host` counts is undefined

- **Root cause:** Neither the unit being counted nor the meaning of "host" is defined, and the two help lines use different units.
- **Location:**
  - The proposed `--max-per-host` short help says "connections".
  - The long help says "network operations".
  - Neither text defines "host".
- **Violated invariant:** A limit must name the quantity it counts and the key it counts by.
- **Cold-reading reproduction:** A user sets `--max-per-host 4` to protect a GitHub Enterprise host.
  - **Unit:** Is 4 a limit on sockets or on member operations? A single member operation might open more than one connection, for example a push that also queries the remote.
  - **Host key:** They have also configured two ssh `Host` aliases (`gh-work`, `gh-personal`) that resolve to the same server. Are those one host or two? Is `host:port` distinct from `host`?
  - **Unparseable hosts:** For "Members whose host cannot be parsed", a user with an ssh alias may wonder whether an alias counts as parseable.
- **Impact:** The per-host protection may silently not apply when aliases are in use, or may be misjudged by a factor equal to the connections per operation.
- **Required correction:**
  - Use one unit in both lines: connections or member operations, whichever is true.
  - Add one sentence defining the host key, for example: "Host is the hostname as written in the remote URL or ssh alias, before ssh config resolution." Or whatever is true.
- **Closure test:** Both lines name the same unit. A cold reader can say whether two ssh aliases for the same server share one limit.

### P3-4 — The lower end of `--jobs` and `--max-per-host` is undocumented, although the sibling flags give 0 a special meaning

- **Root cause:** The proposed text documents only the upward direction ("A larger value is accepted"). It is silent on 0 and on how to request "unlimited", while the neighbouring flags `--ssh-timeout` and `--progress-interval` both give 0 a special meaning.
- **Location:** The proposed short and long help for `--jobs` and for `--max-per-host`.
- **Violated invariant:** Both directions of an option's lifecycle must be visible. The prompt's own criterion is "Both directions should be visible".
- **Cold-reading reproduction:** A user who wants no per-host throttling reads `--ssh-timeout` ("0 = no timeout") and `--progress-interval` ("0 = every update"), then tries `--max-per-host 0`. From help alone they cannot tell whether that means unlimited, is rejected, or deadlocks with zero permits. The same applies to `--jobs 0`. They also cannot tell whether `--jobs 1` gives fully serial behaviour.
- **Impact:** Guesswork on a flag family where 0 is already overloaded. If 0 currently has some accidental behaviour, documenting a different meaning later becomes a compatibility question.
- **Required correction:** State the minimum accepted value and what 0 does (rejected, or unlimited) in the long help of both flags.
- **Closure test:** From `--help` alone, a reader can say what `--jobs 0` and `--max-per-host 0` do.

## 2. Invariant analysis

| Check | --jobs | --max-per-host | --ssh-timeout |
|---|---|---|---|
| Found where it already lives (Global Options on fetch, push, pull) | Pass | Pass | Pass |
| Default stated in short help | Pass (100) | Pass (32) | Pass (9) |
| Default stated in long help | Pass | Pass | Pass |
| Short line says what it does, read cold | Pass | Partial (P3-3) | Fail (P2-1) |
| Short and long agree | Pass | Fail on unit (P3-3) | Fail on scope (P2-1) |
| Reads as a process spawn? | No; "not extra processes" settles it | n/a | n/a |
| Reads as a hard maximum? | Ambiguous (P3-2) | Ambiguous (P3-2) | n/a |
| Lower lifecycle (0 / off) | Missing (P3-4) | Missing (P3-4) | Pass ("0 disables") |
| Upper lifecycle (larger accepted) | Pass | Pass | Implicit; acceptable |
| Retry/backoff findable from help | n/a | n/a | Long help only, under a flag that looks unrelated (P3-1) |

**First-day walkthrough, from help alone:**

1. **Set concurrency:** `gwz --jobs 20 fetch`. This is clear. The Global Options placement and the examples show the syntax. The only open question is whether `--jobs` also affects local commands such as `status`, where it is listed. That is pre-existing and does not change behaviour for a new user.
2. **Set a per-host limit:** `gwz --max-per-host 4 push`. I had to guess:
   - whether the limit counts connections or operations (P3-3);
   - whether ssh aliases share a host (P3-3);
   - whether "32 is not a maximum" means the limit is soft (P3-2);
   - what 0 does (P3-4).
3. **Disable the timeout:** `gwz --ssh-timeout 0 fetch`. The timeout part is clear. I had to guess whether refused or reset retries still happen (P3-1).
4. **Understand what a stall does:** From `-h`, I would conclude it aborts after N seconds. That is wrong (P2-1). From `--help`, I learned it retries up to 4 attempts. I had to guess:
   - the total time to failure (P2-1);
   - whether a stall in the middle of a push transfer is retried (P3-1);
   - whether retries show up in `--verbose` or `--json` (P3-1);
   - whether a member that succeeded on retry counts as having answered for exit codes (P3-1).

**Deferred items respected:** I did not challenge the values 100, 32, 9, or the 4-attempt budget. Every finding is about whether those values and behaviours are stated coherently.

## 3. Risks and next action

- **Residual risk once this is fixed:**
  - All three flags stay absent from top-level `gwz --help`. Discovery depends on `gwz help COMMAND`. This is pre-existing and not a finding against this proposal.
  - The flags are listed on local-only commands such as `status`, where they probably do nothing. This is also pre-existing, and worth a follow-up note.
- **Compatibility risk:** P2-1 is the only item whose cost grows after release. The per-attempt meaning of `--ssh-timeout` is a change to a flag that shipped in 1.0.17. The short line is where that change has to be announced.
- **Next action:** Revise the text only; no new flag is required.
  - Change the `--ssh-timeout` short line and add a worst-case bound (P2-1).
  - Define retry scope, the interaction with `--ssh-timeout 0`, and the absence of an opt-out, and add a one-line pointer in the fetch, push and pull descriptions (P3-1).
  - Reword "not a maximum" (P3-2).
  - Pick one unit and define host for `--max-per-host` (P3-3).
  - State the behaviour of 0 for `--jobs` and `--max-per-host` (P3-4).

I pre-commit to GO on a revision that resolves P2-1, P3-1, P3-2, P3-3 and P3-4 as specified.

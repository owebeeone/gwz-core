# GWZ transport adaptive concurrency design, revision 8 — SURFACE-AXIS REVIEW

**Review object:** `/private/tmp/claude-501/-Volumes-projects-limbo-gwz-dev/351b18f9-4ec1-4306-ac0a-299e9bded6dd/scratchpad/surface/surface-excerpt.md`, sha256 `691e0820060d6424d7ba1615f1bbd2dc3793c36779ce6c354b530ea224ee11ea`. The excerpt names design `GwzTransportAdaptiveConcurrencyDesign.md` at sha256 `0d8d9be981807be51d55e2be205e9dc97e6f95a0db86293aa8450c0f529e0872`. I did not open that design. The hash was verified at start and at end and did not move.
**Baseline:** installed gwz 1.0.17, run from `/Volumes/projects/limbo/gwz-dev`. I read `gwz --help`, `gwz help fetch`, `gwz help clone`, `gwz help pull` and `gwz fetch --help`. I also read `gwz-cli/docs/CLI.md`, `MachineOutput.md`, `Troubleshooting.md`, `Concepts.md`, `MergeRecovery.md`, `ClaudeCode.md`, `commands/fetch.md` and `commands/auth.md`.
**Date:** 2026-10-07
**Axis:** SURFACE, the interface as the person using it meets it: note wording, final error text, error code and JSON names, attempt suffix, options and defaults, discoverability. Independent, adversarial, read-only. Filed verbatim by the lane owner.

**Verdict: NO-GO** — 0 P0, 0 P1, 2 P2, 6 P3. I pre-commit to GO on a revision that resolves P2-1 and P2-2 as specified, provided the P3 items are either fixed or recorded as docs and wording work. They do not block.

---

## 0. Evidence base

Facts about the installed binary and the docs that the findings rely on:

1. **Installed 1.0.17 help is not the target baseline.** `gwz help fetch` says `--max-per-host` "Defaults to 8" and `--jobs` "Defaults to 50". It has no `--max-retries`, `--transport` or global `--quiet`. `gwz fetch --max-retries 1` fails with `error: unexpected argument '--max-retries' found`. `gwz --quiet status` fails the same way.
2. **The docs in the repo describe the candidate build.** `CLI.md` says `--max-per-host` "Defaults to 32. The smallest value is 1. 0 is rejected. Values above 32 are accepted. Each of these operations uses one connection." It also says `--jobs` "Defaults to 100". So "32 requested" matches the candidate docs, not the released binary.
3. **`--max-retries` is never defined as an option anywhere in the docs.** It appears only inside prose. One place is the `--ssh-timeout` paragraph, repeated per command in `CLI.md`: "Retries are controlled by --max-retries (default 3 extra attempts)…". The other is `commands/auth.md:60`: "`--max-retries` has no effect" on the native transport. There is no option entry, no value range, and no mention of throttles.
4. **`--quiet` is a `gwz diff` flag only** ("Suppress all output; implies --exit-code", `CLI.md:1140`, `commands/diff.md`). No network command has it.
5. **The error code `Capacity` appears nowhere in the user docs.** A grep of `docs/` finds no occurrence.
6. **Human failure rows today.** `commands/fetch.md` shows `failed  RemoteRejected: member 'mem_priv' at 'private': failed to connect to 127.0.0.1: Connection refused`. Existing messages name the member, in the form `member '<id>' at '<path>': <reason>`.
7. **The JSON error shape is fixed.** `MachineOutput.md` gives a seven-field error: `code`, `message`, `member_id`, `member_path`, `target_kind`, `detail`, `record_context`. Its examples show `"detail": null`. Existing connection refusals are `code: "RemoteRejected"`.
8. **`gwz:` is a reserved prefix on stderr.** `MergeRecovery.md:63`: "GWZ prints human errors as `gwz: <Code>: <message>`". `ClaudeCode.md:343` and `CLI.md:1711`: every failure is "one line on standard error, `gwz: <cause>; <remedy>`". Other informational lines carry no prefix, for example `url scheme: https (from --url-scheme)` in `MachineOutput.md`.
9. **Progress.** `Concepts.md:235`: "Human mode renders live progress to stderr when stderr is a terminal."
10. **The existing "attempt N of M" suffix is undocumented.** Neither it nor `retry_attempt` appears in any user doc. The note's "same rules as the transport setting's note" precedent is documented only as "Machine modes print no setting notices to stderr".
11. **Troubleshooting.** The only concurrency advice is in "SSH Or Credential Failure": "Use `--jobs` and `--max-per-host` to reduce concurrency against a host." There is no section on throttling or on a `Capacity` error.

---

## 1. Findings

### [P2-1] One error code, `Capacity`, and one detail name, `throttle`, cover unrelated causes, and the discriminator is buried in an optional field

**Location:** excerpt B, lines 18–21 and 23. The text says "a `Retry-After` is longer than 30 s … or a local permit wait expires, the member fails with code `Capacity` … and an optional `FailureDetail` field, `throttle`". It also says "with `--max-retries 0` … a refusal fails its member at once with this error".

**Surface rule violated:** an error code that scripts switch on must mean one thing, and a detail field's name must say what it contains. A name that has to be corrected after release needs a compatibility break.

**Root cause:** the excerpt maps three different situations onto one code and one detail object:
- the server throttled gwz;
- a local permit wait expired, where the proposed message is `local setup capacity exhausted for 30 s (64 supervised jobs in use)`;
- under `--max-retries 0`, any refusal. The text does not say whether the `last_signal` is `refused` or `reset` there, or whether the code changes from today's `RemoteRejected`.

For the local case the design still attaches the object named `throttle`, with `last_signal: "local"`. That is a `throttle` object describing something that is not a throttle.

**User scenario:**
1. A CI script does `jq '.errors[] | select(.code=="Capacity")'` and reruns after a delay. The same code now also arrives for "your own `--jobs` is too low for this host". The remedy is different, and the only way to tell them apart is `detail.throttle.last_signal == "local"`.
2. A user runs `gwz --max-retries 0 fetch` against a host that is simply down. Today the row is `RemoteRejected … Connection refused`. Under the proposed text it fails "at once with this error". If that means code `Capacity` and message `throttled: the server limited this command`, a connection-refused is misreported as a rate limit, and a script keyed on `RemoteRejected` stops matching.

**Impact:** the excerpt does not say which code a plain refusal gets at `--max-retries 0`. `Capacity` is undocumented (evidence 5), so scripts can only infer its meaning from the message. Once `throttle` and `Capacity` ship, splitting them is a break.

**Required correction:**
1. State that a refusal with no throttle evidence keeps its current code (`RemoteRejected`) and message, at every `--max-retries` value. Only a conclusive or confirmed throttle, and a long `Retry-After`, use the throttle error.
2. Either give the local expiry its own code, or at minimum do not attach the `throttle` detail to it. Carry a separate optional detail, or a `cause` discriminator inside the detail, that is a documented top-level key. If one code must stay for older readers, say so, and add `kind: "server_throttle" | "local_capacity"` as the first field of the detail.
3. Add `Capacity` and its `detail` object to `MachineOutput.md` as a documented code with each cause listed.

**Closure check:** a table in the design maps each of {server throttle, long Retry-After, local expiry, plain refusal at `--max-retries 0`} to code, message and presence of `throttle`. `MachineOutput.md` documents the code.

---

### [P2-2] The `throttle` field names mix vocabularies, and one of them contradicts the CLI flag it mirrors

**Location:** excerpt B, line 21. The fields are `limit_observed`, `limit_configured`, `attempts`, `attempts_max`, `elapsed_ms`, `last_signal`, `retry_after_ms`. Compare the human wording in excerpt A and B: "requested", "attempts".

**Surface rule violated:** one concept should have one name across help, human text and JSON. A JSON key is a compatibility commitment.

**Root cause, three problems:**
1. **`attempts_max` versus `--max-retries`.** `attempts_max` is 4 when `--max-retries` is 3 (R + 1). The same word "max" means "extra attempts" in the flag and "total attempts" in the field. A script that compares them gets an off-by-one, and `CLI.md` says "default 3 extra attempts".
2. **`limit_configured` versus "requested" versus `--max-per-host`.** The human message says "(32 requested)", the JSON says `limit_configured`, and the flag is `--max-per-host`. If no one passed the flag, "requested" and "configured" are both false. The default of 32 is neither requested nor configured. `CLI.md` calls it a default, and the existing help says "Maximum".
3. **`limit_observed` is not an observation.** Per D and A, it is the transport's current adaptive limit, a value gwz chose after a throttle. It is not a number the server stated. The field's name invites a consumer to treat it as server truth.

The excerpt also never shows the sibling `retry_attempt` shape, so consistency with it cannot be judged from the excerpt. Existing machine names use snake_case with `_ms` (`timestamp_ms`), so `elapsed_ms` and `retry_after_ms` are fine.

**User scenario:** a dashboard plots `attempts / attempts_max` and `--max-retries` side by side and shows 4/4 for a run at `--max-retries 3`. A user reading "32 requested" runs `gwz fetch` without any flag and wonders what they requested.

**Impact:** all three names are wire keys. Changing them after release is a break, or a permanent alias.

**Required correction:**
- Rename `attempts_max` to `attempts_allowed`, and document `attempts_allowed = max_retries + 1`.
- Use one name for the ceiling everywhere. For example JSON `max_per_host` (or `limit_ceiling`), and human text "(ceiling 32)" or "(--max-per-host 32)". Drop "requested" in the human text.
- Rename `limit_observed` to `limit_applied` or `limit_current`, and define it as "the per-host connection limit gwz was using when the member gave up".
- Show the sibling `retry_attempt` shape in the design so the pair can be judged together.

**Closure check:** the human note, human error and JSON use one name per concept. A field table in `MachineOutput.md` defines each field.

---

### [P3-1] The note's wording does not say whether anything is wrong or what to do, and "requested" is inaccurate for a default

**Location:** excerpt A, line 8: `gwz: github.com: limited to 8 concurrent connections (32 requested)`, `limit now 12`, `limit lifted`.

**Surface rule violated:** a line a user sees unprompted should say what happened, whether it is a problem, and what, if anything, to do.

**Read cold:**
- It does not say who limited ("limited" by the server? by gwz?).
- It does not say whether the run is degraded or still progressing.
- It does not say that no action is needed.
- It does not name the flag that sets the ceiling.
- "32 requested" is false when the user passed nothing (see P2-2).
- "limit now 12" does not say that the limit went up, or what it applies to.
- "limit lifted" reads as "the host no longer limits us". The excerpt means "gwz is back at the ceiling", and the server may re-impose the limit.
- The note does not use the word "throttle" or "slower", so someone searching the docs for it has nothing to search for.

**Required correction (proposed wording):**
- `github.com refused extra connections; using 8 at a time (ceiling 32, --max-per-host). The run continues, no action needed.`
- `github.com: now using 12 connections`
- `github.com: back at the ceiling of 32 connections`

**Closure check:** the strings are fixed in the design and in a `Troubleshooting.md` entry (P3-6).

---

### [P3-2] The note uses the reserved `gwz:` prefix, which docs reserve for failures

**Location:** excerpt A, line 8, `gwz: github.com: …`.

**Surface rule violated:** consistency with how gwz prints stderr. `gwz: <Code>: <message>` and `gwz: <cause>; <remedy>` are the documented failure shapes (evidence 8), and the existing informational line `url scheme: https (from --url-scheme)` has no prefix.

**User scenario:** an agent, hook or wrapper that treats any `^gwz:` stderr line as an error, as the `ClaudeCode.md` hook docs say, flags a healthy run. A reader sees `gwz: github.com: limited …` in the position where a code would be and takes `github.com` for a code.

**Required correction:** drop the prefix, or use a documented informational prefix such as `note:`. State the choice in the design. Note that the cited precedent (`merge_render.rs`) cannot be checked by this axis, and the docs show no `gwz:`-prefixed informational line.

**Closure check:** the design states the stderr line format and where it sits relative to live progress on a TTY. That placement is unspecified now (evidence 9).

---

### [P3-3] "Absent in `--quiet`" refers to a flag no network command has

**Location:** excerpt A, line 8, "Absent in `--json`, `--jsonl` and `--quiet`".

**Surface rule violated:** a documented rule must refer to things the user can do. `--quiet` exists only on `gwz diff` (evidence 4). The installed binary rejects `gwz --quiet status`.

**Impact:** the clause is vacuous, and it implies a global `--quiet` that does not exist. A user who wants a silent run, or a script author, reads a promise that cannot be exercised. The stderr note will appear in cron and CI logs, and the design offers no way to silence it other than a JSON mode.

**Required correction:** delete the `--quiet` clause, or say plainly that the only suppression is `--json`/`--jsonl`. State that the note appears on non-TTY stderr as well, and say why that is acceptable.

---

### [P3-4] The final error text asserts more than gwz knows, and the variants are not fully specified

**Location:** excerpt B, line 22.

**Surface rule violated:** a message should not assert a cause the transport inferred.

**Defects:**
1. "the server limited this command to 8 concurrent connections" states an adaptive estimate as a fact the server gave. A 429 or a reset says nothing about connections.
2. "this command" is wrong if the limit is per IP and shared with other processes or users.
3. When the member fails with no lowering (`--max-retries 0`, or an inconclusive refusal budget), observed equals configured. The template then reads "limited this command to 32 concurrent connections (32 requested)", which is nonsense.
4. The message omits the member prefix every existing row carries (`member 'id' at 'path':`, evidence 6). A human row becomes `failed  Capacity: github.com:443: throttled: …`, where the code and the word "throttled" repeat.
5. The `Retry-After` variant is given as "`… asked to wait 120 s (longer than the 30 s this command holds a host)`". The elided prefix is unspecified. The 30 s cap is not a documented setting, so the user cannot tell whether it can be changed.
6. The local variant has no host and no remedy.
7. There is no next step in any variant: retry later, lower `--max-per-host`, raise `--max-retries`, or "nothing you can change".

**Required correction (proposed wording):**
- `member 'mem_core' at 'gwz-core': github.com:443 kept refusing connections (about 8 at a time was accepted, ceiling 32); gave up after 4 attempts over 21 s; retry later, or lower --max-per-host`
- `…github.com:443 asked us to wait 120 s, longer than the 30 s one command holds a host; retry in 2 min`
- `…local connection slots stayed full for 30 s (64 jobs in use); lower --jobs`

Give a rule for the no-lowering case, for example drop the parenthesis when observed equals configured.

**Closure check:** each cause has full text with member prefix and one remedy, and a table of {case → text} in the design.

---

### [P3-5] "retest after attempt M of M" next to "attempt N of M" is confusing, and the finished members' "attempt N of M" is a copied statement

**Location:** excerpt C, line 28; excerpt D, line 33.

**Surface rule violated:** "attempt N of M" reads as that member's own attempts, and a new suffix should say what it counts.

**Defects:**
- "retest" is internal jargon with no definition in the docs.
- "retest after attempt 4 of 4" does not say whose attempts, or that it was the one connection a different member made to probe the host.
- Members that "fail at once with the recorded failure" and show the recorded "attempt 4 of 4" made no attempt at all. That is a false claim, copied from another member.
- The up to 30 s wait for a retest prints nothing, so gwz looks hung.
- `retry_attempt` and the existing suffix are not documented (evidence 10).

**Required correction (proposed wording):**
- `host unreachable: 4 attempts failed earlier in this run, and one retry after 30 s also failed`, or at minimum `retried once more after the earlier 4 of 4 attempts, still failing`.
- For the members that never tried: `skipped: host unreachable (4 attempts failed on another repository)`.
- Document the suffix in `MachineOutput.md`.

**Closure check:** the Surface re-review the status line requires has both strings, and the docs cover them.

---

### [P3-6] Discoverability: the existing help and docs now mislead, and nothing in the design lets a user find "why fewer connections" or "throttled"

**Location:** excerpt D (the lane owner's summary, not design text), against evidence 2, 3, 5 and 11.

**Defects:**
1. **`--max-per-host` help promises a number.** The help says "Maximum concurrent member operations against one remote hostname … Each of these operations uses one connection." It does not say that gwz may use fewer, that it climbs back, or that the note exists. D calls it "a ceiling", which the help does not. The note says "connections" where the help says "member operations".
2. **`--max-retries` has no option entry at all** (evidence 3). Nothing says that it also bounds throttle requeues (D), and nothing says what `--max-retries 0` does. The excerpt says `--max-retries 0` means no refusal lowers the limit and no adaptation happens at all (B, line 23). That coupling is invisible to a user, and D does not state it. The help text also describes only the wait between "failed setup attempt"s.
3. **No Troubleshooting entry.** Neither `Capacity` nor "throttled" appears in the docs, and the existing advice is "use `--jobs` and `--max-per-host` to reduce concurrency" with no mention of adaptation. The design commits to no doc change.
4. **`--verbose` has no designed content** for any of this ("Not designed here", excerpt A line 11). The one human diagnostic channel the docs name for connection behaviour says nothing about limits.
5. **Version skew.** Installed 1.0.17 documents 8 and 50 (evidence 1), while the candidate docs say 32 and 100. On the released binary the note would say "(8 requested)", which would puzzle a user who ran no flag.

**Required correction:** the design commits to doc work in the same change:
- extend the `--max-per-host` help with "gwz may use fewer connections when the host refuses more, and raises the limit again when it recovers";
- add a `--max-retries` option entry with its throttle role and the `--max-retries 0` effect;
- add a Troubleshooting section, "Fewer connections than --max-per-host, or a throttled error", that shows the note and the error text from P3-1 and P3-4.

**Closure check:** the docs contain the strings and a link from the error text.

---

## 2. First-day walkthrough

A user runs `gwz fetch` on a workspace of 40 GitHub repos, following only `--help` and the docs. Every item below is a point where they have to guess.

1. **Before running.** They read `gwz help fetch`. `--max-per-host` says "Maximum … Defaults to 32" (candidate) or 8 (installed). Nothing says the number can drop. They cannot find `--max-retries` in the option list at all (P3-6).
2. **Mid-run.** On stderr appears `gwz: github.com: limited to 8 concurrent connections (32 requested)`. They did not request 32. They wonder whether `gwz:` means an error (P3-2). They do not know whether something is wrong, who limited, or what to do (P3-1). Searching the docs for "limited", "throttled" or "concurrent connections" finds nothing, and they must guess it is harmless.
3. **Waits.** Some members sit for up to 30 s per `Retry-After` hold with no output ("A Queue wait prints nothing", "a hold … print[s] nothing"). They cannot tell a hang from a throttle wait (P3-5).
4. **Later lines.** `limit now 12`, then `limit lifted`. They cannot tell whether the host stopped limiting or gwz is back to its ceiling (P3-1).
5. **A member fails.** The row reads `failed  Capacity: github.com:443: throttled: the server limited this command to 8 concurrent connections (32 requested); gave up after 4 attempts over 21 s`. They must guess:
   - which member it is, since the member prefix is missing (P3-4);
   - whether `Capacity` is their own machine's (jobs) or the server's (P2-1);
   - why "4 attempts" when `--max-retries` defaults to 3 (P2-2);
   - what to do next, since there is no remedy in the text (P3-4).
6. **Next step.** They try `--max-retries 10` (not documented as an option, and absent from the installed binary). Or they try `--max-per-host 8`, which the existing Troubleshooting advice suggests. They cannot know that the transport already lowers to 8 by itself. They try `--max-retries 0` to fail fast and unknowingly turn off the adaptation (P3-6).
7. **Scripts.** A CI wrapper sees `code: "Capacity"` in `--json` and finds no definition (P2-1). If it also sees `detail.throttle.last_signal: "local"`, it learns only that the code name misled it.
8. **Exit.** Exit code 1 (`Partial`), the same as any other member failure. That is consistent with the docs and not a defect.

---

## 3. Risks and next action

- **Risks that survive the verdict.** The two P2s concern names that will be hard to change once shipped: the code and detail reuse (P2-1), and the field vocabulary (P2-2). Both are cheap to correct before implementation and expensive afterwards. The P3s are wording and docs, fixable at any time, but they decide whether a user can diagnose a throttle without reading source. P3-6 in particular should ship with the code.
- **Not examined.** I did not judge the algorithm. I did not verify the cited sources (`merge_render.rs:245-251`, `protocol.rs:851-857`), because reading source is out of bounds for this axis. The `retry_attempt` shape and the existing "attempt N of M" text could only be checked against the docs, which do not define them.
- **Next action for the lane owner.** Revise the design to cover P2-1 (an error-case table and the local/plain-refusal split) and P2-2 (one vocabulary: ceiling, applied limit, attempts allowed). Fix the exact strings of the note, the error variants and the retest suffix in the design with the proposed wording. Commit to the help, `--max-retries` option entry and Troubleshooting doc changes. Then send the strings back for the Surface re-review the status line requires. I will give a GO on that revision as pre-committed above.

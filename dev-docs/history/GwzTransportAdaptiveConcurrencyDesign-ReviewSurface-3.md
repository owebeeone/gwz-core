# GWZ transport adaptive concurrency design, revision 10 — SURFACE-AXIS FINAL RE-REVIEW

**Review object:** `/Volumes/projects/limbo/gwz-dev/gwz-core/dev-docs/GwzTransportAdaptiveConcurrencyDesign-SurfaceExcerpt-3.md`, sha256 `a0916b0841ca364fffa54c9bf0b2c49cb9b3ef3ca08e9f3ae3c3c54abc586917`, verified at start and at end (unchanged). The excerpt names design sha256 `8fcd784ea7307c15d03877897eb177d8fee7bc0ebf603d45a39d53a036b9d98e`. I did not open that design. The previous object was `-SurfaceExcerpt-2.md` (sha256 `ee66d48a…`), reviewed in `-ReviewSurface-2.md`.
**Baseline:** installed gwz 1.0.17 help, and the repo docs in `gwz-cli/docs/`: `CLI.md`, `MachineOutput.md`, `Troubleshooting.md`, `Concepts.md`, `MergeRecovery.md`, `ClaudeCode.md`, `commands/fetch.md`, `commands/auth.md`. No design, plan, review or source files were read.
**Date:** 2026-10-07
**Axis:** SURFACE, the interface as the person using it meets it. Closure check of P2-3 and P3-1 to P3-6 from the revision-9 re-check, plus an attack on the changed text. Independent, adversarial, read-only. Filed verbatim by the lane owner.

**Verdict: GO** — 0 P0, 0 P1, 0 P2, 6 P3 (all wording or docs, none blocking). The one open P2 from revision 9 is closed. The six P3 items are for the lane owner to fix with the code or record, and none needs a compatibility break to fix later.

---

## Prior-finding closure table

| ID (rev-9 report) | Disposition | Evidence in revision 10, and what remains |
|---|---|---|
| **P2-3** the cause-to-code rule is stated three ways; the same response gets different codes by `--max-retries` | **Closed** | One rule (line 31): `Capacity` iff the member's **own final failure** is a Throttle (a 429, or a 503 carrying `Retry-After`), a `Retry-After` longer than 30 s, or a local permit wait that expired. Everything else, "a bare 503, a reset, a refused connect or a stall included", keeps its code and message. The rule "never" looks at key state and "is the same at every `--max-retries`". Row 1 (line 35), row 4 (line 38) and the `--max-retries 0` bullet (line 51: "with the code the rule above gives it") now quote or refer to it, and no row contradicts it. I walked the matrix {429, 429 + `Retry-After` ≤ 30 s, 429 + `Retry-After` > 30 s, bare 503, 503 + `Retry-After`, reset, refused connect, stall} by {`--max-retries` 0, default, higher}: each cell has one code, identical across the three retry values. `Releases.md` and `MachineOutput.md` are committed to record the mapping change (lines 22, 43). Residuals are P3-5 and P3-6 below. |
| **P3-1** attempt count repeated in the message and in the suffix | **Closed** | Line 42: "The attempt count appears once." The existing suffix `attempt N of M` is rendered from `retry_attempt` and follows the message, and no message in the table repeats it. A full human row is given, and it matches the existing row shape in `commands/fetch.md` (`failed  <Code>: member '<id>' at '<path>': <message>`). Residual: rows 3 and 2, see P3-3. |
| **P3-2** Down-key strings over-claim, drop the reason, fixed "30 s later" | **Closed** | Row 5: `not attempted: github.com:22 failed earlier in this run (<the recorded failure's reason>)`. "Unreachable" is gone from the member strings. Row 6: "`; still failing after a retry about 30 s later`". The note now says "is not answering". |
| **P3-3** "64 jobs … lower --jobs" not actionable | **Closed** | Row 3 states that the 64 is "a fixed internal limit, not `--jobs`" and the message drops the bad remedy: `gwz's 64 connection-setup slots stayed full for 30 s (setups that stopped answering hold them until they time out); retry later`. The Troubleshooting section is committed to show "the 64 setup slots" (line 20). Residual: P3-3 (the stuck-setup lever). |
| **P3-4** naming and wording items (1 to 6) | **Closed** | (1) "accepted" is now "using 8 at a time". (2) Notes name host and port (line 8). (3) "Maximum (ceiling)" is in the help text, and the note reads "ceiling 32 from --max-per-host". (4) `attempts` is the total allowed (line 49, line 21). (5) `last_signal` is `http_429` or `http_503` and absent for `local`. (6) `cause` is explicitly nested under `capacity` and "distinct from the failure's existing top-level `setup_cause`". One new inconsistency from item 2: P3-1 below. |
| **P3-5** note placement, silent holds, `note:` shape undocumented | **Closed**, with residuals in P3-2 | Line 8 gives the TTY rule: "a note is printed as its own line above the progress block, which is redrawn below it". A `Retry-After` hold longer than 1 s now prints a note (line 12). `CLI.md` or `Concepts.md` is committed to state that `note:` is informational and `gwz:` is a failure (line 21). |
| **P3-6** doc gaps (`--ssh-timeout` prose, Troubleshooting pointer, stale D) | **Closed** | Line 19 updates the `--ssh-timeout` retry paragraphs. Line 20 has the "SSH Or Credential Failure" advice point to the new section. Line 62 now says `capacity`. |

---

## 0. Evidence base

1. **Round-1 and round-2 baseline unchanged.** Installed 1.0.17 predates all of this (`--max-per-host` default 8, `--jobs` default 50, no `--max-retries`). The repo `CLI.md` describes the candidate (32 and 100). `--quiet` is a `gwz diff` flag only. `Capacity` appears nowhere in the docs. The existing failure row shape is `failed  RemoteRejected: member '<id>' at '<path>': <reason>`. `MachineOutput.md` documents the seven-field error with `"detail": null` and does not yet document `retry_attempt`.
2. **`--max-per-host` is per hostname today.** `CLI.md:140`: "Maximum concurrent member operations against one remote hostname. The hostname is the host in the remote URL, lowercased, before SSH config is applied. Two URLs share this limit when they contain the same host." This is the sentence P3-1 below turns on.
3. **`Io` is not a documented error code.** `MachineOutput.md`, `Troubleshooting.md` and `CLI.md` never name it. Line 43 of the excerpt calls today's 429 code "an `Io` with status".
4. **Prefixes.** `gwz: <Code>: <message>` for failures; `warning:` has precedent as an informational prefix (`MergeRecovery.md:15`). `note:` is new, and its documentation is committed (line 21).

---

## 1. Findings

### [P3-1] The notes call SSH and HTTPS to one host "separate limits", while `--max-per-host` is documented as one limit per hostname

**Location:** excerpt A line 8 ("A note names the host with its port, since SSH and HTTPS to one host are separate limits") and line 18 (the `--max-per-host` help change), against `CLI.md:140` (evidence 2).

**Surface rule violated:** one number, one meaning across help, docs and output.

**Scenario:** a workspace has members that reach github.com over SSH and others over HTTPS. The help says `--max-per-host 32` bounds "one remote hostname", and "two URLs share this limit when they contain the same host". The user sees two notes, `github.com:22 …` and `github.com:443 …`, each with "ceiling 32 from --max-per-host". They then count up to 64 concurrent operations against github.com and wonder whether the flag lied. Either the ceiling is per host:port, and the help is wrong, or the ceiling is per hostname, and "separate limits" in the note is only about the adaptive limit.

**Correction:** say which in the `--max-per-host` help edit (line 18). For example: "The ceiling applies to each host and port separately; SSH and HTTPS to the same host each get their own." If it stays per hostname, the note must not say that the limits are separate. Update the `CLI.md:140` sentence either way.

**Closure check:** the help paragraph and the note wording describe the same unit.

---

### [P3-2] The new `Retry-After` hold note: unbounded line count, and "waiting" is false for a hold longer than 30 s

**Location:** excerpt A line 12 (`note: github.com:443 asked gwz to wait 20 s; waiting`), against row 2 (line 36).

**Defects:**
1. The note prints "once per hold". Limit-change lines are coalesced to one per host per 2 s (line 10), but hold lines have no cap. A host that keeps answering 429 with `Retry-After: 20` across a 40-repo run can print one line per hold, for as long as the run lasts.
2. A `Retry-After` longer than 30 s is not waited out. The members fail (row 2: "asked to wait 120 s, longer than the 30 s one command holds a host; retry in 2 min"). The text does not say whether the note prints for such a `Retry-After`. If it does, it says "waiting", which is false for that run.

**Correction:** coalesce hold notes with the same 2 s rule, or print one per host until the hold ends. State that a hold over 30 s prints a different note, for example `note: github.com:443 asked gwz to wait 120 s, longer than gwz holds a host; members that need it will fail`.

**Closure check:** the design states the cap and the over-30 s wording.

---

### [P3-3] Rows 2 and 3 leave the "attempt N of M" suffix unspecified, and a member that never started should not show an attempt count

**Location:** excerpt B, line 36 (row 2 detail "and `retry_attempt`"), line 37 (row 3 detail, no mention), line 42.

**Defects:**
1. Row 3 (local slot wait expired) says nothing about `retry_attempt` or the suffix. A member that waited 30 s for a slot and never started a connection made no attempt, so a suffix such as `(attempt 1 of 4)` would repeat the "copied attempt count" defect that P3-5 and the Down rows removed.
2. Row 2 carries `retry_attempt`. A member that fails on its first attempt with a `Retry-After` of 120 s reads `(attempt 1 of 4)`, which suggests three attempts are still available. The message explains why not, but the suffix points the other way.
3. Row 3's remedy is "retry later". `--ssh-timeout` bounds how long a stuck setup holds a slot (`CLI.md`), and the message mentions that "setups that stopped answering hold them until they time out". The user can act on it by lowering `--ssh-timeout` and the message does not say so. With `--ssh-timeout 0` the setups never time out, and "until they time out" is then wrong.

**Correction:** state `retry_attempt` omitted for row 3 (no attempt was made). For row 2, either omit it or accept and document that the suffix counts attempts made. Consider appending "or lower --ssh-timeout" to row 3 and noting the `--ssh-timeout 0` exception.

**Closure check:** each row of the table says whether `retry_attempt` is present.

---

### [P3-4] The throttle message wording does not fit a 429 or `--max-retries 0`

**Location:** excerpt B, row 1 (line 35), line 51.

1. A 429 is an HTTP rate answer, and the message says "kept refusing connections". For a 503 with `Retry-After` from a maintenance window the same words and the remedy "lower --max-per-host" do not fit.
2. At `--max-retries 0` the first refusal is final (line 51). The message is `… kept refusing connections; gave up after 0 s; retry later, or lower --max-per-host`. "Kept" and "gave up after 0 s" are untrue for one refusal. The most effective remedy, removing `--max-retries 0`, is missing. The Troubleshooting section will say so, but the message itself will not, and the line 19 docs say `--max-retries 0` turns the adaptive limit off.

**Correction:** in row 1 use "was told to slow down" (or "refused requests") rather than "connections". When the member's budget is 1, say so: `github.com:443 refused the connection; --max-retries 0 turns off retries and the adaptive limit; retry later, or allow retries`.

**Closure check:** the table states the text for the `--max-retries 0` variant.

---

### [P3-5] The one rule decides the code from the last failure only, and a script can see the code flip with the last attempt; the "iff" also does not cover row 5

**Location:** excerpt B, line 31 and row 5 (line 39).

1. A member that was throttled three times and then hit a reset on its last attempt keeps the reset's code and message, with no sign of the throttling. Another member with the same history whose last attempt happened to be a 429 reports `Capacity`. A script that retries on `Capacity` will miss the first member. This is the consequence of the rule that closed P2-3, and it is acceptable, but a script author must be told.
2. The rule says a member fails with `Capacity` "iff its own final failure is" one of the listed cases. Row 5 members made no attempt, so they have no own failure and report "the recorded failure's code". If a recorded failure were ever a `Capacity`, they would break the "iff". The excerpt does not say whether a Down key can record one.

**Correction:** add one sentence to the `MachineOutput.md` `Capacity` entry: "`Capacity` is decided by the member's last failure; an earlier throttle does not make a later reset a `Capacity`." State in §5.3 whether a recorded Down failure can be `Capacity`, and word the rule's "iff" to exempt row 5 or exclude that case.

**Closure check:** the docs sentence exists, and the rule and row 5 agree.

---

### [P3-6] The `Releases.md` entry names an internal code, and the mapping change is not stated in user terms

**Location:** excerpt A line 22; excerpt B line 43 ("an `Io` with status").

**Defect:** `Io` is not documented anywhere users read (evidence 3). A `Releases.md` entry that says "a 429 … that today fails with its current code reports `Capacity`" does not tell a script author which code to stop matching. A 503 with `Retry-After` also changes, and line 43 mentions only the 429.

**Correction:** the release entry should list, per response, the code before and after as a user sees it in `--json` (`error.code`), for 429, 503 with `Retry-After`, and `Retry-After` longer than 30 s. It should state that a bare 503 is unchanged.

**Closure check:** `Releases.md` has a before-and-after table in `--json` terms.

---

## 2. First-day walkthrough (re-run against revision 10)

The user runs `gwz fetch` on 40 GitHub repos, mixing SSH and HTTPS remotes.

1. **`--help`.** `--max-per-host` is "Maximum (ceiling) …" and says gwz may use fewer, with a `note:` line saying so. `--max-retries` has an option entry with its 3 extra attempts (4 in all) and the `--max-retries 0` effect. Small guess: whether the ceiling is per hostname or per host and port (P3-1).
2. **First note.** `note: github.com:443 refused extra connections; using 8 at a time (ceiling 32 from --max-per-host); continuing, no action needed`. It says who, what, that it is fine, and the flag. It is on its own line above the progress block, and the prefix tells them it is not a failure.
3. **Waits.** `note: github.com:443 asked gwz to wait 20 s; waiting`. They know it is a wait, not a hang. If this repeats, they may see many such lines (P3-2).
4. **Later notes.** `now using 12 connections`, then `back at the ceiling of 32 connections`. Clear.
5. **A member fails.** `failed  Capacity: member 'mem_core' at 'gwz-core': github.com:443 kept refusing connections (using 8 at a time, ceiling 32 from --max-per-host); gave up after 21 s; retry later, or lower --max-per-host (attempt 4 of 4)`. They know the member, the host, the limit, the remedy and that 4 of 4 attempts were used. Guess left: "connections" for what may be a rate limit (P3-4).
6. **A script.** `.errors[] | select(.code=="Capacity")` then `detail.capacity.cause` separates server throttle, long `Retry-After` and local. One rule decides the code, it is stated once and documented in `MachineOutput.md`. The trap that remains is that an earlier throttle followed by a reset reports the reset (P3-5), and that the release note does not name the old code (P3-6).
7. **Local stall.** `gwz's 64 connection-setup slots stayed full for 30 s (setups that stopped answering hold them until they time out); retry later`. Not their `--jobs`, and the text says why. They may wonder whether `--ssh-timeout` helps (P3-3).
8. **Down host.** `note: github.com:22 is not answering; waiting up to 30 s to retry`, then `not attempted: github.com:22 failed earlier in this run (<reason>)`, with the real reason. No guessing.

---

## 3. Risks and next action

- **Residual risk.** None of the six P3 items touches a wire name or an error code, so each can be fixed after release without a compatibility break. The ones worth fixing with the code are P3-1 (the help must state the unit of `--max-per-host`) and P3-6 (a release note that does not name the old code defeats the compatibility record the design added).
- **Not examined.** The algorithm, and the definitions of Throttle, Overload, SATURATED and Down. I took §3.2's "a 429, or a 503 carrying `Retry-After`" as given and judged only whether the surface text contradicts itself. I did not check the cited source lines.
- **Next action for the lane owner.** File this verdict. Fold P3-1 to P3-6 into the doc and wording changes the design already lists in §9, or record them as open P3 items with owners. No re-review is needed unless a wire name or the one rule in §5.3 changes.

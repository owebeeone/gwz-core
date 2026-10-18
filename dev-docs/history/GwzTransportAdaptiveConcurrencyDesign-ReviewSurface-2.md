# GWZ transport adaptive concurrency design, revision 9 — SURFACE-AXIS RE-REVIEW

**Review object:** `/Volumes/projects/limbo/gwz-dev/gwz-core/dev-docs/GwzTransportAdaptiveConcurrencyDesign-SurfaceExcerpt-2.md`, sha256 `ee66d48aa00027983f64883e2a76a039ca420ba32ca1cba04eabc8815936d531`, verified at start and at end. It names design sha256 `8354166a9a355cf694b35bbeb853978950cb7cdadc47cb16ea6c5b719ff8f3b7`. I did not open that design. Round-1 object: `GwzTransportAdaptiveConcurrencyDesign-SurfaceExcerpt.md`, sha256 `691e0820…`.
**Baseline:** installed gwz 1.0.17 (`gwz help fetch`, `gwz --help`) and the docs in `gwz-cli/docs/`: `CLI.md`, `MachineOutput.md`, `Troubleshooting.md`, `Concepts.md`, `MergeRecovery.md`, `ClaudeCode.md`, `commands/fetch.md`, `commands/auth.md`. No design, plan, review or source files were read.
**Date:** 2026-10-07
**Axis:** SURFACE, the interface as the person using it meets it. Re-trace of round-1 findings P2-1, P2-2 and P3-1 to P3-6, plus an attack on the changed text. Independent, adversarial, read-only. Filed verbatim by the lane owner.

**Verdict: NO-GO** — 0 P0, 0 P1, 1 P2 (new), 6 P3. The round-1 P2s are closed in substance. One new P2 comes from the revised case table, which is internally contradictory about which server responses become `Capacity`. I pre-commit to GO on a revision that resolves P2-3 as specified. The P3 items are wording and docs work and do not block.

---

## Prior-finding closure table

| Round-1 ID | Disposition | Where the revision answers it, and what remains |
|---|---|---|
| **P2-1** `Capacity` and `throttle` reused for unrelated causes | **Closed**, with a residual tracked as **P2-3** | §5.3 now has a table of every case with code, message and detail. The detail is renamed `capacity` (after the code it explains) with `cause`: `server_throttle`, `server_retry_after` or `local`, always first (lines 31–42). A plain refusal keeps its current code at any `--max-retries` (row 4, line 40). `MachineOutput.md` is committed to document the code and each `cause` (line 20). Residual: row 4, row 1 and the `--max-retries 0` paragraph give three different definitions of which server responses are throttle evidence. |
| **P2-2** field vocabulary | **Closed** | `attempts_max` is removed. Attempt counts live only in the existing `retry_attempt` `{attempt, attempts}`, "M = `--max-retries + 1`", documented in `MachineOutput.md` (lines 20, 46). `limit_configured` and "requested" are now `max_per_host` (named after the flag) and "ceiling". `limit_observed` is now `limit_applied`, defined as "gwz's adaptive limit, not a number the server stated" (line 43). One word left to define: see P3-4. |
| **P3-1** note wording | **Closed** | New wording says who acted, that nothing is wrong, and the flag, and no longer says "requested" or "lifted" (lines 9–11). Residual wording issues: P3-5. |
| **P3-2** `gwz:` prefix | **Closed** | Notes use `note: ` and the text states that `gwz: ` is reserved for failures (line 8). `warning:` is the existing informational precedent (`MergeRecovery.md:15`), so `note:` is consistent. The docs list does not say that `note:` is a documented stderr line shape: P3-5. |
| **P3-3** vacuous `--quiet` | **Closed** | "only `--json` and `--jsonl` suppress them (no network command has `--quiet`)" (line 8). I confirmed `--quiet` is a `gwz diff` flag only. |
| **P3-4** error text over-claims, no remedy, variants unspecified | **Mostly closed**, residuals in P3-1, P3-2, P3-3 | Each case now has full text after the member prefix, with one remedy (lines 33–35). The parenthesis is dropped when the limit was never lowered. The local row names `--jobs` (see P3-3). Residual: "accepted" and the duplicate counts. |
| **P3-5** "retest" jargon, copied attempt counts | **Mostly closed**, residuals in P3-2 | A member that made no attempt now reports `not attempted: …` with no `retry_attempt` (row 5). A failed retest carries a suffix and no attempt above `M` (row 6). "retest" is gone from the text. Residual: the new strings over-claim, see P3-2. |
| **P3-6** help and docs mislead; nothing discoverable | **Closed as a commitment** | Four docs changes are committed in the same change as the code: the `--max-per-host` help sentence, a `--max-retries` option entry (default, throttle bound, `--max-retries 0` turns the adaptive limit off), a Troubleshooting section, and the `MachineOutput.md` entries (lines 17–20). Residual: P3-6. |

---

## 0. Evidence base

Facts about the existing surface that the findings rely on (unchanged from round 1 unless noted):

1. **Installed 1.0.17.** `--max-per-host` defaults to 8 and `--jobs` to 50. `--max-retries` is rejected as an unexpected argument. The repo docs (`CLI.md`) describe the candidate: `--max-per-host` defaults to 32 and `--jobs` to 100. No doc contains the number 64 (grep of `CLI.md`, `Concepts.md`, `Troubleshooting.md`, `MachineOutput.md`).
2. **`--max-retries` is documented only in prose** inside the `--ssh-timeout` paragraph and in `commands/auth.md:60` ("`--max-retries` has no effect" on the native transport). Round 1 found no option entry. The design now commits to one (line 18).
3. **`Capacity` appears nowhere in the user docs.** The design now commits to documenting it (line 20).
4. **Existing human failure row** (`commands/fetch.md`): `failed  RemoteRejected: member 'mem_priv' at 'private': failed to connect to 127.0.0.1: Connection refused`. So the table's "after the existing `member '<id>' at '<path>': ` prefix" matches the docs.
5. **`MachineOutput.md`** gives the seven-field error and shows `"detail": null`. It does not document `retry_attempt` or the "attempt N of M" suffix. The design commits to adding them.
6. **Stderr.** `Concepts.md:235`: human mode renders live progress to stderr when stderr is a terminal. `gwz:` is the failure prefix (`MergeRecovery.md:63`, `ClaudeCode.md:343`, `CLI.md:1711`).
7. **Excerpt D (lane-owner summary, line 59)** still says "one optional field (`throttle`) in a failure's detail". The design now calls it `capacity`. The extract says D is "unchanged from the revision-8 extract".

---

## 1. Findings

### [P2-3] The cases that become `Capacity` are defined three ways, and the same server response gets different codes depending on `--max-retries`

**Location:** excerpt B, row 1 (line 33), row 4 (line 36), line 40, and the `--max-retries 0` paragraph (line 48).

**Surface rule violated:** an error code that users script against must have one stated mapping from cause to code. Changing it after release is a compatibility break. Round 1 required a table of every case. The table exists, but its rows disagree with each other.

**Root cause:** three statements define throttle evidence differently.
1. **Row 1:** `Capacity` when "the member's budget ran out while the **key** had a Throttle or a confirmed overload during its attempts". That is a property of the shared host key, not of this member's own failures.
2. **Row 4:** a refusal is "unchanged" only if there is "**no 429 or 503** and no confirmed overload on the key". So any 429 or any 503 is evidence, and (by the rule in line 40) must not stay unchanged.
3. **`--max-retries 0` paragraph:** "a 429 or a **503 with `Retry-After`** is a server throttle (`Capacity`), anything else keeps its code". So a 503 without `Retry-After` keeps its code.

Statements 2 and 3 directly contradict each other for a bare 503. Statements 1 and 2 disagree about what counts (key state versus the member's own signals).

**User scenarios:**
1. GitHub returns a bare 503 during an outage. At `--max-retries 3` (row 4 says a 503 is evidence) the member reports `Capacity: … kept refusing connections …; retry later, or lower --max-per-host`. At `--max-retries 0` (line 48) the same 503 keeps today's code. A script that switches on `Capacity` now behaves differently by retry flag, and an outage is described as a connection-capacity problem with a remedy (lower `--max-per-host`) that cannot help.
2. Member A resets because of its own flaky link while another member earlier hit a confirmed overload on the same host. Under row 1 it is labelled `server_throttle`: "kept refusing connections". Under row 4 it is a plain refusal. The user cannot tell which, and the `cause` field is the thing a script relies on.
3. Existing scripts that match today's code for a 429 stop matching. The text says `Capacity` is "unchanged as a code" (line 40), which is true for the name only. The mapping for 429/503 is a behaviour change, and none of the docs listed in line 16 or `Releases.md` records it.

**Impact:** the case-to-code mapping is the interface. It is what a later compatibility break would have to reverse.

**Required correction:**
1. Replace the three statements with one rule, applied identically at every `--max-retries` value. For example: "`Capacity` with `cause: server_throttle` iff the member's own final failure was a 429 or a 503 carrying `Retry-After`, or a refusal on a key whose overload the transport confirmed. A bare 503 without `Retry-After` is not throttle evidence and keeps its code." The wording and the exact rule are the lane owner's to decide, but row 1, row 4 and the `--max-retries 0` paragraph must say the same thing.
2. State whether row 1's condition is about the member's own signals or the key's state.
3. Add a "was RemoteRejected (or whatever it was today), now Capacity" line to `Releases.md`, and to the `MachineOutput.md` entry for `Capacity`.

**Closure check:** one definition appears once in the design and once in `MachineOutput.md`. A matrix of {429, 429 with `Retry-After`, bare 503, 503 with `Retry-After`, reset, connect refused, stall} by {`--max-retries` 0, default, higher} gives one code each, and the same code across the three retry values except where `--max-retries 0` is stated to differ and why.

---

### [P3-1] Row 1 prints the attempt count inside the message, and the existing "attempt N of M" suffix is also rendered from `retry_attempt`

**Location:** excerpt B, row 1 (line 33): "`gave up after 4 of 4 attempts over 21 s`", with detail "`capacity` … and `retry_attempt`"; line 46; line 20.

**Rule violated:** one fact is stated once. The design says attempt counts are carried "once" in `retry_attempt` (line 46), and the table row repeats them in the message.

**Scenario:** the human row carries both the message text "4 of 4 attempts" and, for any rendering that appends the documented "attempt N of M" suffix from `retry_attempt`, a second "attempt 4 of 4". The remedy sits mid-line between the two. Whether the suffix is part of the message column or appended by the renderer is not stated.

**Correction:** state whether the "attempt N of M" suffix is appended to the row-1 message. If it is, drop "4 of 4 attempts" from the message text. If it is not, say so and keep the count in the message only.

**Closure check:** one full example human row for row 1 appears in the design and in the Troubleshooting section.

---

### [P3-2] The Down-key strings over-claim and drop the recorded cause

**Location:** excerpt A line 12; B rows 5 and 6 (lines 37–38); C line 53.

**Defects:**
1. Row 5's message `not attempted: github.com:22 is unreachable (4 of 4 attempts failed earlier in this run)` carries the recorded failure's code but not its reason. A user sees `unreachable` whether the recorded failure was a refused connect, a stall, a setup timeout or an authentication timeout. "Unreachable" is false for the last two. The code and the message can disagree.
2. Row 6 says "a retry **30 s later**", a fixed figure. The note says "waiting up to 30 s", and the wait is bounded by 30 s, not equal to it.
3. The note says `unreachable` for every Down episode. A key goes Down after setup failures, which include timeouts on a slow host.

**Correction:**
- Row 5: `not attempted: github.com:22 failed earlier in this run (<recorded reason>; 4 of 4 attempts)`. Keep `unreachable` only when the recorded failure was a connect failure.
- Row 6: "after a retry about 30 s later", or use the measured seconds.
- Note: `github.com:22 is not answering; waiting up to 30 s to retry`.

**Closure check:** each Down-related string states its cause category and is true for every recorded failure the key can hold.

---

### [P3-3] The local row's "64 jobs in use … lower --jobs" does not connect to any documented number

**Location:** excerpt B, row 3 (line 35).

**Surface rule violated:** a remedy must be actionable from the docs.

**Scenario:** the user never passed `--jobs`. `CLI.md` says `--jobs` defaults to 100 (50 on the installed binary), and "64" appears in no doc. They read "64 jobs in use; lower --jobs" and cannot tell whether 64 is their `--jobs`, a separate internal budget, or a number they can change. If the 64-job budget is a separate permit pool, then lowering `--jobs` to 50 may or may not help.

**Correction:** state in the design how the 64 relates to `--jobs`. If it is the effective `--jobs`, say so in the message (`--jobs 64`). If it is a separate budget, name it and give a remedy that works. Document the number in the Troubleshooting section.

**Closure check:** a user who sets the remedied flag can reach a state where this row no longer occurs, and the docs show the arithmetic.

---

### [P3-4] Smaller wording and naming inconsistencies in the new strings

**Location:** excerpt A lines 9–12; B lines 33, 41–46.

1. **"accepted".** Row 1 says "8 at a time accepted". The JSON text says `limit_applied` is "gwz's adaptive limit, not a number the server stated". The human text still presents the number as something the server accepted. Suggest "using 8 at a time".
2. **Host versus host:port.** The overload notes name `github.com` (no port). The Down note and the error rows name `github.com:22` or `:443`. SSH and HTTPS to github.com are different keys, so a mixed-scheme workspace can print two identical-looking `github.com refused extra connections` lines, and the user cannot tell which protocol each is about.
3. **"ceiling" is a human-text word only.** The note says "ceiling 32, --max-per-host". The new help sentence and the JSON use `--max-per-host` and `max_per_host`. Add "ceiling" to the `--max-per-host` help (for example "Maximum (ceiling) …") so the word in the note is findable. In the note, "(ceiling 32, --max-per-host)" reads as a flag with no value. Suggest "(ceiling 32 from --max-per-host)".
4. **`attempts`.** Within `retry_attempt` `{attempt, attempts}`, `attempts` is the allowed total (`M`), not the number made. The `MachineOutput.md` entry must define it explicitly. A reader sees `attempts: 4` after a member that made 2.
5. **`last_signal: "local"` repeats `cause: "local"`.** Harmless, but redundant. Say that `last_signal` is absent for `cause: local`, or keep it and document why.
6. **`cause` collides with an existing term.** Line 47 cites an existing `SetupFailureCause` enum. If that appears in user-visible JSON, two unrelated `cause` fields exist. Check before shipping.

**Closure check:** the docs list in line 16 includes definitions for items 2, 4 and 5.

---

### [P3-5] Placement and silence rules for the notes are still partly unspecified

**Location:** excerpt A lines 8, 13.

1. "between progress lines when progress is shown" does not say how a note coexists with a live progress renderer on a terminal. `Concepts.md:235` says progress is rendered live to stderr when stderr is a terminal. A line printed into a redrawn status block can be overwritten or leave a gap.
2. "A hold … print[s] nothing" and "A Queue wait prints nothing" stand. A `Retry-After` hold of up to 30 s per hold looks identical to a hang. Only a Down key now gets a "waiting" note. A throttled host with a long `Retry-After` is silent until a member fails.
3. The `note:` line shape is not in the docs list (line 16). Add one sentence to `CLI.md` or `Concepts.md` that says `note: ` lines on stderr are informational and `gwz: ` lines are failures.

**Closure check:** the design states the rendering rule for a TTY, and the docs name the `note:` shape.

---

### [P3-6] Doc commitments are good, with three gaps

**Location:** excerpt A lines 16–20.

1. The existing `--ssh-timeout` prose (repeated per command in `CLI.md`) describes retries as "failed setup attempt" waits only. It is not on the list of docs to change, and it will contradict the new `--max-retries` entry (throttled connections are retried within the same budget).
2. The `Troubleshooting.md` entry is named, but the existing "SSH Or Credential Failure" advice ("Use `--jobs` and `--max-per-host` to reduce concurrency against a host") still needs a pointer, because gwz now lowers the limit by itself.
3. D, the lane-owner summary, still says "one optional field (`throttle`)" (line 59), and "no new JSON/JSONL fields in default output". The field is `capacity`. If that summary is copied into release notes, it will name the wrong field. Not design text, but it will be read by the next reviewer.

**Closure check:** the shipped help, `CLI.md` and `Troubleshooting.md` agree on one description of retries and the limit, and release notes name `capacity`.

---

## 2. First-day walkthrough (re-run against revision 9)

The user runs `gwz fetch` on 40 GitHub repos.

1. **`--help`.** Still says "Maximum …". With the new sentence they learn gwz may use fewer. They find `--max-retries` as an option. Better than round 1. (Skew note: the installed 1.0.17 help predates all of this, 8 and 50 versus 32 and 100.)
2. **First note.** `note: github.com refused extra connections; using 8 at a time (ceiling 32, --max-per-host); continuing, no action needed`. It says who, what, that it is fine, and the flag. They no longer have to guess. Small guess: whether "github.com" means SSH or HTTPS (P3-4).
3. **Waits.** A 429 `Retry-After` hold is silent (P3-5). The user cannot tell a throttle wait from a hang, until a Down note or a member fails.
4. **Later notes.** `now using 12 connections`, then `back at the ceiling of 32 connections`. Clear.
5. **A member fails.** `failed  Capacity: member 'mem_core' at 'gwz-core': github.com:443 kept refusing connections (8 at a time accepted, ceiling 32); gave up after 4 of 4 attempts over 21 s; retry later, or lower --max-per-host`. They know the member, the host, the budget and the remedy. Guesses left:
   - why "attempt 4 of 4" if `--max-retries` is 3 (the docs entry explains, "3 extra attempts, so 4 in all");
   - whether the count is repeated (P3-1);
   - whether a bare 503 outage is the same thing as a throttle (P2-3).
6. **A script.** `.errors[] | select(.code=="Capacity")` then `detail.capacity.cause` tells server throttle from retry-after from local. Good. The remaining trap is that the codes assigned to 429 and 503 depend on `--max-retries` and the key's state (P2-3).
7. **Next step.** The remedy "lower --max-per-host" is actionable. `--max-retries 0` is documented as turning the adaptive limit off. "(64 jobs in use); lower --jobs" is not actionable from the docs (P3-3).
8. **Down host.** `note: github.com:22 is unreachable; waiting up to 30 s to retry`, then members report `not attempted: …` with no reason (P3-2).

---

## 3. Risks and next action

- **Risk surviving the verdict.** P2-3 is the only wire-level commitment still open: the cause-to-code mapping for 429 and 503. The rest are strings and docs, fixable at any time, but P3-3 (a remedy the docs cannot back) and P3-2 (messages that can contradict their code) are worth fixing with the code, because a user will paste exactly those lines into a bug report.
- **Not examined.** The algorithm, the cited sources (`protocol.rs:851-857` and others), and the Overload, Throttle, Suspect and SATURATED definitions. I take the table's words as the surface and judged only whether they contradict each other.
- **Next action for the lane owner.** Rewrite the throttle-evidence rule once and make rows 1, 4 and line 48 quote it (P2-3). Fold in the P3 string fixes, namely the single attempt count (P3-1), Down-key reasons (P3-2), the `--jobs` relation (P3-3) and the small naming items (P3-4). Update D's `throttle` to `capacity`. Resubmit the revised table and I will give a GO as pre-committed above.

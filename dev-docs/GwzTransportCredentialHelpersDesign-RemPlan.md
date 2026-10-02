# GWZ transport credential helpers design (TR1.6) — remediation plan 1

Date: 2026-10-02.

**Object.** Revision 1 of the draft, SHA-256 `49b1a19429eab50dba857ad279d6bb0e3b468f597c587173cf36b5db5110a6da`. A frozen copy is at `GwzTransportCredentialHelpersDesign-rev1.md`.

**Reviews.**

| Review | Verdict | P2 | P3 |
|---|---|---|---|
| [Consistency](GwzTransportCredentialHelpersDesign-ReviewConsistency.md) | GO | 0 | 7 |
| [Safety](GwzTransportCredentialHelpersDesign-ReviewSafety.md) | NO-GO | 1 | 5 |
| [Surface](GwzTransportCredentialHelpersDesign-ReviewSurface.md) | NO-GO | 4 | 10 |

Safety and Surface each pre-committed to GO on a revision that resolves their P2s as specified.

**The rule for revision 2.** It applies every disposition below in one patch. Each blocking finding closes only when the reviewer who raised it re-checks its counterexample against revision 2. The Consistency P3s close on the same re-check.

## Blind convergence

Four sets of findings came from different reviewers independently:
- **M4's text and bound,** from all three axes:
  - Consistency P3-6: `<n>` is not the bound applied;
  - Surface P3-2: M4 names a bound no user can find, and gives no next action;
  - Safety P3-1: the permit wait is charged to the allowance, so M4 can be false and the latch can fire on a lookup that never ran.
- **The `Capacity` outcome,** from Consistency P3-4 and Safety R6.
- **Prompting and the terminal,** from Surface P2-2 (the prompting policy is unstated) and Safety P3-2 and P3-3 (the latch under OQ3, and the terminal claims).
- **M2's next action,** from Surface P2-1 and P3-6 (the `gh` loop, and an example that cannot run on clone) and Surface P2-4 (configuration scope).

## Decisions taken in this plan

These are the lane owner's, and the operator can reverse any of them.

- **E1. URL text reaches git only inside the percent-encoded `url=` line** (Safety P2-1). Under OQ1 (b), the URL's username goes to `git credential fill` only inside the `url=` line, percent-encoded exactly as the parsed URL serializes it. It never goes as a decoded `username=` line, and is never re-serialized from a decoded field without re-encoding. The destination also refuses a username or path that decodes to any control character, before any Open.
  - §2.2 replaces "cannot add a line" with the real bound: the endpoint writes U percent-encoded as parsed, and git refuses a component that decodes to a newline. §10 gains the `%0A`, `%3A` and `%0D` rows.
  - OQ1's options are restated with this form.
- **E2. Each outcome's message is reachable from its own text** (Surface P2-2). §11 states that M2 covers git's "could not read Username/Password" result: no helper answered, and prompting is unavailable or disabled. That holds however `git credential fill` reports it, with a non-zero exit and no fields. M8 covers only a spawn or pipe error, oversized output, and malformed output, each named as its own cause class (Surface P3-7). The notes state the prompting policy that OQ3 decides, with the text for each option.
- **E3. Codes** (Surface P2-3). §11 states, for each of M1–M8, the error code and the facts it carries (`credential_offered`, `authenticated`, the setup cause), in snake_case and PascalCase. A new code is allocated only where neither the existing codes nor the existing facts let a driver tell apart two outcomes with different recoveries. Any allocation is listed with the error catalog's new rows and its protocol-schema change, and goes to the operator as a surface decision.
  - Why: the catalog's own rule allocates a code for outcomes a driver cannot tell apart, and a later allocation would change an outcome's code.
- **E4. Configuration scope is stated, as in 1.0.17** (Surface P2-4).
  - The lookup reads git's system, global and XDG files and `GIT_CONFIG_*`. It reads no repository-local file, and no `includeIf "gitdir:…"` applies.
  - 1.0.17's `Config::open_default()` read no repository configuration either, and libgit2 skips conditional includes without a repository, so this is parity. The notes say so, and give the remedy: put the credential settings in the global file.
  - M2's text names "a helper in your global git configuration".
- **E5. M2 and M3 name a helper-aware next action** (Surface P2-1 and P3-6). The generic step is `git ls-remote <the member's URL>`, which works before the member is cloned. Each message also says that a sign-in helper renews its credential with its own tool, for example `gh auth login` or `gh auth refresh` for `gh`. N8 carries the same sentence.
- **E6. The permit wait is not charged to the interaction allowance** (Safety P3-1, Consistency P3-6, Surface P3-2). The wait for the endpoint's permits is charged to the Open's allocation deadline, and the allowance starts when the lookup starts.
  - M4's `<n>` is the bound applied to that lookup: the allowance, or the Open's interaction deadline when shorter.
  - M4 gains a next action ("sign in once with git, then retry"). It names the bound as fixed, or names the setting that changes it if one exists.
  - A permit wait that runs out has its own §4 row and message, and never sets the latch.
- **E7. The latch** (Safety P3-2). §8 moves the latch into the list of the design's own choices that can break a working setup, conditional on OQ3 (a) or (b). The latch is recommended to apply only under OQ3 (c), where a timeout means a hung helper and not a slow user. OQ3's text and §8 say the same thing.
- **E8. Completion and buffers** (Safety P3-4 and P3-5). The lookup completes when git exits with stdout at EOF; the stderr reader is then abandoned within the same bound. The stdout and stderr buffers are zeroize-on-drop types, pre-sized to the limit, so every exit path zeroizes, abandoned branches and unwinds included.
- **E9. A `Capacity` row** (Consistency P3-4 and Safety R6). §4 gains a row for retained unreaped `git` processes holding the host slots, with its message and a §10 row. Alternatively, §3.3 awaits the host slots within the allocation deadline, as the reuse design does for 1.2.0, and says so.

## Finding dispositions

| Finding | Disposition | Closure |
|---|---|---|
| Safety P2-1 | E1 | Re-read, plus §10's `%0A`, `%3A` and `%0D` rows |
| Safety P3-1 | E6 | A row with nine members over two hosts, as the finding gives it |
| Safety P3-2 | E7 | Re-read |
| Safety P3-3 | §3.2's terminal sentence is scoped to processes in the spawned group. OQ3 and N4 state that a helper prompting through another session's agent, such as gpg-agent's pinentry, is neither stopped nor killed and may stay after M4, as on 1.0.17 | Re-read |
| Safety P3-4 | E8 | Re-read, plus the buffer type confirmed at implementation |
| Safety P3-5 | E8 | A fixture helper with a background child holding stderr returns its credential within one second, with no M4 |
| Safety R2, R3, R4 | N3: a relative helper runs relative to `/`, so `!./bin/x` starts `/bin/x`. N2: a helper 1.0.17 never ran may answer with another account's valid credential. N4 and N7 gain one sentence on a redirect making a sign-in window appear for a host the user did not name | Re-read |
| Surface P2-1 | E5 | The walkthrough's case 2 reaches `gh auth login` from the text alone |
| Surface P2-2 | E2 | The walkthrough's case 4 lands on M2 |
| Surface P2-3 | E3 | §11 states a code and facts for every message, and any new catalog rows |
| Surface P2-4 | E4 | The `includeIf` walkthrough reaches the remedy |
| Surface P3-1 | The notes are written as release text. They carry no D, OQ, TR, S or § tokens; a separate table maps each note to its D rows. Where an open question decides a note, it shows the text for each option. N10 names the switch through TR1.5's hint text: `--transport native` / `GWZ_TRANSPORT=native` under TR1.5's remediation plan D2. N7's Windows sentence gains its remedy | A grep of the notes for those tokens finds nothing |
| Surface P3-2 | E6 | M4 contains a command |
| Surface P3-3 | M6 names the offered schemes and gains `<hint>` | Re-read |
| Surface P3-4 | M5 ends with "point the remote at the URL the server redirects to, then retry" | Re-read |
| Surface P3-5 | M7 names what disables helpers, the API option gwz-py exposes and any CLI form, and how to re-enable them | Re-read |
| Surface P3-6 | E5 | Re-read |
| Surface P3-7 | E2 | Re-read |
| Surface P3-8 | M3 covers a 401 after the credential. A 403 gets its own message, which carries the HTTP status and the server's reason line when present, never a credential | A 403 row in §10 |
| Surface P3-9 | Main is now gwz-core `ab48966f`, where TR2.19 has merged: `errors` lists Failed and Rejected members too, and MachineOutput.md and gwz-py's README now say so. §11 cites the merged text | Re-read |
| Surface P3-10 | §11 lists the pages that change: ErrorCatalog row 32's cause and recovery, and any rows E3 allocates; a Troubleshooting section on HTTPS credential failure, quoting M1–M8 with the recovery for each kind of helper; and Install.md and QuickStart, which name `git` on `PATH` for private HTTPS. M2 and M3 end with that Troubleshooting page's name | Re-read |
| Consistency P3-1 | C3 lists every line the reviewer found, with its replacement: the 401/404 texts at REQ :219, Placement :330-332 and HTTPS §10 :423-424; HTTPS :227; DES :157, :208, :492 and :628; REQ :50 and :76; RETRY :171-172; Plan :36; V110 :84 and V110-amendment :33 and :121; GWZDesign :70-72; GWZRequirements :84-86; Placement :423; and ReleaseReadiness :19 and :65 | The reviewer's greps return only lines in C3's list |
| Consistency P3-2 | §11's docs list gains `gwz-core/docs/TransportPlacement.md:105-109` and its replacement | Re-read |
| Consistency P3-3 | §9.1 gains the contingency for OQ1 (b): HTTPS :148 reads "Pass the URL's username only inside the percent-encoded `url=` line (E1); pass no token argument." A contingency is stated for every open question that changes an amended sentence | Re-read |
| Consistency P3-4 | E9 | The §10 row |
| Consistency P3-5 | Four §10 rows: unusable output (M8, over 16 KiB and malformed); the timeout latch (a second member on the host fails at once, with no spawn); the anonymous POST 401 (its member error text, with no lookup); and the decoded `path` under `useHttpPath` | The rows |
| Consistency P3-6 | E6 | T5(a) asserts that M4 names 2 seconds |
| Consistency P3-7 | OQ4 gains option (d), parity on the same origin: follow a same-origin redirect of the authenticated discovery and resend the held credential to the new path, as libgit2 does. Another origin asks afresh, as under (a). Its hazard: the credential goes, unasked, to a path the server chose on the same origin | OQ4 lists (d) |

## For the lane owner, not this revision

- **C7.** TR2.2's fix may live inside the `gh` spawn that TR1.6 deletes.
- **C8.** TR2.20 and TR2.22 touch the same code in `https_worker`.
- **C2.** "One validated discovery redirect" against the five that discovery follows.

These go to amendment 2's next revision and to the lane schedule. TR2.20 starts now, on main after the split, and TR2.22 rebases on it.

## Operator questions after revision 2

OQ1 to OQ4, with OQ4's new option (d), plus any code allocation from E3.

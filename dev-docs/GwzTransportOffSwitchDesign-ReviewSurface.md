# GWZ transport off switch design (TR1.5) — SURFACE-AXIS REVIEW

**Review object:** `/private/tmp/claude-501/-Users-owebeeone-limbo-gwz-dev/351b18f9-4ec1-4306-ac0a-299e9bded6dd/scratchpad/designs/GwzTransportOffSwitchDesign.md`, SHA-256 `73d63a6f1714c1e73acdf898716f4dc2aae1fd232e7004e6754dc5b3765279f7`, verified at 11:24:35 and again at 11:33:00 AEST (unchanged). §10 only (file lines 142–163, "The surface this freezes"), read 2026-10-02.
**Baseline:** installed `~/.cargo/bin/gwz` reports `gwz 1.0.17`. Read for comparison: `gwz --help`, `gwz help`, `gwz help fetch|push|pull|clone|status|auth|auth identity`, `gwz fetch --help`, `gwz --json help`, `gwz --json help clone`; `gwz-cli/docs/MachineOutput.md`, `Troubleshooting.md`, `Concepts.md` (network/URL-scheme section), `CLI.md` (dev-tree global-options block, which already carries 1.1.0's `--ssh-timeout` text), `commands/auth.md`, `commands/clone.md` and `commands/materialize.md` (URL-scheme rows), `docs/README.md`, `gwz-cli/README.md`, `gwz-py/README.md`. `gwz-py` is not installed on this machine and `gwz-py/docs/` does not exist. One read-only probe of the installed binary: `GWZ_URL_SCHEME=bogus gwz --root ./no-such-workspace --dry-run materialize --lock` (nonexistent root; nothing created).
**Date:** 2026-10-02
**Axis:** SURFACE — the interface as the person using it meets it, from §10 read as shipped help, messages and machine schema, against the existing CLI's help and user docs. Independent, adversarial, read-only. Other axes run in parallel; nothing here relies on them. Filed verbatim by the lane owner.

**Verdict: NO-GO** — 0 P0, 0 P1, 3 P2, 6 P3. I pre-commit to GO on a revision that resolves P2-1, P2-2 and P2-3 as specified below; the P3s have bounded text remedies and do not gate.

---

## 0. Evidence base

What §10 freezes, as read cold:

- Flag `--transport <on|off>`, global, "listed beside `--ssh-timeout`", not in `gwz --help`'s short list. Short help: "Use gwz's transport for network operations (on, the default), or libgit2's native path as gwz 1.0 did (off)". Long help states precedence (flag > `GWZ_TRANSPORT` > `gwz.transport` in global git configuration), names the setting command `git config --global gwz.transport off`, says workspace/repository-scope values are "ignored, with a note", and ends "While the transport is off, each command that uses the network says so once."
- Human lines: the note `gwz: note: the transport off switch is on (<source>): … ; --transport on overrides it`; the ignore note; five refusals (exit 2, `invalid_request` in gwz-py); the `--verbose` line `transport off switch: <on|off> (<source>)`; the other-refusals hint `run with --transport off to use libgit2's native path, as gwz 1.0 did`.
- JSON: `meta.transport_off_switch = {"on": bool, "source": "flag"|"environment"|"user_configuration"|"default", "file": string|null, "ignored": [{"scope": "workspace"|"repository", "file": string}]}`.

What the existing surface establishes (1.0.17 binary and user docs):

- Global options are value-taking with the default in the description text, not in the JSON help's `defaults` (`--ssh-timeout` "Defaults to 3", `--url-scheme` "manifest (as written, default)"); enum options render `[possible values: …]`. §10's short help "(on, the default)" matches this.
- The flag/env pair precedent is `--url-scheme`/`GWZ_URL_SCHEME`: help says "the flag wins", names the persistent form (`.gwz/url-scheme.yml`) **and its clearing** ("an explicit `manifest` clears it"). Troubleshooting's refusal for the unreadable persistent file ends "; delete or repair the file".
- gwz already keeps a per-repository setting in git config through a verb with a get/set/unset lifecycle: `gwz auth identity <remote> [--set PATH|--unset]` (`commands/auth.md`). The same page states, for a process-level setting, when Python applications must set it ("before creating a native backend").
- Env refusal style, probed on the binary: `gwz: GWZ_URL_SCHEME must be manifest, ssh or https, not "bogus"`, exit 2, printed as plain text even under `--json`. §10's `GWZ_TRANSPORT must be on or off, not "<value>"` matches.
- Machine-output conventions (`MachineOutput.md`): presence rules are stated per key (`meta.transport` "optional"; `crash_recovery` "omitted as a whole on any response that made no such decision", inner keys explicit `null`; "repeated fields remain arrays, including when empty"); existing source vocabularies are `url_resolution.source: request|workspace|default` and `meta.transport[].selection_source: ambient|invocation_remote|invocation_default|local_configuration`; the root/member distinction is `target_kind: Root|Member`.
- "transport" already has a user-facing meaning: `meta.transport` is the list of remote **authentication attempt** rows, `--verbose` is "Show one transport diagnostic for every remote authentication attempt", `commands/auth.md` says "Transport results report the credential method…". These rows exist in 1.0.17, i.e. on the native path.
- "native" is already a user-facing word for the libgit2/libssh2 layer (`commands/auth.md`: "the native connection/read timeout", "a native backend").

## 1. Findings

### [P2-1] The reported state inverts the flag's polarity: `--transport off` is reported as `"on": true` and "the transport off switch is on"

**Location (§10):** the note — `gwz: note: the transport off switch is on (<source>)`; the `--verbose` line — `transport off switch: <on|off> (<source>)`, where `<source>` "may also be `--transport on`, `GWZ_TRANSPORT=on` or `default`"; the `--verbose` long help — "It also shows whether the transport off switch is on, and which form set it."; JSON — `meta.transport_off_switch` is `{"on": bool, …}`. Against, in the same section: short help "(on, the default) … (off)", the git key `gwz.transport off`, and the long help's own words "While the transport is off, each command that uses the network says so once."

**Violated expectation:** one word, one meaning, across the flag, the variable, the git key, the messages and the machine field. The flag's `on` means gwz's transport is used; the JSON's `on` means it is not. The long help describes the state directly ("the transport is off"); the note, the verbose line and the JSON describe the state of an "off switch", which negates it.

**Scenario:** a user runs `gwz --transport off --verbose fetch` and reads `transport off switch: on (--transport off)` — "on" and "off" for one state on one line. The next day, with nothing set, `gwz --verbose fetch` prints `transport off switch: off (default)`: is the transport off? A script author writes `if meta.transport_off_switch.on:` to detect gwz's transport, because `--transport on` is the flag they passed, and gets the opposite branch. A support request quoting "the off switch is on" needs a second message to establish which path ran.

**Impact:** a double negative in a shipped human line and an inverted boolean in a frozen JSON field. After release, correcting it renames a JSON field or flips its meaning — a compatibility break either way.

**Required correction:** report the state in the flag's own terms and with the flag's own polarity everywhere, and retire the phrase "off switch" from every user-facing string. Concretely: (a) JSON — carry the literal flag value under a key named for the flag, e.g. `meta.transport_setting: {"transport": "on" | "off", "source": …, "file": …, "ignored": […]}` (any object name that does not say "off switch" is acceptable; `meta.transport` itself is taken by the authentication rows); (b) verbose line — `transport: off (--transport off)` / `transport: on (default)`; (c) note — `gwz: note: the transport is off (<source>): network operations take libgit2's native path, as gwz 1.0 did; --transport on overrides it` and the gwz-py equivalent; (d) `--verbose` long help — "It also shows whether the transport is on or off, and which of --transport, GWZ_TRANSPORT or gwz.transport set it." Option worth weighing while the surface is still open: the family's enum style (`--url-scheme manifest|ssh|https`, `--sync <mode>`) applied here — `--transport gwz|native`, `GWZ_TRANSPORT=native`, `gwz.transport native`, `"transport": "native"` — removes the negation at the root and reads cold without the help.

**Check:** grep the revised §10 for "off switch": zero hits. For each of the four states (default, `--transport on`, `--transport off`, `GWZ_TRANSPORT=off`), the verbose line, the note (if any) and the JSON value contain the same one of {on, off} (or {gwz, native}) as the flag/variable/key that set it.

### [P2-2] The global git configuration is `"user_configuration"` in JSON and "your global git configuration" everywhere a person reads

**Location (§10):** JSON `"source": "flag" | "environment" | "user_configuration" | "default"` and "`file` names the configuration file when `source` is `user_configuration`"; against the long help "which overrides `gwz.transport` in your global git configuration (`git config --global gwz.transport off`)", the ignore note "only from --transport, GWZ_TRANSPORT or your global git configuration", and the refusal hints.

**Violated expectation:** one name per concept across human and machine text, and alignment with the names already in use. git's own name for this scope is `--global` (the command the help itself prints). gwz's existing machine vocabulary for git-config scopes is `selection_source: local_configuration` (`MachineOutput.md`, `meta.transport` rows); its natural counterpart is `global_configuration`. "user" appears nowhere else on the gwz surface.

**Scenario:** a script author sees `"source": "user_configuration"` and searches the help and docs for "user configuration" — no hit. A user told by a script "source is user configuration" runs `git config --unset gwz.transport` in the repository (the "user's" repo) instead of `git config --global --unset gwz.transport`, and the note persists.

**Impact:** frozen JSON enum value; renaming after release is a compatibility break.

**Required correction:** `"source": "global_configuration"`, matching `git config --global`, the help's "global git configuration", and the existing `local_configuration` pattern. Keep "global git configuration" in every human string.

**Check:** the revised §10 contains no "user_configuration"; the JSON `source` value and the human phrase share the word "global".

### [P2-3] "A workspace's git configuration" is undefined on the surface, and the `scope` enum `workspace | repository` names the root/member pair with new words

**Location (§10):** long help "A value in a workspace's or repository's own git configuration is ignored, with a note."; the ignore note "the transport off switch is never read from a <scope>'s git configuration" with "`<scope>` is `workspace` or `repository`"; JSON `"ignored": [{"scope": "workspace" | "repository", "file": string}]`.

**Violated expectation:** every term in help resolves to a thing the user can find. gwz's documented surface has a workspace *root* (itself a git repository) and *members*; the machine vocabulary is `target_kind: Root | Member`. There is no documented "workspace git configuration" distinct from the root repository's `.git/config`; git's own scopes are system/global/local/worktree. A reader cannot tell whether `workspace` means the root repository's local config, git's worktree scope, or something under `gwz.conf/`/`.gwz/`, nor whether `repository` includes the root.

**Scenario:** scenario 5 of the walkthrough with the shared repository being the workspace root: the note says "never read from a workspace's git configuration". The user knows they set nothing "in the workspace", searches the docs for "workspace git configuration", and finds nothing; only `<file>` in the note rescues them. A consumer branching on `ignored[].scope == "repository"` to find member-level noise silently misses or double-counts the root depending on the meaning.

**Impact:** an undefined term in shipped help and a frozen JSON enum whose values collide with gwz's primary noun ("workspace") while ignoring its existing one ("root"). Renaming after release is a compatibility break.

**Required correction:** in the long help, name the two places in gwz's words — "the workspace root's or a member's own git configuration" (or git's: "a repository-local git configuration, root or member") — and use the same words in `<scope>` and in the JSON enum (`root | member`, lowercased to match the style of `source`). If `workspace` actually denotes git's worktree scope, say "worktree" and name that scope instead.

**Check:** the revised §10 defines each scope in one sentence using only terms that appear in `Concepts.md` or git's `git config` scopes, and the ignore note, the `<scope>` list and the JSON enum use those same words.

### [P3-1] The surface says how to set each form and never how to remove one; the family precedent states the undo

**Location (§10):** long help "(`git config --global gwz.transport off`)" — no `--unset`; the note's only hint "`--transport on` overrides it" (a per-command override, not the undo of the form named in `<source>`); the ignore note "ignoring gwz.transport in <file>: … never read from …" — no removal step; the refusal suffix "`; --transport or GWZ_TRANSPORT decides without it`" — a workaround, not a repair.

**Violated expectation:** lifecycle pairs are present and symmetric, as the existing surface does it: `--url-scheme` help names the persistent form *and* "an explicit `manifest` clears it"; the unreadable-file refusal ends "delete or repair the file"; `gwz auth identity` ships `--set`/`--unset` together.

**Scenario:** walkthrough step 3→4. A user sets `git config --global gwz.transport off`, later wants gwz's transport back permanently, and finds no gwz text naming `git config --global --unset gwz.transport`. The nearest thing the surface offers is setting it to `on`, which leaves a key behind that `--verbose` will forever report as `(gwz.transport in ~/.gitconfig)` rather than `(default)`. In scenario 5 the user cannot learn from the note how to silence it.

**Impact:** a bounded documentation/diagnosability gap: the undo is reachable only with outside git knowledge; no compatibility break to fix.

**Required correction:** long help: "…(`git config --global gwz.transport off`; `git config --global --unset gwz.transport` removes it)". Note, when `<source>` is the git key: "…; --transport on overrides it for one command, `git config --global --unset gwz.transport` removes it" (and for `GWZ_TRANSPORT=off`: "unset GWZ_TRANSPORT removes it"). Ignore note: "…; remove it with `git -C <dir> config --unset gwz.transport` to silence this note". Refusals naming `<file>`: add "fix or unset the key" before the "decides without it" clause, as the URL-scheme refusal does.

**Check:** for each of the three forms, §10 contains the command that sets it and the command that removes it within the same help entry or message.

### [P3-2] Presence rules for the state are unstated: which commands print the note, which responses carry the JSON object, whether `ignored` is `[]` or absent, and whether `--jsonl` carries an event

**Location (§10):** "While the transport is off, each command that uses the network says so once."; JSON "`meta.transport_off_switch` is `{…}`" with no sentence on when the key is present; "`ignored`: `[…]`" with no empty-case rule; nothing on `--jsonl`.

**Violated expectation:** `MachineOutput.md` states a presence rule for every key it adds (`meta.transport` "optional"; `crash_recovery` "omitted as a whole on any response that made no such decision", inner keys explicit `null`; "repeated fields remain arrays, including when empty"; the crash-recovery warning "also emits one `Diagnostic` event"). "Uses the network" is not defined for `--dry-run fetch` (contacts nothing), a `Noop` push (contacted no remote — "has no `meta.transport`"), `status`/`ls` (`kind: "members"` has no `meta` at all), or a workspace whose remotes are all `file://`.

**Scenario:** a consumer reads `meta.transport_off_switch.on` on every response and crashes on `gwz --json status`; another treats absence as "transport on" and misreads a `--dry-run fetch`. A `--jsonl` consumer looks for the note as a `Diagnostic` event (crash-recovery precedent) and finds none, or finds one it did not expect.

**Impact:** a scripting contract that cannot be written against until 1.1.0 ships and is observed; additive to fix, so P3.

**Required correction:** add to §10: the set of commands that print the note (define "uses the network" — recommended: every command that would open a remote connection, dry runs included or excluded, stated); the responses on which the JSON object is present (recommended: every `kind: "response"` envelope with non-null `meta`, so a consumer can always read it, with `ignored: []` when empty and `file: null` when not from the git key); and whether `--jsonl` emits the note as a `Diagnostic` event (recommended: yes, mirroring crash recovery, with the response object being "the field to read").

**Check:** §10 (and S7.2's MachineOutput text) contains one sentence each for: human note presence, JSON object presence, empty `ignored`, JSONL.

### [P3-3] Whether a higher-precedence form suppresses validation of a lower one is stated for the git key and not for `GWZ_TRANSPORT` or for ignored scopes

**Location (§10):** "each refusal that names `<file>` ends … `; --transport or GWZ_TRANSPORT decides without it`" (so a bad global key is not consulted once the flag or variable decides); the `GWZ_TRANSPORT` refusals carry no such suffix; the ignore rule "A value in a workspace's or repository's own git configuration is ignored, with a note" says nothing about an invalid value there, and the refusal `gwz.transport in <file> must be on or off, not "<value>"` is not scoped to the global file.

**Violated expectation:** the user can predict, from the help, whether a command runs or refuses.

**Scenario:** a stale `export GWZ_TRANSPORT=yes` in a profile; the user runs `gwz --transport off fetch` — refused with exit 2, or runs? A teammate writes `gwz.transport = yes` into a shared member's `.git/config`: every network command for every teammate refuses (exit 2), or prints an ignore note? The two outcomes differ by an outage.

**Impact:** diagnosability; text-fixable.

**Required correction:** state both rules explicitly. Recommended for symmetry with the file rule: the `GWZ_TRANSPORT` refusals end "; --transport decides without it", and ignored scopes are never parsed — "an ignored value is not checked; any value there is reported in the note as written".

**Check:** §10 names the outcome of `--transport off` with an invalid `GWZ_TRANSPORT`, and of an invalid value in a root or member configuration.

### [P3-4] gwz-py's contract around the strings is unstated: when `GWZ_TRANSPORT` is read, how a Python application chooses per client, and what carries `invalid_request`

**Location (§10):** "gwz-py, a `UserWarning`: … `GWZ_TRANSPORT=on` overrides it"; the ignore note "without `--transport, `" (so gwz-py has no flag); "Refusals, … as `invalid_request` in gwz-py"; the hint "set `GWZ_TRANSPORT=off` to use libgit2's native path".

**Violated expectation:** the precedent for a process-level setting in the Python driver states its timing: `commands/auth.md` — "`--ssh-timeout` … sets the native … timeout at process startup … Python applications must configure it before creating a native backend." `gwz-py/README.md` documents only `gwz.GwzOperationError` (raised on a non-ok aggregate status, carrying `response`); it does not mention `invalid_request`.

**Scenario:** a Python application sets `os.environ["GWZ_TRANSPORT"] = "off"` after constructing `Client` and cannot tell from the surface whether the next `fetch` takes the native path; an application serving two workspaces cannot choose per client at all (environment is process-global); a caller wanting to catch the refusal does not know the exception class or where `invalid_request` appears on it. The warning hint "GWZ_TRANSPORT=on overrides it" when `<source>` is `GWZ_TRANSPORT=off` describes a change to the same variable as an override.

**Impact:** bounded; the Python user guesses at lifecycle and error handling. No compatibility break to fix now.

**Required correction:** one sentence stating when gwz-py reads `GWZ_TRANSPORT` (at import, at `Client` construction, or per operation) and whether a `Client` parameter exists (if the design's other sections add one, §10 should list it as part of the frozen surface); one sentence naming the exception and attribute through which a Python caller sees `invalid_request`; reword the gwz-py hint for the env-var source to "set GWZ_TRANSPORT=on or unset it".

**Check:** §10's gwz-py bullets name the read point, the exception class, and the undo of the variable.

### [P3-5] Git scopes other than global, local and worktree are unaccounted for: a `--system` value's fate is unstated and has no `<scope>` value

**Location (§10):** "only from --transport, GWZ_TRANSPORT or your global git configuration"; "`<scope>` is `workspace` or `repository`"; JSON `"scope": "workspace" | "repository"`.

**Violated expectation:** every value a user can legitimately put somewhere git reads has a stated outcome. git config has system (`/etc/gitconfig`), global, local and worktree scopes, plus `include`/`includeIf`.

**Scenario:** a site administrator deploys `git config --system gwz.transport off` to move a fleet to the native path during an incident. From the surface they cannot tell whether it is honoured, ignored with a note, or ignored silently; if it is ignored, the ignore note's `<scope>` and the JSON enum have no word for it, so either a third value appears unannounced or nothing is said. The same holds for a worktree-scope value and for a value reached through `include.path` from the global file (`file` then names which path?).

**Impact:** text-fixable now; if `system` is ignored-with-note, the enum needs the value before the freeze.

**Required correction:** state that only the global scope is read; state the outcome for system and worktree values (recommended: ignored with the same note, `<scope>` gaining `system` and `worktree`); state that `file` names the file in which git found the key, included files included.

**Check:** §10 lists every git scope and its outcome; the `<scope>` list and the JSON enum match that list.

### [P3-6] "transport" now means two things: the switchable gwz transport, and any remote authentication attempt (`meta.transport` rows, "transport diagnostic")

**Location (§10):** `--transport <on|off>` … "Use gwz's transport for network operations"; `--verbose`'s long help, which already begins "Show one transport diagnostic for every remote authentication attempt" and gains "It also shows whether the transport off switch is on". `MachineOutput.md`: "`meta.transport` is an optional list of rows, one per remote authentication attempt"; these rows exist in 1.0.17, i.e. on the native path.

**Violated expectation:** one term, one meaning on one screen.

**Scenario:** `gwz --transport off --verbose fetch` prints the transport-is-off line and then "transport diagnostic" rows for each remote; the user, told the transport is off, reads the rows as evidence it is not. A script sees `meta.transport` rows next to a transport-off object and concludes the switch was not honoured.

**Impact:** diagnosability; the existing name `meta.transport` is frozen since 1.0, so the fix is text, not a rename.

**Required correction:** one sentence in `--verbose`'s long help and in S7.2's MachineOutput text: "Authentication diagnostics (`meta.transport` rows) are reported on either path." If the drafter adopts `--transport gwz|native` (P2-1's option), the collision also shrinks, since the rows are no longer "the transport" being switched.

**Check:** the `--verbose` help names both paths for the rows.

## 2. First-day walkthrough

Performed from §10 as the shipped help plus the existing `gwz --help`/`gwz help <command>`.

1. **One command.** `gwz --help` names no transport option (consistent: `--ssh-timeout` and `--url-scheme` are also only in the long help). `gwz help fetch` lists `--transport` under Global Options beside `--ssh-timeout`; the short help gives the default and the two values. Command: `gwz --transport off fetch`. Guess points: none to reach the command. The note then prints `the transport off switch is on (--transport off)` — the double negative (P2-1) on the very first use, with a hint (`--transport on overrides it`) for a form the user just typed. A user whose problem surfaced as a refusal gets there faster through the TR1.8 hint; one whose problem is a hang must find `gwz help fetch` themselves (no §10 defect; Troubleshooting is outside §10).
2. **All commands in the shell.** The long help names `GWZ_TRANSPORT` and its precedence: `export GWZ_TRANSPORT=off`. Every network command now prints the note — consistent with the `url scheme: … (from GWZ_URL_SCHEME)` precedent. Undo: not stated; `unset GWZ_TRANSPORT` is shell knowledge (P3-1). Whether `--transport off` rescues a bad value in the variable: not stated (P3-3).
3. **Permanently.** The long help's parenthetical `git config --global gwz.transport off` is the only mention of the key, and sufficient to set it. The note's `<source>` then reads `gwz.transport in /Users/me/.gitconfig` — good. Whether `--system` would have served the whole machine: unknown (P3-5).
4. **Undo all three.** Flag: omit it. Variable: `unset`. Git key: **no gwz text names `git config --global --unset gwz.transport`**; the discoverable route is `gwz.transport on`, which leaves a key behind (P3-1). Verifying the undo with `gwz --verbose fetch` yields `transport off switch: off (default)` — the reader must negate twice to confirm gwz's transport is back (P2-1).
5. **Teammate puts `gwz.transport off` in a shared repository's config.** Each network command prints the ignore note naming `<file>`, so the user learns what happened and where. "Never read from a workspace's git configuration" when the shared repository is the root: the user does not know what a "workspace's git configuration" is (P2-3). No step to silence it (P3-1). If the teammate wrote `yes` instead of `off`, refusal versus note is unspecified (P3-3). The JSON `ignored[]` carries the file but no `member_id`/`member_path`, unlike every other per-repository shape (additive to fix; noted below).

## 3. Risks and next action

- **Cross-reference risk:** §10's long help says "`--max-retries` has no effect" on the native path. The dev-tree `CLI.md` references `--max-retries` inside `--ssh-timeout`'s text but lists no `--max-retries` entry. Confirm the option exists in 1.1.0's help before freezing a sentence that names it.
- **`--json` refusals are plain text:** the installed binary prints `gwz: GWZ_URL_SCHEME must be …` as a bare line even under `--json` (probed, exit 2). §10's "after `gwz: ` with exit 2" inherits that; a script wanting a JSON error envelope for a bad `GWZ_TRANSPORT` will not get one. Pre-existing family behaviour, not a §10 defect; worth one sentence in S7.2.
- **Minor, non-blocking (no finding filed):** the `--verbose` help addition says "which form set it" — "form" is design vocabulary, not a user term (resolved by P2-1's rewrite). The verbose line's `(<source>)` drops the family's `from` (`url scheme: https (from --url-scheme)`). `ignored[]` entries lack `member_id`/`member_path`/`target_kind`; adding them later is additive. "as gwz 1.0 did" appears in every string; harmless now, archaeology in a few releases. `gwz` "stands for the invoking CLI's name" — in a Python application using the API there is no CLI, so the `UserWarning` prefix is unclear.
- **Next action:** revise §10 for P2-1, P2-2 and P2-3 (all string/enum renames before any code freezes them), fold in the P3 sentences, and re-run this axis on the revision. Verdict stands at NO-GO; the pre-commit above applies.

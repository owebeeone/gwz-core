# GWZ transport off switch design (TR1.5) — SURFACE-AXIS REVIEW, ROUND 2

**Review object:** `/private/tmp/claude-501/-Users-owebeeone-limbo-gwz-dev/351b18f9-4ec1-4306-ac0a-299e9bded6dd/scratchpad/designs/GwzTransportOffSwitchDesign.md`, revision 1, SHA-256 `a72f9216d7522bc423507269d38df39db64f5814daa43153f8d7c4c5e620fb05`, verified at 12:12:57 and again at 12:16:27 AEST (unchanged; identical to `reviews/GwzTransportOffSwitchDesign-rev1.md`). §10 only (file lines 158–200, "The surface this freezes"). The frozen rev0 copy `reviews/GwzTransportOffSwitchDesign-rev0.md` hashes `73d63a6f…79f7`, the round-1 object; its §10 was compared from the round-1 reading.
**Baseline:** round 1's baseline unchanged — installed `gwz 1.0.17` (`gwz --help`, `gwz help`, `gwz help fetch|push|pull|clone|status|auth|auth identity`, `gwz --json help`, `gwz --json help clone`); `gwz-cli/docs/MachineOutput.md`, `Troubleshooting.md`, `Concepts.md`, dev-tree `CLI.md` global-options block, `commands/auth.md`, `commands/clone.md`, `commands/materialize.md`, `docs/README.md`, `gwz-cli/README.md`, `gwz-py/README.md` (`gwz-py` not installed; no `gwz-py/docs/`). Added this round: `reviews/GwzTransportOffSwitchDesign-RemPlan.md` (SHA-256 `7715b96e…bdbc`, legitimate input now), and local `git version 2.52.0`'s `git config --help` for the documented `--global`/`--worktree` write targets, read to check the drafter's stated deviations.
**Date:** 2026-10-02
**Axis:** SURFACE — the interface as the person using it meets it, from §10 read as shipped help, messages and machine schema, against the existing CLI's help and user docs. Independent, adversarial, read-only. Other axes run in parallel; nothing here relies on them. Filed verbatim by the lane owner.

**Verdict: GO** — 0 P0, 0 P1, 0 P2, 3 P3 (all new to revision 1, all text-fixable). All nine round-1 findings are closed against their own counterexamples below. No pre-commit clause is needed.

---

## 0. Closure of round-1 findings

Each row re-runs the round-1 counterexample against revision 1's §10. Mechanical checks on the §10 text: `off switch` 0 hits, `on|off` 0, `transport_off_switch` 0, `user_configuration` 0, `which form` 0; `workspace` 1 hit, in "the workspace root's" (the defined use).

| Finding | Round-1 defect | Revision-1 text (§10) and counterexample re-run | Status |
|---|---|---|---|
| P2-1 | `--transport off` reported as `"on": true` / "the transport off switch is on"; verbose `off switch: off (default)` | D2: `--transport <gwz\|native>`, `GWZ_TRANSPORT=gwz\|native`, `gwz.transport = gwz\|native`. Default → `transport: gwz (default)`, JSON absent (defined as gwz by default); `--transport gwz` → `transport: gwz (from --transport)`, `"transport": "gwz", "source": "flag"`; `--transport native` → note "using libgit2's native transport (from --transport native)", `transport: native (from --transport)`, `"native"`; `GWZ_TRANSPORT=native` → the same word in note, line and JSON. `--verbose` help now "names the transport that carried network operations, gwz or native, and what selected it" ("form" gone). | Closed |
| P2-2 | `source: "user_configuration"` vs "your global git configuration" | `"source": "flag" \| "environment" \| "global_configuration" \| "default"`; every human string keeps "your global git configuration". | Closed |
| P2-3 | `scope: workspace \| repository` undefined; collides with gwz's "workspace" | Long help: "the workspace root's or a member's own git configuration (.git/config or config.worktree)"; note `<scope>` is `root` or `member <member ID>`; JSON `"scope": "root" \| "member"` plus `"member_id"` (null for the root). One definition, the same words in all three places, matching `target_kind: Root \| Member` in case-folded form. | Closed |
| P3-1 | Set commands given, removal never | Long help: "export GWZ_TRANSPORT=native sets it; unset GWZ_TRANSPORT removes it", "git config --global gwz.transport native sets it; git config --global --unset gwz.transport removes it". Notes: "omit --transport native", "unset GWZ_TRANSPORT, or pass --transport gwz for one command", "remove it with git config --file <file> --unset gwz.transport". Ignore note and file refusals carry the `--file` removal; refusals add "fix it, or remove it". Drafter's deviation (`--file <file>` rather than `-C <dir>`) checked against git 2.52's documentation: `--global` writes to `~/.gitconfig` when it exists, and `--local` never touches `config.worktree`; `--file` is right for any plain file. Residual for included files is new finding P3-7. | Closed |
| P3-2 | Presence rules unstated (note, object, empty arrays, JSONL) | Note: "for a command whose request is in transport scope". Object: "absent when the gwz transport applies by default with nothing ignored or skipped, and absence means exactly that … Other commands' output never carries it, and neither does an error record whose meta is null." Arrays: "ignored and skipped are arrays, empty when empty"; `file`, `member_id`, `value` null rules stated. JSONL: "emits no Diagnostic event … The final response object's key is the field to read". The declined half (present on every response) is accepted: absence now has one defined meaning, so a consumer is safe either way. | Closed |
| P3-3 | Validation of lower forms when a higher one decides | "`--transport native` with a malformed GWZ_TRANSPORT runs on the native transport, and a malformed value in a root's or a member's configuration gives its note, as written, and no refusal"; `GWZ_TRANSPORT` refusals end "; --transport decides without it"; "An ignored value is never checked." Both round-1 scenarios answered. | Closed |
| P3-4 | gwz-py: read point, per-client choice, exception unknown | "reads GWZ_TRANSPORT and gwz.transport at each network operation's native entry … Client takes no transport parameter. From 1.2.0 a Client reads them when it opens"; refusal is `gwz.errors.GwzBridgeError` with `code == "InvalidRequest"`, message shape given; variable-source hint "set GWZ_TRANSPORT=gwz, or unset it". Deviation (existing `GwzBridgeError`/`InvalidRequest` rather than a new `invalid_request` surface) accepted — it is the class a Python user already catches. | Closed |
| P3-5 | System/worktree/include scopes unaccounted for | Scopes bullet: global files read; root/member `.git/config` and `config.worktree` ignored with a note; system not read, no note; command scope and `GIT_CONFIG_GLOBAL` not read; `file` names the global file whose value decided, includes reported against the including file. Long help: "system git configuration is not read." Every git scope has a stated outcome. Two consequences of the stated choices are new findings P3-7 (includes) and P3-8 (`GIT_CONFIG_GLOBAL`). | Closed |
| P3-6 | "transport" meant the switch and the auth rows | `--verbose` long help: "Authentication rows (meta.transport in JSON) are reported on either transport." | Closed |

## 1. Evidence base

Revision 1's §10 renames the surface per D2 and adds, relative to rev0: a Scopes bullet; a Removal-commands bullet with its git 2.52 rationale; a rewritten `--ssh-timeout` short and long help (D3); a `skipped` list and `; skipped unreadable <file>` verbose suffix (D4); per-source note texts, each ending in its own undo; an `<entry>`/`<scope>` ignore note; refusals with repair and precedence clauses; presence rules; and a gwz-py block (read point, `UserWarning` vs `logging`, `GwzBridgeError`, the `gwz-py: note:` CLI prefix). The `--max-retries` clause is held until TR2.1 adds the flag, which resolves round 1's cross-reference risk.

Family comparisons still hold: default in the description ("gwz (the default)"), enum values, `(from --transport)` matching `url scheme: https (from --url-scheme)`, refusal style `gwz: GWZ_TRANSPORT must be …, not "<value>"` matching the probed `GWZ_URL_SCHEME` refusal, and "native" already being the docs' word for the libgit2 layer.

## 2. Findings (new in revision 1)

### [P3-7] For a value reached through `include.path`, every message names the including file, so the removal and repair commands they give cannot succeed

**Location (§10):** Scopes bullet — "a value reached through `include.path` is reported against the file that included it (§2)"; note — "(from gwz.transport in <file>) … remove it with git config --file <file> --unset gwz.transport"; refusals — "fix it, or remove it with git config --file <file> --unset gwz.transport" and "could not parse <file> for gwz.transport: <cause>; fix the file"; JSON — "`file` names the global file whose value decided".

**Violated expectation:** the standard §10 itself sets in its Removal bullet: a message's command must be one that removes the key "whichever [file] it is" — it rejects `--global --unset` and `-C … --unset` because they exit 5 in some cases. `git config --file X --unset key` edits only X; git never writes into an included file, and with `--file` it does not even follow includes on lookup (`--[no-]includes` defaults to off when a specific file is given). So when the key lives in `~/.gitconfig-work` included from `~/.gitconfig`, the printed command exits 5 and the user, opening `~/.gitconfig`, finds no key.

**Scenario:** `includeIf "gitdir:~/work/"` identity splits and dotfile managers routinely put keys in included files; a teammate's shared include sets `gwz.transport = native`. Every network command prints the note; the user runs its command verbatim; nothing is removed; the note persists. For the parse refusal, a syntax error in the included file is reported as "could not parse ~/.gitconfig … fix the file".

**Impact:** a shipped message whose next command fails for a common configuration; introduced by the rewrite (rev0 printed no commands). No name changes to fix.

**Required correction:** detect the include case (libgit2 reports an entry's include depth) and either (a) name the file that held the key in `<file>` for the note, the refusals and JSON `file`, with the including file beside it ("in <included> (included by <global>)"), or, if the included path is not available, (b) say so instead of printing a failing command — "(from gwz.transport in a file included by <file>); remove it from that included file, or pass --transport gwz for one command" — and add an additive `"included": true` to the JSON. Settle JSON `file`'s meaning for this case before S7.2 freezes the MachineOutput text; changing its meaning later is a soft break even without a rename.

**Check:** with `gwz.transport = native` present only in a file reached via `include.path` from `~/.gitconfig`, the removal command the note prints, run verbatim, exits 0 and the next `gwz --verbose fetch` prints `transport: gwz (default)`; the parse refusal names the file that holds the error.

### [P3-8] Under `GIT_CONFIG_GLOBAL`, the help's own set command writes where gwz does not read, and no user-facing text says so

**Location (§10):** Scopes bullet — "the command scope and `GIT_CONFIG_GLOBAL` are not read" (a design statement, not help); long help — "gwz.transport in your global git configuration (git config --global gwz.transport native sets it; …) … and system git configuration is not read" — silent on `GIT_CONFIG_GLOBAL`; `--verbose` line — names skipped files, not the files consulted.

**Violated expectation:** the help's set command lands the value where gwz reads, or the help says when it does not. git 2.32+ honours `GIT_CONFIG_GLOBAL` for reading and for writing `--global`; gwz reads `$XDG_CONFIG_HOME/git/config` and `~/.gitconfig` regardless.

**Scenario:** a CI image or dotfile setup exports `GIT_CONFIG_GLOBAL=/etc/ci/gitconfig`. The user follows the help; git writes the key there; gwz never reads it; no note prints, `--verbose` says `transport: gwz (default)`, the JSON object is absent. There is no diagnostic that gwz looked elsewhere.

**Impact:** silent non-effect of a documented command in a known environment; bounded, text-fixable (plus one verbose addition).

**Required correction:** long help, after "system git configuration is not read": "nor a file named by GIT_CONFIG_GLOBAL; gwz reads ~/.gitconfig and $XDG_CONFIG_HOME/git/config" (honouring the variable instead is a design choice outside this axis; then nothing is needed). In the `--verbose` line, when the origin is `default` or `from gwz.transport in <file>`, name the global files consulted (e.g. `transport: gwz (default; read /home/u/.config/git/config, /home/u/.gitconfig)`), so the user can see where gwz looked; `skipped` already lists the unreadable ones.

**Check:** with `GIT_CONFIG_GLOBAL` naming a file that holds `gwz.transport = native`, `gwz help fetch` tells the user that file is not read, and `gwz --verbose fetch` names the files it did read.

### [P3-9] `--jobs` and `--max-per-host` each state one default, which is false under `--transport native`

**Location (§10):** `--transport` long help — "With native, --jobs defaults to 50, --max-per-host to 8 and --ssh-timeout to 3, as in gwz 1.0, unless you give them"; `--ssh-timeout` short help — "(0 = no timeout, default 9; 3 with --transport native)". §10 amends neither `--jobs` ("Defaults to 100" in the dev reference) nor `--max-per-host` ("Defaults to 32").

**Violated expectation:** every option's default is stated where the user meets it — its own entry — and the entries agree. `--ssh-timeout` now follows that rule; the other two contradict the `--transport` entry.

**Scenario:** a user with `GWZ_TRANSPORT=native` in their profile reads `gwz help push`: `--max-per-host … Defaults to 32`, plans a 40-member run around it, and runs at 8. Nothing in `--verbose` shows the effective concurrency.

**Impact:** two shipped help entries false under a documented setting; one-line fixes now.

**Required correction:** `--jobs`: "Defaults to 100; 50 with --transport native." `--max-per-host`: "Defaults to 32; 8 with --transport native." — the pattern `--ssh-timeout`'s short help already uses. Optionally the `--verbose` transport line reports the effective three values under native.

**Check:** `gwz help fetch` shows both defaults in each of the three entries, and they agree with the `--transport` entry.

## 3. First-day walkthrough (revision 1)

1. **One command.** `gwz help fetch` lists `--transport <gwz|native>` beside `--ssh-timeout`; short help gives the default and both values. `gwz --transport native fetch` prints `gwz: note: using libgit2's native transport (from --transport native), as gwz 1.0 did; omit --transport native to use gwz's transport`. No guess, no double negative, undo in the line.
2. **All commands in the shell.** The long help names the variable and both directions: `export GWZ_TRANSPORT=native` / `unset GWZ_TRANSPORT`. Each network command's note names "unset GWZ_TRANSPORT, or pass --transport gwz for one command". No guess.
3. **Permanently.** Long help: `git config --global gwz.transport native`. The note then reads `(from gwz.transport in /Users/u/.gitconfig) … remove it with git config --file /Users/u/.gitconfig --unset gwz.transport, or pass --transport gwz for one command`. No guess for the standard setup. Traps: a `GIT_CONFIG_GLOBAL` user's value is never read and nothing says so (P3-8); a user whose dotfiles hold the key in an included file is told a removal command that fails (P3-7).
4. **Undo all three.** Omit the flag; `unset GWZ_TRANSPORT`; `git config --global --unset gwz.transport` (help) or the `--file` form (note). Verification `gwz --verbose fetch` prints `transport: gwz (default)` — read once, understood once.
5. **Teammate's shared repository config.** `gwz: note: ignoring gwz.transport = "native" in /ws/app/.git/config (member mem_app): only --transport, GWZ_TRANSPORT and your global git configuration select the transport; remove it with git config --file /ws/app/.git/config --unset gwz.transport` — what, where, why, next. A value of `yes` is shown as written with no refusal (stated). JSON `ignored[]` carries `member_id`, `file`, `value`. Under gwz-py it is a `logging` record that `warnings.simplefilter("error")` cannot turn into an exception (D1). No guess.

## 4. Risks and next action

- **"Transport scope" is a design term.** The note and JSON presence rules say "a command whose request is in transport scope"; the long help says "each command that uses the network". Neither tells a consumer whether `--dry-run fetch` or a `Noop` push carries the object. Harmless now that absence has one meaning, but S7.2's MachineOutput text should list the commands in user words rather than copy the phrase.
- **"from that operation's snapshot"** (gwz-py bullet): "snapshot" is a gwz command noun (`gwz snapshot`); if the sentence ships in gwz-py's docs, say "from the environment as it is when that operation starts".
- **`--verbose` help "names the transport that carried network operations"** overclaims for `git://`, `http://` and `file://` remotes, which always go native under the default; "the transport selected for network operations" is exact and matches the JSON key `transport_setting`.
- **`export`/`unset` are POSIX-shell forms** in help that ships on Windows; the `--url-scheme` help is shell-neutral ("The environment variable … is an alternative to the flag"). A neutral phrasing avoids a PowerShell user typing `export`.
- **`--json` refusals remain plain text** (family precedent, probed on 1.0.17, unchanged).
- **Next action:** GO. Fold P3-9 and P3-8's help sentence into the TR2.x help edits (one line each); decide P3-7's `file` semantics for includes before S7.2 freezes the MachineOutput text, and add the include-aware note wording with it.

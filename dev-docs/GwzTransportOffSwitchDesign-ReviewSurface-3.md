# GWZ transport off switch design (TR1.5) — SURFACE-AXIS REVIEW, ROUND 3

**Review object:** `/private/tmp/claude-501/-Users-owebeeone-limbo-gwz-dev/351b18f9-4ec1-4306-ac0a-299e9bded6dd/scratchpad/designs/GwzTransportOffSwitchDesign.md`, revision 2, SHA-256 `145af486dc0d757ca6f653c6a8ad1f24235be34381ba35b1412143ac4caf3012`, verified at 13:52:49 and again at 13:55:07 AEST (unchanged; identical to `reviews/GwzTransportOffSwitchDesign-rev2.md`). §10 only (file lines 171–228, "The surface this freezes").
**Baseline:** rounds 1–2 unchanged — installed `gwz 1.0.17` help, `gwz-cli/docs/*.md` user pages, `commands/auth.md`, `gwz-py/README.md` (`gwz-py` not installed). Added this round: `reviews/GwzTransportOffSwitchDesign-RemPlan-2.md` (SHA-256 `0660a79c…67fb`); local `git version 2.52.0`'s `git config --help` for `--[no-]includes` ("Defaults to off when a specific file is given (e.g., using --file, --global, etc)"), `--show-origin`, `--get-all`/`--unset-all` (documented as "Replaced by `git config get/unset --all`", legacy forms retained) and the `gitdir:` conditional-include rule ("The .git location may be auto-discovered, or come from $GIT_DIR"); and one harmless probe, `git config --file /dev/null --includes --show-origin --get-all gwz.transport` (exit 1, key not found: the combination is accepted; nothing written).
**Date:** 2026-10-02
**Axis:** SURFACE — the interface as the person using it meets it, from §10 read as shipped help, messages and machine schema, against the existing CLI's help and user docs. Independent, adversarial, read-only. Other axes run in parallel; nothing here relies on them. Filed verbatim by the lane owner.

**Verdict: GO** — 0 P0, 0 P1, 0 P2, 2 P3 (new to revision 2, text-fixable). P3-7, P3-8 and P3-9 are confirmed closed on the final §10 against their own counterexamples.

---

## 0. Closure of round-2 findings

| Finding | Round-2 defect | Revision-2 text (§10) and counterexample re-run | Status |
|---|---|---|---|
| P3-7 | For a value reached through `include.path`, messages named the including file and gave `git config --file <file> --unset gwz.transport`, which exits 5 | E2: `<where>` is "a file included by `<file>`" for an included value; `<remove>` is then "`git config --file '<file>' --includes --show-origin --get-all gwz.transport` shows which file holds it; remove it there" — a locating command, not a removal that cannot work. §10 states why: "git never writes into an included file, and `git config --global --get-all` does not find a value there, since `--global` turns `--includes` off" (confirmed against git 2.52's docs). Parse refusal now "could not parse `<file>`, or a file it includes, … fix the file and line it names", with libgit2's `<cause>` naming them. JSON gains `"included": bool` at top level and per `ignored` entry, with "`file` then names the file that included it". Counterexample (key only in `~/.gitconfig-work` included from `~/.gitconfig`): the note names the include case; the printed command, run in a matching context, prints `file:/home/u/.gitconfig-work  native`; the user removes it there. Residual on "matching context" is new finding P3-10. | Closed |
| P3-8 | Help's `git config --global` set command writes where gwz does not read under `GIT_CONFIG_GLOBAL`; nothing said so; `--verbose` did not name the files consulted | E7: long help "gwz reads ~/.gitconfig and $XDG_CONFIG_HOME/git/config (by default ~/.config/git/config), not a file named by GIT_CONFIG_GLOBAL, and not system git configuration." `--verbose` line: "When `<origin>` is `default` or names the global configuration, it continues `; read <file>, <file>`, naming the global files gwz read in order, or `; read none`", with examples `transport: gwz (default; read /home/u/.gitconfig)` and `transport: gwz (default; read none; skipped unreadable /home/u/.gitconfig)`. Counterexample (`GIT_CONFIG_GLOBAL=/etc/ci/gitconfig` holding `native`): `gwz help fetch` says that file is not read; `gwz --verbose fetch` prints `transport: gwz (default; read …)` naming where it looked. | Closed |
| P3-9 | `--jobs`/`--max-per-host` stated one default each, false under native | E8: short help "(default 100; 50 with --transport native)" and "(default 32; 8 with --transport native)"; long help "Defaults to 100; 50 with --transport native." / "Defaults to 32; 8 with --transport native." — the `--ssh-timeout` pattern. gwz-py: "an operation's policy carries 50 and 8 when the caller sets no values. `Client`'s `max_connections_per_host` defaults to `None`, so any value passed, 32 included, is sent and honoured." Counterexample (profile `GWZ_TRANSPORT=native`, reading `gwz help push`): both numbers appear in each entry and agree with the `--transport` entry. | Closed |

## 1. Evidence base

Revision 2's §10 changes relative to revision 1: an E1 bullet separating the value rule (quoted, control characters escaped) from the path/ID rule (escaped in messages; shell-quoted in printed commands — POSIX single quotes with `'\''`, Windows double quotes; no command for a path with a control character or, on Windows, `%`, `$` or a backtick, with "remove it from that file" instead); the E2 `<where>`/`<remove>` placeholders used uniformly by the note, the ignore note, the file refusals and all gwz-py strings; `--unset-all` everywhere, including the help's `git config --global --unset-all gwz.transport`; E7's `GIT_CONFIG_GLOBAL` sentence and the `; read …` verbose suffix; E8's four help defaults; E9's wording ("the transport selected for network operations", shell-neutral "unsetting it removes it", "from the environment as it is when that operation starts"); and a gwz-py bullet stating the native defaults 50 and 8, `max_connections_per_host=None`, and that "libgit2's timeout is the process's one clock for both transports: … or 9 seconds, not 1.0.17's 3".

Mechanical checks on the §10 text: `off switch` 0 hits; `user_configuration` 0; `--unset gwz` 0 (every removal is `--unset-all`); `export GWZ_TRANSPORT` 0.

## 2. Findings (new in revision 2)

### [P3-10] The locating command for an included value depends on where it is run, and the message does not say where

**Location (§10):** `<remove>` for an included value — "`git config --file '<file>' --includes --show-origin --get-all gwz.transport` shows which file holds it; remove it there", used by the native note, the ignore note, the file refusals and the gwz-py strings.

**Violated expectation:** a printed command reproduces what gwz saw. git's `gitdir:` conditional include is evaluated against "the location of the .git directory", which "may be auto-discovered, or come from $GIT_DIR" (git 2.52 docs) — that is, the directory the user runs the command in. The common include pattern that reaches a global value is exactly `includeIf "gitdir:~/work/"`-style identity splitting. Run from `~`, the command matches no repository, prints nothing, exits 1, and the message's promise ("shows which file holds it") is not kept.

**Scenario:** `~/.gitconfig` has `[includeIf "gitdir:~/work/"] path = ~/.gitconfig-work`, which sets `gwz.transport = native`. gwz, operating on a workspace under `~/work`, reports "(from gwz.transport in a file included by /home/u/.gitconfig)". The user copies the command into the shell they have open in `~`. No output. They open `~/.gitconfig`, find no key, and are where P3-7 left them.

**Impact:** bounded diagnosability defect in a printed command; no name changes to fix.

**Required correction:** bake gwz's own evaluation context into the command rather than describe it: `git -C '<dir>' config --file '<file>' --includes --show-origin --get-all gwz.transport`, where `<dir>` is the directory gwz evaluated the include against (§2/§3 define it — the workspace root for the global files, the member's directory for a member-scope note), quoted under E1's rule like `<file>`. If the design prefers prose, append "run inside the workspace" to the sentence; the `-C` form is better because the user need not know why.

**Check:** with the `includeIf "gitdir:"` setup above, the command printed by the note, pasted verbatim into a shell whose cwd is `~`, prints the `file:` line naming `~/.gitconfig-work`.

### [P3-11] gwz-py's native `--ssh-timeout` default is 9, the CLI's help says 3, and the gwz-py help line and shared doc sentence are not pinned

**Location (§10):** gwz-py bullet — "libgit2's timeout is the process's one clock for both transports: `Client.configure_transport_timeout`'s value, gwz-py's CLI's `--ssh-timeout`, or 9 seconds, not 1.0.17's 3 (§6)"; `--transport` long help — "With native, --jobs defaults to 50, --max-per-host to 8 and --ssh-timeout to 3, as in gwz 1.0, unless you give them"; `--ssh-timeout` short help — "(0 = no timeout, default 9; 3 with --transport native)". §10 pins no `--ssh-timeout` help line for gwz-py's CLI and names no doc change; the shared page `commands/auth.md` currently reads "`--ssh-timeout SECONDS` sets the native connection/read timeout at process startup (default: 3 seconds; 0 disables it). Python applications must configure it before creating a native backend."

**Violated expectation:** every option's default is stated where the user meets it, and the two CLIs that "follow the same command model" (`gwz-py/README.md`) state their own. The decision to keep one clock in gwz-py is deliberate and reasoned (E4); the gap is the strings: the only shipped text a gwz-py user can read about the native default says 3.

**Scenario:** a Python user with `GWZ_TRANSPORT=native` reads `gwz help fetch` or `commands/auth.md`, expects a 3-second stall limit, and sees 9-second stalls per attempt; a script that budgets 1.0's 3 seconds per unreachable host triples its wall time.

**Impact:** a stated default contradicted by the driver's actual default; text-fixable now.

**Required correction:** pin gwz-py's CLI `--ssh-timeout` help in §10 ("Per-attempt libgit2 connect/read timeout for either transport (0 = no timeout, default 9)" or equivalent), and name the `commands/auth.md` sentence for S7.2 or the migration notes: "default: 9 seconds on either transport in gwz-py; the gwz CLI defaults to 3 with `--transport native`". The gwz-py native notice needs no change.

**Check:** `gwz-py fetch --help` states 9; `commands/auth.md` states both drivers' defaults; the three agree with §10's gwz-py bullet.

## 3. First-day walkthrough (steps 3–5, revision 2)

3. **Permanently.** Long help: `git config --global gwz.transport native`; the note then reads `gwz: note: using libgit2's native transport (from gwz.transport in /Users/u/.gitconfig), as gwz 1.0 did; remove it with git config --file '/Users/u/.gitconfig' --unset-all gwz.transport, or pass --transport gwz for one command, to use gwz's transport`. No guess. A `GIT_CONFIG_GLOBAL` user is told in the help that gwz does not read that file, and `--verbose` names what it read. An include user is told "in a file included by /Users/u/.gitconfig" with a locating command — which works when run in the workspace, and may print nothing from elsewhere (P3-10).
4. **Undo.** Help: `git config --global --unset-all gwz.transport` (also removes a doubled key); `unset`/"unsetting it removes it" for the variable; omit the flag. Verification: `gwz --verbose fetch` prints `transport: gwz (default; read /Users/u/.gitconfig)`. No guess.
5. **Teammate's shared repository config.** `gwz: note: ignoring gwz.transport = "native" in /ws/app/.git/config (member mem_app): only --transport, GWZ_TRANSPORT and your global git configuration select the transport; remove it with git config --file '/ws/app/.git/config' --unset-all gwz.transport`. A path with a space or `$(` is single-quoted and the command runs nothing else; a path with a control character gets "remove it from that file"; on Windows a path with `%` gets the same. A value of `yes` is shown as written, no refusal. JSON `ignored[]` carries `scope`, `member_id`, `file`, `included`, `value`. Under gwz-py it is a `logging` record. No guess; the only residual is the include-context edge (P3-10) when the teammate's value arrives through `includeIf`.

## 4. Risks and next action

- **Windows quoting is not frozen:** "on Windows, double-quoted, which TR1.8 may refine". Acceptable before release; TR1.8's refinement should land before the strings are generated into `CLI.md`, so the reference and the binary agree.
- **Legacy `git config` modes:** `--unset-all`/`--get-all` are documented in git 2.52 as "Replaced by `git config unset/get --all`" but remain accepted (probed) and are the portable choice for older gits. Fine as chosen; revisit only if git removes them.
- **"Transport scope"** remains a design term in the presence rules; S7.2's MachineOutput text should list the commands in user words (carried from round 2).
- **`--json` refusals** remain plain text with exit 2 (family precedent, unchanged).
- **Next action:** GO. P3-10 is a one-token change to one placeholder (`git -C '<dir>' …`); P3-11 is one help line plus one doc sentence. Both fit the TR2.x help edits and S7.2 without another round.

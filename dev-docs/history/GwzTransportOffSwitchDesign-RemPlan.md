# GWZ transport off switch design (TR1.5) — remediation plan 1

Date: 2026-10-02.

**Object.** Revision 0 of the draft, SHA-256 `73d63a6f1714c1e73acdf898716f4dc2aae1fd232e7004e6754dc5b3765279f7`.

**Reviews.** All three reported NO-GO, and each pre-committed to GO on a revision that resolves its P2s as specified.

| Review | P2 | P3 |
|---|---|---|
| [Consistency](GwzTransportOffSwitchDesign-ReviewConsistency.md) | 1 | 5 |
| [Safety](GwzTransportOffSwitchDesign-ReviewSafety.md) | 4 | 2 |
| [Surface](GwzTransportOffSwitchDesign-ReviewSurface.md) | 3 | 6 |

**The rule for revision 1.** It applies every disposition below in one patch. Each finding closes only when the reviewer who raised it re-checks its own counterexample against revision 1.

## Blind convergence

Three pairs of reviewers found the same defect independently:
- **Safety P2-1 and Consistency P3-2:** §3's "the scan never fails a request" against §4's `error` filter. They point in opposite directions: Safety would make the notes unable to refuse, Consistency would qualify §3 to admit the refusal. The decision is under D1.
- **Consistency P2-1 and Safety P3-1:** the files §2 reads are git's `--global` set, not libgit2's, when `XDG_CONFIG_HOME` is empty, under a passwd home, and for a `~/` include. The decision is under D5.
- **Surface's "cross-reference risk" and Consistency P3-1:** the help names `--max-retries`, which TR2.1 adds.

## Decisions taken in this plan

These are the lane owner's, and the operator can reverse any of them before TR2.5.

- **D1. Ignored-value notes never refuse an operation** (Safety P2-1 and Consistency P3-2).
  - In gwz-py, §3's notes are `logging` records on logger `gwz`, at `WARNING`. Only the notice that the native transport is selected is a `warnings` warning, so only it can be made an error, before any effect.
  - Why: §3's premise is that a repository value is ignored by construction and that the scan never fails a request. A file the caller may not own must not be able to refuse the caller's operations. Consistency's alternative would make §3's own invariant false, so this closes its contradiction from the other side.
- **D2. The surface names the transport, not a switch** (Surface P2-1). The flag is `--transport <gwz|native>`, default `gwz`, with `GWZ_TRANSPORT=gwz|native` and `gwz.transport = gwz|native`. Every user-facing string reports the value in those words: the note, the `--verbose` line, the JSON, the help, the gwz-py warnings, and the hint TR1.6 and TR1.8 use.
  - "Off switch" stays the plan's internal name (OD6) and appears in no user-facing string.
  - The JSON object is `meta.transport_setting`, whose `transport` field is `"gwz"` or `"native"`.
  - Why: `on|off` cannot report the state without a double negative ("the transport off switch is on"). It can also be misread as "no network". Surface offered `gwz|native` as the option that removes the negation at the root, and "native" is already gwz's user-facing word for libgit2's layer.
- **D3. With the native transport selected, the defaults are 1.0.17's** (Safety P2-3): `--jobs` 50, `--max-per-host` 8 and `--ssh-timeout` 3 s, when their flags are omitted. Explicit values are honoured.
  - Why: the operator's parity rule. The lane owner verified the three values at gwz-core and gwz-cli `v1.0.17` against `9d4dd92f` and `0218cc7`, which have 100, 32 and 9 s.
  - At 32 per host, a stock OpenSSH server's `MaxStartups` drops native setups, and nothing retries them, so the native transport would fail members that 1.0.17 serves.
- **D4. An unreadable user configuration file carries no value** (Safety P2-2), as git and libgit2 treat it (`config_file.c:117-124`). It is reported in the `--verbose` line and in a `skipped` list in the JSON. An unparsable file is still refused.
- **D5. The files are git's `--global` set, read where `git config --global` writes them** (Consistency P2-1 and Safety P3-1).
  - That is `$XDG_CONFIG_HOME/git/config`, or `$HOME/.config/git/config` when that variable is unset or empty; then `$HOME/.gitconfig`, whose value wins. `HOME` and `XDG_CONFIG_HOME` come from the snapshot, never the passwd entry. An empty or relative `HOME` names no file.
  - §2 states the departures from libgit2's own global lookup:
    - an empty `XDG_CONFIG_HOME`;
    - `uid != euid`;
    - `APP_SANDBOX_CONTAINER_ID`;
    - an `include.path` beginning `~/`, which resolves against libgit2's process-wide home, fixed when libgit2 initialises (in gwz-py, at import).

    It also states that the native path's own reads (the credential callback, `known_hosts`) follow libgit2's rule.
  - Why: the key is written with `git config --global`, so the resolver reads where git writes.
- **D6. The workspace scan is bounded** (Safety P2-4).
  - Each candidate file is opened only if `symlink_metadata` reports a regular file of at most 1 MiB. Anything else is skipped without a note.
  - The scan covers the repositories the request targets, as the operation resolves them, and in gwz-py it runs with the GIL released.
  - "Never fails a request" becomes "never refuses or blocks a request". §3 states that an `include.path` inside a scanned file remains the repository's own hazard, as for any command that opens that repository.
- **D7. Presence rules for the JSON follow the `crash_recovery` precedent** (Surface P3-2, partly disputed). `meta.transport_setting` is absent on a default payload, and absence means the gwz transport by default with nothing ignored or skipped. This keeps default payloads byte-identical, as DR-1 does. When it is present, `ignored` and `skipped` are arrays (empty when empty), and `file` is `null` unless the source is the global configuration.
  - §10 states each rule, and which commands print the note: those whose request is in transport scope. It also states whether `--jsonl` emits a `Diagnostic` event, following MachineOutput's conventions.
  - Surface's "present on every response" is declined, for the byte-identical precedent above.

## Finding dispositions

| Finding | Disposition | Closure test |
|---|---|---|
| Consistency P2-1 | D5 | §9 rows: an empty `XDG_CONFIG_HOME` with the value in `$HOME/.config/git/config` reads it; an empty `HOME` reads no file; a `~/` include resolves as stated. The `sysdir.c` citation re-read against the new text |
| Consistency P3-1 | §9's retry row and the help's `--max-retries` sentence wait on TR2.1. §12 records the edge "TR2.1 ── TR2.5's retry row and help sentence", or that TR2.5 follows TR2.1 | §9 row carries its dependency; §12 names the edge |
| Consistency P3-2 | D1 | §3 and §4 agree; the gwz-py row under the `error` filter (see Safety P2-1) |
| Consistency P3-3 | §10 pins both of gwz-py's CLI lines verbatim, in D2's words with no doubled prefix, and names no flag gwz-py's CLI lacks. §4 says how the CLI derives them | §9's gwz-py row asserts the exact lines |
| Consistency P3-4 | §8 states that the `auto` key covers the native row under line 482's standing recommendation, which amendment 2 §3.19 lets the server design's next revision replace. It names what follows that choice: the "no key depends on it" sentence and OQ3(a)'s `--server auto` clause | The sentence |
| Consistency P3-5 | §12 adds the Python design's §2.5 and §2.8 carve-out for a running native operation's cancel, and the owner of gwz-py's CLI `--transport` in 1.2.0 (CS8.28, or a new step named). CS4.5's "as TR1.5 designs it" is confirmed against §8 | The two changelog entries named in §12 |
| Safety P2-1 | D1 | gwz-py row: under `warnings.simplefilter("error")`, with a repository value present and the gwz transport selected, `call` and `submit` complete on the transport, and the note is a `logging` record. The existing `error` row applies to the native-selected notice only |
| Safety P2-2 | D4 | §9: an unreadable file carries no value and is reported; an unparsable one is refused, naming it. gwz-cli row: `~/.gitconfig` mode 000, fetch on the transport, exit 0, the skipped file shown |
| Safety P2-3 | D3. §6, §5's ledger row, the migration notes and `--ssh-timeout`'s and `--transport`'s help state the values | gwz-cli row: native transport, no flags, 32 members against a fixture sshd at `MaxStartups 10:30:100`: every member succeeds, and the fixture sees at most 8 concurrent connections. The rendered help equals §10 |
| Safety P2-4 | D6 | A FIFO at a non-target member's `.git/config` and a 2 GiB `config` at another: `gwz push --target @root` completes with no note for either. A gwz-py `call` completes while a second Python thread makes progress |
| Safety P3-1 | D5 | As Consistency P2-1 |
| Safety P3-2 | §2 or §8 states that resolution per operation is 1.1.0's. From 1.2.0 a `Client` resolves once at open, which the 1.2.0 notes state | The 1.2.0 gwz-py row in §9 |
| Surface P2-1 | D2 | `off switch` does not appear in §10. For the default, `--transport gwz`, `--transport native` and `GWZ_TRANSPORT=native`, the `--verbose` line, the note and the JSON value use the same word as the form that set it |
| Surface P2-2 | `source` is `"flag" \| "environment" \| "global_configuration" \| "default"` | §10 has no `user_configuration` |
| Surface P2-3 | The scopes are `root` and `member`, defined in the long help as the workspace root's or a member's own git configuration (`.git/config` or `config.worktree`). Each `ignored` entry gains `member_id`, `null` for the root | Each scope is defined once; the note, the `<scope>` list and the JSON enum use the same words |
| Surface P3-1 | Every form's help entry and message names its undo: `git config --global --unset gwz.transport`, `unset GWZ_TRANSPORT`, and `git -C <dir> config --unset gwz.transport` for an ignored value. Refusals that name a file add "fix or unset the key" | Each form's set and remove commands appear in its help entry or message |
| Surface P3-2 | D7 | One sentence each in §10 for the note, the object, the empty arrays and JSONL |
| Surface P3-3 | A form that decides stops lower forms from being read or checked, so the `GWZ_TRANSPORT` refusals end "; --transport decides without it". An ignored scope is never parsed for validity, and its value is reported as written | §10 states the outcome of `--transport native` with an invalid `GWZ_TRANSPORT`, and of an invalid value in a root or member configuration |
| Surface P3-4 | §10's gwz-py bullets state: the read point (each operation's native entry, from its snapshot); that `Client` has no parameter in 1.1.0; the exception and attribute that carry `invalid_request`, verified against `gwz-py/src/gwz/errors.py`; and the hint for the variable's own source ("set GWZ_TRANSPORT=gwz, or unset it") | The bullets |
| Surface P3-5 | Only the global files are read. The help says that system configuration is not read. A system value gets no note, because git and libgit2 do not agree on the system file (for example Homebrew's `/opt/homebrew/etc/gitconfig` against `/etc/gitconfig`), so a note could not be reliable. The worktree scope is `config.worktree`, already scanned as root or member. `file` names the file that held the key, an included file included, or §10 states the limit of what libgit2 exposes | §10 lists every git scope and its outcome |
| Surface P3-6 | `--verbose`'s long help and S7.2's MachineOutput text say that authentication rows (`meta.transport`) are reported on either transport | The sentence |

## Also corrected

- §6's citation `gwz-cli/src/dispatch.rs` becomes `gwz-cli/src/globalargs/dispatch.rs`, which Safety noted.
- §2's Windows sentence takes amendment 2 §3.5's wording: recorded with 1.0.17 on dabeest, with at least one row with `HOME` unset. Consistency noted the gloss.
- §9's size, 310 lines, is over TR2.5's 300-line budget. TR2.5 lands as two steps, and §12 says the plan's changelog names them and that TR2.6 covers both.

## Operator questions after revision 1

The draft's OQ1–OQ4 stand. D2 and D3 go to the operator as applied recommendations.

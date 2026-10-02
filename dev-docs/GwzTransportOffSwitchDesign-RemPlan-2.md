# GWZ transport off switch design (TR1.5) — remediation plan 2

Date: 2026-10-02.

**Object.** Revision 1, SHA-256 `a72f9216d7522bc423507269d38df39db64f5814daa43153f8d7c4c5e620fb05`. A frozen copy is at `GwzTransportOffSwitchDesign-rev1.md`.

**Round-2 reviews.** Every round-1 finding is closed on all three axes.

| Review | Verdict | New P2 | New P3 |
|---|---|---|---|
| [Consistency-2](GwzTransportOffSwitchDesign-ReviewConsistency-2.md) | GO | 0 | P3-6 to P3-9 |
| [Safety-2](GwzTransportOffSwitchDesign-ReviewSafety-2.md) | NO-GO | P2-5 | P3-3 to P3-5 |
| [Surface-2](GwzTransportOffSwitchDesign-ReviewSurface-2.md) | GO | 0 | P3-7 to P3-9 |

Safety pre-commits to GO on a revision that resolves P2-5 as specified.

**This is the second and last remediation round** under the two-round cap. Consistency-2 labels one architectural note: where D3's defaults are resolved. It is non-blocking, and E3 below addresses it.

## Blind convergence

- **The removal command fails for a value reached through `include.path`.** All three axes found this: Consistency-2 P3-8, Safety-2 P3-3 and Surface-2 P3-7.
- **gwz-py's explicit `max_connections_per_host=32`.** Consistency-2 P3-6, with its architectural note, and Safety-2 P3-4.

## Dispositions

**E1. Quoting and escaping** (Safety-2 P2-5).
- §10 states a rule for paths and identifiers, separate from the rule for values.
- In every command gwz prints, `<file>` is single-quoted for POSIX shells, with an embedded `'` written `'\''`. On Windows it is double-quoted, and TR1.8 may refine that.
- In every note, line and message, `<file>` and the member ID are shown with control characters escaped, as `<value>` is, so that one entry is one line.
- The gwz-py strings follow the same rule.
- §9 gains the rows the finding specifies:
  - a targeted member at a path with a space and `$(`: the note's command, run through `sh -c`, removes the key and runs nothing else;
  - a member ID and a member path that each contain a newline: one escaped line each.

**E2. Included and repeated values** (Consistency-2 P3-8, Safety-2 P3-3, Surface-2 P3-7).
- Every removal command uses `--unset-all`, which also removes a single value.
- **git2-rs is off-limits** under the operator's standing rule, so this design does not direct exposing `origin_path` there. Instead, when an entry's `include_depth > 0` (which git2-rs exposes):
  - the note, the refusals and the gwz-py strings say "in a file included by `<file>`";
  - they give the locating command, `git config --global --show-origin --get-all gwz.transport`, not a removal command that cannot work;
  - the JSON gains `"included": true` beside `file`.
- §10's "whichever it is" is qualified to a value the named file itself holds.
- An unparsable included file is refused as "in a file included by `<file>`".
- §9 rows:
  - two `gwz.transport` lines in `~/.gitconfig`: the given command, run as given, removes both;
  - a value held only in an included file: the message names the include case and the locating command;
  - the JSON shows `included: true`.

**E3. Where the native defaults are resolved** (Consistency-2 P3-6 and its architectural note; Safety-2 P3-4).
- First, check whether gwz-core can resolve 1.0.17's defaults (50 and 8) at one site, keyed on whether the operation's backend has a host context: in `resolve_jobs` and `resolve_per_host`, or where their callers have the backend. The defaults would then apply to the CLI, to gwz-py, and to TR2.11's callers without a host context alike, and an explicit value would always be honoured.
- If that is simple, take it, remove the drivers' fill-ins, and say so.
- If not, keep the driver fill-ins, and do both of these:
  - TR2.5 restores gwz-py's `Client(max_connections_per_host=None)` default, 1.0.17's own signature (`v1.0.17` `client.py:201`), so that an explicit 32 is sent and honoured. Amendment 2 §3.1 keeps gwz-py's public API unchanged from 1.0.17, so this restores it.
  - §6 states that TR2.11's native callers run at 100 and 32.
- Either way, §6's "Explicit values are honoured" is made true without exception. §10's gwz-py surface and §5's migration notes carry whatever exceptions remain, such as the process-wide timeout. The §9 rows are those Consistency-2 P3-6 and Safety-2 P3-4 give.

**E4. One clock** (Safety-2 P3-5). §6 gains one sentence: in gwz-py the timeout value is the process's one clock for both transports, so a process that uses both chooses one value for both. The ledger and the migration notes carry it. §9 gains the finding's row.

**E5. The retry plan's clauses** (Consistency-2 P3-7). §12's bullet names the retry plan's §1 table row, §7's stall-default sentence, S2.2's `unwrap_or(9)` and §8. It says the retry plan's status line gains an amendment sentence in the form amendment 2 §3.20 used.

**E6. The citation** (Consistency-2 P3-9). `GIT_CONFIG_PARAMETERS` and `GIT_CONFIG_COUNT` are cited as "which libgit2 does not implement". The server design's exclusion table is cited for `GIT_CONFIG_GLOBAL` only.

**E7. `GIT_CONFIG_GLOBAL` in the help** (Surface-2 P3-8).
- The long help gains: "nor a file named by GIT_CONFIG_GLOBAL; gwz reads ~/.gitconfig and $XDG_CONFIG_HOME/git/config".
- When the source is `default` or the global configuration, the `--verbose` line names the global files gwz read.

**E8. Defaults in each option's help** (Surface-2 P3-9).
- `--jobs` reads "Defaults to 100; 50 with --transport native."
- `--max-per-host` reads "Defaults to 32; 8 with --transport native."
- If E3 resolves the defaults in core, the same sentences hold for every caller.

**E9. Surface-2's and Consistency-2's risk notes,** applied where they are one line each:
- The `--verbose` help says "the transport selected for network operations", not "carried".
- The help names the variable in shell-neutral words, as `--url-scheme`'s does, rather than with `export`.
- gwz-py's bullet says "the environment as it is when that operation starts" instead of "snapshot".
- §2's departures list gains "an empty HOME", which git would read as `/`.
- §3 states that the scan locates `<git dir>` and `<common dir>` from the text of the `.git` file and `commondir`, not through `Repository::open_ext`, so that the FIFO guard runs before any read. That is Safety-2's implementation note.

## Re-checks after revision 2

- **Safety re-checks** P2-5, and its P3-3 to P3-5, on revision 2.
- **Surface confirms** its P3-7 to P3-9 on the final §10, because E1, E2 and E8 change §10's strings.
- **Consistency's P3s** close on its own statement: applied without a further round.

The operator then answers OQ1–OQ4, with OQ3's four items, and D2, D3 and E3 as applied recommendations.

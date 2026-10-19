# gwz.transport setting reader redaction, D50 (gwz-core 21e39505) — CODE-AXIS REVIEW
**Review object:** gwz-core `21e3950530d33d1c97661dc7f1f5bec67ae3a5f5` (5 files in `src/transport_setting*`, +97/−4), and the gwz-core-evidence `f831c3b4` run `campaigns/transport-qualification/runs/2026-10-19-d50/`. **Baseline:** parent `9e8edb80` (the receiving main). **Date:** 2026-10-19. **Axis:** Code. This review was independent, adversarial and read-only. The other axis ran in parallel. Filed verbatim.

**Verdict: GO.** 0 P0, 0 P1, 0 P2, 3 P3.

## 0. Evidence base

- **Tuple at start.** Lane `/Volumes/projects/limbo/gwz-dev-agentwords`. gwz-core HEAD is `21e3950530d33d1c97661dc7f1f5bec67ae3a5f5`, its parent is `9e8edb80e730e86e44894d79d18cef551f70e6e5`, and gwz-core-evidence HEAD is `f831c3b4eff92b8c17844deaab7f067ac2bcb917`. Both trees were clean.
- **The object.** `git show 21e39505`. I diffed its +/− lines against the evidence `raw/patches/gwz-core-d50-final.diff` and they are identical. The file's sha256 `1c984ab6…` matches the README's "full" digest.
- **libgit2 actually linked.**
  - `Cargo.toml:89` builds `gwz-git2` with `vendored-libgit2`, which links `git2-rs/libgit2-sys/libgit2` at `b172e3d18`. This is the same commit as `/Volumes/projects/limbo/gwz-dev/libgit2`.
  - I read these in full or in the relevant parts: `src/libgit2/config_parse.c` (entire), `config_file.c` (open, get/refresh, `parse_include`, `included_path`, conditional includes, `config_file_read*`), `config.c` (`git_config_add_file_ondisk`, error sites), `config_list.c` (error sites), and `util/errors.c` (`git_error_vset`).
- **Reader paths in this module.**
  - `transport_setting.rs`: `resolve`, `last_value`, `parse_failure`.
  - `global.rs`: `resolve`.
  - `scan.rs`: `value_in`.
  - `text.rs`: `Refusal::message`.
- **Driver consumers.** `gwz-cli/src/globalargs/transport.rs:177` and `gwz-py/native/src/route/transport.rs:68-80`.
- **Controlling text.** D50 at `gwz-core/dev-docs/GwzTransportTR18-OperatorDecisions.md:93`. The §10 design words are in `gwz-core/dev-docs/history/reference/GwzTransportOffSwitchDesign.md:43,208`.
- **Evidence run.**
  - `README.md`.
  - `raw/mac/tdd/red.log`: 51 passed, 2 failed. The cause printed is literally `invalid escape at jdoe:hunter2@proxy.example:8080`, from real libgit2.
  - `raw/mac/tdd/green-transport-only-filter.log`: 53 passed.
  - `raw/step3/step3-probe-gwz-1.0.17.log`.
- **Own test run.** I ran `RUSTFLAGS='--cfg gwz_transport_candidate' GWZ_TEST_GIT=real CARGO_TARGET_DIR=/Volumes/projects/limbo/build-targets/d50rev-code cargo +1.95.0 test --lib -- transport_setting` in gwz-core. Result: exit 0, 53 passed, 0 failed, including all three `an_unparsable_file_*` rows. I deleted the target dir afterwards.

## Problem, requirement provenance, and scope

**Problem.** Before the fix, `global.rs:26` put `error.message()` into `Refusal::Unparsable.cause`, and `text.rs` rendered it. libgit2's `unescape_line` (`config_parse.c:316`) sets `"invalid escape at %s"`, where `%s` is the rest of the line after the backslash. In the red log this repeated a proxy user and password.

**Requirement.** D50 says the reader does not repeat libgit2's parse message. It names the file (and the line where known) and points to `git config --list --show-origin`. The operator's 2026-10-19 closeout ("Complete, review and merge the credential-redaction fix first … Keep each package separate") authorizes it.

**Does it close the leak, completely?** Yes.
- `last_value` is the only place this reader gets a libgit2 error. It covers `Config::open`, which parses the file and its includes, and `get_entry`, which can re-parse through `config_file_refresh` (`config_file.c:349`).
- Every `Err` from `last_value` in the deciding path goes through `parse_failure` (`global.rs:24-27`).
- `scan.rs:value_in` throws the error away (`.ok()`), so nothing surfaces there.
- No other transport_setting path reads repository-level, system or other files in a way that surfaces an error.
- Redaction happens when the `Refusal` is built, so the `Debug` output and both drivers' `message(..)` see only the cleaned cause.

**Anything beyond it?** The fallback branch also replaces messages that cannot carry line text: include-depth errors and include I/O errors (P3-1). That is a deliberate fail-closed choice. It is bounded and recoverable, and it remains within the D50 wording "they name the file … and point to …".

**Scope.** All five touched files are under `src/transport_setting*` and the evidence run is the only other addition, which matches the editable set. The generic `git_error` path is untouched, as the deferral requires.

**`parse_failure` against libgit2, attacked:**
- **The allowlist is exact.**
  - `set_parse_error` (`config_parse.c:15-25`) is the only producer of `failed to parse config file: `. A repo-wide grep under `libgit2/src` confirms this.
  - It has 11 call sites with exactly 9 distinct literal phrases (lines 79, 88, 93, 117/127, 144, 176, 203, 212, 402/414). They match `REASONS` character for character.
- **No phrase is followed or preceded by line text.**
  - The format is `"failed to parse config file: %s (in %s:%"PRIuZ"[, column %d])"`. The only variable parts are `reader->path`, `line_num` (`size_t`) and `col` (`int`).
  - `git_error_vset` (`util/errors.c:182-224`) clears the buffer and never chains an earlier error. Only `GIT_ERROR_OS` appends `: strerror`, and the parser does not use that class.
- **The match is anchored.**
  - It uses `strip_prefix` on the full prefix, then `rfind(" (in ")`. The reason has to be exactly a `REASONS` entry, the place has to end with `)`, and line and column have to be decimal digits.
  - `invalid escape at …` cannot satisfy the prefix, because no libgit2 message wraps another.
  - A path that contains ` (in ` moves `rfind` into the path, so the reason no longer matches and the fallback applies. This fails safe.
  - A path that contains `:` or `, column ` is handled by `rsplit_once(':')` on the last colon, because line and column always follow the last colon.
- **The path in the place.**
  - For a top-level file, the path is gwz's own (from the snapshot `HOME`/`XDG_CONFIG_HOME`).
  - For an included file, it is `included_path(dir, include.path value)` (`config_file.c:523-530`). So it holds text from the including file's `include.path` value.
  - D50 explicitly lets the reader name the file, so this is not a D50 breach. See §2 for the precision of the code comment.
- **Multiline values.** `parse_multiline_variable` also reaches `unescape_line`, so the next line's text gets the same fallback.

**Build shape.**
- `src/lib.rs:84` gates the module with `cfg(all(gwz_transport_candidate, any(unix, windows)))`, which my run and the evidence both exercised.
- The new code contains no `cfg`. The new tests sit at file level, outside the existing `cfg_if!` block, and carry only `#[test]`.
- Every body is braced. `Option::is_none_or` is within `rust-version = "1.95"`.
- On Windows, `libgit2_path` passes `C:/…`, and the drive colon is not the last colon, so parsing is unaffected. The Windows receipt `d50-20261019-a` passed.

**Tests.**
- `an_unparsable_file_never_repeats_the_text_of_a_line` drives a real file through real libgit2 (`resolve` → `git2::Config::open`). The fixture `CORP\jdoe:hunter2@…` gives the real `\j` invalid escape. It went red on the original code, as the red log at line 461 shows.
- `…_fixed_reason_keeps_the_reason_and_the_place` checks over-redaction. It would fail if `parse_failure` always fell back to the generic words.

**Design §10 words.** The reference design (`…OffSwitchDesign.md:208`) still says `<cause>` is "libgit2's message" and lacks the show-origin clause. D50 supersedes it, and the design is reference-only for this object (§2).

## 1. Findings

### P3-1 — Non-parse libgit2 errors are relabelled with a false explanation

- **Root cause.** `parse_failure`'s fallback is one fixed sentence for every message that is not a set_parse_error message. That sentence says "the text could not be parsed (libgit2's message is not repeated, because it can hold part of a line…)".
- **Location.** `gwz-core/src/transport_setting.rs:215-219`.
- **Invariant broken.** The refusal should state the actual failure class when libgit2's message cannot carry line text. Two such messages:
  - `"maximum config include depth reached"` (`config_file.c:849`), a fixed string.
  - Include I/O errors from `git_fs_path_set_error(errno, path, "open"/"stat")` (`futils.c:102,110,240`), which contain only a path and a strerror.
- **Reproduction.**
  1. Put `[include]\n\tpath = .gitconfig\n` in `$HOME/.gitconfig` (a self-include).
  2. Call `resolve(None, &home)`.
  3. On `9e8edb80` the cause is `maximum config include depth reached`. On `21e39505` it is the generic sentence: it claims a parse failure, and it gives a line-text reason that does not apply.
  - An included file of mode 000 behaves the same way.
- **Impact.** Diagnosability only. No secret is involved. The user is sent looking for a syntax error in a file that is syntactically valid. The `git config --list --show-origin` pointer recovers the real cause, because git reports the include-depth or access error itself. That keeps this bounded at P3.
- **Correction.** Keep the fail-closed default, but add `"maximum config include depth reached"` to the exact-match keep set. Make the fallback neutral, for example "libgit2 could not read it (its message is not repeated; `git config --list --show-origin` finds the cause)". Do not assert "parsed" or "part of a line".
- **Regression test.** A `files.rs` row with a self-including `.gitconfig`, asserting that `cause` names the include depth (or at least does not say "could not be parsed").

### P3-2 — Contract docs in the edited files still promise libgit2's raw message

- **Root cause.** The change updated `text.rs`'s words but not the prose contracts that describe `cause`.
- **Locations.**
  - `gwz-core/src/transport_setting.rs:15`: "one that cannot be parsed is refused with libgit2's message".
  - `gwz-core/src/transport_setting.rs:141-143`: "`cause` is libgit2's message, which names the file and the line".
  - `gwz-core/src/transport_setting/tests/files.rs:446-448`: "refused with libgit2's message".
  - `gwz-core/src/transport_setting.rs:17`: "the refusals … take §10's words", which now hold only as amended by D50.
- **Invariant broken.** The type and module contract has to match D50, which forbids repeating libgit2's message.
- **Reproduction.** `sed -n '15p;141,144p' src/transport_setting.rs` at `21e39505`.
- **Impact.** The public type's documentation tells a maintainer or a driver that `cause` is libgit2's message. That invites a "fix" that restores the raw message, which is the P0 this object removed. Nothing currently pins the docs.
- **Correction.**
  - Reword both doc sites: `cause` is libgit2's fixed reason and place, or fixed words when libgit2's message could hold line text (D50).
  - Cite D50 next to §10 in line 17.
- **Regression test.** None is needed beyond review. Optionally, a source check that `Unparsable`'s doc does not contain "libgit2's message".

### P3-3 — `parse_failure`'s rejection and column branches are unpinned

- **Root cause.** The pure function has no table rows. The integration rows exercise only two branches: the prefix mismatch (`invalid escape`) and a fixed reason with column 0 (`missing ']'`).
- **Location.** `gwz-core/src/transport_setting.rs:200-214`. The tests are `tests/files.rs:570-621`.
- **Invariant broken.** Each guard that `parse_failure` relies on should be pinned by a test:
  - the `REASONS.contains` exact match;
  - the digit checks on line and column;
  - the `, column ` split.
- **Reproduction (mutation).** Each of these edits keeps all 53 rows green:
  - replace `REASONS.contains(&reason)` with `true`;
  - drop the `digits` checks;
  - break the `, column ` split.
  With current libgit2, every message carrying the prefix also carries a fixed reason, so these guards are defence against a libgit2 upgrade, and they are untested.
- **Impact.** A future loosening (for example `starts_with` or `contains` on the reason) could pass review and the gates. If a later libgit2 adds a set_parse_error call whose `error_str` includes line text, the reader would leak again with no red row.
- **Correction / regression test.** Add a `parse_failure` table test with:
  - each of the 9 phrases, with and without `, column 7`, kept;
  - `failed to parse config file: invalid escape at x:pw (in /f:3)` → fallback;
  - `failed to parse config file: missing ']' in section header extra (in /f:3)` → fallback;
  - a non-numeric line or column (`(in /f:x)`, `(in /f:3, column -1)`) → fallback;
  - a path containing ` (in ` → fallback;
  - a path containing `:` → kept.

No P0. I found no residual leak through this reader after attacking the allowlist, the anchoring, error chaining, multiline values, the refresh-on-get path, includes, and the scan.

## 2. Out-of-scope observations

- **The same leak through other paths (deferred; not a finding).**
  - The evidence step 3 shows installed gwz 1.0.17 printing `invalid escape at jdoe:hunter2@…` for `status`, `--json status`, `commit` and `fetch`, through the generic `git_error` (`src/git/verify_checkout_state.rs:132-141` and the README's list).
  - **Does the design generalize?** The allowlist idea does. The function does not: `parse_failure(&str)` assumes every input is a config-parse failure, and its fallback says so. A general remedy should take the `git2::Error`. It should apply the allowlist only to `class() == Config` messages (the `invalid escape at` producer is `GIT_ERROR_CONFIG`), and it should leave unrelated classes alone. Otherwise every generic error becomes "the text could not be parsed" (compare P3-1).
  - There is an ordering consequence for this reader. In a driver, a workspace or repository open that the generic path has not fixed can print the line before or after the `gwz.transport` refusal. So D50's user-visible guarantee holds only once the generic path is decided.
- **Design text drift (reference-only).** `gwz-core/dev-docs/history/reference/GwzTransportOffSwitchDesign.md:43,208` still defines `<cause>` as libgit2's message, and its words lack the show-origin clause. D50 supersedes it. Amending or annotating it is outside this object's editable set.
- **The included-file path in the place.** The place keeps `include->path`, which comes from the `include.path` value, so it is line text of the including file. D50 permits naming the file. Even so, the code comment's claim "only the two things libgit2 builds from fixed parts are kept" (`transport_setting.rs:183-185`) and the test name "never repeats the text of a line" overstate this slightly. This is for the doc fix in P3-2 if the owner wants it exact.
- **Pre-existing, not this change: an included FIFO.** `state()` guards only top-level files against a FIFO. An `include.path` that names a FIFO is opened by libgit2 (`config_file_read` → `git_futils_readbuffer`) and can block resolution. I read this, I did not run it, and it is unrelated to D50.
- **The text.rs exact-message fixture.** `tests/text.rs:117` still uses a raw libgit2-shaped cause (`failed to parse config file: …`). That is no longer a shape `parse_failure` produces, because the prefix is stripped. It is harmless for a words test, but it is no longer a realistic cause.

## 3. Tuple check at end

- **gwz-core.** HEAD `21e3950530d33d1c97661dc7f1f5bec67ae3a5f5`, parent `9e8edb80e730e86e44894d79d18cef551f70e6e5`. `status --short` is empty.
- **gwz-core-evidence.** HEAD `f831c3b4eff92b8c17844deaab7f067ac2bcb917`. `status --short` is empty.
- **Test target.** I removed `/Volumes/projects/limbo/build-targets/d50rev-code` and confirmed it is gone. Nothing else was written.

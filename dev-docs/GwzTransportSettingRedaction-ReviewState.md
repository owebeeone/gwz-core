# gwz.transport setting reader redaction, D50 (gwz-core 21e39505) — STATE-AXIS REVIEW

**Review object:** `git -C gwz-core show 21e39505` (5 files in `src/transport_setting*`, +97/−4) and the evidence run `gwz-core-evidence/campaigns/transport-qualification/runs/2026-10-19-d50/` (Windows receipt `d50-20261019-a`).
**Baseline:** lane `/Volumes/projects/limbo/gwz-dev-agentwords`. gwz-core `21e3950530d33d1c97661dc7f1f5bec67ae3a5f5`, parent `9e8edb80e730e86e44894d79d18cef551f70e6e5`. gwz-core-evidence `f831c3b4eff92b8c17844deaab7f067ac2bcb917`.
**Date:** 2026-10-19
**Axis:** State. The review is independent, adversarial and read-only. The other axis runs in parallel. This report is filed verbatim.

**Verdict: GO.** P0: 0, P1: 0, P2: 0, P3: 3.

## 0. Evidence base

- **Tuple at start:** both HEADs match the full shas above, the parent is `9e8edb80`, and both trees are clean.
- **Diff read in full:** `parse_failure` in `src/transport_setting.rs:182-220`, the call site in `global.rs:24-27`, the words in `text.rs:40-45`, and the two new test rows plus two changed rows in `tests/files.rs` and `tests/text.rs`.
- **Sinks traced:**
  - gwz-cli `src/globalargs/transport.rs:177-178` turns the refusal into `CliError::invalid_request(error.message(Driver::Cli))`. `src/lib.rs:200-207` sends it to `render_error_json` (`--json`/`--jsonl`) or to `eprintln!("gwz: …")`, then exits 2.
  - gwz-py `native/src/route/transport.rs:68-81` turns it into `ModelError::new(InvalidRequest, refusal.message(Driver::Python))`.
  - No sink receives the `git2::Error`. Only the `String` that `parse_failure` returns goes into `Refusal::Unparsable.cause`.
  - `scan.rs:160` discards `last_value`'s error (`.ok().flatten()`).
  - `rg` finds no logging, tracing, `eprintln!`, global, `static`, `thread_local` or lock in non-test `transport_setting*` code.
- **libgit2 source read** (the vendored fork, `git2-rs/libgit2-sys/libgit2`, 1.9.7):
  - `config_parse.c:15-25` (`set_parse_error`) is the only producer of the prefix `failed to parse config file: `. `rg` over `libgit2/src` finds no other producer.
  - Its 12 call sites use exactly the 9 phrases in `REASONS`, which I checked one by one.
  - `config_parse.c:316` (`invalid escape at %s`) is the line-text leak. It has no prefix, no file and no line.
  - Other errors reachable from `Config::open`/`get_entry`:
    - `maximum config include depth reached` (`config_file.c:849`);
    - `git_fs_path_set_error(…, "open"/"stat")` for an unreadable include (`config_file.c:886-891`);
    - the homedir errors.
- **git2-rs `Error::last_error`** (`git2-rs/src/error.rs:35-64`) copies the message with `from_utf8_lossy` and calls `git_error_clear()`. libgit2's thread-local error therefore does not keep the text after the call.
- **Verbatim copy of `parse_failure` tested in scratch:** I extracted it with `sed` and compiled it with `rustc +1.95.0` in the scratchpad, against 21 adversarial inputs:
  - empty input;
  - the raw `invalid escape`;
  - the prefix with a non-fixed reason;
  - a fixed reason with extra text before or after the place;
  - a missing `)`;
  - an empty, non-digit, fullwidth-digit or negative line or column;
  - a nested ` (in `;
  - a newline in the path, U+FFFD in the path, and a leading space;
  - `C:/…`, `C:\…` and `\\?\C:\…` paths;
  - the depth and open-failure messages.

  Every input either became the fixed fallback or became `<fixed reason> (in <libgit2 path>:<digits>[, column <digits>])`.
- **git 2.52.0 probe** (scratchpad HOMEs, `GIT_CONFIG_NOSYSTEM=1`): `git config --list --show-origin` and `--global --list --show-origin` were run for each case where gwz falls back:
  - a bad escape in `.gitconfig` gives `bad config line 4 in file …/.gitconfig`;
  - a bad escape in an included file gives `bad config line 3 in file …/inc`;
  - an include cycle gives `exceeded maximum include depth (10) while including …`;
  - a mode-000 include gives `unable to access '…/locked': Permission denied`.

  None of these repeats the line's text. The probe directories were deleted afterwards.
- **Optional test run** (the command given in the brief, with `CARGO_TARGET_DIR=/Volumes/projects/limbo/build-targets/d50rev-state`): 53 passed, 0 failed, 3808 filtered out. The log has no occurrence of `hunter2`. The target directory was deleted afterwards.
- **Evidence:**
  - `python3.13 -B tools/verify_archive.py` reports "Verified 5553 migration records; no build/cache directories in archive". That tool checks migration manifests only.
  - I checked the run's own `manifest.json` separately: all 30 sha256 values match, and no file is unlisted or missing.
- **Windows digests recomputed from the commit:**
  - `git diff 9e8edb80 21e39505 | shasum -a 256` gives `1c984ab6…8910`. This equals the receipt's "full" digest at staging start and after the run.
  - `git diff 9e8edb80 21e39505 -- src ':(exclude)*test*'` gives `723ee38c…6999`, which equals the "code-only" digest.
  - The archived `raw/patches/gwz-core-d50-final.diff` is byte-identical to the commit diff.

## Problem, requirement provenance, and scope

- **Problem:** before the change, `global.rs` stored `error.message().to_owned()` in `Refusal::Unparsable.cause`. For a bad escape, libgit2 builds that message from the rest of the line (`config_parse.c:316`). `red.log:461` shows it repeating the fixture password: `invalid escape at jdoe:hunter2@proxy.example:8080`. The defect is real and was reproduced before the fix.
- **Necessity and provenance:**
  - D50 (`GwzTransportTR18-OperatorDecisions.md:93`) requires that the `gwz.transport` reader does not repeat libgit2's parse message, names the file (and the line where known), and points to `git config --list --show-origin`.
  - The operator's closeout of 2026-10-19 authorizes the package and requires it to be kept separate.
  - The change does all three:
    - **No repetition:** an allowlist that fails closed.
    - **The file:** `Refusal.file`.
    - **The line where known:** the place, kept for the 9 fixed reasons.
    - **The pointer:** both in the fallback and in the outer words.
- **Scope:**
  - The diff touches only `src/transport_setting*` and adds one new evidence run.
  - The generic `git_error` mapping and the D43 proxy reader are untouched. The README (step 3 and Limitations) names them and sends them to the operator.
  - Nothing in the change exceeds the authorization.

## 1. Findings

No residual leak exists through this reader (no P0), and no P1 or P2 finding was found. The attacks that failed:

- **Error chains and `source()`:** `Refusal` holds no error and implements no `Error`/`source`.
- **`Debug`:** the derived `Debug` prints only the sanitized `cause`.
- **Logs and `--verbose`:** the module has no logging, and `--verbose` prints only `Setting.read`/`skipped` paths, which a refusal never has.
- **JSON and protocol:** `--json` and gwz-py carry only `message()`.
- **Panics:** no production panic depends on `cause`.
- **libgit2's thread-local error:** git2-rs clears it.
- **The one pass-through:** the only text `parse_failure` passes through is the path part of the place. The only producer of that shape is `set_parse_error`, which writes `reader->path` there: the opened file, or an included file's resolved path. D50 asks for the file to be named.
- **Fail-closed direction:** every malformed, truncated, empty, multi-line-reason or unknown message drops to the fixed words. Non-UTF-8 input was already replaced with U+FFFD by git2, and those paths stay paths.
- **Windows paths:** drive letters and backslash paths keep their location, because `rsplit_once(':')` uses only the last colon. `libgit2_path` also hands libgit2 `/` separators on Windows.
- **Location:** where libgit2 gives a line, it is kept. Where libgit2 gives none, git's command names the file and line, or the include problem, in every fallback kind I probed.
- **State:** `parse_failure` is a pure `&str -> String` with only a `const` table.

### P3-1. The allowlist and digit checks that make the reader fail closed are not pinned by any test

- **Root cause:** the leak test (`tests/files.rs` `an_unparsable_file_never_repeats_the_text_of_a_line`) uses a bad escape. libgit2 reports that without the `failed to parse config file:` prefix, so `strip_prefix` alone sends it to the fallback. The fixed-reason row covers only the case that is accepted. No row gives `parse_failure` a message that has the prefix but fails `REASONS.contains`, the `digits(line)` check or the `digits(column)` check.
- **Location:** `src/transport_setting.rs:200-213`, and the absence of a table test for it in `src/transport_setting/tests/`.
- **Invariant:** only the nine fixed reasons and a numeric place pass through. Everything else becomes the fixed words.
- **Reproduction:**
  1. Remove `REASONS.contains(&reason) &&` (or `digits(line) &&`) from line 211.
  2. Run the 53 tests. Every row still passes:
     - the leak row still reaches the fallback through `strip_prefix`;
     - the fixed-reason row still matches;
     - the `text.rs` rows build their `Refusal` directly.
- **Impact:** a later edit could loosen the guard the commit claims, and every gate would stay green. No leak exists today, because `set_parse_error` passes only fixed phrases. The risk is a regression in coverage.
- **Correction and regression test:** add a table test of `parse_failure` over inputs like the scratch set in §0:
  - prefix with a non-fixed reason;
  - fixed reason with extra text, before or after the place;
  - missing `)`;
  - empty, non-digit or negative line or column;
  - nested ` (in `;
  - empty input;
  - a Windows drive path, which should be kept.

  Each row asserts the exact output.

### P3-2. The in-code contract still says the refusal carries libgit2's message

- **Root cause:** two doc comments were not updated with the behaviour:
  - the module table row says an unparsable file "is refused with libgit2's message" (`src/transport_setting.rs:15`);
  - `Refusal::Unparsable`'s doc says "`cause` is libgit2's message, which names the file and the line" (`src/transport_setting.rs:140-143`).

  The pre-existing test doc comment for `an_unparsable_file_is_refused_with_libgit2s_file_and_line` (`tests/files.rs:445-447`) says the same.
- **Location:** the lines above. All of them are in the editable scope.
- **Invariant:** after D50, `cause` is either a fixed reason with its place or the fixed words. It is never libgit2's message.
- **Reproduction:** read the two doc comments against `parse_failure`.
- **Impact:** a later reader of the public `cause` field, for example the parked D43 reader reusing it, is told it holds libgit2's message. That invites passing the raw message back in.
- **Correction:** state D50's rule in both places: a fixed reason and place, or fixed words, and never the line's text.
- **Regression test:** none needed, since this is documentation. It could be folded into P3-1's table by naming the contract.

### P3-3. The fallback refusal reads awkwardly and in part wrongly

- **Root cause:** the outer words (`text.rs:40-45`) always say "fix the file and line it names, or find them with `git config --list --show-origin`". The fallback itself says "the text could not be parsed (… `git config --list --show-origin` finds the line)". Together they give:

  > could not parse <file>, or a file it includes, for gwz.transport: the text could not be parsed (libgit2's message is not repeated, because it can hold part of a line; `git config --list --show-origin` finds the line); fix the file and line it names, or find them with `git config --list --show-origin`; …

  The fallback names no line. The command appears twice. "The text could not be parsed" is false for the other fallback kinds: an include cycle, which libgit2 reports as `maximum config include depth reached`, and an unreadable include, which libgit2 reports as `failed to open '<path>': Permission denied`. Both of those messages were safe to repeat.
- **Location:** `src/transport_setting.rs:213-218`, and `text.rs:40-45`.
- **Invariant:** under §10 and D50, the refusal names what it knows and points to the command that finds the rest.
- **Reproduction:** take the words for a `.gitconfig` with `[include] path = .gitconfig`, or with an include of mode 000. The refusal says the text could not be parsed. My git probe shows the real causes are the include depth and permission.
- **Impact:** diagnosability only, and bounded. The suggested command names the culprit in every case I probed, so the user can still repair the config.
- **Correction:**
  - make the fallback say why the message is not shown, without claiming a parse failure, for example "libgit2's message is not repeated, because it can hold part of a line";
  - say the command once;
  - optionally, keep the fixed include-depth phrase as a tenth allowed reason.
- **Regression test:** an exact-message row for the fallback in `tests/text.rs`.

## 2. Out-of-scope observations

These are deferred under the brief. None of them is a finding against this object.

1. **The generic `git_error` mapping still leaks the same password, and this refusal's remedy points the user at it.**
   - The run's `raw/step3/step3-probe-gwz-1.0.17.log` shows the installed gwz 1.0.17 printing `invalid escape at jdoe:hunter2@proxy.example:8080` for `status`, `--json status`, `commit` and `fetch`.
   - On the candidate build, `prepare_transport` (`gwz-cli/src/lib.rs:200`) refuses in-scope operations first, with the redacted words. But those words end with "--transport or GWZ_TRANSPORT decides without it". A user who follows that advice skips the reader, the operation's own repository open parses the same global file, and libgit2's raw message is printed through the generic mapping.
   - Commands outside the transport scope never reach this reader at all.
   - So D50's fix to this reader does not, on its own, keep the password off the terminal.
   - **Impact:** a confirmed credential exposure, and a pre-existing defect.
   - **Suggestion:** apply the same allowlist at the generic mapping. The README's list (`verify_checkout_state.rs`, `preservation_image.rs`, `local_clone/adapters/*`, `operation/commit_log/*`, `gitbackend/transport.rs`) is the place to start.
   - **Status:** this goes to the operator. It does not block this package's stated deliverable.
2. **The fixture password in the evidence is a test value, not a real credential.** The literal `hunter2`/`jdoe` appears in `red.log:461` (it demonstrates the defect), in the step-3 probe, in `tests.patch` and in the final diff. It is acceptable.
3. **The Windows evidence gives counts only.** The receipt records 4019 passed with the 4 known host gaps. It does not list test names, so the archive does not show by name that the two new rows ran on Windows. This is the receipt tool's standard form. Its digests do bind the receipt exactly to `21e39505`'s change; see §0.

## 3. Tuple check at end

- **gwz-core:** HEAD is `21e3950530d33d1c97661dc7f1f5bec67ae3a5f5` and the tree is clean.
- **gwz-core-evidence:** HEAD is `f831c3b4eff92b8c17844deaab7f067ac2bcb917` and the tree is clean.
- **Cleanup:** the build target `/Volumes/projects/limbo/build-targets/d50rev-state` and the scratch probe HOMEs were deleted. Nothing in the lane was modified.

# TR2.22 unconditional configuration view mechanism

Date: 2026-10-03. Status: **accepted at reviewed core `daeb4e17dd414ccdb670f9ab74333dd84ef0ba19`, root `054d1dddc345452a2285e74c31f5ea4c0b0d5360`, evidence `84fef16e6b225cf6668d4bbd4fb8c4678b649723`, transport `9f9f0dc4dd82e6329d6ce53e102a231e214ff673` after [Consistency-1](GwzTransportCredentialHelperConfigurationView-ReviewConsistency-1.md) and [Safety-1](GwzTransportCredentialHelperConfigurationView-ReviewSafety-1.md) reported GO. This accepts the mechanism only; implementation is pending.**
The nonblocking Consistency P3-1 outcome-supersession clarification is folded
into this adoption package below; it adds no decision beyond the reviewed M2 rule.
Original reviewers performed the complete corrected-mechanism review and
counterexample closure, following the operator's direction to use old reviewers.
The
accepted helper-context amendment remains separate and unchanged. No transport
wire or public GWZ API is added here.

## Executed counterexample and local API feasibility

The product regression
`https_auth::runner_tests::globally_matching_conditional_include_does_not_replace_unconditional_helper`
uses the actual supervised `/usr/bin/git` child, env_clear plus an isolated
snapshot, cwd `/`, an unconditional include selecting helper A, and a global
remote URL activating `includeIf hasconfig:remote.*.url` selecting B after an
empty reset. It fails: Git selects B. Synthetic credential bytes are compared
without printing them. The latest focused run has 26 other auth tests green;
this scope refusal remains red. Cwd `/` excludes repository configuration and
repository-dependent conditions, but does not exclude every conditional include.

A disposable Rust 1.95 physical spike executed the complete chosen primitive:
Git NUL discovery, actual Git parsing of bounded source bytes through stdin, an
ordered unconditional walk, a process-lifetime flattened environment view, and real credential fill.
The original configuration selects hasconfig helper B; the view selects A.
Assertions also pass for actual system/XDG/global order, Git-decoded PARAMETERS
and COUNT overlays with duplicate/reset order, captured-HOME and relative root
and nested includes, missing and empty includes, valueless versus empty fields,
newline/tab/quote/backslash and non-UTF-8 values, and subsection normalization.
Final controlled discovery contains only the flattened command entries, suppressing
Apple Git's leading installation source as well as original system/XDG/global
files. Synthetic credential answers are captured and wiped, never printed.
Repeated identical system/global root paths produce two complete visits, with
their duplicate/reset/helper sequence preserved. Fixture modes and successful
cleanup are checked; fixtures also have a cleanup
owner on assertion failure. This is feasibility, not product acceptance.

The first candidate used the existing libgit2 in-memory parser successfully,
but a production dual-parser comparison would require Git to parse every source
anyway. The chosen primitive therefore uses Git alone. No git2-rs/libgit2-sys
binding, gwz-git G0 scope change, or unsafe core FFI is required. This avoids
assuming Git and libgit2 have identical configuration grammars. No protected
dependency source changed. The [concise spike receipt](GwzTransportCredentialHelperConfigurationView-Spike.md)
names the retrospectively indexed private evidence and its access requirement.

## Preserved policy and exact mechanism

Preserve TR1.6 §3.2's exclusion of **every** conditional include, its ordered
system/global/XDG/GIT_CONFIG_* sources and legitimate unconditional includes.
Preserve `git -c core.askPass= credential fill`, URL-only encoded input,
terminal-prompt refusal, 120-second effective interaction limit and helper
result/parser/retry rules except the explicit controlled-environment E2BIG
spawn-outcome exception below. A new process-lifetime flattened configuration view implements
that existing policy; `--no-includes` is never applied to the final lookup as
an alleged equivalent configuration.

After both helper admissions, use the already resolved Git executable and the
same filtered captured environment for one supervised, read-only discovery:

`git config --no-includes --null --show-origin --show-scope --list`

This discovery reads the actual Git installation's root configuration files
and parses GIT_CONFIG_PARAMETERS/GIT_CONFIG_COUNT using Git's own parser. It
performs no credential lookup, repository operation or network request. Cwd
remains `/`, with GIT_DIR/GIT_COMMON_DIR/GIT_WORK_TREE removed. The no-includes
switch applies only to discovering root files and command-scope entries.
Both global files retain their Git order. Absent or empty root files contribute
no entries, as in discovery; a failed discovery remains a failed lookup.
Origin paths are byte paths resolved against discovery cwd `/` when relative,
with no ambient-home or libgit2 default-file discovery.

Decode only Git's NUL framing: scope NUL origin NUL name/newline/value NUL;
no newline denotes a valueless entry, while a trailing newline denotes an empty
value. Reject malformed framing without displaying bytes. Admit system/global
file origins and command/`command line:` entries. Also admit a bounded leading
run of `unknown`/`file:` installation origins, only before the first known scope.
The physical Apple Git has exactly this shape; it is not relabeled system.
Reject every other unknown scope/origin, and any unknown file after known scope.
All roots, including this leading installation run, count toward the same bounds.
Keep each contiguous file-origin/scope run as one root occurrence, preserving
run order. Never deduplicate by physical or lexical path: the same file named
by GIT_CONFIG_SYSTEM and GIT_CONFIG_GLOBAL is visited twice in its two scopes.
Multiple entries within one run do not cause repeated file parsing. Command
entries are separately retained in their original order. Cwd `/` admits no local/worktree scope: any
such scope fails closed before credential fill. A Git lacking these discovery
options cannot silently fall back to the broader scope; this lookup fails as
no helper credential (M2), with native transport available under the existing
escape hatch. No new minimum Git version or application error code is declared.

For each file root, read bounded regular-file bytes into core-owned zeroizing
memory and supervise this read-only parse child, writing those bytes on stdin:

`git config --no-includes --null --file - --list`

Use the same Git, captured environment and cwd. The explicit-file command
excludes root discovery and PARAMETERS/COUNT overlays; the spike asserts this
with both overlays present. Decode only NUL name/optional-value framing; Git
itself parses configuration grammar. Close/reap the child before releasing its
buffer/pipe owner. No named source copy or configuration view is created.
The source bytes are already bounded before entering this parse child. No
repository command is introduced.

Walk parsed entries in source order. On `include.path`, resolve absolute paths
unchanged and relative paths against the original source file's directory,
expanding `~/` only against a nonempty absolute captured HOME byte path. Absent,
empty or relative HOME refuses as M2 only when `~/` is required. Validate that
anchor before any include open; never use a parent cwd or ambient home. Both
relative source and command-cwd anchors are explicit absolute paths before I/O. Missing include targets contribute no entries, matching Git;
other open/read errors fail closed. Empty include files contribute no entries.
Command-scope relative includes resolve against cwd `/`. Preserve path bytes,
including non-UTF-8, rather than using lossy conversion. Unsupported tilde-user
or prefix-interpolation forms fail closed as M2; they never become a different
helper selection. Their support is not inferred from ordinary relative paths.
Ignore every normalized `includeif.<condition>.path`, regardless of condition,
and omit all include directives from the final view. Nested unconditional
includes are visited exactly at their directive position. All other entries
retain their ordered normalized name and optional value bytes, including
credential URL subsections, duplicate helpers, resets and non-UTF-8 values.
No handwritten configuration grammar, native parser binding or ambient home
fallback is added.

Encode the flattened entries as Git's process-lifetime GIT_CONFIG_PARAMETERS
syntax: single-quote the normalized key, append `=` and a single-quoted value
only for a present value, with Git-compatible apostrophe escaping. Valueless
`'key'` and empty `'key'=''` remain distinct. Entries are space-separated in
original order. Preserve all subsection/value bytes including non-UTF-8; NUL is
unrepresentable and refuses before final fill. This is an encoder of parsed
entries, not a configuration grammar parser. No secret content enters errors.

The controlled child environment starts from the original filtered snapshot,
then removes GIT_CONFIG_SYSTEM; sets GIT_CONFIG_GLOBAL=/dev/null and
GIT_CONFIG_NOSYSTEM=1; replaces GIT_CONFIG_PARAMETERS with the flattened bytes;
and removes GIT_CONFIG_COUNT and every numbered GIT_CONFIG_KEY_*/VALUE_* after
folding their effective entries. `/dev/null` is the existing OS-owned empty
source on this Unix mechanism; it is never written. No Windows mechanism is
inferred. Other captured entries stand, and final `-c core.askPass=` remains
last with GIT_TERMINAL_PROMPT=0. Helpers inherit this controlled environment.
There is no remaining include directive to reactivate conditional sources.

Before fill, perform the same supervised controlled discovery command. Require
exact ordered name/optional-value equality to the flattened entries, every
entry in command/`command line:` scope, and no file/installation/system/global
origin. Empty views produce no entries. A parser/encoder discrepancy, unexpected
origin or unsuccessful verification refuses as M2, never chooses another
helper. Git normalizes section/variable case, preserves quoted subsection case,
and lowercases legacy dotted subsections; the spike proves all three and the
null/empty/escaping/non-UTF-8/environment-overlay cases. No second grammar or
FFI binding remains. The original user configuration is never written.

The encoded parameter allocation counts against the 4-MiB preparation ceiling.
OS execution/environment limits may be lower and include the complete captured
environment. `ArgumentListTooLong` while spawning controlled verification or
fill is M2 configuration refusal, not missing Git or a missing-Git latch. There
is no truncation, file fallback, wider-scope retry or invented platform capacity.
The physical OS E2BIG refusal occurred before child execution. Same-user or
privileged OS inspection can observe child environment/pipe memory; the design
does not claim otherwise. No such bytes enter GWZ diagnostics or retained data.

## Bounded ownership, clocks and cleanup

Both admissions and their allocation provenance remain exactly as accepted.
One interaction deadline starts after admission, before configuration discovery;
it includes discovery, each source parse, file/environment preparation, the
final ordered round-trip/origin check, and final fill. No stage starts a
fresh 120 seconds. Every read-only parse/discovery and fill child uses its own
new process group,
with the same group kill/reap/500-ms retained-cleanup rules. Only one child is
active for this lookup at a time. No source path, configuration field, value,
helper output or stderr enters Failure, events or logs.

Proposed explicit preparation ceilings: 1 MiB per regular source file, 4 MiB
cumulative source/encoded-parameters/discovery/parser-output bytes, 4,096 parsed entries,
include depth 10
(matching the Git recursive-include ceiling), and 128 files visited, repeated visits
included. Open with nonblocking file flags, then refuse FIFOs/devices/directories before
reading; permit symlinks only
to a regular opened file. Check the opened handle, not only a prior pathname.
Detect include cycles through the depth/count bounds. An oversized/unreadable/
unrepresentable view yields Authentication/M2 with no credential offered;
it does not misuse the 16-KiB answer's M8 output-limit cause. The 16-KiB final
credential answer bound stands unchanged. Initial discovery necessarily uses
Git to parse original root files before their paths are known: its stdout is
bounded and its child is clocked/killed, but these limits do not claim to cap
Git's internal allocation or original-file reads. Per-file input limits apply
when core opens discovered roots and includes for their stdin parse buffers.
This native-child limit is explicit for Safety review, not concealed by the
bounded stdin buffers used in later stages. These preparation limits are new
explicit mechanism limits, not alleged pre-existing configuration ceilings.

Blocking file reads/parsing run in a context-owned bounded worker, never on a
Tokio executor thread. Its endpoint/host permits remain owned through worker
join, including cancellation, timeout and an uninterruptible filesystem read.
The AuthOwner retains an unfinished worker just as it retains an unreaped
child; pending cleanup includes both. Polling/joining only an already-finished
worker never blocks an executor. The worker checks the cancellation token and
same interaction deadline before/after each bounded operation and disposes
its zeroizing buffers on every exit. A 500-ms cleanup miss is cleanup-pending,
not a disposal acknowledgement or permission to reuse its slots. No global
supervisor/counter is added, and existing SSH global-job debt is not expanded.

No core-owned named sensitive filesystem artifact exists during preparation,
fill, cancellation, drop or process death. A context-owned worker holds source,
parsed-entry, encoded-parameter and pipe buffers in zeroizing allocations. It
checks cancellation/deadline before and after each bounded operation. Encoded
bytes are passed as borrowed OsStr to Command; unavoidable Command/OS/native
Git copies expire through command/child ownership. Wipe whole capacity of
core-owned buffers after joined/retained ownership ends. Drop/kill/process death
needs no directory scan, durable recovery marker or independent filesystem
supervisor, because this mechanism never writes sensitive config copies.
Memory/OS copies are not promised physically zeroized on process death.
The inherited bounded environment-copy exception to §3.4 requires Safety review.

## Source owners and binding boundary

Core's `src/git/endpoint/https_auth` owns this mechanism: private configuration
view modules own zeroizing entry/path/value buffers, the bounded file worker,
process-lifetime buffer ownership and ordered walk/encoder. Its existing supervised child
runner owns discovery/parse/fill groups, deadline, answer buffers and retained
cleanup; the AuthOwner retains both admissions through every unfinished worker
or child. Endpoint environment construction remains the snapshot producer.
The source-spawn inventory records these concrete read-only commands. Product
regressions belong beside the existing auth runner and endpoint route tests.
No source outside these owners needs a configuration API change. In particular,
there is no git2-rs/libgit2-sys binding, gwz-git wrapper/G0 amendment, CLI/Python
schema, transport wire or public GWZ API change. Core-owned sensitive buffers
are zeroized; unavoidable native Git and OS copies expire through child/handle
ownership and are not claimed zeroized by Rust. At most 128 source-parse children,
one initial discovery, one controlled ordered round-trip/origin verification
and one fill are launched: 131 sequential children, all under one deadline.
This accepted mechanism supplies no permission to mutate protected dependencies.

## Precise supersessions and product checks

This accepted amendment supersedes TR1.6 §3.2's literal snapshot delta
and “Nothing else is added” sentence for the controlled configuration variables
named above; its cwd/no-local/every-conditional-include policy stands. It adds
the bounded supervised read-only discovery/parse/verification commands and context-owned view
preparation before §3.1's otherwise unchanged final credential-fill command.
It qualifies §3.3's lookup-start statement so the same interaction deadline
covers all preparation, without extending clocks. It adds the explicit bounded process-lifetime configuration
environment-copy ownership exception to §3.4, with no new diagnostic data.
It also narrowly supersedes §4's M1 row/latch for a found Git that cannot
start, and §11's corresponding M1 code and clone treatment: `ArgumentListTooLong`
while spawning controlled verification or fill is Authentication/M2, with no
missing-Git latch and existing M2 member treatment (`remote_rejected` for
fetch/push; private clone members skipped). Missing or otherwise unexecutable
Git still takes M1; no other spawn outcome changes. The integrated E2BIG test
must prove no helper execution, no latch on a subsequent lookup and those
boundary outcomes, with a missing/unexecutable-Git M1 control.
The permanent process-spawn inventory must name discovery, explicit-file parses,
controlled-origin verification and final fill;
no native credential callback, CLI/Python schema or Windows mechanism changes.

Required integrated regressions: unconditional A-only; unconditional include A
with working local B, gitdir/onbranch B and globally matching hasconfig B;
conditional directives supplied through environment command entries; Git's
actual system file (including a non-/etc Git installation), global/XDG order,
GIT_CONFIG_GLOBAL/SYSTEM/NOSYSTEM, PARAMETERS and COUNT ordering/resets;
relative and captured-HOME `~/` unconditional includes; nested legitimate
includes and ignored unknown/future conditions; value/null/non-UTF-8/escaping
round-trip; absent/relative/empty HOME refusal and no parent-cwd reads; bounded/cyclic/FIFO and symlink-handle refusal; prepared view plus
helper answer/environment sentinel redaction and helper inheritance; cancellation/drop/timeout during discovery,
worker preparation and fill; stopped worker/child retains both quotas and
buffers until joined; no named config copy on failure/drop/kill/process death;
OS environment-size refusal before helper execution; no repository
configuration and no network/repository Git command. Repair/undo fixtures
compare synthetic answers without printing them. Keep the executed hasconfig
counterexample red until the adopted mechanism makes that real child green.

The root owner's adoption supplies authority to implement this exact mechanism;
product implementation still requires its integrated regressions and independent
settled-tree acceptance before release.
The already accepted username/timing/fixed-cause implementation and existing
lifecycle/secret work may proceed independently. This is a mechanism correction
for one discovered scope gap, not an authentication-policy expansion.

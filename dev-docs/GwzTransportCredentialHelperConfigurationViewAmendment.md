# TR2.22 unconditional configuration view mechanism

Date: 2026-10-03. Status: **DRAFT; not accepted or implemented**.
Root settles and independently reviews this mechanism before adoption. The
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
Git NUL discovery, actual Git parsing of private bounded source copies, an
ordered unconditional walk, a private flattened view, and real credential fill.
The original configuration selects hasconfig helper B; the view selects A.
Assertions also pass for actual system/XDG/global order, Git-decoded PARAMETERS
and COUNT overlays with duplicate/reset order, captured-HOME and relative root
and nested includes, missing and empty includes, valueless versus empty fields,
newline/tab/quote/backslash and non-UTF-8 values, and subsection normalization.
Final controlled discovery contains only the private global view, suppressing
Apple Git's leading installation source as well as original system/XDG/global
files. Synthetic credential answers are captured and wiped, never printed.
Repeated identical system/global root paths produce two complete visits, with
their duplicate/reset/helper sequence preserved. Private modes and successful
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
terminal-prompt refusal, 120-second effective interaction limit and all helper
result/parser/retry rules. A new private flattened configuration view implements
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
memory, create a private 0600 copy, and supervise this read-only parse child:

`git config --no-includes --null --file <owned source copy> --list`

Use the same Git, captured environment and cwd. The explicit-file command
excludes root discovery and PARAMETERS/COUNT overlays; the spike asserts this
with both overlays present. Decode only NUL name/optional-value framing; Git
itself parses configuration grammar. Close/reap the child before wiping and
unlinking its source copy. Never read user files through an unbounded Git parse
child: the owned copy is already bounded. No repository command is introduced.

Walk parsed entries in source order. On `include.path`, resolve absolute paths
unchanged and relative paths against the original source file's directory,
expanding `~/` only against captured HOME. If HOME is absent when required,
refuse the lookup. Missing include targets contribute no entries, matching Git;
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

Serialize parsed entries into one native Git configuration file. This is a
writer for already parsed names/values, not a replacement parser. It must
preserve normalized section/variable and exact subsection/value bytes,
valueless versus empty, duplicate/reset order and escaping. Before final fill,
parse the owned view with the same supervised explicit-file
command and require exact ordered name/optional-value equality to the flattened
entries. Git normalizes section/variable case, preserves quoted subsection case,
and lowercases legacy dotted subsections; the writer consumes those parsed names.
This final comparison covers every input, not only test examples. Any writer or
unsupported grammar discrepancy fails closed as M2. The physical spike covers
case normalization, non-UTF-8, valueless and escaped-value round trips.
Unrepresentable bytes fail closed as M2, never
become a different helper selection. NUL in a source or decoded field is
unrepresentable and fails before final fill. No secret content is formatted
into any error.

The final fill child receives the original filtered snapshot except:
GIT_CONFIG_SYSTEM is removed, GIT_CONFIG_GLOBAL names the private flattened
source, and GIT_CONFIG_NOSYSTEM=1 suppresses system and the observed leading
Apple installation source;
GIT_CONFIG_PARAMETERS, GIT_CONFIG_COUNT and numbered GIT_CONFIG_KEY_*/VALUE_*
are removed after their effective entries have been folded into the view.
The single explicit global source suppresses XDG and original global files.
Before fill, supervised controlled discovery must contain only that global
file origin (or no entries if empty), with no installation/system/other origin;
otherwise refuse the lookup. This structural check never displays source paths
or configuration values. Other snapshot entries stand.
The final command's `-c core.askPass=` remains last and GIT_TERMINAL_PROMPT=0
still wins. Helpers inheriting these settings see the same flattened view;
there is no original conditional include left to re-activate in a helper.

## Bounded ownership, clocks and cleanup

Both admissions and their allocation provenance remain exactly as accepted.
One interaction deadline starts after admission, before configuration discovery;
it includes discovery, each source parse, file/view preparation, the final
round-trip and controlled-origin checks, and final fill. No stage starts a
fresh 120 seconds. Every read-only parse/discovery and fill child uses its own
new process group,
with the same group kill/reap/500-ms retained-cleanup rules. Only one child is
active for this lookup at a time. No source path, configuration field, value,
helper output or stderr enters Failure, events or logs.

Proposed explicit preparation ceilings: 1 MiB per regular source file, 4 MiB
cumulative source/view/discovery/parser-output bytes, 4,096 parsed entries,
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
when core opens discovered roots and includes for their private parse copies.
This native-child limit is explicit for Safety review, not concealed by the
bounded copies used in later stages. These preparation limits are new
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

The worker creates a unique private scratch directory (0700), bounded source copies and the flattened
configuration file (0600), using create-new ownership under the runtime's scratch parent.
Hold the directory/files through discovery preparation, final fill and its
joined/retained cleanup; do not remove them while a live child may read them.
No repository files or user configuration are written. Memory buffers wipe
whole capacity on drop. On disposal, best-effort overwrite scratch file bytes,
close and unlink files/directory; do not claim physical secure erasure on an
SSD or that unlink zeroizes storage. Failure to remove private scratch remains
explicit pending cleanup owned by the context and is not reported as clean.
Library/parser and OS copies have their ordinary lifetimes; no secret Debug
or log form is introduced. This private configuration-copy exception requires
Safety acceptance rather than being inferred from the existing buffer rule.

## Source owners and binding boundary

Core's `src/git/endpoint/https_auth` owns this mechanism: private configuration
view modules own zeroizing entry/path/value buffers, the bounded file worker,
private scratch ownership and ordered walk/writer. Its existing supervised child
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
one initial discovery, one view round-trip, one controlled-origin verification
and one fill are launched: 132 sequential children, all under one deadline.
This is a proposal, not permission to mutate the protected dependencies.

## Precise supersessions and product checks

This DRAFT, if accepted, supersedes only TR1.6 §3.2's literal snapshot delta
and “Nothing else is added” sentence for the controlled configuration variables
named above; its cwd/no-local/every-conditional-include policy stands. It adds
the bounded supervised read-only discovery/parse/verification commands and context-owned view
preparation before §3.1's otherwise unchanged final credential-fill command.
It qualifies §3.3's lookup-start statement so the same interaction deadline
covers all preparation, without extending clocks. It adds the explicit private
scratch-copy ownership/cleanup exception to §3.4, with no new diagnostic data.
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
round-trip; bounded/cyclic/FIFO and symlink-handle refusal; prepared view plus
helper answer sentinel redaction; cancellation/drop/timeout during discovery,
worker preparation and fill; stopped worker/child retains both quotas and
scratch until joined; failed scratch removal remains pending; no repository
configuration and no network/repository Git command. Repair/undo fixtures
compare synthetic answers without printing them. Keep the executed hasconfig
counterexample red until the adopted mechanism makes that real child green.

Root must settle this single concrete mechanism and obtain independent review
before any child configuration environment or view semantics are implemented.
The already accepted username/timing/fixed-cause implementation and existing
lifecycle/secret work may proceed independently. This is a mechanism correction
for one discovered scope gap, not an authentication-policy expansion.

# Git library Q2 — source keys and isolated consumer composition

Date: 2026-09-21. Status: **bounded implementation in progress**.
Authority: [Q1 qualification map](GwzGitLibraryQualification.md),
[accepted library design](GwzGitLibraryDesign.md), operator continuation.

## Scope and ownership

Q2 prepares a local integration candidate without changing product dependency
selection. It has two independently assessable results:

1. Correct `tests/transport_native/prove.py` source-file keys to use Git's
   slash-separated representation on every path flavour. Native filesystem
   operations retain native paths. The correction must preserve literal POSIX
   backslashes, nested exclusions and all existing file/content/mode/symlink
   and C gitlink admission checks.
2. Prepare isolated copies of the root workspace, standalone core, standalone
   CLI, Python extension and `gwz-git`, resolving each independent lock against
   the same admitted Rust/sys/C sources. Inspect resulting graphs and attempt
   local builds and entry-point smoke tests. Record failures as failures.

Public owned paths: this document, the Q1 status pointer, and
`tests/transport_native/{prove.py,test_prove.py}`. Ceilings: 25 added runner
lines, 140 added test lines, 180 new report lines. Root checkpoint and generated
review prompts/reports are process records. No public product runtime changes.

Private campaign owns `gwz-core-evidence/campaigns/git-library/README.md`,
`runner/compose.py` (at most 300 maintained Python lines), named run evidence,
and one archive catalog entry. Existing public source admission is reused;
the new apparatus covers independent consumer locks, absent from the native
proof fixture. It is not a public CI dependency. Runtime copies and build
outputs live outside repositories; Windows fixtures, when executed, use
unique `D:/gwz-tests/` roots.

## Candidate composition

Use Rust fork `ce78628308e11b4e8901d5061602619109bce21a`, nested C
`b172e3d187a4b6866fd9f696f40a1b8e7f56d348` and library
`aa77c2ce5ad0bf6b4f4b64b2d8fd75e8547c3b4c`. Record all other input revisions
and hashes for each run. Export immutable tracked blobs and modes, including
files marked export-ignore. Reuse the public admission checks on the original
fork before and after qualification. Do not copy build trees or ambient config.

In copied core only, add the library as a path dependency, leaving operations
unchanged. This proves coexistence of core's direct native calls and the
library's dependency; it does not migrate a call site. Copied library retains
its existing SHA-256 and forced-vendoring features. Supply explicit git2 and
libgit2-sys source patches at every consuming Cargo root, rather than assuming
a dependency's patch propagates. Preserve every existing consumer feature.

The root workspace retains its existing taut-shape patch. Standalone CLI must
be outside that workspace; Python retains its own workspace/lock. Metadata
must identify each actual workspace root, one git2 provider, one native links
provider, exact source paths/versions and resolved feature union. No original
manifest, lock, fork or Cargo registry source may be changed.

Allowed candidate lock deltas are adding `gwz-git`, its dependency edge from
core, replacing git2/sys registry provenance with admitted paths, and moving
sys 0.18.5+1.9.4 to 0.18.8+1.9.7 where necessary. All other package records and
edges must remain unchanged. Unexpected resolution changes stop that row and
are reported for a separate decision; no broad dependency refresh is allowed.

## Verification and acceptance

First demonstrate regression failure in the uncorrected source verifier, then
green Python guards. Simulated Windows path-flavour checks establish only key
normalization. Native Windows mode/symlink support and execution remain Q1
gates; do not weaken admission checks to obtain an apparent platform pass.

Run locked/offline Rust 1.95 metadata independently for every candidate graph
after the explicit, bounded lock update. Attempt core/library and CLI builds,
Python extension build/import/health and CLI version/help checks. Run the
existing focused core characterization suite against the candidate when the
graph is admitted. Source admission runs before and after these attempts.
Retain exact commands, statuses, locks, metadata and logs privately; public
summary distinguishes resolution, build, smoke and operation results.

Review tier: dual retained Code/State at the aggregate settled-tree checkpoint;
Code attacks path portability, graph/source/feature composition and claim
accuracy; State attacks admission, isolation, lock drift and evidence integrity.
No API or CLI surface is frozen by this candidate. P0–P2 findings block; P3s
are recorded without creating new packages. At most two remediation rounds.

Passing local composition is not native qualification of every consumer. Every
consumer still needs actual linked version/vendor/object-format assertions,
full operation parity and the Q1 five-target native matrix. The local source
copy is not remote-only distribution proof. Publication, clean source
reconstruction, production activation and fallback removal remain separate.

## Results

Verifier implemented at core `3829f2bb30e96241f73368fcfe26cc7877497e5b`:
14 added runner lines, 86 added test lines; existing admission rules remain.
Drafter observed the new traversal regression fail before the correction;
owner reran the complete Python guard suite afterward: 12 passed on macOS.
The runner and test fixtures also select `D:/gwz-tests` on Windows. This is
source-level regression evidence, not native Windows execution.

All five independent graphs pass final locked/offline metadata and source,
version, feature and lock guards. Local build evidence is deliberately split
across recorded attempts; failed attempts remain failed:

| Consumer | Executed local result | Evidence run |
|---|---|---|
| Root workspace | Build; CLI version/help pass | local-b |
| Core standalone | Build; 17 characterization tests pass | local-b |
| CLI standalone | Build; version/help pass using its own lock | local-b |
| Python standalone | Maturin wheel build; direct extension import/health pass | python-d |
| Library standalone | Build; 13 integration tests and 7 documentation checks pass | local-b |
| All five | Final runner metadata, unique source/providers, exact features and allowed locks pass | metadata-e |

Source baseline for these runs is core `3829f2bb30e96241f73368fcfe26cc7877497e5b`;
other library/fork/C pins above and Q1 CLI/Python pins remain unchanged. Each
run records the full tuple, exported source fingerprints, exact command and
runner hash. Current campaign runner is 231 lines. Raw evidence is
[private: git-library campaign](../../gwz-core-evidence/campaigns/git-library/README.md),
under `runs/2026-09-20-*` (UTC; local date 2026-09-21). Source admission passed
before/after each run; successful candidate rows also compare source snapshots
after testing. This does not claim failed rows completed every later check.

Three execution lessons are incorporated without widening product scope:

- Initial `cargo update -p git2` also rewrote a tempfile/getrandom dependency
  edge. The guard refused root/core. Resolving the candidate with metadata
  preserves that edge, verified against the same strict lock comparison.
- Raw Cargo builds do not supply this macOS Python extension's required linker
  setup. Using the project's maturin build path fixes that build invocation.
- Maturin's metadata discovery did not receive command-line source patches and
  encountered duplicate native providers. Explicit patches in the copied
  Python root manifest align discovery with compilation. The original Python
  manifest remains unchanged. Python packaging plus import passes in python-d.

The final metadata-only run covers the final runner across all five rows;
local-b/python-d supply the build evidence for identical product revisions.
No single all-green build run or installed-wheel/protocol suite is claimed.
Manual guard checks additionally reject wrong/duplicate native packages, an
extra library dependency, a foreign package and the observed unrelated edge.

Retained aggregate Code/State review pending. Production manifests, locks,
call sites, source publication and transport activation remain unchanged.
Next required gate is native per-consumer identity/object-format and platform
qualification, followed by independently reproducible source packaging before
any production activation. Remaining C1/H1 operation evidence proceeds under
its own accepted scopes; this candidate does not freeze those APIs.

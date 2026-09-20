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

Pending execution and retained review. Original product source selection is
unchanged throughout this package.

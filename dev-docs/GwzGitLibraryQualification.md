# gwz-git consumer and native-source qualification

Date: 2026-09-21. Status: **Q1 inventory/plan accepted; activation pending**.
Operator sequencing, 2026-09-21: outstanding platform and selected-source/
distribution checks are deferred into one later batch against the integrated
stack. They do not block local SSH pool/per-remote implementation. See
[continuation and deferred batch](GwzRemoteTransportSshIntegration.md).
2026-09-22 update: the operator resumed that batch. See
[Q6 execution and current limits](GwzRemoteTransportQualification.md); older
qualification rows below remain historical rather than current parity claims.
Authority: [next-package scope](GwzGitLibraryNextPackages.md),
[library design](GwzGitLibraryDesign.md), [G0 acceptance](GwzGitLibraryG0.md).
Follow-up: [Q2 candidate](GwzGitLibraryCandidate.md) records the source-key
correction and isolated local composition evidence; native Windows and the
remaining matrix below are still pending. [Q3 native consumers](GwzGitLibraryNativeConsumers.md)
adds instrumented macOS arm64 execution; it does not close other platforms,
full operation coverage or distribution.
[Q4 Windows](GwzGitLibraryWindows.md) records native source/binding/library
execution and its limits; it does not close the full Windows consumer matrix.
[Q5 Windows consumers](GwzGitLibraryWindowsConsumers.md) records the bounded
four-consumer follow-up and its execution/review status. Read that report for
current Windows artifact evidence; the baseline inventory below remains historical.
This is an inventory and executable gate plan, not all-consumer qualification.
No source distribution, production dependency, lock or runtime changed.

## Exact inspected baseline

| Source | Revision |
|---|---|
| Root workspace | `3589aa05092abb7c38d90747384954565510301f` |
| Core | `6586768396886fe1aeb1371bbd3064377cfa70ec` |
| CLI | `7db07bbdefd2897c07fd0f9e550bf032bd8b1314` |
| Python extension | `d07d55dacb1725d9306be9c04d157ac29a78e000` |
| gwz-git | `aa77c2ce5ad0bf6b4f4b64b2d8fd75e8547c3b4c` |
| Rust fork | `ce78628308e11b4e8901d5061602619109bce21a` |
| C sibling and nested gitlink | `b172e3d187a4b6866fd9f696f40a1b8e7f56d348` |

Audit used GWZ's active-member inventory and manifest searches for direct git2,
sys and indirect core dependencies. Root/core/gwz-git/gwz-py each passed
`cargo +1.95.0 metadata --locked --offline --format-version 1` in its own
workspace. Each resolved exactly one git2 and one `links = "git2"` sys provider.
That is uniqueness within each graph, not agreement across graphs, native
linkage proof, or a build result. Host: aarch64-apple-darwin; Git oracle 2.52.0.

## Consumer and lock matrix

All listed git2 versions are 0.21.0. Registry means crates.io provenance;
local means the prepared sibling path source. Native feature abbreviations:
N = HTTPS/SSH/SHA-256; V = forced vendoring; S = SHA-256 only.

| Consumer / independent lock root | git2/sys source and sys version | Features / evidence |
|---|---|---|
| Root workspace `Cargo.lock` (CLI plus core path dependency) | Registry; 0.18.8+1.9.7 | N; metadata pass; default/cred enabled |
| Core `Cargo.lock` | Registry; 0.18.8+1.9.7 | N; metadata pass; direct sys pin is dev-only |
| `gwz-core/crates/repo-inspect` | Core or consuming root lock | Declares S, no defaults; feature union follows each root |
| `gwz-core/crates/local-testrepo` | Core lock, dev fixtures | Declares S, no defaults; not a production activation consumer |
| CLI standalone `gwz-cli/Cargo.lock` | Registry; 0.18.5+1.9.4 | Lock inspected only; isolated standalone resolution/build pending |
| Python `gwz-py/Cargo.lock` | Registry; 0.18.5+1.9.4 | N; independent metadata pass; core production dependency |
| `gwz-git/Cargo.lock` | Local fork and local sys; 0.18.8+1.9.7 | S+V; metadata pass; G0 native evidence only |
| Core isolated `tests/transport_native/Cargo.lock` | Registry baseline; 0.18.8+1.9.7 | N+V; runner substitutes exact source in isolated copies |

Core's dev-only sys pin does not constrain a downstream consumer of its normal
library dependency. Thus Python and standalone CLI currently record C 1.9.4,
while root/core locks record C 1.9.7. Neither registry composition incorporates
our fork fixes. These differences are activation blockers, not a request to
upgrade unrelated dependencies silently. Standalone CLI tests must run outside
the enclosing root Cargo workspace or they will exercise the wrong lock.

Active member disposition beyond the product consumers above:

- `git2-rs` and `libgit2` provide the implementation. Fork `git2-curl` and
  `systest` are upstream auxiliary consumers: include supported feature/build
  checks when packaging, without activating them as GWZ dependencies.
- `taut`, `taut-shape`, `taut-shape-rs`, `taut-shape-py`, and `gwz-transport`
  have no current core/git2 dependency. The SSH proof uses ssh2, not git2.
- `gwz-core-evidence` is private campaign storage, not a public build consumer.
  Archived checkouts/runners and root `scratch/` are not production graphs.
- Root is the CLI Cargo coordinator, already represented above. The new
  `gwz-git` remains an unpublished member with no remote.

## Source and packaging gates

1. Select one exact Rust/sys/C composition for every consuming Cargo root.
   Preserve each consumer's intended features, including SHA-256's native ABI;
   check minimal and full supported feature unions. A dependency's Cargo patch
   does not propagate into its caller. An integrator-owned candidate must
   explicitly align root, standalone CLI, core, Python and any published-library
   test consumer, then check duplicate source IDs/native `links` conflicts.
2. Reuse `tests/transport_native/prove.py` with an explicit `--git2-source`
   or `--git2-archive`. Exact file/mode/hash admission and nested C gitlink
   checks must pass before and after candidate tests. G0 records nine native
   tests in each mode and ten runner guards; Q1 does not rerun those results.
   Archive mode retains stock C and characterizes its known fetch failure;
   it is not evidence that the fork's C correction was distributed.
3. Correct and regress Windows admission before relying on that runner there.
   Source inspection: `verify_copy` uses `str(relative / name)` for exclusions
   and file-set names, whereas `read_tree` stores slash-delimited Git paths.
   `PureWindowsPath('src') / 'error.rs'` formats as `src\error.rs`, not
   `src/error.rs`; nested `.git` exclusions are affected too. This is a
   source-level counterexample, not an executed Windows qualification run.
   A future bounded runner package needs portable path keys plus disabled-
   platform/source checks and native Windows execution; Q1 changes no runner.
4. Prove actual linked native identity in each test binary, not only Cargo
   metadata: version, vendored/system selection and compiled object formats.
   Candidate qualification fixes vendoring; an accidental system library cannot
   supply evidence for the patched C source. Lock versions must not drift.
5. Prepare independently fetchable pinned sources (C commit before Rust gitlink,
   then library/package source), retaining licence files and patch provenance.
   Rust git2/sys manifests declare MIT OR Apache-2.0; native COPYING includes
   GPLv2 with its linking exception; gwz-git declares GPL-2.0-only. Preserve all
   notices in the actual distribution. No publication is performed by Q1.
6. Reconstruct in a clean environment without sibling checkouts, Cargo-cache
   modifications or the private evidence member. Test the actual selected
   distribution form, its recursive native source and downstream patch behavior.
   Prepared-workspace success and crate archive success are insufficient alone.

## Native matrix and operation gates

Target rows mirror CLI `.github/workflows/platform-gate.yml` and its release
matrix; this table is a requirement, not a claim those jobs ran for gwz-git.

| Native target | Current evidence for selected fork/G0 | Still required |
|---|---|---|
| aarch64-apple-darwin | G0 local suite and native source/archive proofs pass | Every consumer/feature/package build and operation qualification |
| x86_64-apple-darwin | Pending | Same native execution and distribution proof |
| aarch64-unknown-linux-gnu | Pending | Same native execution and distribution proof |
| x86_64-unknown-linux-gnu | Pending | Same native execution and distribution proof |
| x86_64-pc-windows-msvc | Pending; runner path issue identified | Runner regression/correction, then same native gates |

Use Rust 1.95, locked dependencies and offline builds after deliberate source
acquisition. Every required consumer row needs SHA-1/SHA-256 execution,
version/provider assertions and its applicable package surface: CLI binary,
Python extension/import boundary, core/library downstream use and native fixture.
Windows test roots are unique `D:/gwz-tests/` directories. Native execution,
not a cross-compilation pass or another OS result, closes a platform row.

L1 additionally retains tag declared/actual type consistency, missing targets,
ref/FETCH_HEAD/cancellation/partial-outcome characterization before fallback
removal. C1/H1 add limited evidence, not complete commit/tag/history parity.
No all-platform CI dispatch or source publication was performed in Q1. No
all-consumer acceptance is possible while any table row is pending or divergent.

Next bounded work: runner portability regression/correction; isolated consumer
composition and distribution proposal; remaining operation evidence. Each gets
owned paths, numerical scope and review before changing shared source selection.
A final separately reviewed activation package must cover all consumers,
compatibility, native matrix and denied-Git runtime tests before removing a
fallback or enabling production transport. Q1 itself is the readiness map.

Q1 accepted after [Code GO](../../dev-docs/GwzGitLibraryEvidence-ReviewCode.md)
at core `deba48c93a04e6aaf0bab36066b12d1547d5469e`, root
`9fc664de8389ea334f36bc41135cd59448893e05`. This accepts the audited readiness
map only; no pending qualification row is promoted to a pass.

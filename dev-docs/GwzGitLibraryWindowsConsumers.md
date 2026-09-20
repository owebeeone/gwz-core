# Git library Q5 — native Windows consumer qualification

Date: 2026-09-21. Status: **native execution passed; review pending**.
Authority: [Q1](GwzGitLibraryQualification.md), [Q2](GwzGitLibraryCandidate.md),
[Q3](GwzGitLibraryNativeConsumers.md), [Q4](GwzGitLibraryWindows.md),
operator continuation.

## Bounded scope

Complete the four remaining Windows native consumer probes: workspace CLI,
standalone CLI, standalone core example, and Python native extension. Each has
an independently resolved candidate lock and separate native build target.
The standalone library is already covered by Q4. This is qualification of
instrumented artifacts, not activation, ordinary command parity, wheel/package
installation, remote-only source reconstruction, or release readiness.

Use unchanged Q2 composition to export committed sources and admit the pinned
Rust/C/library sources on macOS. Reuse Q3 instrumentation and its exact public
probe in external copies. Retain original/candidate locks, source revisions,
instrumentation bytes and source manifests. Transfer exact file bytes and real
symlink targets to fresh Windows paths under `D:/gwz-tests/`. Verify the complete
file/type/link inventory before and after native execution. Q4's policy applies:
Git executable modes remain recorded provenance; Windows mode bits do not prove
POSIX permission equivalence. No product source/manifest/lock is edited.

Rust fork `ce78628308e11b4e8901d5061602619109bce21a`, C
`b172e3d187a4b6866fd9f696f40a1b8e7f56d348`, and library
`aa77c2ce5ad0bf6b4f4b64b2d8fd75e8547c3b4c` remain fixed.
Record all consumer/core fixture revisions at export. Source archives contain
no Git worktrees, build outputs, caches or credentials. Preparation refuses
unsafe/colliding paths and escaping links; Windows admission requires exact
exported bytes and link targets, not a checkout with normalization enabled.

## Execution contract

Use native Windows x86_64 MSVC and Rust1.95, fresh Cargo home and per-row targets,
fixture-owned HOME/Git/TMP configuration, and explicit Python interpreter.
Before and after every Cargo-bearing invocation refuse both Cargo configuration
filenames through the working directory's ancestors and Cargo home. Deliberate
locked registry acquisition precedes offline locked metadata/build commands.
Recheck native workspace root, unique git2/sys/library paths, versions, sole
native provider and vendored/SHA256/SSH/HTTPS features for every consumer.

Run the existing probe inside both CLI executables, the core example, and the
Python cdylib loaded explicitly through Python's extension loader. Python also
asserts `health() == "ok"`. Runtime PATH has no Git executable. Independently
validate native identity, both object-ID widths/blob hashes, distinct commits,
corrected noncommit-hint fetch and raw native diagnostics. Retain artifact
hashes before/after execution. No wheel or installed Python package is claimed.
All fixtures, caches, checkouts and builds remain outside the evidence archive.
Retain failed attempts separately; do not overwrite a previous run to resume.

## Ownership and exit

Private owned paths: campaign `runner/windows_consumers.py` (350 lines),
`runner/test_windows_consumers.py` (110 lines), one frozen source-preparation
script (180 lines), README extension and named raw runs with exact inputs.
Reuse existing helpers; no changes to accepted Q2/Q3/Q4 runners are planned.
Public owned paths: this report (180 lines), Q1 status pointer (15 added lines).
Root checkpoint/review/prompt records belong to the owner. Production budget:
zero lines and zero files. An unrelated platform/product defect stops its row
for reporting and a separately bounded correction.

Exit requires actual native execution of all four rows, guards, source/lock/
artifact checks, and retained peer-blind Code/State review at a settled tuple.
Code checks graph/artifact/probe identity; State checks isolation, admission,
integrity and failure attribution. P0–P2 block; at most two remediation rounds.
Other platforms, ordinary package behavior, independently fetchable source
distribution and production activation remain separate gates.

## Results

Executed on Dabeest, Windows NT10.0.26200 x86_64 MSVC, Python3.13.5,
Rust1.95.0. Source core `95ad5b6cca0b2692598cbf8ae3d0381567658603`, CLI
`7db07bbdefd2897c07fd0f9e550bf032bd8b1314`, Python
`d07d55dacb1725d9306be9c04d157ac29a78e000`; Rust/C/library pins above unchanged.

| Native consumer | Result |
|---|---|
| Workspace CLI executable | Locked graph, build and both-format probe pass |
| Standalone CLI executable | Independent lock/graph, build and both-format probe pass |
| Standalone core example | Locked graph, example build and both-format probe pass |
| Python native extension | Locked graph, cdylib build, explicit load, health and both-format probe pass |

Every row reports vendored libgit2 1.9.7 with SSH/HTTPS and passes SHA1/SHA256
object/hash/record checks, corrected local fetch and raw native diagnostics.
All object IDs equal the prior Q3 macOS results. Complete source/link/lock
fingerprints match before/after; helper and artifact hashes match; Cargo
configuration absence holds before/after acquisition, metadata and builds.
Probe PATH is empty. Native guard suite passes (five tests); owner Mac consumer
guards pass (four tests); preparation path guards pass on both hosts.

[Private evidence](../../gwz-core-evidence/campaigns/git-library/README.md)
requires member access: `2026-09-21-q5-composition-a` retains exact exports,
locks, instrumentation and transfer records; `2026-09-21-q5-windows-a` retains
native logs, commands, source fingerprints, helper/artifact hashes and derived
owner verification. Source archives/build outputs remain external. Preparation
used an explicitly recorded one-line Windows absolute-drive validation
adaptation; original pack script retained unchanged. All four native rows
passed on the first execution; no product/platform correction was necessary.

The new private runner is 191 lines and guards 59 lines. Existing Q2/Q3/Q4
helpers and all production sources are unchanged. Q4 plus Q5 establishes the
five Windows instrumented consumer shapes within their stated bounds; it does
not establish general command parity, installed-package behavior, other native
targets, remote-only reconstruction or production activation.

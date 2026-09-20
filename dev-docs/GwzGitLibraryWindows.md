# Git library Q4 — native Windows qualification

Date: 2026-09-21. Status: **execution passed; retained review pending**.
Authority: [Q1 qualification](GwzGitLibraryQualification.md),
[accepted Q3](GwzGitLibraryNativeConsumers.md), operator continuation.

## Scope and sources

Qualify source admission and the existing library/native-binding fixtures on
Dabeest, Windows x86_64 MSVC, Rust 1.95.0. Use fresh unique directories below
`D:/gwz-tests/`; compiled outputs, checkouts and Cargo cache remain external to
all repositories. This closes only those executed Windows rows, not the five
consumer matrix or production activation. The existing Q3 campaign remains
macOS-only and unchanged.

Pin Rust fork `ce78628308e11b4e8901d5061602619109bce21a`, C
`b172e3d187a4b6866fd9f696f40a1b8e7f56d348`, library
`aa77c2ce5ad0bf6b4f4b64b2d8fd75e8547c3b4c`. Record the core fixture revision.
Transfer immutable source bundles/exports and hashes to an external Windows
runtime. Preserve historical Git objects required by source admission and the
exact nested C gitlink. Disable checkout line-ending conversion and require
real source symlinks; refusal to create them stops admission. Set both
`core.autocrlf=false` and `core.eol=lf`, including for text attributes.
Git for Windows can rewrite link-target separators;
the campaign may recreate a real link with its exact immutable Git target only
after verifying that separator conversion is the sole checkout difference.
Record each recreation; plain-file substitution or any other target drift
still refuses admission. No remote branch
publication, product dependency/lock change, or user Cargo configuration edit.
The transfer is local-source distribution rehearsal, not remote-only fetching.

## Admission policy and bounded correction

First run the existing source-admission tests and attempt admission on native
Windows. Record any failing counterexample before correction. Git tree modes
remain the source authority. POSIX filesystems must match executable bits;
Windows exposes suffix-derived executable bits rather than POSIX permissions,
so those bits cannot establish or refute equality with a Git executable mode.
If that counterexample occurs, make this explicit in the verifier: preserve
exact source type, symlink target, file set, bytes, revision and gitlink checks,
while checking executable-bit equality only on POSIX. Do not turn links into
ordinary files, omit license links, or omit source paths to obtain a pass.
Regression checks must reject regular-file/symlink substitution on both
platforms and POSIX executable drift; native Windows must actually exercise
script modes and real symlinks. No ACL/execution-policy equivalence is claimed.

Public owned paths: `tests/transport_native/prove.py` (at most 30 added lines),
`test_prove.py` (at most 100 added lines), its README (at most 35 added lines),
this report (at most 180 lines), and Q1 status pointer. Native run windows-d
also exposed invalid file URLs made from Windows canonical paths. Extend the
owned test-only scope to `src/lib.rs`, the three native integration test files,
and `consumer_probe.rs` (at most 80 added Rust lines total): encode file URLs
from libgit2's normalized repository paths, preserving explicit native file
transport and escaping path bytes. Re-run the failing native cases and macOS
regressions; no production/library/fork Rust code, manifest or lock changes.
The Q3 probe remains self-contained for its existing consumer instrumentation. Unexpected defects
outside these bounds stop the affected row and receive a precise report.

Private owned paths: git-library campaign `runner/windows.py` (at most 260
lines), `runner/test_windows.py` (at most 100 lines), README extension, named raw runs with frozen inputs/results. The runner
is campaign orchestration, not a public build dependency. It must use fresh
paths, retain command/exit/output records and source fingerprints, preserve
failed attempts, and verify source before and after tests. Avoid a generalized
packaging or remote execution framework. Root checkpoint/review records belong
to the integrator.

## Execution and acceptance

Use a fixture-owned Cargo home/config, HOME and Git configuration. Refuse both
Cargo config filenames in every effective working-directory ancestor and the
Cargo home; record their absence before/after each Cargo invocation, including
the proof runner's temporary D: child. Never modify external configuration. TMP/TEMP
also point below the unique D: root so unchanged tests using the OS temporary
directory cannot create fixtures on C:. Download locked registry dependencies
deliberately before offline execution; retain locks and prohibit unrelated
resolution changes. Never modify installed Cargo registry sources or personal
configuration. Existing compilers/toolchains may reside on C:.

Run the public Python guard suite and patched-source native proof (nine tests,
including local fetch and per-remote callbacks). Run the existing `gwz-git`
integration and documentation tests. Reuse the exact public Q3 consumer probe
in an isolated library example to assert vendored 1.9.7, minimal features,
SHA-1/SHA-256 objects, corrected local fetch and owned native diagnostics.
Runtime probe PATH contains no Git executable; other fixture tests may use Git
as their documented local server/oracle. All test servers are local.

This package is accepted only after actual native execution, unchanged
source/lock checks, and retained dual Code/State review at a settled tuple.
Code attacks platform policy, binding/library assertions and claim boundaries;
State attacks source/link admission, fixture containment, lock/source drift and
failure attribution. P0–P2 block; at most two remediation rounds. No public
product API freezes here. The scope and results are reviewed together; the
explicit Windows mode policy above precedes any verifier implementation.

The four remaining consumer shapes on Windows, other native target rows,
full operation parity, independently fetchable sources and package publication
remain later gates. Keep forks on C 1.9.7. No endpoint activation or fallback
removal follows from this evidence.

## Results

Executed on Dabeest, Windows NT 10.0.26200, Python3.13.5, Rust1.95.0
x86_64-pc-windows-msvc. Final public fixture revision:
`5f6cc919c880f7c540013bbfbc3bcb0fa81f2ec8`; library/Rust/C pins above unchanged.
[Private raw evidence](../../gwz-core-evidence/campaigns/git-library/README.md)
retains git-library runs `2026-09-21-windows-a` through `windows-f`, plus
`2026-09-21-mac-q4-e`. The archive requires private-member access.

| Final Windows row (windows-f) | Observed result |
|---|---|
| Python source/lock guards | 13 passed; one explicit POSIX-only skip |
| Patched-source native binding proof | All nine tests pass, including per-remote callbacks and corrected local fetch |
| Existing gwz-git suite | 13 integration tests and seven documentation checks pass |
| Instrumented library example | Vendored 1.9.7, minimal features, SHA-1/SHA-256 objects, local fetch and raw diagnostics pass |
| Source/lock/artifact integrity | Before/after source admission and fingerprints pass; probe binary unchanged |

Q3's unchanged parser independently validates Windows probe output; all IDs
match Q3's macOS library output. The probe executes with an empty tools PATH.
Native fixture tests use Git only as their documented local servers/oracles.
Fresh registry acquisition preceded locked offline execution. Target/cache and
fixtures remain on D: outside repositories. Local-source Git bundles preserve
provenance but are not independently fetchable remote source distribution.

Failures remain explicit: windows-a reproduces executable-mode refusal;
windows-b refuses Git-normalized link target text; windows-c refuses CRLF
checkout under text=auto; windows-d passes G0 but rejects malformed Windows
fixture file URLs before fetch. The final run corrects source preparation and
test-only URL construction, without weakening bytes/type/link admission or
changing product, native-fork or library behavior. The helper uses normalized
repository paths and percent-encodes path bytes; explicit file transport stays.
MacOS regression: nine native tests and both-format minimal library probe pass.

Change size: five added verifier lines, 44 added Python test lines, 34 added
Rust fixture lines (four replaced URL lines); 237-line private Windows runner and 37-line config guard suite.
Initial Code review returned GO; State P2-1 found that Cargo ancestor config
was not checked. [Remediation](../../dev-docs/GwzGitLibraryWindows-RemPlan.md)
adds explicit refusal/regression and a fresh native rerun. Windows-e alone is
not configuration-isolated acceptance evidence. Native windows-f repeats all rows successfully, with before/after absence
records for both config names throughout every Cargo search chain and Cargo
home. The new refusal guard passes on Windows and macOS. State closure is pending. This qualifies Windows source
admission, the native fixture and G0 library only. Root/standalone CLI, core
and Python consumer artifacts on Windows, the other native targets, broader
operation coverage, distribution/publication and activation remain pending.

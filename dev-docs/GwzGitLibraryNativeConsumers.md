# Git library Q3 — native consumer execution

Date: 2026-09-21. Status: **bounded qualification in progress**.
Authority: [Q1 gates](GwzGitLibraryQualification.md),
[accepted Q2 composition](GwzGitLibraryCandidate.md), operator continuation.

## Scope

Exercise the native library inside instrumented, isolated candidate executables
for the root CLI, standalone CLI, core downstream example, Python extension,
and library downstream example. Q2 established dependency composition and
ordinary build/smoke behavior. Q3 adds native version/vendor/features and
SHA-1/SHA-256 object and local-fetch observations from the running consumers.
This is a manual qualification campaign, not a release/CI compiler probe.

One public test fixture, `tests/transport_native/consumer_probe.rs` (at most
200 lines), owns the common assertions. Its adjacent `consumer_probe.md` is at
most 80 lines. This report is at most 180 lines. No product Rust source,
manifest, dependency lock, native source or existing public API changes.

Private owned paths: git-library campaign `runner/native.py` (260 lines),
`runner/test_native.py` (100 lines), README extension and named raw runs.
Reuse the accepted Q2 composition runner unchanged for source admission,
independent locks and features. Runtime copies, fixtures, wheels and targets
remain external. Windows execution, when attempted, requires unique
`D:/gwz-tests/` roots. The current execution target is local macOS arm64;
other native targets remain pending, not simulated by these checks.

## Instrumentation and observations

Only copied consumers receive instrumentation, whose exact bytes and hashes
are retained with the run. Core gets a test module; CLI's copied entry point
calls it only for a campaign environment variable. Core/library examples call
the same fixture. The copied Python extension registers a campaign-only probe
function, invoked through an explicitly loaded built extension. These are
instrumented artifacts, not unchanged production/release binaries. No new
diagnostic API, CLI option, wire field or transport interface is shipped.

The fixture creates fresh isolated repositories and makes no Git subprocess
calls. It asserts libgit2 1.9.7 and vendoring, reports actual native SSH/HTTPS
capabilities, and exercises both object formats. It writes a binary blob,
tree and deterministic commits, reads them through the native API and owned
`gwz-git` records, and verifies full IDs, parent order, signatures and bytes.
The independent Python reader computes the Git blob SHA from canonical input;
incorrect hashes or malformed/incomplete results fail the row. Full-feature
consumers must report SSH/HTTPS; the minimal library must report neither.

Each format also exercises the corrected local-fetch case: a receiver has a
shared noncommit hint, then imports a genuinely new commit. Preserve the hint
and verify the resulting ref/object. A malformed graft fixture checks the raw
native diagnostic class preservation. These observations supplement source
admission; they do not imply all Git operations or network paths are qualified.

Run a controlled negative case first: the minimal library with exactly stock
C 1.9.7 (`49e408b3208bc3093757a1c2db938d3590f3f412`) in an isolated copy.
The same-version control must fail specifically at the noncommit-hint fetch;
a build failure, missing executable or unrelated panic does not count. Then
run all five consumers with the accepted C correction. Record the control
deviation explicitly; never present it as an admitted patched candidate.

## Guardrails and acceptance

Use explicit Rust 1.95, offline locked builds after Q2's bounded resolution,
controlled Git configuration and separate per-consumer artifact directories.
Keep native vendoring explicit. Verify candidate source snapshots, locks and
fork admission before/after execution. Record exact commands, versions,
input/instrumentation/artifact hashes, exit status, stdout and stderr. Failed
attempts remain failed; any retry uses a fresh run. Parser tests must reject
wrong native identity/features, altered blob hashes and incomplete results.

The private campaign is not a dependency of public builds. The public fixture
can be reused later by native qualification infrastructure. Native Windows
mode/symlink admission, the remaining OS/architecture matrix, full operation
parity, clean remote-only source reconstruction, distribution/publication,
production activation and fallback removal remain separate gates.

Aggregate review tier: retained dual Code/State, at a settled source/evidence
tuple. Code checks binary routing, format assertions and claim limits; State
checks fixture isolation, negative-control attribution and provenance. No
production surface is frozen. P0–P2 block; at most two remediation rounds.

## Results

One complete native run passed on macOS arm64, Rust 1.95.0. Public fixture
baseline: core `b68d3c5aa53165e868acbae2f5cc13699951c8e0`; unchanged G0
library/Rust/C sources and Q1 CLI/Python pins. Full tuple, commands, frozen
inputs and artifact hashes are in [private raw evidence](../../gwz-core-evidence/campaigns/git-library/runs/2026-09-21-native-a/README.md).

| Instrumented artifact | Native features | SHA1 / SHA256 objects, fetch, diagnostics |
|---|---|---|
| Root workspace CLI executable | Vendored 1.9.7; HTTPS + SSH | Both pass |
| Standalone core downstream example | Vendored 1.9.7; HTTPS + SSH | Both pass |
| Standalone CLI executable | Vendored 1.9.7; HTTPS + SSH | Both pass |
| Explicitly loaded Python extension | Vendored 1.9.7; HTTPS + SSH | Both pass |
| Library downstream example | Vendored 1.9.7; neither HTTPS nor SSH | Both pass |

Stock C 1.9.7 built successfully and failed specifically at the required
noncommit-hint fetch with InvalidSpec (-12). The patched same-version builds
passed that operation for both formats in every row. All five consumers
returned identical deterministic IDs; Python independently verified the Git
blob hashes. Malformed-graft opens preserve raw class 36/code -1 for each
format through the owned library error. This combines executed behavior with
source admission, rather than claiming a version string identifies a patch.

Three parser tests pass, including rejection of altered identity/features,
object hash/width, fetch/error results and incomplete/duplicate frames.
Fixture rustfmt passes. Actual size: 168 public Rust lines, 196 private runner
lines, 41 parser-test lines. Public scope/docs and private raw evidence are the
only other additions. No existing Q2 runner or production dependency changed.

Original fork admission passed before/after; copied source, locks and built
artifact hashes remained unchanged during each probe. Runtime PATH was empty
and Git config isolated. Native execution made no Git subprocess calls.
Retained aggregate review is pending. Native Windows and other architectures,
full operation/network coverage and source distribution remain open; no
production endpoint activation or fallback removal follows from Q3 alone.

# Remote transport adapter foundation

Date: 2026-09-20. Status: implementation candidate, no production activation.
Authority: Remote Transport Design §8 and Plan Phase 3, following accepted
NativeBinding qualification. This slice prepares a distributable binding and
the blocking read/write boundary before attaching an SSH session worker.

## Dependency choice

Prepare a local `gwz-git2` 0.21.0-gwz.1 candidate with library name `git2`, using
the exact accepted upstream archive and two-file patch. Keep upstream licenses,
record provenance and test the resulting Cargo package. Preparation publishes
nothing. Existing product manifests keep stock git2 until distribution and
platform gates are met. An upstream release remains the preferred long-term
exit; the fork provides a concrete fallback without a root-only Cargo patch.

All direct first-party git2 consumers must eventually select the same package:
core, repo-inspect, local-testrepo, and CLI's dev-dependency. Preserve each
feature set and native libgit2 identity. Reconcile root, standalone core and CLI
release locks at activation. Do not mix official and forked Rust types or claim
a path-only dependency works for registry consumers. The locally prepared
candidate is reviewable; publication and remote repository creation remain
separate actions. Its proposed package name/version is not a registry reservation.

## Blocking Git boundary

Keep host-specific code in core, outside gwz-transport. The first module is
`src/git/endpoint/stream_io.rs`, compiled by the isolated consumer qualification
fixture before production module/dependency wiring. It wraps the accepted
Stream futures in std::io Read/Write without a private byte protocol, physical
carrier, socket, buffering layer or new CLI/core API. The fixture runner copies
that exact source into an isolated core-shaped tree so it can be tested against
the verified transport archive without a sibling checkout.

Each blocking call owns a wake token and condition variable. Poll outside the
wake lock; retain wakeups that happen before sleeping; tolerate spurious wakes.
Do not consume the calling thread's park token. Writes retain normal partial
write behavior and std::io write_all drives repeated writes. An independent host
must service message delivery and timers; a blocked Git call must never become
the only pump. Timer-driven coalescing must work without libgit2 calling flush.

Preserve errors as std::io error sources. Cancellation is a non-retryable error
at this boundary (std::io write_all retries Interrupted), timeout is TimedOut,
delivery loss is BrokenPipe, and protocol failures are InvalidData. EOF alone
returns zero; errors cannot masquerade as EOF. Explicit close returns cleanup
facts; drop relies on final Stream-owner cancellation and cannot declare reuse.

## Gates and remaining scope

TDD: missing manifest transformer and missing bridge API compile failures first;
then exact alias/identity tests and bounded concurrent bridge tests. Qualify
packaged fork against all seven native binding tests. Verify the archive-backed
consumer suite plus cancellation, EOF/error separation, partial writes, bounded
backpressure, and coalescing from a separate host timer. No SSH trust, network
pump, pool disposal or cross-platform evidence is implied by these tests.

Budgets: 160 distribution runner lines, 150 bridge source lines, 300 new test
lines plus concise documentation. No new production runtime owner. Review tier:
dual Code/State for the ownership boundary, plus Surface for the blocking API
and distribution commands; retain original reviewers. Acceptance of this slice
is not completion of Phase 3. Next is the nonblocking SSH session owner,
credential/known-host parity, pool integration, and complete network-entry
coverage before activation.

## Qualification record

Rust 1.95/macOS: 21 isolated archive-backed consumer tests pass, including three
new blocking-I/O tests. Seven native binding tests pass both before packaging
and against the exact extracted fork archive. Three distribution-transformer,
two native provenance and nine archive-admission Python tests pass. Formatting
passes. Missing implementation compile failures were observed first; two fixture
errors (endpoint-only timeout state and missing reverse EndWrite) were corrected
before green. Cancellation's non-Interrupted mapping prevents retry loops.

Candidate archive SHA-256:
`2c0544413ee18231fb9185ad29cb82ffa34c223245cd044523be1515897f68af`.
The pinned packaging lock covers optional/dev resolution independently of the
locked native fixture graph; the latter may change only package identity/source.
Production Cargo manifests, locks, SSH code and transport-owned code are unchanged.
The existing consumer archive provenance remains
`986033108eab2967028dc52c69f94e859ed6cbb78384648f03e88d9703383191`.

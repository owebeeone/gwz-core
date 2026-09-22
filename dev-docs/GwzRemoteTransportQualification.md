# Transport qualification and performance — Q6 batch A

Status: **execution complete for batch A; aggregate review pending**.
No production activation or complete platform-parity claim.
Operator authorized the deferred platform/source batch and local measurements on
2026-09-22. This supersedes the prior deferral for those checks. H2 acceptance is
historical implementation evidence; the qualification defect below blocks rollout.
Authority: RemoteTransport Plan Phase 6 and GitLibrary Qualification matrix.

## Matrix and scope

| Target | Executable scope | Current limitation |
|---|---|---|
| macOS ARM64 | Integrated host/endpoint, source admission, local SSH/HTTPS measurements | Local loopback results are not WAN or real-account qualification |
| Linux x86-64 (WSL) | Portable transport and integrated Unix candidate | Isolated native Linux execution; no native Windows implication |
| Windows x86-64 MSVC | Portable transport and selected native Git-binding fixture | Integrated host, SSH network/key/agent adapters are Unix-gated and unimplemented on Windows |
| Linux ARM64 | Environment inspected | Available VM lacks Rust and shares host disk pressure; not run |
| macOS x86-64 | No available runner identified | Not run |

A successful zero-test Unix-gated executable on Windows is not a passing endpoint
row. Exact sources are copied to isolated native runtime directories with hashes;
Windows fixtures/builds remain under D:/gwz-tests. No production dependency,
wire protocol, publication, push, tag or real credential configuration is changed.

Private raw evidence lives in
`gwz-core-evidence/campaigns/transport-qualification/runs/2026-09-22-q6-a/`.
The campaign records source/archive identities, adapted instrumentation and raw
failures. Public tests and builds do not depend on the private archive.

## Qualification discovery — shared-session retirement

Two independent performance runs failed on the fourth repeated 4 MiB HTTPS clone
with CarrierLost. Retained State investigation identified a generic host lifecycle
P2: successful mux request retirement was recomputed on subsequent driver passes.
The mux had removed the request, so finish returned InvalidRequest; after the old
five-second cleanup deadline the host closed a healthy shared session.

This is a post-acceptance candidate defect, not a released defect. H2's passing
short-lived fixtures did not expose it. Correction records mux retirement as a
monotonic fact independently of physical cleanup. A deterministic product
regression completes a request, backdates both driver and endpoint cleanup ages,
and requires both sessions and a subsequent request to remain usable. Runtime red
is captured; a prior missing-import fixture compiler failure is retained separately.

The deterministic regression and corrected workload pass on macOS and Linux.
The correction and its evidence require retained Code/State aggregate review.
No timeout or protocol semantics are relaxed to make performance tests pass.

## Source qualification

Exact Rust/sys/C member admission and its 14 source-guard tests pass locally.
Native Windows portable transport and all nine selected Git-binding tests pass.
Initial Windows tar extraction failed on a symlink; explicit Python reconstruction
then preserved all five exact link targets and regular-file hashes before tests.
The original failed extraction remains recorded. Native source bundle qualification
does not establish independently fetchable publication or installed consumers.

Q5 State P3-1 runner diagnostic retention is corrected with completion (success
and failure) and timeout regressions, awaiting retained verification. Historical
Q5 snapshots are unchanged.

## Measurement protocol

Use existing actual host/endpoint paths in an instrumented external core copy.
Keep fixture/server/repository setup out of timers. Record 11 cold and 11 pooled
advertisement samples per scheme; report sample 0 separately as warm-up. Assert
receipt reuse and connection identity, retain every raw sample, and report median
and observed range rather than extrapolated WAN speedup. Sequential 4 MiB
incompressible HTTPS clones verify exact resulting content and object identity.

The initial runs reproduce the retirement defect and are diagnostic evidence,
not accepted performance baselines. Corrected runs and a macOS confirmation pass. No construction defaults changed.
Full workspace fetch/push, both placements, sustained concurrency/backpressure
memory, real-account/WAN checks and coalescing comparisons remain to measure.

## Batch A results

| Gate | macOS ARM64 | Linux x86-64 / WSL | Windows x86-64 |
|---|---|---|---|
| Portable transport | Pass | Pass | Pass |
| Corrected integrated host | 53 pass | 53 pass | Unimplemented |
| Endpoint suite | Historical H2 69 pass | Current 69 pass | Unimplemented |
| Selected native Git binding | Current source admission/14 guards; native tests historical | Matching source/lock graph; isolated native tests not rerun | Current nine pass |
| Runner diagnostic regressions | Ten pass | Not rerun | Ten pass |
| SSH/HTTPS performance fixture | Pass, repeated | Pass | Unimplemented |

Portable counts are 140 passed and two ignored per platform including doctests;
ignored campaigns remain unexecuted. Counts are observations, not acceptance pins.

Ten measured loopback advertisement exchanges after sample0 warm-up:

| Host / scheme | Cold median (observed range), ms | Pooled median (observed range), ms |
|---|---:|---:|
| macOS / SSH | 136.7 (124.5–177.8) | 74.3 (62.6–82.7) |
| macOS / HTTPS | 104.1 (89.9–137.9) | 65.6 (58.3–75.7) |
| Linux / SSH | 157.2 (152.0–162.4) | 65.8 (60.8–65.9) |
| Linux / HTTPS | 61.2 (56.0–66.8) | 50.5 (45.3–50.6) |

All measured warm exchanges reuse one connection. Four sequential verified 4 MiB
HTTPS clones deliver median application-payload rates 2.13 MiB/s on Mac and 2.70
MiB/s on Linux. Mac confirmation gives 2.23 MiB/s and 49.9 MB process maximum RSS.
The process includes fixtures; this is not a bounded-memory proof. These fixtures
use selected-key SSH and anonymous TLS HTTPS; they do not benchmark Gh helper
latency. Different hosts/filesystems prevent an OS speed comparison. Linux runs
from the WSL-mounted D: filesystem. Timers omit setup and request registration.

Private [run record](https://github.com/owebeeone/gwz-core-evidence/tree/main/campaigns/transport-qualification/runs/2026-09-22-q6-a)
requires private-member access; local unpushed commits are authoritative until
separately published. First failed runs, corrections, every sample, exact inputs
and command output are retained. No public build depends on this archive.

## Remaining Phase 6 work

Finish both-placement aggregate fetch/push/post-push reads, compare immediate
emission with 100 ms and other coalescing settings, and measure sustained-memory
behavior before selecting defaults. Implement the missing Windows integrated
adapter before attempting its platform gate. Intel Mac/Linux ARM64 runners and
independently reproducible source distribution/installed consumers remain open.
Physical wire/iroh stays outside this cycle, as instructed. Phase 6 is not complete.

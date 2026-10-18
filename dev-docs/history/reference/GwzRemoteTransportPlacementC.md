# Endpoint placement C — in-process message embedding

Status: **accepted for same-process embedding only**, 2026-09-22, at the exact
tuple below after retained Code and State GO. No blocking findings; State P3-1
remains a tracked evidence follow-up. Annotation commits do not expand the
reviewed implementation.
Authority: the operator's narrowed C scope in
[Placement Design](GwzRemoteTransportPlacementDesign.md#operator-scope-clarification--2026-09-22).
A/B remain accepted. This package proves embedding, not production activation.

## Boundary and proof

The Rust CLI passes core-generated operation structs directly; e.g.
`gwz-cli/src/clirequest/common.rs` holds `InitFromSourcesRequest`, and
`globalargs/dispatch.rs` invokes the core init handler with it. The candidate
fixture exercises that same generated request/response and direct-handler
boundary. It does not build or activate a new CLI command or replace its dispatcher.

The Python branch executes the actual `gwz-py/src/gwz/protocol/codec.py` in an
embedded Python interpreter in the same Rust test process. Test-only injection
selects the accepted candidate generated dataclasses/IR. Its decode/encode path
handles each live attachment and the ordinary request/final response. It is not a
mock codec, subprocess, or separately running service. The production Python
native extension dispatcher is not activated or replaced by this proof.

Both consumers carry the shared transport Envelope inside the existing optional
RequestMeta/ResponseMeta fields, preserving request IDs and ordinary body fields.
The fixture routes attachment-bearing messages to the transport port before
operation/result dispatch; only the ordinary attachment-free request invokes the
handler, and only its ordinary final response is an operation result. No new
Taut service method, external command, or alternative serialization schema.

Tests exercise real loopback SSH with separate endpoint/core credential
homes, payloads larger than the receive window, bounded one-message forwarding
slots, paused delivery, cancellation and logical closure. Exact original bytes
must appear in the cloned repository. Existing compatibility and host tests remain
applicable; this package does not claim every command or full executable coverage.

## Future wire plausibility

The same generated wrappers encode the complete message as CBOR, including the
request ID, session/stream IDs and typed transport payload. Transport data crosses
the boundary as bytes, never Rust handles, Python objects, callbacks or pointers.
The candidate roundtrips those bytes in memory. A future host connection needs to
preserve per-direction message ordering, bounded admission, two-way progress,
receiver affinity and closure notification. Those are host obligations, not a
new transport-owner API. Framing, authentication of that host connection,
reconnection/process death and actual wire interoperability are not tested here.
Iroh implementation and physical wire qualification are outside this cycle.

## Gates

One aggregate retained Code/State review on the committed candidate plus its
recorded evidence. No public API change; the accepted Surface guide remains the
embedding API. Platform and selected-source checks stay deferred together.


## Local results

Complete host suite33 passed, including eight embedding tests: Rust/Python live
Git exchange, cancellation and logical closure while delivery is paused, wrapper
correlation/payload preservation, and malformed present Python attachment rejection.
The live exchange pauses Data too and reconstructs an exact 256KiB payload through
Data/Window credit flow. Python preparation/retained-candidate13 pass; regeneration,
scoped formatting and the default production library check pass. No whole-core or
platform pass claim. The wrapper boundary red and helper compile failures are
preserved in the [private run](../../gwz-core-evidence/campaigns/ssh-integration/runs/2026-09-22-placement-c-in-process/README.md)
(access required), with final source and consumer/input hashes.

## Acceptance record

Retained [Code GO](../../dev-docs/GwzRemoteTransportPlacementC-ReviewCode.md) and
[State GO](../../dev-docs/GwzRemoteTransportPlacementC-ReviewState.md) reviewed:

| Repository | Revision |
| --- | --- |
| `.` | `f3ad29ae5aa55ebd4e558f3f11a078e6b837196e` |
| `gwz-core` | `c5dd307142e6958160efabf36a8521b5f104c157` |
| `gwz-transport` | `03d3011b3ae9b8205bcf07f7f7862194af114856` |
| `taut` | `bcf98b64d465fc54841121b6d1a2d46940f81a3c` |
| `gwz-cli` | `7db07bbdefd2897c07fd0f9e550bf032bd8b1314` |
| `gwz-py` | `d07d55dacb1725d9306be9c04d157ac29a78e000` |
| `git2-rs` | `ce78628308e11b4e8901d5061602619109bce21a` |
| `libgit2` | `b172e3d187a4b6866fd9f696f40a1b8e7f56d348` |
| `gwz-core-evidence` | `d096a9dcf0d43e79ea32bced5b802bf8a877ce1d` |

One aggregate gate, zero remediation rounds. Code found no P0–P3; State found
one nonblocking P3 at settled review. No known released escaped defect. Both
reviewers independently reran the eight embedding tests successfully. Owner
host33 and Python13 results above remain the full local evidence. The captured
initial adapter red is an implementation-stage unimplemented-boundary failure;
compiler-attempt source hashes were not all captured. Wall time was not captured.
Core delta: test/harness581 additions and2 deletions across7 files; documentation94
additions across2 files; authored production implementation0.

**Open State P3-1 — later activation gate:** assert `CleanupReport` from request
finish and endpoint/runtime shutdown, inject retained work through the existing
seam, and demonstrate unexpected pending work fails before successful retirement
passes. Current tests prove waiter release and return from teardown, not complete
physical worker retirement. This GO must not be cited as that stronger evidence.

Next programme work is the Phase5 HTTPS adapter/authentication design and its
reviewed implementation boundary. Platform and selected-source qualification
remain together in the operator-deferred batch; frontend/production activation
and release remain separate. No physical wire or iroh work is added to this cycle.

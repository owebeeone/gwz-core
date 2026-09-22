# Endpoint placement C — in-process message embedding

Status: **implemented proof; pending aggregate review.** 2026-09-22.
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

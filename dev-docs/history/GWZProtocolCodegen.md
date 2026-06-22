# GWZ Protocol Codegen

GWZ Core uses taut as the protocol authority for v0 request, response, event,
and result messages.

## Source

- Schema: `protocol/gwz.taut.py`
- Generated Rust: `src/protocol/generated.rs` and supporting protocol modules
- Conformance tests: `tests/protocol.rs`

Generated protocol code is checked into the repository so downstream crates can
build without running the generator during normal compilation.

## Rules

- Protocol schema changes MUST start in `protocol/gwz.taut.py`.
- Generated Rust MUST NOT be hand-edited.
- Regenerated output MUST pass `cargo test`.
- Wire enum values covered by protocol tests MUST stay stable unless a protocol
  major version change is intentional.

## Deferred Protocol Work

- Persistent operation logs under `.gwz/operations/<operation-id>.jsonl`.
- Transport bindings beyond in-process request/response use.
- Expanded source catalog messages after v0 manifest-local source records.

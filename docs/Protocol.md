# Protocol

The GWZ protocol is authored in `protocol/gwz.taut.py` and generated into Rust
types under `src/protocol/generated.rs`. The schema identity used by current
requests is `gwz.protocol/v0`.

## Service

The taut service is `GwzCore`. It contains unary request/response methods for
workspace operations, a log-shaped event stream, and an operation-result query.
See [MessageCatalog](MessageCatalog.md) for the generated method table.

Core service methods map to Rust handlers. `events.subscribe` and
`operation.result` model runtime observation of operation records.

## Transport

`gwz-core` does not define a network transport. A caller can use the generated
types in-process, or serialize them with the generated CBOR runtime exposed as
`gwz_core::Cbor`, `gwz_core::encode`, and `gwz_core::decode`.

Transport bridges should preserve:

- request and response message names;
- `RequestMeta.request_id`;
- `ResponseMeta.operation_id`;
- envelope aggregate status and per-member status;
- unknown-field behavior supplied by the taut runtime when crossing versions.

## Envelopes

Unary operation responses wrap `ResponseEnvelope` in an operation-specific
response struct. The envelope carries metadata, member records, and operation
errors. A successful transport call can still contain an operation-level
rejection or per-member failure.

## Events

`events.subscribe` streams `OperationEvent` values. Events carry operation id,
request id, sequence, timestamp, event kind, severity, optional member context,
optional error, optional attribution, and optional transfer progress.

## CLI-Local Exec Values

`ExecMode`, `ExecRequest`, `ExecResponse`, and `ExecResult` exist in the GWZ
schema for `gwz forall` machine output. They are not service methods and have no
`gwz-core` handler. The CLI lists members through `LsRequest`, executes child
processes locally, and can shape results with these types.

## Corpus

`protocol/corpus/golden.json` and `protocol/corpus/rust/vectors.rs` are
conformance artifacts for generated protocol encoding. They must be regenerated
when the taut schema changes. A stale generated protocol or corpus should fail
verification.

## Evolution

- Keep field tags stable.
- Do not reuse retired tags or names.
- Prefer additive optional fields.
- Regenerate bindings, corpus, and catalog after schema edits.
- Keep Rust API, protocol API, and workspace artifact schemas documented as
  separate contracts.

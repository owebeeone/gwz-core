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

Branch and stash are normal core service methods. `BranchRequest` supports
list/create/delete and current-attached-branch merge behavior. Clean branch
merges report the resulting commit per member; conflicted merges report
`BranchActionResult.conflicted` with per-member conflict paths and leave the
native Git merge state intact for user resolution. `StashRequest` supports
push/list/apply/pop/drop coordinated bundle behavior.

`clone_local_workspace` (`ActionKind` 27) and `local_family` (`ActionKind`
28) are the local clone family methods allocated 2026-09-05 for the LCM1.0c
checkpoint: `CloneLocalWorkspaceRequest` creates a named local clone
(`LocalCloneMode` verbatim/clean/bare), `LocalFamilyRequest` lists, disposes
or disbands family members (`LocalFamilyOp`, named `force_hazards`
waivers), and the optional `MergeRequest.local_source_name` (tag 9) selects
a family member as a merge source. `CloneWorkspaceRequest.url` stays
required. LCM1.0c follow-up 2 (operator rulings 2026-09-05) adds
`CloneLocalWorkspaceRequest.copy_source` (tag 6, the `--from <name|path>`
selector; refused as unsupported until LCM3.2), the `gwz local list`
payload `LocalFamilyResponse.members` (a list of `LocalFamilyMemberEntry`:
`name`, `kind`, `recorded_state`, `observed_state`, `path`, optional
`last_error`, whose enums `LocalMemberKind`, `LocalMemberState` and
`LocalObservedState` mirror the pure family model one-for-one) and
`GwzErrorCode.unknown_local` (62), the family-only merge miss. Every
local-family handler still refuses with `unsupported_operation` before any
effect; the product contract is the gwz-dev
`dev-docs/GwzLocalCloneDesign.md`.

Git paths are byte strings and are not guaranteed to be UTF-8. Conflict-path
fields retain ordinary printable UTF-8 unchanged. A path requiring escaping is
double-quoted; quotes, backslashes, and familiar control bytes use backslash
escapes, while invalid bytes use uppercase `\xNN`. These values are stable
diagnostics for human and machine output, not UTF-8 path selectors that can
necessarily be passed back to an operating-system API.

## Transport

The message boundary is intentionally transport-neutral. A caller can use the
generated types in-process, or place a bridge between the client and a
`gwz-core` host. The host can run in another process or on another machine as
long as it has access to the workspace being operated on.

Generated messages have deterministic CBOR encoding through
`gwz_core::Cbor`, `gwz_core::encode`, and `gwz_core::decode`. Taut's IR-driven
JSON codec provides a language-neutral JSON representation of the same
messages. A JSON bridge can therefore accept a service method plus its request
message, dispatch it to core, and return response, event, and operation-result
messages without parsing CLI output or reproducing command behavior.

Use the Taut schema-driven JSON rules rather than ad hoc object serialization;
enum values, integers, byte fields, optional values, and future unknown fields
must retain their protocol meaning.

`gwz-core` does not itself define an HTTP endpoint, daemon, authentication
scheme, or deployment topology. The embedding application owns those choices.
This separation is deliberate: core defines workspace semantics and messages,
while the transport defines how a remote or local client reaches them.

Transport bridges should preserve:

- service and method names;
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

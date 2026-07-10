# gwz-core Docs

These docs describe `gwz-core` v0.3.0 as an embeddable Rust engine and taut
protocol contract for GWZ workspaces.

Use these pages when you are writing a caller, UI, agent, transport bridge, or
test harness that talks directly to the library. Use `gwz-cli` docs for
terminal workflows and command-line output.

## Guide

- [Why GWZ](WhyGwz.md) - the multi-repository problem GWZ solves, what it
  adds to Git, and when it is a useful fit.
- [Embedding](Embedding.md) - how to call `gwz-core` directly.
- [OperationModel](OperationModel.md) - request metadata, selection, policy,
  dry-run, attribution, aggregate status, partial results, and events.
- [RustApi](RustApi.md) - public modules and intended Rust entrypoints.
- [WorkspaceArtifacts](WorkspaceArtifacts.md) - on-disk manifest, lock,
  snapshot, and local runtime state.
- [GitBackend](GitBackend.md) - Git backend boundary, credentials, progress,
  timeouts, tags, and fallback behavior.
- [MemberListing](MemberListing.md) - `LsRequest`, `LsResponse`,
  `MemberEntry`, materialization filtering, and `forall` reuse.
- [TagManagement](TagManagement.md) - `TagRequest`, `TagOp`, `TagInfo`, real
  Git tag behavior, and `materialize --tag`.

## Protocol

- [Protocol](Protocol.md) - GWZ service shape, transport expectations,
  envelopes, CLI-local values, and corpus rules.
- [MessageCatalog](MessageCatalog.md) - generated catalog of service methods,
  enums, messages, fields, tags, and request/response mapping.
- [ErrorCatalog](ErrorCatalog.md) - stable `GwzErrorCode` values, likely causes,
  and recovery guidance.
- [EventCatalog](EventCatalog.md) - `OperationEvent`, event kinds, severity,
  progress counters, and JSONL rendering expectations.
- [Regeneration](Regeneration.md) - how to regenerate and check protocol
  bindings, corpus vectors, and the message catalog.

## Layers

`gwz-core` has three separate contract layers:

- Rust library API: public modules, generated Rust protocol types, and
  synchronous `handle_*` entrypoints.
- Taut protocol API: `protocol/gwz.taut.py`, generated native types, CBOR
  encoding, service methods, events, errors, and corpus vectors.
- Workspace artifact schemas: YAML files under `gwz.conf/` plus local runtime
  state such as `.git/info/exclude`.

Tags are real Git refs in v0.3.0. There is no live `gwz.conf/tags` artifact; if
you see that path in older design history, treat it as removed history only.

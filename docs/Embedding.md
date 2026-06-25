# Embedding gwz-core

`gwz-core` is useful when a tool needs the GWZ engine without invoking the CLI:
an agent, desktop UI, test harness, local service, or command wrapper can build
typed requests and call the same handlers the CLI uses.

The library does not parse command-line flags, render terminal output, execute
`forall` child processes, or own a network transport. It accepts generated taut
request structs and returns generated response structs.

## Basic Pattern

1. Create a backend, usually `gwz_core::git::Git2Backend::new()`.
2. Build a request with `RequestMeta`.
3. Choose a start path or explicit `WorkspaceRef`.
4. Pass a caller-owned operation id.
5. Inspect `response.meta.aggregate_status`, then member responses and errors.

```rust
use std::path::Path;

use gwz_core::git::Git2Backend;
use gwz_core::workspace_ops::handle_ls;
use gwz_core::{LsRequest, RequestMeta, WorkspaceRef};

fn list_members(root: &str) -> gwz_core::model::ModelResult<Vec<gwz_core::MemberEntry>> {
    let request = LsRequest {
        meta: RequestMeta {
            request_id: "req-list".to_owned(),
            schema_version: "gwz.protocol/v0".to_owned(),
            workspace: Some(WorkspaceRef {
                root: Some(root.to_owned()),
                ..WorkspaceRef::default()
            }),
            ..RequestMeta::default()
        },
        include_unmaterialized: Some(true),
    };

    let response = handle_ls(Path::new(root), request, "op-list")?;
    Ok(response.members.unwrap_or_default())
}
```

Handlers that talk to Git accept a backend reference. Read-only member listing
does not need a backend because it uses only the manifest and lock.

## Metadata

Set `RequestMeta.request_id` to caller-owned correlation data. The handler
returns it in `ResponseMeta` and events. Set `schema_version` to
`gwz.protocol/v0` for the current schema. Use `RequestMeta.workspace.root` when
the caller already knows the workspace root; otherwise handlers discover upward
from the supplied start path.

Use `RequestMeta.attribution` when the logical actor, Git author, or Git
committer should be recorded separately from the process identity. The current
handlers validate attribution data and echo it into responses/events where
applicable.

## Selection And Policy

`RequestMeta.selection` narrows operations to selected active members:

- omitted selection means active members for most workspace operations;
- `all=true` means all active members and cannot be combined with filters;
- `member_ids` and `paths` select explicit members.

`RequestMeta.policy` carries partial behavior, destructive behavior, sync mode,
unsupported-member behavior, remote name, concurrency, per-host concurrency, and
progress throttling. Not every handler uses every field.

## Events

Simple synchronous calls can ignore event sinks. Longer operations such as
materialize, pull, and push have event-aware variants or event parameters that
emit `OperationEvent` records:

- `operation_started` and `operation_finished`;
- `member_started`, `member_progress`, and `member_finished`;
- `reset` when an operation event buffer overflows in `OperationRuntime`.

Use `operation::NullSink` when an event sink is required but the caller does not
need events.

## Errors

Handlers return `ModelResult<T>`. A returned `ModelError` means the operation
could not produce a response. A successful response can still carry rejected,
failed, skipped, or partial member records. Always inspect the envelope status
and member statuses.

See [ErrorCatalog](ErrorCatalog.md) and [OperationModel](OperationModel.md).

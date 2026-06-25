# Rust API

The crate root re-exports generated taut protocol types from
`gwz_core::protocol::generated::*` and exposes modules for artifacts, Git,
model validation, operations, status, workspace discovery, and workspace
operations.

## Public Modules

| Module | Purpose |
| --- | --- |
| `artifact` | Read/write manifest, lock, and snapshot YAML artifacts. |
| `git` | `GitBackend`, `Git2Backend`, Git status/head/remote/result types, transfer progress, and timeout configuration. |
| `model` | Core ids, model errors, source kinds, desired refs, selection, policy, and attribution validation. |
| `operation` | Operation runtime, events, aggregate/member execution helpers, concurrency helpers, and response envelope helpers. |
| `protocol` | Generated taut protocol module and conversion helpers. |
| `runtime` | Clock and id helpers. |
| `status` | `handle_status` and status projections. |
| `workspace` | Workspace path parsing, discovery, and create preflight. |
| `workspace_ops` | Synchronous operation handlers. |

## Common Imports

```rust
use gwz_core::git::Git2Backend;
use gwz_core::operation::NullSink;
use gwz_core::workspace_ops::{
    handle_capture, handle_commit, handle_create_repo, handle_ls, handle_materialize,
    handle_pull_head, handle_push, handle_stage, handle_tag,
};
use gwz_core::{RequestMeta, Selection, WorkspaceRef};
```

## Handler Map

| Request | Entrypoint |
| --- | --- |
| `CreateWorkspaceRequest` | `workspace_ops::handle_create_workspace` |
| `InitFromSourcesRequest` | `workspace_ops::handle_init_from_sources` |
| `AddExistingRepoRequest` | `workspace_ops::handle_add_existing_repo` |
| `CreateRepoRequest` | `workspace_ops::handle_create_repo` |
| `MaterializeRequest` | `workspace_ops::handle_materialize` |
| `StatusRequest` | `status::handle_status` |
| `LsRequest` | `workspace_ops::handle_ls` |
| `SnapshotRequest` | `workspace_ops::handle_snapshot` |
| `TagRequest` | `workspace_ops::handle_tag` |
| `CaptureRequest` | `workspace_ops::handle_capture` |
| `CommitRequest` | `workspace_ops::handle_commit` |
| `StageRequest` | `workspace_ops::handle_stage` |
| `PullHeadRequest` | `workspace_ops::handle_pull_head` or `handle_pull_head_with_events` |
| `PullSnapshotRequest` | `workspace_ops::handle_pull_snapshot` |
| `PushRequest` | `workspace_ops::handle_push` or `handle_push_with_events` |

`handle_clone_workspace` is a Rust convenience entrypoint for clone +
materialize-lock. It records the operation as materialization and does not add a
new wire request type.

`ExecRequest`, `ExecResponse`, and `ExecResult` are generated types for CLI
support. They have no `gwz-core` service method and no core handler.

## Backend Injection

Use `Git2Backend` for normal embedding. Tests can implement `GitBackend` to
isolate filesystem or remote behavior. The trait boundary is intentionally
large enough to keep policy in core handlers and Git mechanics behind one
interface.

## CBOR

The crate exposes `gwz_core::encode`, `gwz_core::decode`, and `gwz_core::Cbor`
from the generated taut runtime. Use generated `to_cbor`/`from_cbor` methods on
protocol structs when building a custom transport.

## Version

`gwz_core::version()` returns the crate package version. The current crate
version is v0.3.0.

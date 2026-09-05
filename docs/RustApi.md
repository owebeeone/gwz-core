# Rust API

The crate root re-exports generated taut protocol types from
`gwz_core::protocol::generated::*` and exposes modules for artifacts, Git,
model validation, operations, status, workspace discovery, and workspace
operations.

## Public Modules

| Module | Purpose |
| --- | --- |
| `artifact` | Read/write manifest, lock, and snapshot YAML artifacts. |
| `git` | `GitBackend`, `Git2Backend`, Git status/head/remote/result types, transfer progress, timeout configuration, and the anonymous local fetch/push ports. |
| `local_clone` | Thin local clone family adapters: request-shape validation, the `LocalTransport` adapter over `GitBackend`, the family-merge wrapper (whose resolver miss is `unknown_local`), and the `list` projection of the family model's observation-only listing onto `LocalFamilyResponse.members`. Library logic lives in the crates under `crates/`. |
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
    handle_branch, handle_capture, handle_commit, handle_create_repo, handle_ls,
    handle_materialize, handle_pull_head, handle_push, handle_repo_sync, handle_stage,
    handle_stash, handle_tag,
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
| `RepoSyncRequest` | `workspace_ops::handle_repo_sync` |
| `MaterializeRequest` | `workspace_ops::handle_materialize` |
| `StatusRequest` | `status::handle_status` |
| `LsRequest` | `workspace_ops::handle_ls` |
| `SnapshotRequest` | `workspace_ops::handle_snapshot` |
| `TagRequest` | `workspace_ops::handle_tag` |
| `BranchRequest` | `workspace_ops::handle_branch` |
| `StashRequest` | `workspace_ops::handle_stash` |
| `CaptureRequest` | `workspace_ops::handle_capture` |
| `CommitRequest` | `workspace_ops::handle_commit` |
| `StageRequest` | `workspace_ops::handle_stage` |
| `PullHeadRequest` | `workspace_ops::handle_pull_head` or `handle_pull_head_with_events` |
| `PullSnapshotRequest` | `workspace_ops::handle_pull_snapshot` |
| `PushRequest` | `workspace_ops::handle_push` or `handle_push_with_events` |
| `MergeRequest` | `workspace_ops::handle_merge_with_local_family` (routes a `local_source_name` selector through the family wrapper, otherwise `handle_merge_with_events`) |
| `CloneLocalWorkspaceRequest` | `workspace_ops::handle_clone_local_workspace` |
| `LocalFamilyRequest` | `workspace_ops::handle_local_family` |

`handle_merge_with_events` remains the public merge engine entry; it refuses
a request that still carries `local_source_name`, so drivers dispatch merges
through `handle_merge_with_local_family`. At the LCM1.0c checkpoint the two
local-family handlers and the family branch of the merge wrapper validate
request shape and then refuse with `unsupported_operation` before any
effect.

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

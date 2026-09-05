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
| `local_clone` | Thin local clone family adapters: request-shape validation, the `LocalTransport` adapter over `GitBackend`, the family-merge wrapper (whose resolver miss is `unknown_local`), the one error table `errors` (since LCM1.1 fix 1 a §4.0 hazard is `unsupported_source_layout`, a stopped copy `copy_failed`, a moved source `source_drift`, a failed completion rule or a cancelled install `destination_incomplete`), the `list` projection of the family model's observation-only listing onto `LocalFamilyResponse.members` (with the observed root in `root_path`), and -- since LCM1.1 -- `adapters/` (one thin adapter per library port: the install ports over `gwz-repo-inspect`, `gwz-family-store`, the conf-integrity helpers and `gwz-history-check`; the disposal ports; the design §4.1 exclusion set; the destination Git-configuration installer; an ordinary recursive remover), `create` (`gwz clone --local`, verbatim, composed over `gwz_workspace_install::install`) and `dispose` (`dispose --keep` over `gwz_local_disposal::dispose`, and `disband` over the store session). Library logic lives in the crates under `crates/`. |
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
through `handle_merge_with_local_family`. Since LCM1.1 (lane C wiring)
`handle_clone_local_workspace` creates a verbatim clone end to end
(`local_clone::create`: the family observation, the source inventory and
snapshot before the family lock, founding when the workspace is in no
family, then `gwz_workspace_install::install` over the real adapters -- the
`creating` row, the destination, `gwz-refcopy` with design §4.1's
exclusions, the destination's Git configuration, pointer and marker, the
completion check, the source recheck, the manifest last, then `ready`; the
response message names the destination, the recorded path, the copy counts
and the family), `handle_local_family` lists every member's observed target
through the store, detaches a member with `--keep` and disbands a family
(`local_clone::dispose`). Still refusing with `unsupported_operation` after
request shape and the family observation, before any effect: `--clean` and
`--bare` clones (LCM3.1 / LCM2.3), ordinary `dispose` without `--keep`
(its fresh work/history checks are LCM2.1), `--from` (LCM3.2), and the
family branch of the merge wrapper past resolution (LCM1.2).

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

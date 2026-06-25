# gwz-core Reference

This page is the practical reference for embedding `gwz-core`. For command-line
workflows, use `gwz-cli`; it is the canonical user-facing driver for this
library.

## Request Model

Every operation is a generated taut request struct. Each request carries
`RequestMeta` plus operation-specific fields.

```rust
use std::path::Path;

use gwz_core::git::Git2Backend;
use gwz_core::workspace_ops::handle_create_repo;
use gwz_core::{CreateRepoRequest, RequestMeta, WorkspaceRef};

fn create_member_repo() -> gwz_core::model::ModelResult<()> {
    let backend = Git2Backend::new();
    let request = CreateRepoRequest {
        meta: RequestMeta {
            request_id: "req-1".to_owned(),
            schema_version: "gwz.protocol/v0".to_owned(),
            workspace: Some(WorkspaceRef {
                root: Some("/work/my-ws".to_owned()),
                ..WorkspaceRef::default()
            }),
            ..RequestMeta::default()
        },
        member_path: "tools/new-lib".to_owned(),
        ..CreateRepoRequest::default()
    };

    let response = handle_create_repo(&backend, Path::new("/work/my-ws"), request, "op-1")?;
    match response.response.meta.aggregate_status {
        gwz_core::AggregateStatus::Ok => {
            for member in response.response.members {
                eprintln!("created {} at {}", member.member_id, member.member_path);
            }
        }
        other => {
            for error in response.response.errors {
                eprintln!("{:?}: {}", error.code, error.message);
            }
            eprintln!("operation ended as {:?}", other);
        }
    }

    Ok(())
}
```

`request_id` is caller-owned correlation data. `operation_id` is supplied by the
driver or runtime and is returned in responses and events. `schema_version`
should identify the protocol version the caller expects.

## Response Model

Most operation responses wrap a `ResponseEnvelope`:

- `meta` identifies the request, action, aggregate status, operation id, and
  optional attribution.
- `members` reports per-member plan or execution state.
- `errors` reports operation-level errors that are not tied to exactly one
  member.

Check the aggregate status first, then inspect member statuses and errors. Do
not infer success from an empty error list alone; partial and rejected operations
may still include useful member records.

Longer-running drivers can use `OperationRuntime` for accepted responses,
operation events, and final `OperationResult` lookup. The synchronous `handle_*`
functions are the simpler entrypoints for direct embedding and tests.

## Request Types

| Request | Handler | Purpose |
| --- | --- | --- |
| `CreateWorkspaceRequest` | `workspace_ops::handle_create_workspace` | Create an empty workspace manifest and lock at `workspace_root`. |
| `InitFromSourcesRequest` | `workspace_ops::handle_init_from_sources` | Create or plan a workspace from source URLs, cloning each source into a member path. |
| `AddExistingRepoRequest` | `workspace_ops::handle_add_existing_repo` | Register an existing Git repository as a workspace member without recloning it. |
| `CreateRepoRequest` | `workspace_ops::handle_create_repo` | Initialize a new local Git repository and add it to the workspace. |
| `MaterializeRequest` | `workspace_ops::handle_materialize` | Move selected members to an explicit lock, snapshot, tag, or commit target. |
| `StatusRequest` | `status::handle_status` | Report selected member Git state, lock match state, and optional combined status projections. |
| `LsRequest` | `workspace_ops::handle_ls` | List members from manifest plus lock without calling Git. |
| `SnapshotRequest` | `workspace_ops::handle_snapshot` | Write a named snapshot of selected member states. |
| `TagRequest` | `workspace_ops::handle_tag` | Manage real Git tags across selected members, and the root for local operations. |
| `CaptureRequest` | `workspace_ops::handle_capture` | Capture observed selected member state into the lock without worktree mutation. |
| `CommitRequest` | `workspace_ops::handle_commit` | Commit selected members, refresh the lock, then commit root metadata last. |
| `StageRequest` | `workspace_ops::handle_stage` | Route pathspecs to owning member/root repositories and stage them. |
| `PullHeadRequest` | `workspace_ops::handle_pull_head` | Fetch and integrate selected members to configured upstream heads. |
| `PullSnapshotRequest` | `workspace_ops::handle_pull_snapshot` | Materialize selected members to a named snapshot. |
| `PushRequest` | `workspace_ops::handle_push` | Push selected members to a remote/refspec, using request policy where supported. |
| `ExecRequest` | none | CLI-local `forall` support data; `gwz-core` does not execute commands. |

## Common Request Fields

`RequestMeta.workspace` chooses or verifies the workspace. If `root` is omitted,
handlers that operate inside an existing workspace discover upward from the
provided start path. If `workspace_id` is set, the manifest must match it.

`RequestMeta.selection` limits operations to a subset of members. If omitted,
handlers generally operate on active members. Use `all=true` for all active
members and `member_ids` or `paths` for explicit subsets.

`RequestMeta.policy` carries operation policy such as atomic vs partial
behavior, destructive behavior, sync behavior, remote name, and concurrency.
Not every v0 handler implements every policy field.

`RequestMeta.dry_run` asks handlers to plan without mutating where supported.
Responses then carry planned member actions instead of final state.

`RequestMeta.attribution` separates the logical actor from Git object identity.
This allows an external driver or agent to say who requested the operation and
which author/committer identity should be used for Git objects.

## Artifacts And Tags

Durable workspace artifacts live under `gwz.conf/`:

- `gwz.conf/gwz.yml` for the manifest;
- `gwz.conf/gwz.lock.yml` for the lock;
- `gwz.conf/snapshots/<snapshot-id>.yaml` for snapshots.

Tags are not GWZ artifacts in v0.3.0. `TagRequest` manages real Git refs named
`refs/tags/<name>`. `MaterializeTargetKind::Tag` checks out members that carry
the named Git tag and skips untagged members by default.

## Direct vs CLI Use

Use `gwz-core` directly when embedding workspace behavior in an agent, UI, test
harness, or another local service.

Use `gwz-cli` when you want command behavior, argument parsing, terminal/JSON
rendering, and the user workflow for init, status, snapshot, tag, pull, and push.

## Further Reading

- [Embedding](Embedding.md)
- [OperationModel](OperationModel.md)
- [RustApi](RustApi.md)
- [WorkspaceArtifacts](WorkspaceArtifacts.md)
- [GitBackend](GitBackend.md)
- [MemberListing](MemberListing.md)
- [TagManagement](TagManagement.md)
- [Protocol](Protocol.md)
- [MessageCatalog](MessageCatalog.md)
- [ErrorCatalog](ErrorCatalog.md)
- [EventCatalog](EventCatalog.md)
- [Regeneration](Regeneration.md)

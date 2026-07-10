# Error Catalog

`GwzErrorCode` is the stable protocol error enum. Rust handlers use
`model::ErrorCode` and convert it to this protocol enum.

| Code | Likely Cause | Recovery |
| --- | --- | --- |
| `ok` | No error. | No action. |
| `invalid_request` | Missing required field, invalid id/path, invalid selection, duplicate snapshot, signed tag without a message. | Fix the request before retrying. |
| `workspace_not_found` | No GWZ workspace found or workspace id guard did not match. | Run from a workspace or set `WorkspaceRef.root` to the correct root. |
| `workspace_already_exists` | Create/clone target already contains a GWZ workspace. | Choose a different empty target or use existing-workspace operations. |
| `nested_workspace` | A create/add path would nest one GWZ workspace inside another. | Move the target outside the active workspace boundary. |
| `manifest_not_found` | Reserved protocol value for missing manifest. | Ensure `gwz.conf/gwz.yml` exists. |
| `manifest_invalid` | Artifact YAML failed parsing or schema shape validation. | Repair or regenerate the manifest/lock/snapshot from a known good state. |
| `schema_unsupported` | Artifact schema does not match the v0 schema. | Use a compatible `gwz-core` version or migrate the artifact. |
| `member_not_found` | Selected member id/path is absent, member is unmaterialized for an operation that needs a repo, or lock state is missing. | Check `gwz ls --unmaterialized`, materialize the member, or correct selection. |
| `member_inactive` | Explicit selection named an inactive member. | Reactivate the member or select an active member. |
| `path_escape` | A member path or stage pathspec escapes the workspace/member boundary. | Use workspace-relative paths inside the root. |
| `path_collision` | Member paths collide, clone target is non-empty, or target exists with incompatible shape. | Choose a non-overlapping member path or empty target. |
| `path_reserved` | A member path uses reserved workspace metadata such as `gwz.conf`. | Choose a different member path. |
| `unsupported_source_kind` | Operation supports Git members only and selected member has another source kind. | Skip unsupported members by policy where supported, or avoid selecting them. |
| `unsupported_operation` | v0 handler does not implement the requested option or target. | Use a supported v0 mode. |
| `dirty_member` | Operation would overwrite local work or reset without destructive policy. | Commit or otherwise save local work, clean the member, or allow destructive reset when appropriate. |
| `diverged_member` | Fast-forward is not possible, branch checkout would orphan work, or pull found divergence. | Merge/rebase/reset with explicit policy, or resolve the member manually. |
| `missing_remote` | Required fetch/push remote or branch is absent. | Add/configure the remote or supply `policy.remote`/request remote. |
| `snapshot_not_found` | Materialize/pull snapshot target does not exist. | List snapshots or create the snapshot first. |
| `lock_not_found` | Lock file or selected member lock record is missing. | Capture/materialize to create lock state, or avoid selecting that member. |
| `tag_not_found` | `materialize --tag` found no member carrying the tag. | Fetch/list tags or choose a tag present in selected members. |
| `tag_invalid` | Git rejected a tag operation after tag-specific error mapping. | Inspect the message, local tag state, signing config, or remote tag policy. |
| `remote_rejected` | Remote rejected push/tag push/delete. | Inspect remote permissions, protected refs, credentials, or refspec. |
| `git_command_failed` | libgit2 or porcelain `git` primitive failed. | Inspect the message and reproduce in the affected repo with Git. |
| `external_tool_missing` | Reserved for missing external tooling. | Install the required external tool. |
| `operation_not_found` | Runtime event/result lookup used an unknown operation id. | Use the operation id returned in the accepted response. |
| `attribution_denied` | Reserved for rejected attribution policy. | Adjust caller identity or policy. |
| `permission_denied` | Reserved for filesystem/authorization denial. | Check filesystem permissions or credentials. |
| `io_error` | Filesystem read/write/fsync/rename error. | Check disk, permissions, and path availability. |
| `internal_error` | Serialization or invariant failure. | Treat as a bug and capture diagnostics. |
| `branch_detached_head` | Current-branch snapshot found a selected member on a detached HEAD. | Switch that member to a branch or snapshot a named branch. |
| `branch_unborn_head` | Current-branch snapshot found a selected member with no born attached branch. | Create the first commit or snapshot a named existing branch. |
| `branch_mixed` | Current-branch snapshot found selected members attached to different branch names. | Narrow the selection or use `snapshot --branch <name>`. |
| `stash_not_found` | Requested stash bundle is missing, or no eligible latest bundle exists. | Run `gwz stash list` or provide an existing `stash_id`. |
| `stash_incomplete` | Local bundle metadata and native Git stash payloads no longer match, or a partial restore needs explicit selection. | Inspect `gwz stash list --expanded`; recover/drop native stashes manually if needed. |
| `stash_conflict` | Native stash restore reported a conflict. | Resolve the affected member repository and retry or clean up the stash explicitly. |
| `source_identity_mismatch` | A repository being attached or assigned an existing source identity does not contain every commit required by historical snapshot/marker evidence. | Fetch the missing history into the repository, verify it is the intended source, and retry. |

Errors can appear as a returned `ModelError`, an operation-level `GwzError` in
`ResponseEnvelope.errors`, or a member-scoped `MemberResponse.error`.

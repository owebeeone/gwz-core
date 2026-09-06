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
| `unknown_local` | `gwz merge --remote <name>` named no ready local-family member: the name is absent from the family index, reserved (`origin`), or its row is `creating`/`disposing` (the message says which). Merge resolves family names only; it never falls back to a Git remote. | Run `gwz local list`; name a `ready` member, or use `gwz merge <ref>` for a Git ref and `gwz pull --head --remote <git-remote>` for a Git remote. |
| `unsupported_source_layout` | `gwz clone --local` found a design §4.0 hazard in a source repository -- `.git` as a file (gitfile / linked worktree), a common directory outside the member, `objects/info/alternates` or `http-alternates`, metadata or an object store reached through a symlink outside the repository, `core.worktree`/`core.hooksPath`/`include.path`/`includeIf`/`url.*.insteadOf` naming a path outside dest, a partial clone, a `GIT_*` override -- or a `.git` entry that is not a repository. Refused before reservation; nothing written. v0 refuses and never rewrites. | Repair the source layout (dissociate the alternates, convert the gitfile checkout, move the escaping configuration or hooks inside the repository, unset the override), or clone from another ready member. |
| `copy_failed` | The local clone's tree copy stopped: permission, space, I/O, metadata, or an entry the copier does not copy (FIFO, socket, device). The `creating` row and the partial destination are retained; the source is unchanged. | Read the row's `last_error` (`gwz local list`), fix the cause, detach the remains with `gwz local dispose <name> --keep`, retry. |
| `source_drift` | The source changed between the local clone's snapshot and its publication (a ref, HEAD, branch, remote or the manifest/lock digest moved, or a gwz merge opened): the destination is a copy of a moving source and was not marked ready. The row and directory are retained. | Quiesce the source's writers (design §2), `gwz local dispose <name> --keep` the retained destination, retry. |
| `destination_incomplete` | The local clone's destination failed a completion rule before `ready`: an object missing from its own store, HEAD off the frozen source HEAD, an inadmissible layout or a connectivity walk past the verification ceiling (§4.0 dest-complete), a §4.1 residual, a pointer or `.gwz/merge/` fault, an unrecaptured lock or marker -- or the install was cancelled, which leaves the same shape. The `creating` row and the directory are retained; `gwz local list` shows `creating/incomplete`. | Read the row's `last_error`, inspect the destination, `gwz local dispose <name> --keep`, retry; a ceiling refusal names the limit and the store's object count. |
| `pairing_mismatch` | `gwz merge --remote <name>` found the two workspaces no longer the same shape: a lock member id present on one side only, the same id recorded at different paths, the same id with a different `source_id`, or a selected `@root` with no root to pair (design §6). Refused before any fetch; nothing written. | Align the family (`gwz local list`, `gwz ls` in both workspaces): re-add or detach the odd member, or clone afresh; then retry. |
| `import_incomplete` | The family merge's import stopped before the engine was entered: a fetch into a receiver or a receiver read failed, or the import was cancelled. The import refs created before the stop (`refs/gwz/local-imports/<transfer-id>`) are retained and named in the message; no merge record was opened. | Fix the cause the message names (the receiver's repository, permissions, disk), then retry: the next invocation mints a fresh transfer id. The retained refs are ordinary Git refs; gwz never prunes them. |

Errors can appear as a returned `ModelError`, an operation-level `GwzError` in
`ResponseEnvelope.errors`, or a member-scoped `MemberResponse.error`.

## Local Clone Family

The LCM1.0c checkpoint (2026-09-05) allocated no new error code; follow-up
2 (operator ruling 2026-09-05, gwz-dev `dev-docs/GwzLocalCloneDesign.md`
revision 9 §6/§7, §11 item 13) allocates exactly one, `unknown_local` (62),
for the product design's `UnknownLocal` outcome: a family-only merge
selector (`MergeRequest.local_source_name`, `gwz merge --remote <name>`)
that names no ready family member. The message carries the state detail —
the row's recorded lifecycle state when a row exists, "no ready family
member" when none does — and there is no Git-remote fallback. Everything
else reuses existing codes: `invalid_request` (malformed name, missing
dispose name, unknown hazard, `keep` with hazards, `local_source_name` on a
non-start merge, an empty `copy_source`), `merge_validation_failed` (a
`local_source_name` that reaches the merge engine instead of the family
wrapper), `unsupported_operation` (family dry-run, a present `copy_source`
until LCM3.2, and every mode or operation not yet implemented), and
`missing_remote` (pull/push token that is neither a ready family member nor
a Git remote — pull/push keep their Git-remote fallback and never answer
`unknown_local`).

LCM1.1 (lane C wiring) allocated nothing, and its fix 1 (lane C, 2026-09-06;
gwz-dev `dev-docs/GwzLocalClone-LCM1.0c-Checkpoint.md` §14) allocated four
codes for the local-create outcomes the wiring had folded into
`unsupported_operation` and `io_error`, which a driver could not tell from
"not built yet" and a plain I/O failure. The one table is
`local_clone::errors::{install_port_code, install_refusal_code,
install_error_code}`:

| Outcome | Code | Typed cause (call site) |
| --- | --- | --- |
| a design §4.0 source-layout hazard, or a `.git` entry that is not a repository | `unsupported_source_layout` (63) | `InstallPortError::Layout(LayoutError::Unsupported \| NotARepository)` at the capture before the family lock (`create::port_error`) and `InstallRefusal::SourceLayout` at admission (`install_refusal_code`) |
| the tree copy stopped | `copy_failed` (64) | `InstallError::Copy` for every `CopyErrorCategory` but `DestinationNotEmpty` (still `path_collision`), `Unimplemented` (`unsupported_operation`) and `Cancelled` (below) |
| the source moved between the snapshot and publication | `source_drift` (65) | `InstallPortError::Drift` from `recheck_source` (`InstallError::Source`) |
| a completion rule failed, or the install was cancelled | `destination_incomplete` (66) | `InstallError::Incomplete` (every `CompletionFault`: §4.0 dest-complete as `NotIndependent`, §4.1 residuals, pointer and merge-store faults, `LockNotRecaptured`, `MarkerNotRegenerated`), `InstallError::Cancelled` and a copy cancelled between entries -- an interruption leaves exactly the shape a failed rule leaves (design §4 step 4), the one `local list` reports as `creating/incomplete`; the message says which |

`unsupported_operation` now means exactly "not built yet": the clean and
bare modes, `--from` (LCM3.2), ordinary `dispose` (LCM2.1), a family
`dry_run`, an `Unimplemented` port, store or copier, and
`LayoutError::Unimplemented`. `io_error` now means exactly an I/O failure:
an inspector that could not read far enough to classify
(`LayoutError::ReadFailed`, lane I proposal I-3), a destination that could
not be allocated or observed, a configuration that could not be read or
written (`InstallPortError::{Construction, Configuration, Destination}`), a
path that does not resolve, and the store's `Io`/`Partial`. `gwz clone
--local` otherwise reuses, as LCM1.1 recorded: `path_collision` (the
destination is not empty, or is already a workspace; a taken name, path or
allocation), `open_operation` (the source has an open gwz merge; the family
lock is held), `invalid_request` (the clone name is a Git remote of a source
repository, the addressed workspace is not a ready member of its family, a
destination that cannot be spelled root-relative) and `member_not_found`
(`local_clone::errors::refusal` maps the family model's refusals).
`gwz local dispose --keep` and `disband` reuse `member_not_found` (no such
member; the workspace is in no family), `invalid_request` (the root, a
target containing the working directory, a path mismatch), `open_operation`
(the family lock is held) and the store's codes.

LCM1.2 (lane C, 2026-09-06; gwz-dev
`dev-docs/GwzLocalClone-LCM1.0c-Checkpoint.md` §16) serves `gwz merge
--remote <name> [<ref>]` end to end -- the import through one retained ref
`refs/gwz/local-imports/<transfer-id>` in every paired receiver, then one
delegation to the public merge engine entry -- and allocated two codes for
the import outcomes that would otherwise have folded into
`invalid_request`/`member_not_found` and `git_command_failed`. The one
table is `local_clone::errors::import_error_code`, over
`gwz_local_import::ImportError`:

| Outcome | Code | Typed cause (call site) |
| --- | --- | --- |
| the two workspaces' member sets do not correspond (missing or extra id, the same id at different recorded paths), or the same id has a different `source_id` | `pairing_mismatch` (67) | `ImportError::PairingIncomplete` from `pair_participants` (before the first transport call), and the wrapper's own `source_id` cross-check (`family_merge::identity_mismatches`) |
| the selected source ref does not resolve in one or more paired sources | `merge_validation_failed` (reused) | `ImportError::SourceMissing` at capture; the family merge validates its source before transfer (design §6.1), where the ordinary merge lets libgit2 answer `git_command_failed` at planning time |
| the fresh, collision-checked import name already exists in a receiver | `path_collision` (reused) | `ImportError::RefCollision`: a namespace collision at a target that exists, nothing written; the next invocation mints another id |
| a received object id differs from the captured one | `source_drift` (65, reused) | `ImportError::VectorMismatch`: the source moved between capture and fetch; the refs created so far are retained and named |
| the transfer stopped, a receiver could not be read, or the import was cancelled | `import_incomplete` (68) | `ImportError::TransferFailed`, `ImportError::Cancelled` (no producer in the wired slot, as for the create's cancellation) |

Every import refusal's message names the step, the source, the import
name, the typed cause and every retained import ref (or that nothing was
written), and says that the engine was not entered. A refusal the engine
makes after the import -- a dirty member, an open merge it finds, drift --
travels unchanged with the retained refs named after it; the import refs
are ordinary Git refs that nothing in gwz prunes (design §6.2). Before any
fetch the wrapper also refuses `open_operation` when the addressed
workspace already has an open merge record ("nothing was imported"), so a
start the engine's own gate would refuse leaves no ref behind; the family
`dry_run` stays `unsupported_operation`.

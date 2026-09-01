# Operation Model

Every GWZ operation is a generated taut request with shared `RequestMeta`.
Responses wrap a `ResponseEnvelope` unless the method streams events or returns
an operation result.

## Request Metadata

`RequestMeta` fields:

| Field | Meaning |
| --- | --- |
| `request_id` | Caller-owned correlation id echoed in responses and events. |
| `schema_version` | Protocol version expected by the caller, currently `gwz.protocol/v0`. |
| `workspace` | Optional root and workspace id guard. |
| `selection` | Optional member filter. |
| `policy` | Optional operation policy. |
| `dry_run` | Plan without mutation where supported. |
| `attribution` | Optional actor and Git object identity metadata. |

`operation_id` is not in the request. The embedding driver supplies it when it
calls a handler. The id is returned in `ResponseMeta.operation_id` and in event
records.

## Workspace Resolution

If `WorkspaceRef.root` is set, handlers use that path. Otherwise existing
workspace handlers discover upward from the supplied start path. If
`WorkspaceRef.workspace_id` is set, the manifest id must match.

## Selection

Omitted selection generally means active members. Explicit selection accepts
member ids or workspace-relative member paths. `all=true` selects all active
members and is rejected if filters are also present.

Some handlers require lock records for selected members. For example,
materialization, branch, stash, tag management, capture, snapshot, commit, and
narrowed stage-all resolve through the lock and can return `lock_not_found` for
a selected member that has no lock entry. `ls` is manifest-tolerant and can list
configured but unmaterialized members.

## Policy

`OperationPolicy` carries:

| Field | Used For |
| --- | --- |
| `partial` | Whether a selected-member failure can be isolated. v0 handlers are conservative and preflight before broad mutation where practical. |
| `destructive` | Allows destructive reset/materialize behavior when set to `allow`. |
| `sync` | Pull/head behavior: fetch-only, ff-only, merge, rebase, reset, or driver-selected. |
| `unsupported_member` | Fail or skip members whose source kind cannot run the operation. |
| `remote` | Preferred remote name for fetch/push/tag operations. |
| `concurrency` | Maximum concurrent member jobs. |
| `progress_min_interval_ms` | Per-member progress event throttling. |
| `max_connections_per_host` | Per-host network concurrency cap. |

## Dry Run

Handlers that support dry-run return planned member changes with
`MemberStatus::Planned` and a `PlannedChange`. Dry-run responses commonly use
`AggregateStatus::Accepted` for accepted plans.

Dry-run is a planning request, not a transaction reservation. A caller that
executes later must be prepared for state to change between plan and apply.

## Responses

`ResponseEnvelope` contains:

- `meta`: request id, schema version, action, aggregate status, operation id,
  message, and attribution.
- `members`: per-member state, status, plan, Git status, or member-scoped error.
- `errors`: operation-level errors that are not tied to one member.

`AggregateStatus` values:

| Value | Meaning |
| --- | --- |
| `accepted` | A plan or asynchronous operation was accepted. |
| `ok` | Operation completed successfully. |
| `noop` | All selected members had nothing to do. |
| `rejected` | Preconditions or policy rejected the operation. |
| `partial` | Some members applied and others failed. |
| `failed` | Operation failed without successful member application. |
| `dirty` | Dirty-state specific aggregate value reserved by the protocol. |
| `conflicted` | At least one member reached a reportable conflict state. |

`MemberStatus` values:

| Value | Meaning |
| --- | --- |
| `planned` | Planned but not executed. |
| `ok` | Member applied successfully. |
| `noop` | Member needed no change. |
| `skipped` | Member was intentionally skipped by policy. |
| `rejected` | Member failed preconditions or policy. |
| `failed` | Member attempted work and failed. |
| `conflicted` | Member reached a conflict state. |

## Events

Event-aware operations emit `OperationEvent` records with monotonic per-operation
sequence numbers. Transfer progress can be throttled with
`progress_min_interval_ms`. `OperationRuntime` stores bounded event history; if
the buffer overflows it emits a `reset` event and history before that event is
incomplete.

See [EventCatalog](EventCatalog.md).

## Workspace Mutator Lock

Branch and stash mutators serialize through a workspace-wide advisory lock at
`.gwz/locks/workspace-mutator.lock`. The lock is taken before mutating native
Git state or `.gwz/` stash registry files. The lock file may remain after a
process exits; an unlocked file is not stale, and the operating system releases
the held lock if the process dies.

The lock protects cross-process operations in normal local filesystems. Network
filesystems with unreliable advisory locking are unsupported for concurrent GWZ
mutations; run branch and stash mutators serially there.

## Checked Merge Artifacts And Filesystem Identity

`gwz merge --no-ff` records its operation through the checked merge artifact
catalog under `.gwz/catalog-final`. The catalog identifies its own files and
directories by a *durable* identity that survives renames and process exits, so
that an interrupted merge can prove on restart which objects it created. That
needs two things from the filesystem, and it asks for them when the operation
moves FORWARD under its lock — at a checked start or resume, and before an
interrupted ordinary merge is migrated to the checked record form. An abort
never asks:

* **Persistent file handles** — `name_to_handle_at` on Linux,
  `ATTR_CMN_OBJPERMANENTID` on macOS, 128-bit file ids on NTFS.
* **A mount identity**, so a rename can be proved to stay on one filesystem.

Filesystems that do not expose both are refused with a message naming the
capability. **On Linux the admitted filesystem is `ext4` and nothing else** —
btrfs, xfs and zfs are refused, as are `tmpfs`, overlay and container
filesystems and every network mount (NFS, SMB/CIFS, SSHFS and other FUSE
mounts). On macOS: local APFS or HFS+. On Windows: NTFS.

**What refuses:** `gwz merge --no-ff`, and `gwz merge --resume` of a merge
record already at v1.

**What never refuses:** `gwz merge --abort`, with or without `--preserve`, on a
record of either version — so a `--no-ff` merge left open on a workspace that
later becomes incapable can always be cleared. Nor does an ordinary or
`--ff-only` merge, including resuming one interrupted during finalization: such
a record is eligible for an automatic upgrade to the v1 lifecycle, and when the
catalog is unavailable that upgrade is declined before it writes anything and
the v0 lifecycle completes the merge itself. Nor do `gwz repo create`,
`init-from-sources`, `gwz merge --status`, GC, or the workspace mutation guard,
none of which reaches the catalog.

**Workaround:** run the workspace on a filesystem that exposes persistent
handles. A merge already open can be cleared with `gwz merge --abort`; a new one
can be started without `--no-ff`.

## Branch And Stash Outcomes

`gwz branch --create --switch` and `gwz materialize --switch` rewrite the lock
from observed post-switch member state. A dirty member may switch when the
target branch resolves to its current `HEAD`; GWZ only reattaches `HEAD` and
preserves staged, unstaged, and untracked changes. A dirty switch to a different
commit is rejected even under force policy. Branch create attempts rollback of
branches created earlier in the same operation when a later member fails.
Branch delete preflights selected members and reports post-preflight failures as
partial rather than claiming transactional deletion.

`gwz stash` records one local bundle for a coordinated push. Clean members are
stored as no-op members; dirty members carry push lifecycle and restore state
separately so a partial or interrupted push remains inspectable. Restore
operations default to preserving index state, require clean destinations, and
keep unresolved or missing native payloads visible through bundle metadata.

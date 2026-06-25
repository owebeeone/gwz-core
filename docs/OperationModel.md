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
materialization, tag management, capture, snapshot, commit, and narrowed
stage-all resolve through the lock and can return `lock_not_found` for a
selected member that has no lock entry. `ls` is manifest-tolerant and can list
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
| `conflicted` | Conflict-specific aggregate value reserved by the protocol. |

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

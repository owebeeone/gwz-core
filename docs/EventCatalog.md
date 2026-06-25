# Event Catalog

`OperationEvent` is the durable/progressive event record for long-running
operations.

## Fields

| Field | Meaning |
| --- | --- |
| `operation_id` | Runtime operation id. |
| `request_id` | Caller correlation id from `RequestMeta`. |
| `sequence` | Monotonic sequence number within the operation. |
| `timestamp_ms` | Unix epoch milliseconds. |
| `kind` | `EventKind`. |
| `severity` | `Severity`. |
| `member_id` | Optional member id for member-scoped events. |
| `member_path` | Optional workspace-relative member path. |
| `message` | Human-readable message, not a machine contract. |
| `member` | Optional member snapshot. |
| `error` | Optional protocol error. |
| `attribution` | Optional echoed operation attribution. |
| `progress` | Optional Git transfer progress counters. |

## EventKind

| Kind | Emitted When |
| --- | --- |
| `operation_started` | Event-aware operation begins. |
| `member_started` | Member work begins. |
| `member_progress` | Git transfer progress is reported. |
| `member_finished` | Member work completes. |
| `artifact_written` | Reserved for artifact write notifications. |
| `operation_finished` | Event-aware operation completes. |
| `reset` | Runtime event buffer overflowed and prior history is incomplete. |

## Severity

| Severity | Meaning |
| --- | --- |
| `debug` | Diagnostic detail. |
| `info` | Normal progress. |
| `warn` | Recoverable anomaly such as event history reset. |
| `error` | Operation or member error. |

Current event emitters mostly use `info`; runtime overflow uses `warn`.

## GitTransferProgress

| Field | Meaning |
| --- | --- |
| `phase` | `GitProgressPhase`. |
| `received_objects` | Objects received or written so far. |
| `total_objects` | Total objects when known. |
| `received_bytes` | Bytes received or written so far. |
| `indexed_deltas` | Deltas resolved so far. |
| `total_deltas` | Total deltas when known. |

`GitProgressPhase` includes `enumerating`, `counting`, `compressing`,
`receiving`, `resolving`, `checking_out`, and `writing`. The current Git2
transfer callback reports `receiving` and `resolving`.

## JSONL Rendering

CLI or transport JSONL renderers should emit one event per line, preserve
`operation_id`, `request_id`, `sequence`, `kind`, and member context, and avoid
parsing `message` as a stable contract. Progress events may be coalesced by
`OperationPolicy.progress_min_interval_ms`.

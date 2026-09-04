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

**A merge runs on every filesystem. Crash recovery is a capability, not a
gate.**

A merge can record its operation through the checked merge artifact catalog
under `.gwz/catalog-final`. The catalog identifies its own files and
directories by a *durable* identity that survives renames and process exits, so
that an interrupted merge can prove on restart which objects it created. That
is what "crash recovery" means here, and it needs two things from the volume:

* **Persistent file handles** — `name_to_handle_at` on Linux,
  `ATTR_CMN_OBJPERMANENTID` on macOS, 128-bit file ids on NTFS.
* **A durable filesystem identity**, so the volume itself can be named across
  a reboot and a rename can be proved to stay on one filesystem.

### The decision, made once

At merge start GWZ probes the workspace volume for exactly that, before it
takes any lease, writes any record, or runs any Git work. The answer is decided
**once per invocation** and used for the whole invocation:

* **Above the bar** — the catalog is activated exactly as before. Nothing is
  printed.
* **Below the bar, by default** — GWZ prints **one** warning and the merge
  continues, without activating the catalog:

  ```text
  warning: crash recovery is unsupported on btrfs (no durable filesystem identity). Merge will continue. Use --filesystem-strict to refuse.
  ```

  The filesystem is named, or `unknown` when it cannot be named. The
  parenthetical is exactly one of `no durable filesystem identity`,
  `remote filesystem`, or `volatile filesystem`.
* **Below the bar, with `--filesystem-strict`** — the merge is refused before
  any lease, record, or Git mutation, with a typed `UnsupportedOperation`
  naming the capability and the two ways forward: run without
  `--filesystem-strict` to proceed without crash recovery, or clear an open
  merge with `gwz merge --abort`.

Below the bar, crash recovery is **absent, not degraded**. The merge itself is
not weakened: the same participants, the same order, the same verification, the
same published composition evidence. What is missing is the catalog's evidence
that an interrupted *start* would have used to prove on restart which objects
it created.

### Volumes without persistent file handles

Persistent file handles are the second of the two requirements above, and some
volumes lack them while the merge still needs to run there — overlayfs mounted
without `nfs_export`, and some FUSE mounts. Those volumes are below the bar
already, so they warn; the same probe that decides the bar also answers this
question, and the answer changes two things.

The **merge record is written directly** rather than through the checked
artifact boundary. The write is still staged, renamed, flushed and read back to
verify its bytes; what it does not carry is the catalog, which this volume
cannot host at all. There is **no second warning** for it — the one diagnostic
above gains a clause instead:

```text
Selected-root and --preserve abort may refuse until the workspace is on a handle-capable volume.
```

Machine consumers read the same fact from the response rather than from stderr:
`crash_recovery.handles_ok` is `false` on such a volume, `true` on a below-bar
volume whose handles work, and absent above the bar.

That clause is the second change. A plain abort still clears the record — it
touches no checked artifact. An abort that must **re-verify** one, though —
a selected root's manifest and lock, a preservation bundle under `--preserve`,
or the merge's published evidence — needs handles this volume does not have,
and refuses. One escape works from there: copy the whole workspace onto a
volume that proves handles (a local APFS or HFS+ volume on macOS; ext4, xfs or
f2fs on Linux; NTFS on Windows) and run `gwz merge --abort` there, adding
`--preserve` if that was the door that refused.

A power loss mid-merge on such a volume is operator cleanup rather than a
recovery GWZ performs: nothing on that volume can prove after a reboot which
objects the interrupted attempt created.

`--filesystem-strict` is accepted only on a merge start. On `--continue`,
`--abort`, `--status` or `--gc` it is refused as an invalid request. There is
no environment variable and no configuration key: the flag is the whole
control surface.

### What "below the bar" means, per platform

The bar is identity-based. It is not a filesystem-name test, and
`--filesystem-strict` does not restore one.

* **Linux.** A volume is above the bar when `FS_IOC_GETFSUUID` answers with a
  nonzero 16-byte volume UUID and `name_to_handle_at` returns a persistent
  handle. **ext4, xfs and f2fs are admitted alike** — they publish their UUID
  to the VFS. **btrfs is below the bar** because it never publishes its UUID to
  the VFS, so the ioctl answers `ENOTTY`; the gap is
  `no durable filesystem identity`. Kernels before 6.9 have no such ioctl at
  all, so every volume is below the bar there. `tmpfs` and `ramfs` are refused
  as `volatile filesystem`: their contents do not survive power loss, so
  recovery evidence written on them cannot be trusted after a crash — even
  though they do answer both syscalls. Network mounts (NFS, SMB/CIFS, SSHFS
  and other FUSE mounts) fail the identity probe and are named as
  `remote filesystem`.
* **macOS.** Local APFS or HFS+ is above the bar. A volume without
  `VOL_CAP_FMT_PERSISTENTOBJECTIDS` is below it; a non-local volume is named as
  `remote filesystem`.
* **Windows.** NTFS is above the bar. Replacing that name test with the
  capability flag `FILE_SUPPORTS_OPEN_BY_FILE_ID` is a named follow-up, not
  yet built.

### What never depended on this

* **Interrupting a live merge.** Ctrl-C, `gwz merge --abort` and
  `gwz merge --continue` of a merge already open work the same below the bar as
  above it. They never needed the catalog.
* **A later continue or abort** uses what its start opened — a catalog, or
  none — and does not consult `--filesystem-strict` again.
* **Abort is capability-free by path** on the open merge record when it
  touches no checked artifact. An abort that must re-verify checked artifacts —
  preservation bundles, a selected root's manifest and lock, or the merge's
  published evidence — still goes through the checked boundary and its weaker
  legacy identity probe (2026-09-02,
  `GwzM5-8R2E-CapabilityFreeAmendment.md` §6).
* **How the merge was started.** Ordinary, `--ff-only`, custom-message and
  `--no-ff` starts all write the same merge record and reach this door alike,
  so the decision, the warning, `--filesystem-strict` and the `crash_recovery`
  object never depend on which of them you ran. There is no second record
  format and no upgrade between formats.
* `gwz repo create`, `init-from-sources`, `gwz merge --status`, GC, and the
  workspace mutation guard never reach the catalog.

### Stated limits

* **Without persistent file handles the limit is on abort, not on start.**
  The merge runs to completion on such a volume — the record takes the raw
  write described above — but an abort that must re-verify a checked artifact,
  a selected root's manifest and lock or a preservation bundle under
  `--preserve`, needs handles the volume does not have and refuses. Copying the
  workspace onto a handle-capable volume and aborting there is the escape;
  plain participant-only abort keeps working. The sentence at the top of this
  section is not qualified: the merge itself still runs on every filesystem.
* **A workspace moved between volumes mid-merge re-decides.** The decision is
  not recorded in the catalog or in the merge record, so a `--continue` decides
  again, on the volume it finds. Same volume, same answer. Moved onto an
  above-bar volume, a catalog is bootstrapped mid-attempt, which is harmless
  because the catalog tracks only its own publications. Moved onto a below-bar
  volume, the continue proceeds without one and warns.
* **The warning is per invocation.** A workspace on btrfs sees one line per
  start and one per continue. There is no cross-invocation suppression; adding
  one would need persistent state or configuration.
* Reconciling the catalog's identity regime with the checked boundary's weaker
  legacy probe remains DR-1's parked item
  (`GwzM5-8DR1-FilesystemIdentity-Design.md`,
  `GwzM5-8DR1-Reconciliation-Design.md`).

### Machine consumers

Every merge response that made this decision carries a `crash_recovery` object
— `supported`, the `filesystem` name when it can be named, the `gap` when it is
not supported, and `handles_ok` below the bar. JSON, porcelain and `--jsonl`
consumers read it there and never need to parse stderr; the CLI's
`docs/MachineOutput.md` carries the rendered shape.

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

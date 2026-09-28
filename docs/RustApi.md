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
| `session_host` | The core session host's frozen foundations (see "Session Host" below): `HostContext` with its `ShutdownReport`, `EnvironmentSnapshot`, `Limits` with `MAX_READ_WAIT` and `MAX_FRAME_BYTES`, `SessionOptions`, `open` and `ClientChannel`. Nothing calls them yet. |
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
`--bare` clones (LCM3.1 / LCM2.3) and `--from` (LCM3.2). Since
LCM1.2 (lane C, 2026-09-06) the family branch of the merge wrapper
(`local_clone::family_merge`) runs end to end: after the family observation
and resolution it reads the addressed workspace's manifest and lock, the
verb's selection and the open-merge envelope (read-only), takes the family
lock, pairs every selected receiver with the named member's repository by
lock member id (the root separately, included by default and by `@all`),
captures each source id, fetches it through the anonymous local transport
into one fresh `refs/gwz/local-imports/<transfer-id>` per receiver,
verifies the received vector (`gwz_local_import::prepare_import`), then
clears the selector, sets that ref as `source_ref` and calls
`handle_merge_with_events` exactly once, still under the family lock and
holding no receiver workspace lock. The response is the engine's, with the
import summarised in `meta.message` when the engine left it empty; an
engine refusal after the import carries the retained refs after its own
message. The import outcomes are `pairing_mismatch` (67) and
`import_incomplete` (68), plus `merge_validation_failed`, `path_collision`
and `source_drift` reused (`docs/ErrorCatalog.md`, "Local Clone Family").
Since LCM2.1/LCM2.2 (lane C, 2026-09-06) ordinary `gwz local dispose
<name>` runs end to end (`local_clone::dispose::delete`): under the family
lock `gwz_local_disposal::dispose` validates the name, the root and the
working directory, observes every repository in the deletion tree through
the real ports (`local_clone::adapters::disposal`: the root, every member
and every unmanaged nested repository, bare ones included -- layout, work
with ignored entries, byte-compared suppressed paths, sparse absence and
native operation state, and the protected-root inventory; GWZ's own
runtime directory and separately inspected repositories are not the
root's work), classifies the work, asks `gwz-history-check` once per
surviving family repository whether every protected root is preserved
whole, and refuses on any known hazard `--force` did not name and on any
unknown evidence whatever was named; only then does it write `disposing`,
remove the validated directory once and remove the row. An absent target
is the stale-row exit; an interrupted removal stops and is reported, never
replayed. The disposal outcomes are `unwaived_hazard` (69),
`unknown_evidence` (70) and `disposal_incomplete` (71)
(`local_clone::errors::dispose_error_code`; `docs/ErrorCatalog.md`).
Present gwz stash records still refuse as unknown evidence: decoding them
is deferred, and the message says so and that `--keep` detaches.

`handle_clone_workspace` is a Rust convenience entrypoint for clone +
materialize-lock. It records the operation as materialization and does not add a
new wire request type.

`ExecRequest`, `ExecResponse`, and `ExecResult` are generated types for CLI
support. They have no `gwz-core` service method and no core handler.

## Session Host

`session_host` holds the first interfaces of the core session host that the
core session contract specifies (gwz-dev `dev-docs/GwzCoreSessionDesign.md`,
built by steps CS1.4, CS1.5 and CS1.9 of `dev-docs/GwzCoreSessionPlan.md`). A
driver opens a session with them. The channel's `send` and `recv` arrive with
CS1.2, so a session cannot carry calls yet.

```rust
use gwz_core::session_host::{EnvironmentSnapshot, HostContext, SessionOptions, open};

let host = HostContext::new(); // one per driver process, shared by its sessions
// The driver reads its own environment, once, at its edge; core never does.
let environment = EnvironmentSnapshot::from_os_pairs(std::env::vars_os())?;
let mut options = SessionOptions::new(host.clone(), environment);
options.limits.running_operations = 4; // `open` validates the limits
options.transport_off = false; // the driver's resolved off switch
let channel = open(options)?; // the client end of the in-process channel
drop(channel); // the session ends, and its snapshot is zeroized
let report = host.shutdown(); // at the driver's end: what remains after at most 5 s
```

- `HostContext` holds what one driver's sessions share (contract §5.6). A
  clone is another handle to the same context; sessions keep theirs, and core
  keeps none in a static. It starts its supervisor thread only for its first
  job. At the driver's end, `shutdown()` disposes what the host context holds
  within the 5-second cleanup bound (the connection reuse design's §7) and
  returns a `ShutdownReport` of what remains. It returns within the bound even
  when a job never finishes; the supervisor then polls that job to its end.
  Once `shutdown` has begun, `open` refuses the host context with
  `invalid_request`. A later call, from any handle, returns the same report,
  and a concurrent one waits for the first. Dropping the host context without
  `shutdown` disposes the same way, without waiting, and reports nothing: once
  every handle is gone, the thread stops when each job has finished or, having
  panicked, been set aside. After `shutdown`, the drop disposes nothing more.
- `ShutdownReport` carries the two facts of the contract's §13
  `CleanupReport`, so a driver can add it to its session's close report.
  `pending_local_work` counts the jobs still running at the bound and the jobs
  set aside after a panic. `peer_cleanup_confirmed` is false while the host
  context has no endpoint registry: as a session that ran no network operation
  reports `(0, false)`, no peer cleanup occurred (contract §8). The type is
  `non_exhaustive`.
- `EnvironmentSnapshot` is the session's endpoint environment (§5.6). Core
  never reads the process environment: the driver reads it at its edge and
  passes the pairs in. A Rust driver passes `std::env::vars_os()` to
  `from_os_pairs`, as above. `from_byte_pairs` takes byte-string pairs, from a
  driver such as the Python bridge: raw bytes on POSIX, WTF-8 on Windows, so
  non-UTF-8 bytes and unpaired surrogates survive. Both refuse an entry no
  environment can hold (an empty name, a NUL, or `=` after a name's first
  character, and, for `from_byte_pairs` on Windows, bytes that are not WTF-8)
  with `invalid_request`, naming only the entry's index. A repeated name keeps
  its first value. Names compare byte for byte on POSIX, and on Windows
  ordinally ignoring case, as the OS and std's `Command` compare them. The
  snapshot is secret-bearing: its `Debug` output is its entry count, and it
  has no `Display`, `Clone` or serialization. It is zeroized when its session
  ends: it drops with the session's context, and each name and value
  overwrites its whole allocation, spare capacity included, before it is
  freed. The copies that std's `Command` and the OS make for a child are
  outside it.
- `Limits` carries the contract's §1 limits and defaults: 8 running and 64
  queued operations, a 128-entry operation table, 8 direct workers, 4096
  events per operation log, 64 open logs, 1024 outstanding calls, a 64-frame
  control reserve, 1 MiB per read and a 60-second close wait.
  `MAX_READ_WAIT` (30 seconds) and `MAX_FRAME_BYTES` (64 MiB) are fixed.
  `open` refuses with `invalid_request`, before any effect, an operation
  table smaller than the running plus the queued operations. It also refuses
  a count below 1, an event log below 2, queue sizes that overflow, a read
  size above 32 MiB and a close wait above one hour. A read may use half the
  frame: the read path counts each record's encoded size against
  `read_bytes`, so a reply is at most `read_bytes` plus its envelope, which
  the frame's other half bounds. A close wait of zero detaches every running
  worker at once.
- `SessionOptions` carries the host context, the snapshot, the limits and
  `transport_off`, the off switch's value as the driver resolved it, false by
  default (the core server design's §5). The snapshot never carries the
  switch, and core never derives it from the snapshot or from the process's
  own environment or configuration. Build it with `SessionOptions::new`; the
  type is `non_exhaustive`, and so is `Limits`.
- `open(options)` validates the limits and refuses a host context that has
  been shut down, each with `invalid_request` before any effect, then creates
  the session's context on the calling thread and returns a `ClientChannel`.
  Dropping the `ClientChannel` ends the session and drops its context,
  snapshot included.

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

# GWZ Core Design

Status: accepted

This document describes how GWZ Core satisfies the accepted v0 direction.

`GWZRequirements.md` is the baseline for required behavior. This accepted design
is authoritative for implementation choices where later decisions refined or
superseded earlier requirement text.

## Design Goals

- Keep the core as a library with no required daemon.
- Keep all public API traffic message-oriented and taut-defined.
- Treat the `gwz` CLI as a thin driver over the same messages used by every
  other driver.
- Use a native GWZ manifest and lock format.
- Preserve explicit attribution seams so non-CLI drivers can attach an actor and
  Git object identity to operations instead of inheriting ambient process state.
- Treat selection-wide mutation as atomic-by-default through preflight and
  explicit partial-mode policy.
- Keep Git storage ordinary and non-bare for v0.
- Prefer a Rust-native Git backend using gitoxide/gix.
- Keep room for later source catalogs, non-Git materialization, file watching,
  worktrees, and local mirror caches without putting them in v0.

## Module Layout

```text
gwz-core
  model
    workspace
    member
    source
    lock
    snapshot
    selection
    policy
    operation
  artifact
    manifest_io
    lock_io
    snapshot_io
    tag_io
    atomic_write
    schema_version
  protocol
    gwz.taut.py
    generated message types
    dispatcher
  operations
    operation trait
    operation registry
    planner
    preflight
    executor
    event_stream
    artifact_commit
  git
    backend trait
    selected backend
    status parser
  runtime
    clock
    id generator
    concurrency limiter
    member locks
  errors
    typed errors
    taut error enum
  testing
    temp repo fixtures
    golden artifact fixtures
    protocol corpus

gwz
  cli parser
  request builder
  response/event renderer
```

The module names are descriptive, not a required crate/module tree.

## Architectural Shape

GWZ Core should use a ports-and-adapters shape:

```text
driver
  -> taut request
  -> protocol dispatcher
  -> operation registry
  -> pure planning code
  -> backend ports
  -> taut response/events/result
```

The core should keep these boundaries separate:

- `model`: pure value types with no filesystem or Git access.
- `artifact`: deterministic read/write of workspace files.
- `operations`: request validation, selection resolution, planning, preflight,
  execution orchestration, and artifact commit.
- `git`: the Git backend port and its selected implementation.
- `protocol`: generated taut messages plus mapping between protocol values and
  internal model values.
- `runtime`: clocks, ids, concurrency limits, event channels, and member locks.

The public library surface should be message-first:

```text
submit(request) -> immediate response
subscribe(operation_id) -> operation event stream
wait(operation_id) -> final operation result
```

Typed Rust helper APIs may exist, but they should be thin constructors around
taut requests. They must not become a second behavioral API.

## Low-Boilerplate Operation Framework

Each operation should plug into one common framework instead of hand-rolling
command-specific control flow.

Conceptual operation interface:

```text
OperationSpec
  action_kind
  decode(request)
  validate(input)
  load_context(input)
  resolve_selection(context, input)
  plan(context, selection, input)
  preflight(context, plan)
  execute_member(context, member_plan)
  commit_artifacts(context, execution_report)
  summarize(context, execution_report)
```

Most operations should only implement the parts that are specific to that
operation. The framework owns:

- request id and operation id assignment
- attribution validation and propagation
- workspace discovery and artifact loading
- policy resolution
- selection resolution
- atomic-by-default preflight gating
- parallel member execution
- per-member mutation locks
- event sequencing
- aggregate status calculation
- final response/result assembly

Shared operation values:

```text
WorkspaceContext
  root
  manifest
  lock
  runtime

OperationPlan
  operation_id
  action_kind
  selection
  member_plans
  artifact_effects

MemberPlan
  member_id
  path
  planned_action
  preconditions
  target_ref

PreflightReport
  aggregate_status
  member_responses

ExecutionReport
  aggregate_status
  member_results
  artifact_results
```

This keeps commands easy to test:

- validation tests do not need Git repositories
- selection and policy tests do not need filesystem mutation
- planning tests can use in-memory manifest and lock fixtures
- preflight and execution tests can use temporary local repositories
- protocol tests can round-trip taut messages independently of Git

## Artifact Layout

One workspace root contains:

```text
workspace/
  gwz.yml
  gwz.lock.yml
  tags/
    <tag-name>.yml
.gwz/
  snapshots/
    <snapshot-id>.yaml
```

`workspace/` contains versioned workspace artifacts. `.gwz/` is internal runtime
state.

Decision: v0 uses the directory layout above. Older draft/requirements text that
named flat files such as `workspace.gwz.yaml` and `workspace.gwz.lock.yaml` is
superseded by this accepted design.

Persistent operation event logs under `.gwz/operations/` are deferred. V0 keeps
operation events in memory and lets drivers render or persist event streams.

## Workspace Discovery

Commands that operate on an existing workspace should discover the workspace
root by walking upward from the current directory until they find:

```text
workspace/gwz.yml
```

The first match wins.

This means running `gwz` from inside a materialized subrepo still acts on the
containing GWZ workspace, not on the subrepo as an independent workspace.

If no workspace is found, commands that require an existing workspace fail with
`workspace_not_found`.

`gwz init` is different: it targets the current directory unless a driver or CLI
explicitly supplies another root.

To avoid discovery shadowing, v0 treats an active member root containing its own
`workspace/gwz.yml` as a nested active GWZ workspace and rejects it during member
validation. A fresh workspace checkout that has `workspace/gwz.yml` but no `.gwz/`
is still a valid root; commands may create `.gwz/` as needed.

## Manifest Shape

The manifest is YAML and uses a native GWZ layout:

```yaml
schema: gwz.workspace/v0
workspace:
  id: ws_01JZ...

members:
  - id: mem_01JZ...
    path: repos/example
    type: git
    source_id: src_01JZ...
    active: true
    desired:
      branch: main
    remotes:
      - name: origin
        url: git@github.com:example/example.git
        fetch: true
        push: true
```

Design rules:

- `member_id` is the stable member identity.
- `source_id` is separate even when v0 sets it equal to `member_id`.
- `path` is a materialization location, not identity.
- `remotes` is a deterministic list of remote specs, not a YAML map. Readers
  must reject duplicate remote names.
- `desired.git_tag`, when present, means a Git tag/ref in the member
  repository. GWZ workspace tags are separate artifacts under `workspace/tags/`
  and are never stored as member desired refs.

## Path Validation

Member paths are always relative to the workspace root.

Path validation must reject:

- absolute paths
- paths that escape the workspace root
- paths that collide with another member path
- paths under `workspace/`
- paths under `.gwz/`

`workspace/` is reserved for versioned GWZ metadata. `.gwz/` is reserved for
internal runtime state.

## Lock Shape

The lock records resolved state, not desired state:

```yaml
schema: gwz.lock/v0
workspace_id: ws_01JZ...
manifest_schema: gwz.workspace/v0
created_at: "2026-06-15T00:00:00Z"
members:
  mem_01JZ...:
    path: repos/example
    source_id: src_01JZ...
    source_kind: git
    commit: abc123...
    branch: main
    detached: false
    upstream: origin/main
    dirty: false
    materialized: true
```

The lock is regenerated by operations that change the current materialized
workspace state and is written atomically.

## Artifact Write Policy

Default v0 artifact writes:

```text
create workspace
  writes workspace/gwz.yml and creates .gwz/
  does not write workspace/gwz.lock.yml unless explicitly requested

init from sources
  writes workspace/gwz.yml
  writes workspace/gwz.lock.yml after successful materialization

add existing repository
  writes workspace/gwz.yml
  writes workspace/gwz.lock.yml with the added member's current resolved state

create repository
  writes workspace/gwz.yml
  writes workspace/gwz.lock.yml with the new local-only member state

materialize to lock
  reads workspace/gwz.lock.yml
  does not rewrite the lock by default

materialize to head
materialize to snapshot
materialize to tag
pull to head
pull to snapshot
  writes workspace/gwz.lock.yml after successful materialization/update

snapshot
  writes .gwz/snapshots/<snapshot-id>.yaml
  does not rewrite the lock

tag
  writes workspace/tags/<tag-name>.yml
  does not rewrite the lock

status
push
  do not write GWZ artifacts by default
```

If an operation is rejected during preflight, no artifact writes occur. If an
unexpected host failure happens after mutation begins, artifact writes must
reflect only successfully completed artifact commits and the final result must
report partial failure.

## Snapshot Shape

Snapshots use the same resolved member-state shape as the lock, plus a snapshot
identity and selection:

```yaml
schema: gwz.snapshot/v0
workspace_id: ws_01JZ...
snapshot_id: snap_demo
created_at: "2026-06-15T00:00:00Z"
created_by:
  actor_id: agent_01JZ...
selected_members:
  - mem_01JZ...
members:
  mem_01JZ...:
    path: repos/example
    source_kind: git
    commit: abc123...
```

Snapshots are GWZ-owned records. They are not Git tags.

## GWZ Tag Shape

GWZ tags are workspace-scoped, versioned GWZ artifacts.

They are not Git tags. A GWZ tag records the resolved member refs for one
workspace tag name without writing `refs/tags/*` inside member repositories.

```yaml
schema: gwz.tag/v0
workspace_id: ws_01JZ...
tag: demo
created_at: "2026-06-15T00:00:00Z"
created_by:
  actor_id: agent_01JZ...
selected_members:
  - mem_01JZ...
members:
  mem_01JZ...:
    path: repos/example
    source_kind: git
    commit: abc123...
```

The same tag name may exist in different workspaces without colliding, even if
those workspaces share one or more member repositories.

GWZ tag files are intended to be versioned with the workspace metadata.

Git tags are optional per-repository publication artifacts and should be handled
by an explicit export or publish operation, not by default GWZ tag creation.

## Identity

Use opaque stable ids:

```text
workspace id: ws_<ulid>
source id:    src_<ulid>
member id:    mem_<ulid>
operation id: op_<ulid>
snapshot id:  user-provided slug or snap_<ulid>
```

The exact id generator can be changed later as long as ids remain opaque,
stable, and serializable.

Path changes do not change `member_id`. Remote URL changes do not change
`member_id`. If the same logical source is used in two workspaces, both members
may share `source_id` but must have distinct `member_id` values.

## Git Backend

The design exposes an internal `GitBackend` boundary:

```text
status(repo) -> GitStatus
clone(url, path, remote, target) -> GitCloneResult
fetch(repo, remote) -> GitFetchResult
fast_forward(repo, upstream) -> GitUpdateResult
checkout_commit(repo, commit) -> GitUpdateResult
create_repo(path) -> GitCreateResult
add_remote(repo, name, url) -> GitRemoteResult
push(repo, remote, refspec) -> GitPushResult
read_ref(repo, ref_spec) -> GitRefResult
```

Backend calls should receive operation attribution context. Calls that create
Git objects should accept explicit Git object identities instead of reading only
ambient Git config.

V0 should use ordinary non-bare repositories.

The preferred backend is Rust-native and should use gitoxide/gix where it
provides the required behavior. The v0 backend must be selected by a capability
spike before the main backend implementation lands. The spike must prove clone,
fetch, fast-forward, checkout, status, and push against local fixtures.

If gix cannot satisfy the v0 surface cleanly, v0 may use git2 behind the same
`GitBackend` trait. Shelling out to `git` should remain a last-resort fallback
for isolated missing behavior, not the primary implementation.

The backend boundary exists so backend choice stays an implementation detail
rather than becoming public API behavior.

## Root/Member Boundary

Member repositories are nested git repos at `root/<member.path>`. The root repo
treats each member as opaque — it never tracks or recurses into member files — by
**hiding** every member from the root via a gwz-managed block in the root repo's
local `.git/info/exclude`, alongside `/gwz.conf/.tmp/`.

`gwz.yml` is authoritative for membership and `gwz.lock.yml` is the authoritative
record of each member's commit; members are *not* tracked in the root (no gitlinks,
no committed member entries). The exclude block is **local and never committed** —
`sync_workspace_boundary` regenerates it from the lock on every lock-writing op
(reconciling added/removed members, preserving any user lines), so a fresh clone
re-derives it on the next gwz run; we don't persist it. gwz writes no `.gitignore`
at all (a user's own `.gitignore` is left untouched).

The only path that records workspace composition in root history is the `gwz commit`
verb: it commits members first, re-locks from their new HEADs, then commits the root
(`gwz.conf`) last — so the committed lock reflects the post-commit member state. This
lands back on the original AD2 disposition (the lock SHA, not a gitlink, is the
record); the earlier gitlink boundary was implemented then reverted — see
`GWZGitlinkPlan.md` for that historical design.

## Operation Flow

Every mutating selection-wide operation follows the same pipeline:

```text
request
  -> validate message
  -> load workspace artifacts
  -> resolve selection
  -> build plan
  -> preflight all selected members
  -> if dry_run: return plan
  -> if preflight failed: reject before mutation
  -> execute member plans in parallel
  -> stream operation events
  -> write lock/snapshot if required
  -> return final aggregate + per-member result
```

Atomic-by-default means GWZ Core does not begin mutation if preflight fails.
It does not guarantee transactional rollback for unexpected host or Git failures
after mutation begins. Unexpected mid-operation failures are reported as
per-member failures and aggregate partial-failure status.

## Selection Resolution

The resolver accepts:

```text
all active members
member ids
member paths
```

V0 does not need named selections.

Resolution returns a sorted deterministic list of member ids. Unknown,
inactive, or ambiguous references fail before preflight.

## Policy Resolution

Policy is resolved in this order:

```text
operation request override
member override
workspace default
built-in default
```

V0 built-in defaults:

```text
atomic: true
partial: false
destructive: false
sync: ff-only
unsupported_member: fail
```

`SyncBehavior` applies only to remote-backed members that can move toward a
remote ref. Local-only members produce `PlannedAction.noop` and
`MemberStatus.noop`; `noop` is not a sync policy.

`driver_selected` is a reserved policy escape hatch for future drivers that
provide an explicit sync-policy resolver. If no resolver is registered, v0
should reject `driver_selected` with `unsupported_operation`.

## Attribution And Git Identity

GWZ operations must not rely on ambient process identity as the only way to
record who caused an operation. Non-CLI drivers need to carry an actor from the
request message.

The model separates three identities:

```text
operation actor
  who or what requested the GWZ operation

Git object identity
  author/committer/tagger identity written into Git objects when an operation
  creates commits or annotated tags

remote credential principal
  the identity accepted by a remote Git service for fetch/push
```

API clients should populate these fields as follows:

```text
OperationActor
  Put the authenticated requester or responsible principal here. This is the
  GWZ audit identity: "who caused this GWZ operation?"

GitObjectIdentity.git_author
  Put the person or agent that should be recorded as the author of newly
  created Git content. This answers: "who originated the change?"

GitObjectIdentity.git_committer
  Put the person, agent, or service that actually wrote the Git object. This
  answers: "who committed/materialized this Git object?"

credential_ref
  Put an opaque driver-owned reference to credentials used for remote Git
  operations. This answers: "which credential should the driver use?"
```

Typical client choices:

```text
human using local CLI
  actor: optional or local user
  git_author: local Git author config when creating Git objects
  git_committer: local Git committer config when creating Git objects
  credential_ref: omitted

AI agent acting on behalf of a human
  actor: the agent/session principal
  git_author: the human owner if the human is the author of intent, otherwise
    the agent identity
  git_committer: the agent identity, because the agent wrote the Git object
  credential_ref: a driver-owned credential reference if remote access is needed

autonomous agent creating its own work
  actor: the agent principal
  git_author: the agent Git identity
  git_committer: the agent Git identity
  credential_ref: a driver-owned credential reference if remote access is needed

service automation applying an approved change
  actor: the automation or approval principal, depending on driver policy
  git_author: the original change author
  git_committer: the automation/service identity
  credential_ref: service credential reference
```

If a request supplies an actor but omits Git object identity, object-creating
operations should either use driver policy to derive Git identity or fail with
`attribution_denied`. They should not silently fall back to ambient process Git
config unless the driver explicitly allows that fallback.

GWZ Core can control Git object identity for operations that create Git objects
by passing explicit signatures to the Git backend. Future commit, merge, rebase,
and annotated Git-tag operations should use request-provided Git identity when
present, subject to driver policy.

GWZ Core can record the operation actor in operation events, operation results,
and GWZ-owned artifacts. That actor is GWZ attribution, not proof by itself. A
driver that accepts requests from users or agents is responsible for
authenticating the actor and deciding whether the requested Git identity is
allowed.

GWZ Core cannot make a remote forge attribute a push to an arbitrary actor. Push
attribution on services such as GitHub, GitLab, Gitea, or Forgejo follows the
credential used by the remote operation. The protocol may carry an opaque
`credential_ref`, but credential resolution and authorization stay outside GWZ
Core.

The CLI may omit attribution or derive it from local Git config for human
display. A non-CLI driver should populate attribution from its message/session
principal instead of from the OS user running the process.

## Message Protocol

The taut schema should be authored as `gwz.taut.py`. The field inventory below
is the design contract; exact syntax may adjust to the taut compiler.

Timestamps are `INT` milliseconds since the Unix epoch UTC. Paths are strings.
Workspace-relative paths must stay relative; workspace roots may be absolute.

Action requests are typed messages. Responses use a shared envelope so the CLI,
UI, and future daemon can render every operation with one code path.

Long-running actions return `accepted` plus an `operation_id`, then emit
`OperationEvent` messages and finish with `OperationResult`. Short actions may
return a final response without an operation id.

```python
from taut.ir.dsl import BOOL, BYTES, INT, STR, Enum, F, List, Msg, Ref, method, schema, service

SCHEMA = schema(
    # ---- enums ------------------------------------------------------------
    Enum("ActionKind",
         create_workspace=0,
         init_from_sources=1,
         add_existing_repo=2,
         create_repo=3,
         materialize=4,
         status=5,
         snapshot=6,
         tag=7,
         pull_head=8,
         pull_snapshot=9,
         push=10),

    Enum("SourceKind",
         git=0,
         archive=1,
         package=2,
         local=3,
         generated=4),

    Enum("AggregateStatus",
         accepted=0,
         ok=1,
         noop=2,
         rejected=3,
         partial=4,
         failed=5),

    Enum("MemberStatus",
         planned=0,
         ok=1,
         noop=2,
         skipped=3,
         rejected=4,
         failed=5),

    Enum("MaterializeTargetKind",
         lock=0,
         head=1,
         snapshot=2,
         tag=3,
         commit=4),

    Enum("SyncBehavior",
         fetch_only=0,
         ff_only=1,
         merge=2,
         rebase=3,
         reset=4,
         driver_selected=5),

    Enum("PartialBehavior",
         atomic=0,
         partial=1),

    Enum("DestructiveBehavior",
         refuse=0,
         allow=1),

    Enum("UnsupportedMemberBehavior",
         fail=0,
         skip=1),

    Enum("PlannedAction",
         noop=0,
         clone=1,
         fetch=2,
         fast_forward=3,
         checkout=4,
         init_repo=5,
         add_manifest_member=6,
         write_manifest=7,
         write_lock=8,
         write_snapshot=9,
         write_tag=10,
         push=11),

    Enum("LockMatch",
         unknown=0,
         matches=1,
         differs=2,
         missing=3),

    Enum("EventKind",
         operation_started=0,
         member_started=1,
         member_progress=2,
         member_finished=3,
         artifact_written=4,
         operation_finished=5,
         reset=6),

    Enum("Severity",
         debug=0,
         info=1,
         warn=2,
         error=3),

    Enum("GwzErrorCode",
         ok=0,
         invalid_request=1,
         workspace_not_found=2,
         workspace_already_exists=3,
         nested_workspace=4,
         manifest_not_found=5,
         manifest_invalid=6,
         schema_unsupported=7,
         member_not_found=8,
         member_inactive=9,
         path_escape=10,
         path_collision=11,
         path_reserved=12,
         unsupported_source_kind=13,
         unsupported_operation=14,
         dirty_member=15,
         diverged_member=16,
         missing_remote=17,
         snapshot_not_found=18,
         lock_not_found=19,
         tag_not_found=20,
         tag_invalid=21,
         remote_rejected=22,
         git_command_failed=23,
         external_tool_missing=24,
         operation_not_found=25,
         attribution_denied=26,
         permission_denied=27,
         io_error=28,
         internal_error=29),

    # ---- common request/response values ----------------------------------
    Msg("WorkspaceRef",
        F("root", 1, STR, optional=True),
        F("workspace_id", 2, STR, optional=True)),

    Msg("OperationActor",
        F("actor_id", 1, STR),
        F("display_name", 2, STR, optional=True),
        F("email", 3, STR, optional=True),
        F("authority", 4, STR, optional=True)),

    Msg("GitObjectIdentity",
        F("name", 1, STR),
        F("email", 2, STR),
        F("time_ms", 3, INT, optional=True),
        F("timezone_offset_minutes", 4, INT, optional=True)),

    Msg("OperationAttribution",
        F("actor", 1, Ref("OperationActor"), optional=True),
        F("git_author", 2, Ref("GitObjectIdentity"), optional=True),
        F("git_committer", 3, Ref("GitObjectIdentity"), optional=True),
        F("credential_ref", 4, STR, optional=True)),

    Msg("Selection",
        F("all", 1, BOOL, optional=True),
        F("member_ids", 2, List(STR)),
        F("paths", 3, List(STR))),

    Msg("OperationPolicy",
        F("partial", 1, Ref("PartialBehavior"), optional=True),
        F("destructive", 2, Ref("DestructiveBehavior"), optional=True),
        F("sync", 3, Ref("SyncBehavior"), optional=True),
        F("unsupported_member", 4, Ref("UnsupportedMemberBehavior"), optional=True),
        F("remote", 5, STR, optional=True),
        F("concurrency", 6, INT, optional=True)),

    Msg("RequestMeta",
        F("request_id", 1, STR),
        F("schema_version", 2, STR),
        F("workspace", 3, Ref("WorkspaceRef"), optional=True),
        F("selection", 4, Ref("Selection"), optional=True),
        F("policy", 5, Ref("OperationPolicy"), optional=True),
        F("dry_run", 6, BOOL, optional=True),
        F("attribution", 7, Ref("OperationAttribution"), optional=True)),

    Msg("ResponseMeta",
        F("request_id", 1, STR),
        F("schema_version", 2, STR),
        F("action", 3, Ref("ActionKind")),
        F("aggregate_status", 4, Ref("AggregateStatus")),
        F("operation_id", 5, STR, optional=True),
        F("message", 6, STR, optional=True),
        F("attribution", 7, Ref("OperationAttribution"), optional=True)),

    Msg("GwzError",
        F("code", 1, Ref("GwzErrorCode")),
        F("message", 2, STR),
        F("member_id", 3, STR, optional=True),
        F("member_path", 4, STR, optional=True),
        F("detail", 5, STR, optional=True)),

    # ---- model projections ------------------------------------------------
    Msg("RemoteSpec",
        F("name", 1, STR),
        F("url", 2, STR),
        F("fetch", 3, BOOL, optional=True),
        F("push", 4, BOOL, optional=True)),

    Msg("DesiredRef",
        F("branch", 1, STR, optional=True),
        F("commit", 2, STR, optional=True),
        F("git_tag", 3, STR, optional=True),
        F("local_only", 4, BOOL, optional=True)),

    Msg("SourceUrl",
        F("url", 1, STR),
        F("path", 2, STR, optional=True),
        F("remote_name", 3, STR, optional=True),
        F("branch", 4, STR, optional=True)),

    Msg("MemberSpec",
        F("member_id", 1, STR),
        F("path", 2, STR),
        F("source_id", 3, STR),
        F("source_kind", 4, Ref("SourceKind")),
        F("active", 5, BOOL),
        F("desired", 6, Ref("DesiredRef"), optional=True),
        F("remotes", 7, List(Ref("RemoteSpec")))),

    Msg("MaterializeTarget",
        F("kind", 1, Ref("MaterializeTargetKind")),
        F("name", 2, STR, optional=True),
        F("commit", 3, STR, optional=True)),

    Msg("ResolvedMemberState",
        F("member_id", 1, STR),
        F("path", 2, STR),
        F("source_id", 3, STR),
        F("source_kind", 4, Ref("SourceKind")),
        F("commit", 5, STR, optional=True),
        F("branch", 6, STR, optional=True),
        F("detached", 7, BOOL, optional=True),
        F("upstream", 8, STR, optional=True),
        F("dirty", 9, BOOL, optional=True),
        F("materialized", 10, BOOL),
        F("remotes", 11, List(Ref("RemoteSpec")))),

    Msg("GitStatus",
        F("member_id", 1, STR),
        F("branch", 2, STR, optional=True),
        F("detached", 3, BOOL),
        F("head", 4, STR, optional=True),
        F("upstream", 5, STR, optional=True),
        F("ahead", 6, INT, optional=True),
        F("behind", 7, INT, optional=True),
        F("staged", 8, INT),
        F("unstaged", 9, INT),
        F("untracked", 10, INT),
        F("dirty", 11, BOOL)),

    Msg("PlannedChange",
        F("action", 1, Ref("PlannedAction")),
        F("from_ref", 2, STR, optional=True),
        F("to_ref", 3, STR, optional=True),
        F("message", 4, STR, optional=True)),

    Msg("MemberResponse",
        F("member_id", 1, STR),
        F("member_path", 2, STR),
        F("source_kind", 3, Ref("SourceKind")),
        F("status", 4, Ref("MemberStatus")),
        F("error", 5, Ref("GwzError"), optional=True),
        F("planned", 6, Ref("PlannedChange"), optional=True),
        F("state", 7, Ref("ResolvedMemberState"), optional=True),
        F("git_status", 8, Ref("GitStatus"), optional=True),
        F("lock_match", 9, Ref("LockMatch"), optional=True)),

    Msg("ResponseEnvelope",
        F("meta", 1, Ref("ResponseMeta")),
        F("members", 2, List(Ref("MemberResponse"))),
        F("errors", 3, List(Ref("GwzError")))),

    Msg("OperationEvent",
        F("operation_id", 1, STR),
        F("request_id", 2, STR),
        F("sequence", 3, INT),
        F("timestamp_ms", 4, INT),
        F("kind", 5, Ref("EventKind")),
        F("severity", 6, Ref("Severity")),
        F("member_id", 7, STR, optional=True),
        F("member_path", 8, STR, optional=True),
        F("message", 9, STR, optional=True),
        F("member", 10, Ref("MemberResponse"), optional=True),
        F("error", 11, Ref("GwzError"), optional=True),
        F("attribution", 12, Ref("OperationAttribution"), optional=True)),

    Msg("OperationResult",
        F("operation_id", 1, STR),
        F("request_id", 2, STR),
        F("action", 3, Ref("ActionKind")),
        F("aggregate_status", 4, Ref("AggregateStatus")),
        F("started_at_ms", 5, INT),
        F("finished_at_ms", 6, INT),
        F("members", 7, List(Ref("MemberResponse"))),
        F("errors", 8, List(Ref("GwzError"))),
        F("attribution", 9, Ref("OperationAttribution"), optional=True)),

    # ---- action requests --------------------------------------------------
    Msg("CreateWorkspaceRequest",
        F("meta", 1, Ref("RequestMeta")),
        F("workspace_root", 2, STR),
        F("workspace_id", 3, STR, optional=True)),

    Msg("InitFromSourcesRequest",
        F("meta", 1, Ref("RequestMeta")),
        F("workspace_root", 2, STR),
        F("sources", 3, List(Ref("SourceUrl"))),
        F("target", 4, Ref("MaterializeTarget"), optional=True),
        F("workspace_id", 5, STR, optional=True)),

    Msg("AddExistingRepoRequest",
        F("meta", 1, Ref("RequestMeta")),
        F("repository_path", 2, STR),
        F("member_path", 3, STR, optional=True),
        F("member_id", 4, STR, optional=True),
        F("source_id", 5, STR, optional=True)),

    Msg("CreateRepoRequest",
        F("meta", 1, Ref("RequestMeta")),
        F("member_path", 2, STR),
        F("initial_branch", 3, STR, optional=True),
        F("member_id", 4, STR, optional=True),
        F("source_id", 5, STR, optional=True)),

    Msg("MaterializeRequest",
        F("meta", 1, Ref("RequestMeta")),
        F("target", 2, Ref("MaterializeTarget"))),

    Msg("StatusRequest",
        F("meta", 1, Ref("RequestMeta"))),

    Msg("SnapshotRequest",
        F("meta", 1, Ref("RequestMeta")),
        F("snapshot_id", 2, STR)),

    Msg("TagRequest",
        F("meta", 1, Ref("RequestMeta")),
        F("tag_name", 2, STR)),

    Msg("PullHeadRequest",
        F("meta", 1, Ref("RequestMeta"))),

    Msg("PullSnapshotRequest",
        F("meta", 1, Ref("RequestMeta")),
        F("snapshot_id", 2, STR)),

    Msg("PushRequest",
        F("meta", 1, Ref("RequestMeta")),
        F("remote", 2, STR, optional=True),
        F("refspec", 3, STR, optional=True)),

    # ---- action responses -------------------------------------------------
    Msg("CreateWorkspaceResponse", F("response", 1, Ref("ResponseEnvelope"))),
    Msg("InitFromSourcesResponse", F("response", 1, Ref("ResponseEnvelope"))),
    Msg("AddExistingRepoResponse", F("response", 1, Ref("ResponseEnvelope"))),
    Msg("CreateRepoResponse", F("response", 1, Ref("ResponseEnvelope"))),
    Msg("MaterializeResponse", F("response", 1, Ref("ResponseEnvelope"))),
    Msg("StatusResponse", F("response", 1, Ref("ResponseEnvelope"))),
    Msg("SnapshotResponse", F("response", 1, Ref("ResponseEnvelope"))),
    Msg("TagResponse", F("response", 1, Ref("ResponseEnvelope"))),
    Msg("PullHeadResponse", F("response", 1, Ref("ResponseEnvelope"))),
    Msg("PullSnapshotResponse", F("response", 1, Ref("ResponseEnvelope"))),
    Msg("PushResponse", F("response", 1, Ref("ResponseEnvelope"))),

    service("GwzCore",
        method("create_workspace", role="in",
               params=[("request", Ref("CreateWorkspaceRequest"))],
               out=Ref("CreateWorkspaceResponse")),
        method("init_from_sources", role="in",
               params=[("request", Ref("InitFromSourcesRequest"))],
               out=Ref("InitFromSourcesResponse")),
        method("add_existing_repo", role="in",
               params=[("request", Ref("AddExistingRepoRequest"))],
               out=Ref("AddExistingRepoResponse")),
        method("create_repo", role="in",
               params=[("request", Ref("CreateRepoRequest"))],
               out=Ref("CreateRepoResponse")),
        method("materialize", role="in",
               params=[("request", Ref("MaterializeRequest"))],
               out=Ref("MaterializeResponse")),
        method("status", role="in",
               params=[("request", Ref("StatusRequest"))],
               out=Ref("StatusResponse")),
        method("snapshot", role="in",
               params=[("request", Ref("SnapshotRequest"))],
               out=Ref("SnapshotResponse")),
        method("tag", role="in",
               params=[("request", Ref("TagRequest"))],
               out=Ref("TagResponse")),
        method("pull_head", role="in",
               params=[("request", Ref("PullHeadRequest"))],
               out=Ref("PullHeadResponse")),
        method("pull_snapshot", role="in",
               params=[("request", Ref("PullSnapshotRequest"))],
               out=Ref("PullSnapshotResponse")),
        method("push", role="in",
               params=[("request", Ref("PushRequest"))],
               out=Ref("PushResponse")),
        method("events.subscribe", role="out", shape="log",
               params=[("operation_id", STR)],
               out=Ref("OperationEvent")),
        method("operation.result", role="out",
               params=[("operation_id", STR)],
               out=Ref("OperationResult"))),
)
```

The first implementation should generate Rust types from this schema and keep a
small hand-written mapping layer between generated protocol types and internal
model types. That mapping layer is where validation errors become `GwzError`
values.

## Operation Runtime

The runtime owns:

- an operation registry keyed by `operation_id`
- a bounded in-memory event buffer per running operation
- a global concurrency limit
- a per-member mutation lock

Execution is parallel across members and serialized per member.

Status/read-only operations may run concurrently with other reads. Mutating
operations require the per-member mutation lock.

V0 runtime decision:

```text
backend calls: synchronous
operation execution: std thread based worker pool
async runtime requirement: none
public API: synchronous submit/subscribe/wait functions
```

`submit(request)` validates the request, registers the operation, starts
background execution for long-running operations, and returns an immediate
accepted response or a rejection. It does not require callers to provide an async
runtime.

`subscribe(operation_id)` returns a stream/receiver over the in-memory event
buffer for that operation. Event emission must not block member execution
indefinitely. V0 uses a bounded ring buffer. If the buffer overflows, the runtime
drops older buffered incremental events, records overflow state, and keeps a
reset event plus later events so subscribers know incremental event history is
incomplete. The final `OperationResult` is retained separately and must not be
dropped by event-buffer overflow.

`wait(operation_id)` waits for the final `OperationResult`. It must not require a
caller to drain `subscribe()` to avoid deadlock. The final result is stored in
the operation registry until the operation is collected by explicit cleanup or
driver-owned lifecycle policy.

V0 uses taut for generated message types and service shape, not as a required
RPC transport. The in-process dispatcher decodes the action request, routes by
action kind/service method, and invokes the matching operation spec.

## CLI Driver Design

The `gwz` CLI is the first driver, not a separate architecture.

CLI flow:

```text
argv
  -> parse command
  -> discover workspace root when command requires one
  -> build taut request
  -> submit request to in-process GWZ Core
  -> render immediate response
  -> if accepted: render events until OperationResult
```

The CLI should not call Git directly. It should not read or write workspace
artifacts directly. All behavior goes through the same message path used by any
future daemon, UI, or test harness.

Suggested v0 command mapping:

```text
gwz init
  -> CreateWorkspaceRequest

gwz init <url>...
  -> InitFromSourcesRequest

gwz add <repo-path>
  -> AddExistingRepoRequest

gwz repo create <member-path>
  -> CreateRepoRequest

gwz materialize --lock
gwz materialize --head
gwz materialize --snapshot <name>
gwz materialize --tag <name>
  -> MaterializeRequest

gwz pull --head
  -> PullHeadRequest

gwz pull --snapshot <name>
  -> PullSnapshotRequest

gwz snapshot <name>
  -> SnapshotRequest

gwz tag <name>
  -> TagRequest

gwz push [--remote <name>] [--refspec <refspec>]
  -> PushRequest

gwz status
  -> StatusRequest
```

Global CLI options should map to common message fields:

```text
--root <path>       RequestMeta.workspace.root
--member <id>       RequestMeta.selection.member_ids
--path <path>       RequestMeta.selection.paths
--all               RequestMeta.selection.all
--dry-run           RequestMeta.dry_run
--partial           OperationPolicy.partial = partial
--force             OperationPolicy.destructive = allow
--sync <mode>       OperationPolicy.sync
--remote <name>     OperationPolicy.remote or action-specific remote
--jobs <n>          OperationPolicy.concurrency
--json              render final response/result as JSON
--jsonl             render response, events, and result as JSON lines
```

For `gwz init`, positional arguments are source URLs. No `--repo` flag is
needed. `gwz init` with no positional arguments creates an empty workspace in
the current directory.

Commands that require an existing workspace use upward discovery from the
current directory unless `--root` is supplied. `gwz init` does not do upward
discovery for its target; it targets the current directory or `--root`.

CLI rendering should be replaceable:

```text
human renderer
  concise command output for terminals

json renderer
  final taut JSON projection for scripts

jsonl renderer
  response/event/result stream for tools and tests
```

The JSON and JSONL renderers are important for testing. CLI integration tests
should assert on protocol-shaped output, not on human formatting.

## V0 Operation Designs

### Create Workspace

Inputs:

- workspace root
- optional workspace id

Preflight:

- if `workspace/gwz.yml` already exists at the target root, reject as
  `workspace_already_exists`
- if an ancestor contains `workspace/gwz.yml`, reject as
  `nested_workspace`
- if the target root contains ordinary files but no GWZ workspace, allow
  initialization

Effects:

- create `workspace/gwz.yml`
- create `.gwz/`
- optionally create `workspace/gwz.lock.yml`

### Initialize From Sources

CLI shape:

```text
gwz init git@github.com:org/repo-a.git git@github.com:org/repo-b.git
```

For `gwz init`, positional arguments are source URLs.

Inputs:

- workspace root
- ordered source URL list
- optional workspace id
- optional target, defaulting to head

Preflight:

- apply the same workspace-existence checks as Create Workspace
- validate every source URL
- validate derived or supplied member paths
- fail before mutation if any source URL or member path is invalid

Flow:

- derive member paths for URLs without explicit paths
- create workspace artifacts
- create one member per URL
- materialize all members to the target
- write `workspace/gwz.lock.yml`
- return aggregate result and per-member results

Default path derivation uses `repos/<repo-name>`, where `<repo-name>` is the
last non-empty path segment of the source URL with a trailing `.git` suffix
removed. The derived name must be normalized to a safe relative path segment.
Empty names, unsafe names, and collisions fail validation unless the request
supplies explicit paths.

### Add Existing Repository

Inputs:

- workspace root
- repository path
- optional member id
- optional source id

Flow:

- verify path is inside workspace
- verify path is a Git repository
- read current Git state
- add manifest entry
- update lock with the added member's current resolved state

### Create Repository

Inputs:

- workspace root
- member path
- optional branch name
- optional member id/source id

Flow:

- create directory
- initialize ordinary Git repository
- add manifest entry with local-only marker
- update lock

### Materialize To Lock

Inputs:

- workspace root
- optional selection
- lock target

Flow:

- resolve selected members
- preflight dirty state and availability
- clone missing members
- checkout locked commits

### Materialize To Head

Inputs:

- workspace root
- optional selection

Flow:

- resolve selected members
- clone missing remote-backed members at configured branch
- local-only members become `noop`
- existing members use pull-to-head semantics

### Pull To Head

Default policy:

- all selected members must be clean
- all remote-backed members must be able to fast-forward
- local-only members return `noop`
- diverged members block the operation

### Pull To Snapshot

Default policy:

- all selected members must be clean
- selected members move to exact snapshot commits
- missing snapshot entries block the operation
- destructive checkout requires explicit policy

### Snapshot

Inputs:

- workspace root
- optional selection
- snapshot id

Flow:

- resolve selection
- read current member states
- write snapshot file atomically

### Tag

Inputs:

- workspace root
- tag name
- optional selection

Flow:

- validate tag name in the workspace tag namespace
- resolve selection
- read current member refs
- write `workspace/tags/<tag-name>.yml` atomically
- return aggregate result and per-member results

GWZ tag creation does not create Git tags in member repositories.

### Push

Default policy:

- selected Git members must have selected push remotes
- local-only members without a remote are skipped or fail according to policy
- per-member push outcomes are required

## Error Model

Errors are exposed as typed taut enum values through `GwzErrorCode`.

The protocol enum is the registry for error-code names and integer values. Error
values must be stable once published. New errors may be appended, but existing
numeric values must not be reused for a different meaning.

## Testing Design

V0 tests should be organized by requirement families:

```text
artifact tests
  manifest parse/write
  lock parse/write
  snapshot parse/write
  tag parse/write
  atomic write behavior

identity tests
  generated ids persist
  member id survives path change
  source id/member id relationship

selection tests
  all active members
  by id
  by path
  unknown/inactive failures

git tests
  add existing repo
  create repo
  status clean/dirty/diverged
  pull head ff-only
  local-only noop

operation tests
  dry-run plan
  preflight rejection before mutation
  accepted operation event stream
  attribution propagated to responses and events
  per-member response entries
  partial mode visibility

protocol tests
  request/response serialization
  attribution serialization
  error code stability
  operation event ordering

cli tests
  argv to request mapping
  workspace discovery behavior
  json/jsonl renderers
  human renderer smoke tests
```

## Design Deferrals

The following are intentionally not designed in detail for v0:

- source catalog persistence
- archive/package/local/generated materialization
- selection-wide branch and merge
- file watching
- bare repository/worktree/mirror-cache storage
- remote capability enforcement
- persistent operation event logs

# Workspace Artifacts

`gwz-core` v0.3.0 stores durable workspace metadata under `gwz.conf/` in the
workspace root repository.

## Paths

| Path | Schema | Meaning |
| --- | --- | --- |
| `gwz.conf/gwz.yml` | `gwz.workspace/v0` | Manifest: workspace id and configured members. |
| `gwz.conf/gwz.lock.yml` | `gwz.lock/v0` | Resolved member state for materialization. |
| `gwz.conf/snapshots/<snapshot-id>.yaml` | `gwz.snapshot/v0` | Named captured member state. |
| `gwz.conf/.tmp/` | local only | Reserved temporary area excluded from the root Git repository. |
| `.git/info/exclude` | local only | Workspace boundary excludes for member repos and `gwz.conf/.tmp/`. |

There is no live `gwz.conf/tags` path in v0.3.0. Older design history may
mention tag artifacts; current `gwz tag` manages real Git refs.

## Manifest

The manifest records active members and their source metadata.

```yaml
schema: gwz.workspace/v0
workspace:
  id: ws_01
members:
  - id: mem_app
    path: repos/app
    type: git
    source_id: src_app
    active: true
    desired:
      branch: main
    remotes:
      - name: origin
        url: git@example.com:org/app.git
        fetch: true
        push: true
```

Member paths are workspace-relative, cannot escape the root, cannot enter
reserved `gwz.conf` paths, and cannot collide with each other by ancestor or
descendant relationship.

## Lock

The lock records resolved member state. It is the source for
`materialize --lock` and for member listing materialization flags.

```yaml
schema: gwz.lock/v0
workspace_id: ws_01
manifest_schema: gwz.workspace/v0
created_at: 2026-06-15T00:00:00Z
members:
  mem_app:
    path: repos/app
    source_id: src_app
    source_kind: git
    commit: abc123
    branch: main
    detached: false
    upstream: origin/main
    dirty: false
    materialized: true
```

`capture`, `commit`, selected materialize targets, pull/head, and clone flows can
rewrite the lock. The lock is written from observed post-mutation state where
the operation changes a worktree.

## Snapshots

Snapshots are named records under `gwz.conf/snapshots/`. A snapshot stores the
selected member ids and a member-state map.

```yaml
schema: gwz.snapshot/v0
workspace_id: ws_01
snapshot_id: pre-release
created_at: 2026-06-15T00:00:00Z
created_by:
  actor_id: agent://local/session
selected_members:
  - mem_app
members:
  mem_app:
    path: repos/app
    source_kind: git
    commit: abc123
    branch: main
    detached: false
    materialized: true
```

Duplicate snapshot ids are rejected. Listing snapshots treats a missing
snapshot directory as empty.

## Atomic Writes

`artifact::write_atomic` writes a unique temp file next to the target, fsyncs
the temp file, renames it into place, and best-effort fsyncs the containing
directory. `write_manifest_and_lock` stages both files first, then publishes the
manifest and publishes the lock last. True cross-file atomicity is not possible
on a normal POSIX filesystem; publishing the lock last avoids a lock that
references members missing from the manifest.

## Git Tags

`gwz tag` creates, lists, deletes, fetches, and pushes real Git tags in member
repositories and, for local operations, the workspace root repository. It does
not write a GWZ tag artifact.

# Workspace Artifacts

`gwz-core` v0.3.0 stores durable workspace metadata under `gwz.conf/` in the
workspace root repository. Local runtime state lives under `.gwz/` and is not
portable workspace intent.

## Paths

| Path | Schema | Meaning |
| --- | --- | --- |
| `gwz.conf/gwz.yml` | `gwz.workspace/v0` | Manifest: workspace id and configured members. |
| `gwz.conf/gwz.lock.yml` | `gwz.lock/v0` | Resolved member state for materialization. |
| `gwz.conf/snapshots/<snapshot-id>.yaml` | `gwz.snapshot/v0` | Named captured member state. |
| `gwz.conf/.tmp/` | local only | Reserved temporary area excluded from the root Git repository. |
| `.gwz/stash/bundles/<stash-id>.yaml` | `gwz.stash-bundle/v0` | Local coordinated stash bundle registry metadata. |
| `.gwz/locks/workspace-mutator.lock` | local only | Workspace-wide advisory lock used by branch and stash mutations. |
| `.gwz/local-family.yml` | `gwz.local-family/v1`, local only | Local clone family index; exists only at the family root. |
| `.gwz/local-family.lock` | local only | Advisory family lock for local create, dispose, disband and family exchanges; exists only at the family root. |
| `.gwz/family-root` | `gwz.family-root/v1`, local only | Clone pointer to the registering root (`family_id` plus root path). |
| `.gwz/local-clone-allocation` | local only | Ordinary allocation-id marker written for a clone destination. |
| `.git/info/exclude` | local only | Workspace boundary excludes for member repos, `gwz.conf/.tmp/`, and `.gwz/`. |

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

`capture`, `commit`, selected materialize targets, pull/head, branch switch, and
clone flows can rewrite the lock. The lock is written from observed
post-mutation state where the operation changes a worktree. `repo sync`
refreshes manifest metadata only; it does not rewrite the lock.

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

## Stash Bundles

Coordinated stash metadata is stored locally under `.gwz/stash/bundles/`.
Bundle files are YAML records named by `stash_id`, for example
`.gwz/stash/bundles/stash_2026_06_25T10_00_00Z.yaml`.

The registry records the selected member ids, per-member path, branch/head
before the push, dirty summary, native stash object id, display ref, push
lifecycle, restore state, warning, and drift metadata. Native Git stash payloads
remain in each selected member repository; the registry only groups and tracks
them. If `.gwz/` is removed, bundle grouping is lost, but native GWZ-prefixed
stash entries can still appear as orphans during stash listing.

The workspace root repository is not a stash participant. `gwz stash` applies
only to selected workspace members and the `.gwz/` registry is excluded from
root Git status.

## Runtime Locks

Branch and stash mutations acquire an advisory exclusive lock at
`.gwz/locks/workspace-mutator.lock` before mutating native Git state or stash
registry files. The lock file may remain after a process exits; an unlocked file
is not stale. If a process dies while holding the lock, the operating system
releases the file lock with that process. Concurrent mutators on network
filesystems with unreliable advisory locking are unsupported.

`gwz merge` additionally activates the checked merge artifact catalog under
`.gwz/catalog-final` while holding that lock, on a volume that can host it. The
catalog needs persistent file handles and a durable filesystem identity. Where
the volume cannot prove them the merge still runs: GWZ warns once and proceeds
without the catalog, and `--filesystem-strict` turns that warning back into a
refusal at the start — see
[Checked Merge Artifacts And Filesystem Identity](OperationModel.md#checked-merge-artifacts-and-filesystem-identity).
No other workspace mutation requires them.

## Local Clone Family Files

A local family is the original workspace (`root`) plus its named local
clones. The family index lives only at `root`; every clone stores a pointer
and an allocation marker. A workspace never holds both an index and a
pointer. These are local runtime files under `.gwz/`: they are never
copied into a clone, never enter `gwz.conf/`, and never record a family
member as a Git remote. Format 1 was frozen 2026-09-05 for the LCM1.0c
checkpoint; the field names are constants of the `gwz-family-model` crate
and `gwz-family-store` is the only writer.

```yaml
# .gwz/local-family.yml (root only)
schema: gwz.local-family/v1
family_id: fam_01
root:
  allocation_id: alloc_root
members:
  A:
    path: ../gwz-dev-A        # root-relative
    kind: checkout            # checkout | bare
    state: ready              # creating | ready | disposing
    allocation_id: alloc_a
    source_path: .            # root-relative path of the source member
    mode: verbatim            # verbatim | clean | bare
    last_error: null          # optional diagnostic for an incomplete row
```

```yaml
# .gwz/family-root (every clone)
schema: gwz.family-root/v1
family_id: fam_01
root_path: /Users/me/limbo/gwz-dev
```

```text
# .gwz/local-clone-allocation (every clone; one line)
alloc_a
```

The encoded index is limited to 1 MiB; an oversize or malformed file refuses
mutation and is retained for inspection. The store rereads and validates
the index under `.gwz/local-family.lock` (an ordinary OS advisory try-lock
released with its handle) and publishes it by same-directory temporary
write and rename. This is best-effort metadata publication, not a
power-loss-safe multi-file transaction, and reads never create the lock
file. `gwz local list` is observation-only.

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

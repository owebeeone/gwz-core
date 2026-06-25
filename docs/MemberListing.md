# Member Listing

`LsRequest` is the core operation behind `gwz ls` and the member discovery path
used by `gwz forall`.

## Request

```text
LsRequest {
  meta: RequestMeta,
  include_unmaterialized: Option<bool>
}
```

Selection rides in `meta.selection`. `include_unmaterialized` defaults to
`false`.

## Behavior

`handle_ls` is read-only and does not call Git. It reads:

- `gwz.conf/gwz.yml` for configured active members and selected member ids;
- `gwz.conf/gwz.lock.yml` when present to decide whether each member is
  materialized.

If the lock is absent, all members are treated as unmaterialized. Explicit
selection still validates against the manifest, but an explicitly selected
unmaterialized member is omitted unless `include_unmaterialized` is true.

## Response

`LsResponse` contains the standard response envelope plus `members:
Option<Vec<MemberEntry>>`.

`MemberEntry` fields:

| Field | Meaning |
| --- | --- |
| `id` | Manifest member id, for example `mem_app`. |
| `path` | Workspace-relative member path. |
| `abspath` | Absolute path on the current host. |
| `materialized` | True when the lock marks the member materialized. |

The CLI can render `path`, `abspath`, or both, but the core protocol always
returns both path forms.

## forall Reuse

`gwz forall` is CLI-local execution. It reuses member listing to choose
materialized members, then packages those `MemberEntry` values in `ExecRequest`.
The core protocol defines `Exec*` messages so machine output has a typed shape,
but `gwz-core` never executes child processes and has no service method for
`ExecRequest`.

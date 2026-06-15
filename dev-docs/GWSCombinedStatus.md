# GWS Combined Status

`gws status` returns a workspace-level status view that is equivalent in spirit
to running `git status` across every selected member and merging the result into
one stable view.

## Goals

- A user MUST get one combined status across all active repos by default.
- File paths in the combined view MUST be workspace-relative and MUST include
  the member path prefix.
- Branch state MUST be consolidated across selected repos.
- Branch differences between repos MUST be explicit in the response.
- A user MUST be able to request the per-member summary view when needed.
- The mode MUST be available through taut protocol messages and the CLI.

## CLI Contract

Default combined mode:

```text
gws status
```

Useful variants:

```text
gws status --combined
gws status --json
gws status --jsonl
gws status --porcelain
gws status --no-files
gws status --no-branches
gws status --no-combined
```

Rules:

- `gws status` MUST request workspace-level Git status data by default.
- `--combined` MAY be accepted as an explicit spelling of the default.
- `--no-combined` MUST request the per-member summary status mode.
- `--porcelain` MUST imply `--combined`.
- `--no-files` MUST suppress file-change records.
- `--no-branches` MUST suppress branch consolidation records.
- `--combined` and `--no-combined` MUST NOT both be supplied.
- `--porcelain` and `--no-combined` MUST NOT both be supplied.
- `--no-files` and `--no-branches` MUST only apply to combined mode.
- `--no-files` and `--no-branches` MUST NOT both be supplied.
- Status-specific flags MUST be rejected for non-status commands.

## Combined Human Output

Human output SHOULD use this shape:

```text
workspace status: dirty

branches:
  main: repos/core, repos/ui
  feature/app: repos/app

branch differences:
  repos/app is on feature/app; majority is main

changes:
   M repos/core/src/lib.rs
  A  repos/ui/src/new.rs
  ?? repos/app/notes.md
```

Ordering MUST be deterministic:

1. Branch groups sorted by branch label.
2. Members within branch groups sorted by member path.
3. File changes sorted by workspace-relative path.

## Protocol Contract

`StatusRequest` SHOULD carry:

- `mode`: summary or combined.
- `include_file_changes`: whether file-change records are requested.
- `include_branch_summary`: whether branch consolidation is requested.
- `path_style`: member-relative or workspace-relative path reporting.

`StatusResponse` SHOULD carry an optional workspace-level Git status payload in
addition to the existing per-member response envelope.

The workspace payload MUST include:

- `clean`: true only when every selected member is clean.
- `file_changes`: combined file records, if requested.
- `branches`: one branch state per selected member, if requested.
- `branch_groups`: consolidated member groups by branch label, if requested.
- `branch_differences`: explicit differences when selected repos are not on the
  same branch label or detached/unborn state.

## File Records

Each file record MUST include:

- `member_id`
- `member_path`
- `repo_path`, relative to the member repository
- `workspace_path`, relative to the GWS workspace root
- `index_status`
- `worktree_status`
- optional `original_repo_path` for rename/copy-style status

Status codes SHOULD preserve Git porcelain v1 meaning where practical:

- ` `: unmodified
- `M`: modified
- `A`: added
- `D`: deleted
- `R`: renamed
- `C`: copied
- `U`: unmerged
- `?`: untracked
- `!`: ignored

## Branch Consolidation

Each selected member MUST produce one branch state:

- branch name when on a branch
- detached marker and head commit when detached
- unborn marker when the branch has no commits
- upstream/ahead/behind when known

Branch labels are normalized as:

- branch name for normal branch heads
- `detached@<short-head>` for detached heads
- `unborn:<branch>` for unborn branches

If more than one label is present, `branch_differences` MUST list the members in
each non-majority group and the majority label when one exists. If there is no
majority, the response MUST still list every branch group.

## Deferred Implementation Notes

- The v0 Git backend currently returns status counts, not per-file records.
- Implementing combined status requires extending the backend status surface to
  expose file-level status entries.
- Ahead/behind counts may remain absent until upstream tracking support is
  added.

# Tag Management

`gwz-core` v0.3.0 treats `gwz tag` as multi-repository Git tag management. Tags
are real Git refs named `refs/tags/<name>`.

There is no GWZ tag artifact and no live `gwz.conf/tags` directory.

## Request

`TagRequest` fields:

| Field | Meaning |
| --- | --- |
| `meta` | Request metadata, including selection and optional policy remote. |
| `op` | `TagOp`: create, list, fetch, push, or delete. |
| `name` | Tag name for create/delete/push-one. Omitted for list, fetch-all, or push-all. |
| `message` | Annotation message. Creates an annotated tag when present. |
| `signed` | Create a signed tag. Requires `message`. |
| `remote` | Remote override for fetch, push, list-remote, or remote delete. |
| `all` | Schema support for all-tags operations. Current push-all behavior is name omitted. |

## Operation Scope

Local operations span the selected members plus the committed workspace root:

- create;
- local list;
- local delete.

Remote operations span selected members only because the workspace root is
local-only for tag fan-out:

- fetch;
- push;
- list with `remote`;
- delete with `remote`.

Selection is resolved through the manifest and lock. Selected members must have
lock records.

## TagOp

| Op | Behavior |
| --- | --- |
| `create` | Create the tag in root and selected member repos that have a commit and do not already have the tag. Existing tags are left unchanged. |
| `list` | Count local tags across root plus selected members, or remote tag refs across selected members when `remote` is set. |
| `fetch` | Fetch all remote tags into each selected member from `remote` or `origin`. |
| `push` | Push one named tag when `name` is set; push every local tag when `name` is absent. |
| `delete` | Delete local tag refs, or push delete refspecs to a remote when `remote` is set. |

`TagResponse.tags` is populated for list operations. Each `TagInfo` reports a
bare tag name and the number of repositories carrying or advertising it.

## Annotated And Signed Tags

`message` makes `tag_create` use an annotated tag. `signed=true` uses a signed
tag and requires `message`; the handler rejects signed tag creation without a
message before fanning out to repositories.

## materialize --tag

`MaterializeTargetKind::Tag` means: for each member that has
`refs/tags/<name>`, materialize that member to the commit the tag points at.
Members without the tag are skipped by default. If no selected/materialized
member carries the tag, the operation returns `tag_not_found`.

For an explicit selection, selected members must be present in the tag-derived
target set; selecting a member that lacks the tag is an error.

## Errors

Common errors:

- `invalid_request`: missing required tag name, signed tag without a message, or
  invalid selection.
- `lock_not_found`: selected member has no lock record.
- `tag_not_found`: materialization target tag is absent from all members.
- `tag_invalid`: Git rejected tag create/delete/fetch/push in a tag-specific
  path.
- `missing_remote` or `remote_rejected`: remote tag operations cannot reach or
  update the configured remote.

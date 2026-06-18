# gwz-core

`gwz-core` is the embeddable Rust control plane for a GWZ workspace: a local
workspace made from many repositories, driven by typed requests instead of
ad-hoc shell glue.

It gives tools, agents, and UIs one stable place to ask:

- What repositories are in this workspace?
- What Git state are they in?
- Can this workspace be materialized to a lock, snapshot, tag, head, or commit?
- What changed, what failed, and which member caused it?

The point is not to replace Git. The point is to make multi-repo workspace
operations deterministic, inspectable, and scriptable without forcing callers
to know the artifact layout or reimplement cross-repo policy.

## Why Embed It

- **Typed protocol:** callers build generated request structs and receive typed
  responses, member results, errors, and operation metadata.
- **Workspace artifacts:** manifest, lock, snapshots, and GWZ tags are read and
  written through one library boundary.
- **Git backend boundary:** Git behavior is isolated behind `GitBackend`; the
  default backend supports local, SSH, and HTTPS Git repositories.
- **Agent-friendly surface:** every operation can carry request metadata,
  selection, dry-run policy, attribution, and per-member status.
- **CLI-ready, UI-ready:** `gwz-cli` is the thin command driver; richer tools can
  call the same request handlers directly.

For command workflows and examples, use `gwz-cli` as the definitive how-to. This
crate is the reusable engine behind that command.

## Small Shape

```rust
use std::path::Path;

use gwz_core::git::Git2Backend;
use gwz_core::workspace_ops::handle_create_repo;
use gwz_core::{CreateRepoRequest, RequestMeta, WorkspaceRef};

fn create_member_repo() -> gwz_core::model::ModelResult<()> {
    let backend = Git2Backend::new();
    let request = CreateRepoRequest {
        meta: RequestMeta {
            request_id: "req-1".to_owned(),
            schema_version: "gwz.protocol/v0".to_owned(),
            workspace: Some(WorkspaceRef {
                root: Some("/work/my-ws".to_owned()),
                ..WorkspaceRef::default()
            }),
            ..RequestMeta::default()
        },
        member_path: "tools/new-lib".to_owned(),
        ..CreateRepoRequest::default()
    };

    let response = handle_create_repo(&backend, Path::new("/work/my-ws"), request, "op-1")?;
    for member in response.response.members {
        eprintln!("{}: {:?}", member.member_path, member.status);
    }
    Ok(())
}
```

## Documentation

- [docs/Reference.md](docs/Reference.md) describes the request/response model,
  request types, and direct library usage.
- `protocol/gwz.taut.py` is the protocol contract used to generate Rust types.
- `dev-docs/` contains design history and implementation plans.

## Development

```text
cargo fmt
cargo test
cargo fmt --check
```

## Release

`gwz-core` is a Rust library crate. It does not publish binary installer assets.
Create a GitHub Release from a version tag to run the release verification
workflow, then point binary crates such as `gwz` at that Git revision or tag.

When `protocol/gwz.taut.py` changes, regenerate through the taut workflow and
run `cargo test`; do not hand-edit generated protocol output.

## License

`gwz-core` is licensed under GPL-2.0-only, the same license family used by Git.

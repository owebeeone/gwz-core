# gwz-core

GWZ (Git Workspace Zone) coordinates multiple ordinary Git repositories as one
reproducible, inspectable workspace. `gwz-core` is the embeddable Rust engine
that owns most of that workspace behavior behind typed Taut service messages.

The core was designed from scratch to run in-process or behind a separate
client boundary. A local adapter can call it directly; a remote adapter can
carry the same typed operations using deterministic CBOR or schema-driven JSON.
This crate is transport-neutral and does not itself start a server or daemon.

Use the primary [`gwz` CLI](https://github.com/owebeeone/gwz-cli) for terminal
workflows and follow the hosted
[Quick Start](https://owebeeone.github.io/gwz-cli/QuickStart/). Embed this crate
when a tool, UI, agent, bridge, or service needs workspace operations without
reimplementing cross-repository policy. Read [Why GWZ](docs/WhyGwz.md) for the
product model and comparison with lighter repository fan-out tools.

## Why Embed It

- **Typed message boundary:** generated requests and responses carry member
  results, structured errors, operation metadata, and streaming events.
- **Workspace artifacts:** the manifest, lock, snapshots, and markers are read
  and written through one policy boundary; tags remain real Git refs.
- **Git backend boundary:** Git behavior is isolated behind `GitBackend`; the
  default backend supports local, SSH, and HTTPS repositories.
- **Local or remote composition:** clients can share the same operation model
  whether the core is linked into the process or hosted elsewhere.
- **CLI-ready and UI-ready:** `gwz-cli` is a command driver over these requests;
  richer clients can use the same handlers directly.

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

- [Core documentation index](docs/README.md)
- [Why GWZ](docs/WhyGwz.md)
- [Embedding](docs/Embedding.md)
- [Protocol and transports](docs/Protocol.md)
- [Practical reference](docs/Reference.md)
- [Generated message catalog](docs/MessageCatalog.md)

The message catalog is generated from `protocol/gwz.taut.py`; do not hand-edit
generated protocol output.

## Development

```sh
cargo fmt
cargo test
cargo fmt --check
```

## Release

`gwz-core` is a Rust library crate and does not publish binary installers.
Create a GitHub Release from a version tag to run release verification, then pin
binary or binding repositories to that revision.

## License

`gwz-core` is licensed under GPL-2.0-only, the same license family used by Git.

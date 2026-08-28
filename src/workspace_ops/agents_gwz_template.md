# GWZ Workspace

This repository is managed by GWZ, a multi-repository workspace tool.

For workspace-wide status, staging, and commits, use `gwz status`, `gwz add`,
and `gwz commit`. Do not substitute per-repository Git loops.

## Workspace Integrity
- **Never manually edit `gwz.conf/gwz.yml` or `gwz.lock.yml`.**
- These files represent the authoritative, system-managed state of the workspace.
- To add, remove, or modify member repositories, use the appropriate `gwz` commands (e.g., `gwz repo attach`, `gwz repo clone`, `gwz repo detach`).
- **Never manually edit this file.** It is system-managed. Change agent instructions via the `gwz` template and `gwz init --update`.

Install `gwz` from the latest release:

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/owebeeone/gwz-cli/releases/latest/download/gwz-installer.sh | sh
```

Or install from source:

```sh
cargo install --git https://github.com/owebeeone/gwz-cli
```

If the workspace is not cloned yet:

```sh
gwz clone <workspace-git-url> [directory]
```

If this root repository is already cloned:

```sh
gwz materialize --lock
gwz status
```

Docs:

- `gwz --help`
- Quick Start: https://owebeeone.github.io/gwz-cli/QuickStart/
- Full documentation: https://owebeeone.github.io/gwz-cli/

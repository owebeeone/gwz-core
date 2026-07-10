# Why GWZ

Git is excellent at managing one repository. A product assembled from many
repositories creates a different problem: developers must know which
repositories belong together, where each one should be checked out, which
revisions form a working set, and how to inspect or change that set without
losing track of partial failures.

GWZ turns that collection into a reproducible workspace while leaving every
member as an ordinary Git repository.

## The Problem GWZ Solves

Multi-repository development commonly relies on setup notes, shell scripts,
and local convention. Over time those approaches drift:

- two developers can have different repositories or revisions while believing
  they have the same workspace;
- a command that must run across several repositories has inconsistent
  selection, ordering, output, and failure behavior;
- changes to workspace composition are not reviewed with the source changes
  that depend on them;
- automation must rediscover repository layout and reimplement Git policy;
- a failure halfway through a cross-repository operation can be difficult to
  understand or recover from.

GWZ gives the workspace itself a small, tracked root repository. Its
`gwz.conf/` metadata records the member designations and the exact composition
needed to reproduce the workspace. Commands then operate through that shared
model instead of through unrelated loops of shell commands.

## What GWZ Adds

### Reproducible composition

The manifest describes which repositories are members and where they live.
The lock records the current resolved composition. Snapshots and Git tags
provide named points that can be inspected or materialized later.

### One view across repositories

`status`, `diff`, `add`, `commit`, `pull`, `push`, branch, stash, snapshot, and
materialization workflows use the same member selection and report results per
repository. A caller can see what succeeded, what failed, and which member was
responsible.

### Explicit repository membership

GWZ distinguishes creating a repository, cloning a new member, registering an
existing checkout, temporarily detaching a designation, and attaching that
historical designation again. Repository identity and workspace membership do
not have to be inferred from directory names or remote URLs.

### A stable engine for more than one interface

`gwz-core` implements the workspace model, Git operations, typed requests,
responses, errors, dry-run policy, and operation events. The primary Rust
`gwz` CLI and the Python bindings and `gwz-py` CLI exercise the same core
semantics, while other tools and agents can embed the library directly.

## What GWZ Does Not Replace

GWZ is not a new version-control system, a source host, a package manager, or a
build system. Member repositories remain normal Git repositories: their
branches, commits, remotes, credentials, and hosting workflows continue to
work with standard Git tools.

GWZ coordinates those repositories as one development workspace. It is useful
when the workspace composition and cross-repository workflow need to be
repeatable, inspectable, and usable by both humans and automation.

## When GWZ Fits

GWZ is a good fit when:

- a product or platform is intentionally split across several repositories;
- contributors need a one-command way to obtain a known working checkout;
- changes regularly span repository boundaries;
- CI, developer tools, UIs, or agents need typed workspace operations rather
  than project-specific shell orchestration;
- repository membership changes should retain history and be reviewable.

For a single repository with no coordinated external members, ordinary Git is
usually enough.

## Where To Start

- To use GWZ from the terminal, follow the
  [GWZ Quick Start](https://github.com/owebeeone/gwz-cli/blob/main/docs/QuickStart.md).
- To understand the command surface, use the
  [GWZ CLI documentation](https://github.com/owebeeone/gwz-cli/tree/main/docs).
- To embed the engine, continue with [Embedding](Embedding.md) and
  [OperationModel](OperationModel.md).


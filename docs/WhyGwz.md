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

## How GWZ Differs From Repository Fan-out Tools

GWZ is in the same broad problem space as tools such as `vcstool`, Google's
`repo`, West, and Git submodules, but it makes a different architectural
choice.

For example, [`vcstool`](https://github.com/dirk-thomas/vcstool) deliberately
keeps no state beyond the working copies it discovers on disk. It recursively
finds repositories, invokes their native VCS clients, and can import or export
a YAML description of repository paths, URLs, and versions. That is a useful,
lightweight model when the main requirement is to obtain a set of repositories
or run a VCS command across them.

GWZ treats the workspace as a durable, versioned object:

- a root Git repository owns the reviewed manifest, lock, and snapshots;
- member ids and source ids provide stable identity beyond a directory scan;
- membership has a lifecycle, including explicit detach and attach, rather
  than being inferred only from which repositories are currently on disk;
- operations share typed selection, policy, dry-run, attribution, event,
  error, and per-member result contracts;
- workspace composition changes can be committed alongside the changes that
  depend on them;
- the execution engine is an embeddable message service, not only a command
  that fans native VCS arguments out to local directories.

The choice is not “GWZ does everything and the other tools do nothing.” If a
portable repository list and batch VCS invocation are the whole requirement, a
smaller fan-out tool may be the better fit. GWZ is aimed at workspaces whose
composition, coordinated operations, automation contract, and recoverability
are part of the product's development state.

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

### A message-driven engine for local, embedded, and remote clients

GWZ was designed from scratch so the execution engine is not fused to a
particular CLI or even to the client process. `gwz-core` contains the bulk of
the workspace behavior: artifact handling, Git operations, selection, policy,
locking, typed errors, dry-run planning, per-member results, and operation
events.

The public operation surface is described as a Taut `GwzCore` service. Clients
send named request messages and receive named response messages rather than
asking core to parse command-line arguments. The Rust `gwz` CLI, the Python
bindings and `gwz-py` CLI, agents, UIs, and services can therefore use the same
operation model.

That message boundary also allows the client and `gwz-core` to run separately.
An adapter can host core beside the workspace and expose it to a client in
another process or on another machine. Messages have deterministic CBOR
encoding and a schema-driven JSON representation, so a client can drive the
service with JSON without linking Rust or reproducing the workspace logic.
Long-running operations expose event messages and later result lookup instead
of requiring a terminal session to remain the control plane.

`gwz-core` intentionally does not mandate an HTTP, RPC, queue, or daemon
implementation. It supplies the service and message contracts; an embedding
application chooses the transport, authentication, authorization, and process
topology. “Remote-capable” is therefore an architectural property, not a claim
that the core crate itself starts a network server.

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
- the client should run separately from the machine or service that owns the
  workspace checkout;
- a non-Rust client needs a schema-driven JSON or CBOR contract instead of a
  shell-command protocol;
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

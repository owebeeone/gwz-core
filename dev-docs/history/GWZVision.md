# GWZ Vision

GWZ is the local-first source and workspace substrate for Glial, Grazel,
Gryth, and Glade.

It exists because the system needs both of these capabilities:

1. A Glade-participating AI agent MUST be able to create, branch, edit, build,
   and install code into a live application without waiting for a central Git
   host or repository ceremony.
2. A developer or agent MUST be able to materialize a coherent local workspace
   from many independently governed sources without using Git submodules or
   forcing everything into a monorepo.

GWZ is therefore not just a submodule replacement. It is a local-first source
catalog plus a materialized workspace manager.

## Core Thesis

The source of truth for a working application is not one repository. It is a
live workspace assembled from many source authorities, build outputs, and
runtime-delivered modules.

GWZ SHOULD make that workspace explicit, observable, reproducible, and
AI-readable.

The long-term loop is:

```text
agent creates or selects source
agent writes code
GWZ observes source and file state
Grazel builds and packages it
Glade installs it into the live application fabric
Gryth projects the source/build/app state to the user
agent or human decides whether to publish outward
```

## Why Not Monorepo

A monorepo makes local access easy, but it makes sharing harder. Code that is
useful outside the tree still needs packaging, versioning, ownership, release
policy, and dependency boundaries. In a monorepo those boundaries are easy to
defer, so useful experiments tend to become coupled to the tree before they are
ready to graduate.

A monorepo also ingests inertia. Once the path of least resistance is "put it
in the existing tree", new ideas inherit the existing build, review, dependency,
and release habits. For edge innovation, it is often better to create a new repo
immediately, move fast, and fix packaging or publication once the idea proves
worth sharing.

GWZ MUST allow new code to start outside any central repository structure.

## Why Not Submodules

Git submodules encode federation in Git internals. They are sharp for branch
work, local edits, nested status, onboarding, and agent workflows.

GWZ MUST NOT rely on submodules as its workspace model.

## Why Not Registry-Only

Package registries are the right destination for stable artifacts, but they are
not enough for early development. Agents and humans need to co-edit raw source,
change producers and consumers together, and install intermediate builds into a
live app before publishing a versioned package.

GWZ SHOULD support registry and archive artifacts, but it MUST treat raw source
materialization as a first-class workflow.

## Two Planes

GWZ has two separate planes.

### Source Authority Plane

The source authority plane answers:

```text
Where can this source be born?
Who owns it?
Where can it publish?
Which remotes mirror it?
What trust domain does it belong to?
```

Examples:

- a newly created local Git repository with no remote yet
- a local mirror of a GitHub repository
- a Forgejo or Gitea repository
- an archive source such as a `.tar.gz` or `.zip`
- a package registry artifact
- generated source owned by a build or app-generation process

The identity of a source MUST NOT be its GitHub URL. A source may be local and
unpublished. A remote URL is publication metadata.

### Workspace Materialization Plane

The workspace materialization plane answers:

```text
Which sources are present here?
At which paths?
At which revisions or content digests?
Which roles do they play?
Which build adapter owns each member?
What is their current live state?
```

This plane is the submodule replacement. It materializes selected source
members into one local workspace without making the parent Git repository own
their commits.

## First-Class Objects

### Source

A source is an origin of code or content.

Required fields SHOULD include:

- stable source id
- source kind: `git`, `archive`, `package`, `local`, or `generated`
- owner or declared authority
- trust domain
- publication state
- remotes, if any

### Workspace

A workspace is a named local projection over selected sources.

Required fields SHOULD include:

- workspace id
- workspace root
- members
- policy
- lock file reference

### Member

A member is a source materialized into a workspace.

Required fields SHOULD include:

- member id
- source id
- local path
- role
- build adapter
- desired revision, version, or digest
- writable flag

### Lock

A lock records the resolved state of a workspace.

For Git members, the lock SHOULD record:

- commit
- branch or detached state
- remotes
- upstream tracking state
- dirty flag at pin time

For archive and package members, the lock SHOULD record:

- resolved URL or package coordinate
- digest
- extraction or materialization metadata

### Live State

Live state is the current observed workspace condition.

It SHOULD include:

- file change generation
- per-member status
- branch and commit information
- dirty counts
- untracked counts
- build daemon status
- build graph health
- last scan time
- errors and degraded states

Live state MUST be streamable to Grazel, Gryth, Glade, and AI agents.

## Agent Workflow

The target agent workflow is:

```text
gwz source create gryth-weather-panel --kind git
gwz workspace add gryth-weather-panel --path repos/gryth-weather-panel
agent writes code
gwz status/watch emits changes
grazel builds the module
glade installs the module into a live Gryth application
agent observes behavior through the app layer
agent iterates
gwz publish gryth-weather-panel --to github:org/gryth-weather-panel
```

The local create path MUST succeed without a remote.

Publication SHOULD be a later explicit step.

## CLI Shape

The CLI is not the architecture, but the first CLI SHOULD make the model
obvious:

```text
gwz source create NAME
gwz source fork SOURCE
gwz source publish SOURCE --to PROVIDER
gwz init git@github.com:org/repo-a.git git@github.com:org/repo-b.git
gwz workspace add SOURCE --path PATH
gwz sync
gwz status
gwz watch --jsonl
gwz pin
```

## Relationship To Grazel

GWZ owns workspace source and live state.

Grazel owns build analysis, execution, and package production.

Grazel MAY consume GWZ as a library for:

- workspace discovery
- file invalidation
- Git status
- build adapter selection
- daemon/workarea status projection

GWZ MUST NOT become the build system.

## Relationship To Gryth

Gryth consumes GWZ state as a UI projection.

The Gryth workspace UI SHOULD show:

- source catalog entries
- materialized workspace members
- Git status
- build state
- live app install state
- publish state
- errors and required actions

Gryth SHOULD NOT need to shell out to Git or inspect random filesystem paths
directly when GWZ can provide the same facts.

## Relationship To Glade

Glade distributes state and app/module installation records.

GWZ SHOULD provide canonical workspace facts that can be carried over Glade.
Glade SHOULD carry the provenance of live application code back to the source
member, commit, build artifact, and installing principal.

## Security Posture

GWZ v0 may be local and permissive, but the model MUST preserve security seams.

The following actions SHOULD be capability-gated when exposed to agents or
remote participants:

- source creation
- branch creation
- file writes
- build execution
- app/module installation
- remote publication
- credential use
- changing trust policy

Remote publication MUST be separate from local source creation.

## Non-Goals

GWZ is not:

- a hosted GitHub replacement
- a package registry
- a build system
- a code review system
- a general application deployment system

GWZ MAY integrate with Forgejo, Gitea, GitHub, package registries, and archive
stores, but those integrations are adapters.

## Initial Implementation Direction

Start with a Rust library and a thin CLI.

Suggested crate split:

```text
grazel-workspace-core
  manifest model
  lock model
  source catalog model
  materialization API
  live state API
  Git adapter
  archive adapter

grazel-workspace
  gwz CLI
```

The first implementation SHOULD be read-mostly and snapshot-first:

- create local Git sources
- add existing Git sources
- materialize members
- scan status
- watch file changes
- emit JSONL events
- pin lock state

It SHOULD avoid remote forge automation until the local model is stable.

## Requirements Baseline

`GWZRequirements.md` is the authority for the completed v0 requirements.

The settled v0 baseline is:

- `workspace.gwz.yaml` is the manifest filename.
- `workspace.gwz.lock.yaml` is the lock filename.
- `.gwz/` is the internal state directory.
- Git is the required v0 source kind.
- Ordinary non-bare Git repositories are the v0 storage backend.
- A Rust-native Git backend using gitoxide/gix is preferred.
- A separate source catalog is deferred.
- Archive, package, local, and generated source materialization are deferred.
- File watching, branch selection, merge selection, alternate Git storage, and
  remote capability enforcement are deferred.

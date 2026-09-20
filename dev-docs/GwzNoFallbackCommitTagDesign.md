# Commit/tag characterization and bounded design input

Status: L3-A characterization package, State review.  This document records
the physical-backend observations required by the accepted
`GwzNoFallbackCheckpoint.md` package.  It proposes a later private design; it
does not change the backend, protocol, options, or operation policy.

## Package boundary

The only files owned by this package are:

* `src/git/gitbackend/commit_tag_characterization.rs` (292 test lines).
* `dev-docs/GwzNoFallbackCommitTagDesign.md` (this document).

The accepted ceilings are 0 production additions/moves, 500 test lines, 0
tool lines, 250 document lines, and 2 files.  The existing test-only module
declaration in `src/git/gitbackend.rs:413-417` is integrator wiring and is
read-only here.  No production implementation, manifest, shared contract,
Git state, or commit was changed.

The tests use `Git2Backend::create_repo`, repository-local config, and child
Git processes spawned by the existing backend.  Each case self-reexecutes in
a clean test process with only `PATH` and a test marker retained, clearing
`GIT_EDITOR`, `VISUAL`, `EDITOR`, `GIT_CONFIG_*`, `GPG_*`, and other ambient
variables before the backend spawns Git.  The repository pins
`core.hooksPath`, `core.editor=/usr/bin/false`, and `gpg.format=openpgp` before
seeding.  Hook and mock-signer fixtures live below `.git` in a temporary
repository.  They do not mutate process-wide environment, global config, a
user keyring, or a personal key.

## Existing contract and routes

The preserved backend signatures in `src/git/gitbackend/contract.rs` are:

```rust
fn commit(&self, path: &Path, message: &str, all: bool)
    -> ModelResult<GitCommitResult>;
fn tag_create(
    &self, path: &Path, name: &str, message: Option<&str>, signed: bool,
) -> ModelResult<GitTagResult>;
fn tag_delete(&self, path: &Path, name: &str) -> ModelResult<()>;
```

The current physical routes are still subprocess routes:

| API | Current implementation and exact invocation | Fresh verification |
|---|---|---|
| `commit` | `src/git/gitbackend/repository.rs:291-332`; `git -C <path> commit`, optional `-a`, then `-m <message>` | Reads fresh HEAD and requires it to advance; returns `GitCommitResult { commit }`. |
| `tag_create` | `src/git/gitbackend/refs.rs:164-217`; `git -C <path> tag`, `-s` when `signed`, otherwise `-a` when `message` exists, optional `-m`, then name | Requires the name in `tag_list`; resolves `refs/tags/<name>^{commit}` and returns `GitTagResult { name, commit }`. |
| `tag_delete` | `src/git/gitbackend/refs.rs:232-263`; `git -C <path> tag -d <name>` | Requires the name to be absent from `tag_list`. |

Failure from a nonzero child status is `ErrorCode::GitCommandFailed` with the
child stderr.  The operation callers retain their existing idempotence and
remote planning decisions; this package does not widen them.

## Observed characterization

The five focused tests in the owned module establish these facts on the
physical backend:

* `commit(..., false)` commits the index tree while leaving a later worktree
  edit unstaged. `commit(..., true)` behaves like `git commit -a`: it includes
  modified tracked content and leaves an untracked file out of the new tree.
* `prepare-commit-msg` can replace the message supplied by `-m`; the committed
  bytes are the hook-written message. A failing `pre-commit` returns an error,
  leaves HEAD at the prior commit, and leaves the staged change available.
* A nonzero `post-commit` hook does not roll back the commit and the backend
  observes success with an advanced HEAD. This is distinct from a rejecting
  pre-commit hook.
* With signing disabled, `message=None` creates a lightweight tag pointing
  directly at HEAD, while a message creates an annotated tag object. The
  `reference-transaction` hook observes both create transactions and the
  delete transaction; deletion removes the tag and the postcondition check
  sees it absent.
* With repository-local `tag.gpgSign=true`, a no-message `git tag` does not
  silently bypass signing as a lightweight tag: Git infers an annotated tag
  and, in the hermetic fixture, fails at the required editor before invoking
  the signer. A message then reaches the configured temporary signer; its
  deliberate failure returns an error and publishes no tag. This is an
  observed Git/config interaction, not a proposed policy.

The characterization intentionally does not call a real signer or inspect a
personal key. It records invocation and failure only. It also keeps the
existing default behavior for `commit.gpgSign`, `tag.gpgSign`, identities,
hooks, editor, and other Git configuration visible for the later replacement.

## Required rows still omitted

These rows remain design/test work for a later package and must not be inferred
from the five tests:

* author/committer identity precedence across repository, environment, and
  explicit operation inputs; timestamp and timezone precedence;
* successful OpenPGP, SSH, and x509 signing/verification, signer payload and
  format obligations, and configured signer exit/error mapping;
* `commit.gpgSign` success/failure, explicit `signed=true` tag behavior with a
  real compatible signer, and annotated-tag payload bytes;
* full reference-transaction phases/payloads, reflog text, interruption and
  concurrent ref races, and Windows hook execution;
* unborn HEAD, merge-parent commits, empty commits, SHA-256 repositories,
  invalid names, missing deletes, and the workspace operation fan-out;
* preservation of configuration defaults for merge, stash, fetch, and other
  operations, which must not acquire commit/tag hooks accidentally;
* the no-Git build/runtime gate and any native binding or C implementation.

## Later private design shape

When the omitted rows are complete, a replacement should remain behind the
existing three methods and one private physical-backend owner.  The owner can
separate concerns internally without adding a public protocol:

1. Resolve repository-local and ambient Git configuration, identity, editor,
   hooks, signing mode, and message semantics before constructing objects.
2. Build the commit tree from the current index, with an explicit tracked-only
   update for `all=true`; preserve hook ordering and the observed failure
   boundaries before publishing the branch ref.
3. Construct lightweight or annotated tag objects according to Git's effective
   configuration.  Signed tags need an explicit native/binding decision for
   signer invocation, exact payload format, and error mapping; a no-message
   request must not be made into an unconditional lightweight bypass.
4. Publish refs with the required compare-and-swap/reflog/reference-hook
   behavior, then perform the same fresh postconditions as the current routes.

This is an internal decomposition only.  It does not authorize a new option,
new tag policy, signer format, or hook scope.  Existing subprocess routes stay
in place until the omitted evidence, design review, and a separately bounded
replacement package pass their gates.

## Reproduction and results

From `gwz-core`, the focused command was:

```text
rustup run 1.95.0 cargo test --locked --lib \
  gitbackend::commit_tag_characterization -- --nocapture
```

Result: `5 passed; 0 failed` (outer harness; each clean child also passed).
The owned file also passes:

```text
rustfmt --check src/git/gitbackend/commit_tag_characterization.rs
```

The workspace-wide formatter check was not used as package acceptance because
other lanes had concurrent unowned edits; only the owned file was checked.

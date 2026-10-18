# Commit/tag characterization and bounded design input

Accepted **L3-A characterization and design input only**, reviewed at core
`c63f497df29d51ad5d864738fa0056b513c3ab7d` after
[State GO](../../dev-docs/GwzNoFallbackCharacterization-ReviewState.md).
Replacement design/implementation and activation remain separate gates.

Status: L3-A characterization package, State review.  This document records
the physical-backend observations required by the accepted
`GwzNoFallbackCheckpoint.md` package.  It proposes a later private design; it
does not change the backend, protocol, options, or operation policy.

## Package boundary

The only files owned by this package are:

* `src/git/gitbackend/commit_tag_characterization.rs` (304 test lines).
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
  The post-failure index entry is resolved to its blob and checked byte-for-byte.
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
rustfmt +1.95.0 --check --edition 2024 src/git/gitbackend/commit_tag_characterization.rs
```

The workspace-wide formatter check was not used as package acceptance because
other lanes had concurrent unowned edits; only the owned file was checked.

Review follow-up: the post-rejection index assertion passed in a fresh
Rust 1.95 locked focused run (five tests and their clean children).

## C1 identity/date/message evidence — 2026-09-21

Status: C1 characterization accepted after [State GO](../../dev-docs/GwzGitLibraryEvidence-ReviewState.md) at core `deba48c93a04e6aaf0bab36066b12d1547d5469e`, root `9fc664de8389ea334f36bc41135cd59448893e05`, under
[NextPackages](GwzGitLibraryNextPackages.md). Owned new test child
`src/git/gitbackend/commit_tag_identity_characterization.rs`: 394 test lines,
plus one integrator declaration inside the existing braced test cfg boundary.
No production behavior, library API, dependency or lock changes.

Seven exact tests re-execute in hermetic children: cleared environment, fixture
HOME/USERPROFILE, explicit fixture global config, disabled system config,
empty hooks directory, disabled commit/tag signing and a failing editor.
Windows fixtures select unique `D:/gwz-tests/` roots; only macOS arm64 ran here.
The parent verifies the selected test actually passed exactly once. Temporary
configuration and repositories are removed by TempDir ownership.

| Row | Observed Git 2.52.0 result |
|---|---|
| C-ID | Repository user identity overrides fixture global identity; author and committer environment overrides act independently. Annotated tagger follows committer, not author, identity. No local identity falls back to fixture global. |
| C-DATE | Author `2001-02-03T04:05:06 +0530` stores seconds `981153306`, offset `330`; committer `2002-03-04T05:06:07 -0700` stores `1015243567`, offset `-420`. Annotated tagger uses the committer date; a lightweight tag resolves directly to a commit object. |
| C-MSG | Input `\n  body  \n# comment\nnext  \n\n`: default stores `  body\n# comment\nnext\n`; strip stores `  body\nnext\n`; verbatim retains the input exactly. Ordinary nonempty `-m` calls need no editor. |
| C-EMPTY | Empty/whitespace messages return `GitCommandFailed`; HEAD/branch ref, HEAD reflog, staged path/mode/OID/blob and tracked worktree bytes remain unchanged. Raw index bytes change on the first rejection; COMMIT_EDITMSG is empty after both sequential attempts. No blanket repository rollback is promised. |

The first run passed five cases and failed two draft assumptions. libgit2's
parsed message accessor skipped the leading blank line, so C-MSG now reads the
raw ODB payload after the header separator; verbatim bytes were present all
along. Empty-message failure changed raw index bytes, so C-EMPTY asserts
logical index entries/blob contents separately and positively observes the raw
index change on the first rejection. These are fixture/evidence corrections,
not product fixes or proof that a failed commit leaves no side effects.

From core, Rust 1.95 locked command:

```sh
cargo +1.95.0 test --locked --lib characterization -- --nocapture
```

Passed **17 tests**: seven C1, five existing L3-A, two H1 and three existing
L4-A, plus their exact child executions; zero failures/ignored. Owned-source
rustfmt and changed-range whitespace checks pass. Existing history fixture
changes justified including all characterization tests in this focused run.
No no-Git replacement, successful signer, hook expansion, filter parity,
SHA-256 mutation, concurrent refs, interruption or cross-platform execution
is claimed. Those remain requirements before a mutation API/implementation freeze.

State P3-1 disposition: describe only measured first-rejection raw-byte change,
not a parsed extension change. Empty and whitespace cases run sequentially;
the second begins with already-modified administrative files. Exact extension
identity and fresh-repository per-variant administrative transitions remain
pending before mutation parity is frozen. Both cases do assert protected
refs/reflog, logical index entries/blob, and tracked worktree contents.

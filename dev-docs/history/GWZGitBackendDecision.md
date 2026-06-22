# GWZ Git Backend Decision

Status: accepted for v0

## Decision

GWZ Core v0 uses `git2` behind the internal `GitBackend` trait, with SSH and
HTTPS transport support enabled.

The backend boundary remains mandatory. Public APIs MUST NOT expose `git2`
types, and operation code MUST depend on the `GitBackend` trait rather than the
selected backend implementation.

## Spike Result

The v0 capability spike evaluated gix first because the design prefers a
Rust-native gitoxide/gix backend when it can satisfy the required behavior
cleanly.

The gix package compiled locally and exposes repository initialization,
discovery, clone, status, fetch, and lower-level checkout/push primitives.
However, the v0 push and arbitrary checkout path requires composing several
feature-gated lower-level APIs, and the default feature set pulls in a large
dependency surface. That is acceptable for a future backend, but too much risk
for this checkpoint.

The v0 `git2` implementation passed local, networkless fixture tests for:

- repository detection
- ordinary non-bare repository creation
- clone from a local repository
- clone target preflight rejection before mutation
- remote read and add
- clean, untracked, unstaged, and staged status reporting
- push to a temporary bare repository
- fetch from a temporary bare repository
- fast-forward update from a fetched remote-tracking ref
- checkout of a specific commit into detached HEAD state

GWZ configures explicit remote callbacks for clone, fetch, and push.
`Git2Backend::new()` supports SSH-agent credentials, username-only SSH prompts,
default platform credentials, and configured Git credential helpers for HTTPS
username/password credentials. `Git2Backend::without_credential_helpers()`
disables credential helper execution for sandboxed callers.

## Consequences

GWZ Core can proceed with the v0 workspace operations without shelling out to
`git` as the primary implementation.

Future gix work MAY add another implementation behind the same `GitBackend`
trait. That work SHOULD be driven by a focused backend parity test suite rather
than by operation-level rewrites.

V0 Git object identity plumbing is represented in the protocol and model, but
no v0 backend operation creates commits, merges, rebases, or annotated tags.
Those future object-creating operations MUST accept explicit Git object
identity from operation attribution instead of relying only on ambient Git
configuration.

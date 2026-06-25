# Git Backend

`GitBackend` is the boundary between GWZ policy and Git mechanics. Core
handlers decide which members to operate on, how to preflight, how to write
artifacts, and how to interpret partial results. The backend performs concrete
Git operations.

## Trait Responsibilities

The trait covers:

- repository creation, clone, fetch, push, and ls-remote;
- fast-forward, merge, rebase, reset, commit checkout, and branch checkout;
- status, HEAD, remotes, ref reads, and ancestry checks;
- staging, committing, tag create/list/delete/fetch;
- optional transfer progress during clone/fetch-like work.

Handlers rely on backend self-verification. Checkout/update primitives re-open
repositories and verify HEAD/worktree state before reporting success. Commit and
tag primitives verify that Git advanced or created the expected object.

## Git2Backend

`Git2Backend::new()` uses libgit2 with configured credential helpers enabled.
`Git2Backend::without_credential_helpers()` disables credential-helper lookup,
which is useful for tests or hosts that want credential prompts to fail fast.

`git::set_server_timeout_ms(ms)` sets libgit2's process-wide SSH/network server
timeout. Call it once at process startup before network operations or worker
threads begin. A positive value prevents stalled SSH handshakes from hanging
forever; `0` disables the timeout.

## Credentials

Credential behavior is intentionally bounded:

- SSH uses `ssh-agent` and offers the agent identity once per connection.
- If the agent identity is rejected, GWZ returns a clear authentication error
  rather than retrying indefinitely.
- Username credentials are supplied when libgit2 asks for username-only auth.
- HTTPS username/password helpers are used only when credential helpers are
  allowed.
- Default credentials are offered if libgit2 allows them.

No protocol field carries secret material. `OperationAttribution.credential_ref`
is only a driver-local handle.

## Transfer Progress

Backends that support transfer progress emit `GitTransferProgress` values. The
current Git2 implementation maps libgit2 counters into:

- `receiving` while objects are being received;
- `resolving` after received objects reach the total and deltas are resolving.

`OperationPolicy.progress_min_interval_ms` rate-limits per-member progress
events at the `EventEmitter` boundary. The first update for a member always
emits.

## Concurrency

Pull, push, and materialize flows can use `OperationPolicy.concurrency` for a
global member-job limit and `max_connections_per_host` for remote host caps.
Members whose remote host cannot be parsed are bounded only by the global
limit.

## Tag Primitives

Tags are real Git refs:

- `tag_create` calls porcelain `git tag` so hooks, signing, tagger config, and
  local Git behavior are honored.
- `tag_list` returns sorted local tag names.
- `tag_delete` calls porcelain `git tag -d` and verifies removal.
- `tag_fetch` fetches `+refs/tags/*:refs/tags/*`.
- push uses concrete `refs/tags/<name>:refs/tags/<name>` refspecs.

Annotated tags are created when a message is provided. Signed tags require a
message.

## CLI Fallback Rules

Some primitives intentionally call the `git` CLI instead of using libgit2:

- `commit` uses `git commit` to honor hooks, signing, and committer config.
- `tag_create` and `tag_delete` use `git tag` for the same reason.

These are backend implementation choices. Callers still interact through
`GitBackend`.

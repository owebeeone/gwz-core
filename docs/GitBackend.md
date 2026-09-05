# Git Backend

`GitBackend` is the boundary between GWZ policy and Git mechanics. Core
handlers decide which members to operate on, how to preflight, how to write
artifacts, and how to interpret partial results. The backend performs concrete
Git operations.

## Trait Responsibilities

The trait covers:

- repository creation, clone, fetch, push, and ls-remote;
- fast-forward, merge, rebase, reset, commit checkout, branch checkout, branch
  list/create/delete/switch, and stash push/list/apply/pop/drop;
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

## Anonymous Local Transport

`fetch_anonymous(path, url, refspecs)` and `push_anonymous(path, url,
refspec)` are the local clone family's transfer ports (LCM1.0c, 2026-09-05;
gwz-dev `dev-docs/GwzLocalCloneDesign.md` §6.2). Every behavior below is
observed by `cargo test -p gwz-core --lib local_clone::tests::transport`.
They differ from `fetch` and `push` in every way that matters for a family
exchange:

- `url` is an existing local repository path. A URL scheme, scp-like
  syntax or a missing directory refuses with `invalid_request` before any
  effect. The admitted directory is canonicalised and handed to libgit2 as
  a `file://` URL (LCM1.0c-rem1, Code P2-1): libgit2 matches its transport
  table by prefix before its heuristics, and the heuristic for a bare
  string on macOS/Linux selects SSH for any `:` in the string before it
  tests for a directory, so a bare path under a `:`-containing directory
  left the local transport and failed as a host lookup. With the `file://`
  form the local transport is the only one that can run, whatever the path
  contains (`anonymous_ports_stay_local_for_a_peer_path_containing_a_colon`);
  results and error messages name the path the caller passed.
- Refspecs are explicit and required; nothing is inferred from a remote's
  configuration because no remote is configured. The peer is an anonymous
  in-memory remote and nothing is persisted in `.git/config`.
- No credential, ssh-agent or progress callbacks are attached, and tags are
  not followed. No fetch record is written: `update_fetchhead(false)` is
  requested and honoured for the record. Measured (LCM1.0c-rem1, State P2-1
  / Code P3-4): libgit2 1.9.7 nevertheless truncates the receiver's
  `FETCH_HEAD` to empty on every fetch and creates it when absent
  (`remote.c`, `git_remote_update_tips` -> `truncate_fetch_head`, not gated
  by the flag), so a prior fetch record in the receiver does not survive a
  family import. Both arms are asserted by `local_clone::tests::transport`;
  the file is outside the port contract and nothing in gwz reads it.
- A rejected ref update is `remote_rejected`, never a silent success:
  libgit2 refuses a non-fast-forward update without `+` before the transfer
  (`NotFastForward`, mapped here), and any per-ref rejection the receiving
  side reports through the push status callback is collected and mapped the
  same way; `push` ignores that callback today.
- libgit2's local transport refuses every push into a **non-bare**
  repository ("local push doesn't (yet) support pushing to non-bare repos";
  observed by `local_clone::tests::transport`, reported as
  `git_command_failed` with the receiver unchanged). A family push through
  this port therefore reaches a bare hub only; publishing into a checkout
  member is the receiver-side `fetch_anonymous` form of the same transfer,
  and the checked-out-branch protection of design §6.1 belongs to the
  family push wrapper, not to the port.

The family import wrapper fetches into
`refs/gwz/local-imports/<transfer-id>`, a namespace separate from
`refs/gwz/merge/...`, and retains those refs.

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

## Branch Primitives

Branch commands use real local Git branches in selected member repositories.

- `branch_list` reports local branches and marks the current branch.
- `branch_create` resolves the requested start ref per repository. An existing
  branch at the same commit is a no-op; an existing branch at another commit is
  rejected as divergence.
- `branch_delete` refuses to delete the current branch.
- `switch_branch` attaches an existing local branch without creating or moving
  the branch ref. When the target is at current `HEAD`, it preserves the index,
  worktree, and untracked files without checking out the tree; dirty switches
  that move commits are rejected. Branch-switch workspace preflight also calls
  `repository_state` for clean-looking repositories, so custom backends that
  implement branch switching must implement that observation method.
- `checkout_branch` remains the materialize-restore primitive that can create a
  branch at a saved commit when that is safe; command branch switching uses
  `switch_branch` instead.

## Stash Primitives

Stash commands use native Git stash payloads in each selected member repository
plus GWZ registry metadata under `.gwz/stash/bundles/`.

- `stash_push` supports tracked-only, include-untracked, and include-ignored
  modes. Include-ignored also includes untracked files.
- `stash_list` returns native stash entries with stable object ids and display
  indices.
- `stash_apply`, `stash_pop`, and `stash_drop` resolve the current native stash
  index immediately before mutation by object id first and GWZ message prefix
  second. `stash@{n}` is display text only because indices move after stash
  mutations.
- Restore defaults preserve index state. Native restore conflicts are reported
  as `stash_conflict`; missing native payloads are reported as
  `stash_incomplete`.

## CLI Fallback Rules

Some primitives intentionally call the `git` CLI instead of using libgit2:

- `commit` uses `git commit` to honor hooks, signing, and committer config.
- `tag_create` and `tag_delete` use `git tag` for the same reason.

These are backend implementation choices. Callers still interact through
`GitBackend`.

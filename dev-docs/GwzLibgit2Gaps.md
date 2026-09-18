# GWZ libgit2 gaps: where gwz-core runs the git command line, and how to stop

Status: **todo, not started** (2026-09-19). Owner: Gianni. This document
records every place gwz-core product code runs the `git` executable, why each
one exists, and what it would take to remove it. No decision is taken here on
which route to use for any case.

Facts read on gwz-core `main` at 1.0.17: `git2` 0.21.0 over `libgit2-sys`
0.18.8 (libgit2 1.9.7, pinned exactly in `Cargo.toml`).

## 1. Goal

gwz-core performs every Git operation through libgit2, with no `git`
executable on `PATH`.

Why it matters:

- `git` on `PATH` is today a hard runtime requirement for `gwz commit`,
  `gwz tag` and path-filtered `gwz log`, and a soft one for `gwz merge
  --remote <lane>`. Nothing checks for it at start-up; the first use fails
  with `GitCommandFailed`.
- The spawned `git` reads the user's whole configuration and runs with the
  user's environment, so its behaviour is not pinned by the gwz release. The
  libgit2 path is.
- Every spawn is a process per repository per operation. `gwz log -- <path>`
  spawns one process per emitted commit per repository (section 2.4).
- An embedded host (gwz-py's native extension, a future service) inherits the
  requirement without knowing it.

Non-goal: removing `git` from the tests. Test fixtures may keep using it, and
parity tests against native `git rev-list` are the evidence for case 2.4.

## 2. The four cases

Found with `grep -rn 'Command::new("git")'` over `src/` and `crates/`,
excluding test modules. Three further hits in `src/` are inside `#[cfg(test)]`
modules (`local_clone/adapters/object_census.rs`, `diff/output.rs`,
`workspace_ops/historical_identity.rs`) and are out of scope.

### 2.1 Lane import falls back to `git fetch` (conditional)

- **Where.** `src/git/gitbackend/transport.rs`, `fetch_anonymous` and
  `fetch_anonymous_with_git`. Reached by `gwz merge --remote <lane>` and every
  other family import.
- **What runs.** libgit2's anonymous local fetch first. Only when it fails with
  exactly `object is not a committish` does the same fetch rerun as
  `git -C <receiver> fetch --no-write-fetch-head --no-tags <peer> <refspecs>`.
- **Why.** A libgit2 defect. The code comment records it as: the file
  transport walks every ref in the receiver while it builds the pack and
  treats each as a commit, so a valid ref that points directly at a tree fails
  the whole fetch. Which repository's refs trigger it (receiver, peer or
  either) has not been pinned by a test; S1.2 needs that answer. Core Git
  accepts such refs. This workspace has them: Codex writes
  `refs/codex/turn-diffs/checkpoints/...` refs that point at trees, so without
  the fallback no lane of gwz-dev could be merged.
- **Match on error text.** The trigger is a string comparison on libgit2's
  message. A libgit2 upgrade that rewords it silently turns the fallback off
  and the failure back on.

Routes to remove it:

- **A. Build the pack ourselves.** Both ends are local repositories gwz has
  open. Walk the wanted commits with a revwalk that hides what the receiver
  already has, feed a `PackBuilder`, write the pack into the receiver's object
  database, then set the import refs. No transport, so no ref walk over
  unrelated refs. This also removes the `FETCH_HEAD` truncation the current
  path documents as outside its promise.
- **B. Patch libgit2.** Fix the local transport to peel or skip non-commit
  refs, carry the patch in a vendored `libgit2-sys`, and send it upstream. The
  exact `libgit2-sys` pin already exists, so a fork is mechanically possible,
  but every libgit2 upgrade then carries a rebase.
- **C. Hide the offending refs from the transport.** No way to do this with
  stock libgit2 is known; unverified. Listed so it is checked once, not
  rediscovered.

### 2.2 Every `gwz commit` runs `git commit` (always)

- **Where.** `src/git/gitbackend/repository.rs`, `commit`. Decision AD1
  ("per-primitive CLI fallback, self-verifying").
- **What runs.** `git -C <repo> commit [-a] -m <message>`, then a fresh read
  that HEAD advanced.
- **Why.** libgit2's commit honours none of: the hooks (`pre-commit`,
  `prepare-commit-msg`, `commit-msg`, `post-commit`), commit signing
  (`commit.gpgsign`, `gpg.format` openpgp, ssh and x509, `user.signingkey`,
  `gpg.program`, `gpg.ssh.program`), or the full identity resolution
  (`GIT_AUTHOR_*`, `GIT_COMMITTER_*`, `user.useConfigOnly`, `author.*`,
  `committer.*`). A member repository's own policy must behave under
  `gwz commit` as it does under `git commit`.

What removal means: reimplementing those three behaviours over libgit2.

- Hooks: locate `core.hooksPath`, run each hook executable with Git's
  environment and arguments, honour exit codes and the message file rewrite.
  This still spawns processes, but the hooks themselves, not `git`.
- Signing: `Repository::commit_create_buffer` then `commit_signed`, with the
  signature produced by running the configured signer. Still a spawn, of `gpg`
  or `ssh-keygen`, not of `git`.
- Identity: a resolver matching Git's precedence, tested against `git var
  GIT_AUTHOR_IDENT` in fixtures.
- `-a`: stage tracked modifications and deletions with the index API, honouring
  the same filters `git add -u` applies.

### 2.3 Every tag creation runs `git tag` (always)

- **Where.** `src/git/gitbackend/refs.rs`, `tag_create`. Decision AD1.
- **What runs.** `git -C <repo> tag [-s | -a] [-m <message>] <name>`, then a
  fresh read that the tag exists.
- **Why.** The same three gaps as 2.2: signing (`-s`, `tag.gpgSign`), tagger
  identity, and hooks. Lightweight and unsigned annotated tags need none of
  them except identity.

Removal follows 2.2's signing and identity work. A lightweight tag could move
to libgit2 today.

### 2.4 `gwz log -- <pathspec>` runs `git rev-list` per step (always, when paths are given)

- **Where.** `src/operation/commit_log/mod.rs`, the `PathWalk` state. Plain
  `gwz log` with no pathspec walks with libgit2 and spawns nothing.
- **What runs.** `git --git-dir <repo> rev-list --max-count=1 --skip=<n>
  [--first-parent] <pushes> ^<hides> -- <pathspecs>`, once per emitted commit,
  with `GIT_OPTIONAL_LOCKS=0` and `GIT_NO_LAZY_FETCH=1` so it stays a pure read.
- **Why.** Requirement L-RNG (gwz-cli `dev-docs/history/GwzLogRequirements.md`):
  pathspec routing, including the long-form `:(...)` magic and the short `:!`
  and `:^` exclusions, must match native `git rev-list`'s complete commit
  sequence. libgit2 has no path-limited revision walk with Git's history
  simplification, and its pathspec matcher does not implement Git's magic.
- **Cost today.** One process per commit per repository, each re-walking from
  the start with a growing `--skip`, so a long path-filtered log is quadratic.

Routes to remove it:

- **A. Implement the walk.** A libgit2 revwalk plus per-commit tree diffs
  against each parent limited to the pathspec, with Git's default history
  simplification (TREESAME pruning and parent rewriting) and a pathspec-magic
  parser of our own. The existing native-`rev-list` parity fixtures are the
  acceptance test.
- **B. Keep `git`, fix the cost.** One long-lived `git rev-list` per
  repository read as a stream, instead of one process per commit. This does
  not meet the goal, but it removes the quadratic behaviour and is small.

## 3. What is already libgit2 and therefore already inconsistent with 2.2

Merge commits (`merge_prepared.rs`, `merge_recovery.rs`), the configuration
gate's commit (`workspace_bootstrap/conf_gate.rs`) and stash entries
(`stash.rs`, `preservation.rs`) are created through libgit2. They run no hooks
and are never signed, whatever the repository's configuration says. A
repository with `commit.gpgsign=true` therefore gets signed `gwz commit`
commits and unsigned `gwz merge` commits. Doing 2.2 properly gives one code
path both can use; until then this asymmetry should at least be documented for
users.

## 4. Outside gwz-core, for completeness

gwz-cli's Claude Code fallback hook (`gwz-cli/src/hook/fallback.rs`) runs
`git worktree add` and friends. That is deliberate: in a non-GWZ repository
the hook's job is to do exactly what Claude Code's own git worktree would have
done. It is not a libgit2 gap and is not part of this todo.

Credential helpers are not a case: they run through libgit2's own callbacks
under `CredentialHelperPolicy`, not by spawning `git credential`.

## 5. Todo, phased

Phases are ordered so the cheapest real improvements land first and each is
shippable alone. Steps inside a phase are independent unless stated.

### Phase 1: stop depending on an error string, and make the requirement visible

- **S1.1.** Probe for `git` once per process where a spawn is about to happen
  and refuse with a typed error naming the verb and the reason, instead of
  `GitCommandFailed: failed to run git ...`.
- **S1.2.** Replace the `error.message() == "object is not a committish"` test
  in 2.1 with a check made before the fetch: does the repository whose refs
  libgit2 walks (first establish which, with a test on each side) hold a ref
  whose target is not a commit or a tag that peels to one? Then the choice of
  path no longer depends on libgit2's wording.
- **S1.3.** Document the `git` requirement and the section 3 asymmetry in
  gwz-cli `docs/Install.md` and `docs/commands/merge.md`.

### Phase 2: remove the conditional fallback (case 2.1)

- **S2.1.** Decide route A or B from section 2.1. A is recommended: it removes
  the transport, the fallback and the `FETCH_HEAD` side effect together, and
  needs no fork.
- **S2.2.** Implement it behind the existing `fetch_anonymous` port so the
  family merge callers do not change.
- **S2.3.** Tests: a receiver and a peer each holding a ref to a tree, a ref to a
  blob and an annotated tag of a tree imports cleanly with `git` absent from
  `PATH`; the existing import and preservation tests pass unchanged.

### Phase 3: path-filtered log (case 2.4)

- **S3.1.** Route B first, as an independent small change: one streaming
  `git rev-list` per repository.
- **S3.2.** Pathspec-magic parser with table tests against Git's documented
  forms.
- **S3.3.** Path-limited walk with history simplification over libgit2.
- **S3.4.** Switch `PathWalk` to it once the native parity fixtures pass, and
  keep those fixtures as the regression suite.

### Phase 4: commit and tag (cases 2.2, 2.3)

- **S4.1.** Identity resolver with fixtures checked against `git var`.
- **S4.2.** Signing through `commit_create_buffer` and `commit_signed` for
  openpgp, ssh and x509, spawning the configured signer.
- **S4.3.** Hook runner for the four commit hooks, honouring `core.hooksPath`.
- **S4.4.** `commit -a` staging through the index API. Depends on nothing.
- **S4.5.** Move `commit` and `tag_create` onto S4.1 to S4.4; lightweight tags
  can move as soon as S4.1 lands.
- **S4.6.** Route merge commits, the configuration gate's commit and stash
  signatures through the same path, closing section 3.

Phase 4 is the largest and the least urgent: it replaces one spawn per
commit with a body of code that must track Git's behaviour. It should start
only if embedding without `git` becomes a real requirement.

## 6. Open questions

- Is "no `git` on `PATH`" a product requirement, or is "no dependence on
  libgit2's error text and no quadratic log" enough? Phases 1 to 3 deliver the
  second without Phase 4.
- For 2.1, is a carried libgit2 patch acceptable at all, given the exact
  `libgit2-sys` pin and the release train's crates.io publication?
- Should hooks run for merge commits once section 3 is closed? Git runs
  `pre-merge-commit` and `commit-msg` there, not `pre-commit`.

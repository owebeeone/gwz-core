# GWZ Audit Resolution Plan Review 55

Status: review of `dev-docs/GwzAuditResolutionPlan.md`
Date: 2026-06-17
Reviewer: GPT-5.5

Verdict: revise before implementation.

The plan is strong on the discovered mutation-order class, but it still treats
the current libgit2/plumbing implementation and `.gitignore` member-boundary
model as mostly settled. That is the wrong default after this incident. The
resolution plan should add two blocking architecture decisions before the
god-file split and before any behavioral remediation:

1. GWZ code must not compose low-level Git ref/index/worktree mutations with
   libgit2 or by touching `.git` internals. Mutating Git operations should be
   delegated to Git's porcelain behavior, or to a backend primitive proven to
   provide equivalent porcelain semantics as an indivisible operation.
2. The root/member boundary must be redesigned. `.gitignore` is a workaround for
   hiding nested repositories from the root, not a durable workspace model.
   Submodule/gitlink semantics should be evaluated as a first-class boundary
   option, even if GWZ still does not delegate workspace orchestration to
   `git submodule`.

## Findings

### P0: Plan does not make "no Git internals" a blocking architecture gate

Reference: `dev-docs/GwzAuditResolutionPlan.md:39-43`,
`dev-docs/GwzAuditResolutionPlan.md:109-120`,
`dev-docs/GWZGitBackendDecision.md:1-57`

The plan proposes a phased operation helper around the current backend shape,
but the confirmed bug was caused by hand-composing lower-level Git operations:
checkout tree, move ref, set head, then infer that the result is equivalent to a
safe Git porcelain command. Reordering one sequence is not enough. It still
leaves GWZ in the business of guessing porcelain semantics from plumbing pieces.

This should become a blocking policy decision:

- No direct `.git` file mutation.
- No libgit2 ref/index/worktree mutation sequencing in operation code.
- No direct `git2::Repository` use outside the Git backend, except tests.
- For v0, mutating Git operations should use the `git` CLI porcelain behind
  `GitBackend`, unless a backend operation is proven as a single porcelain-grade
  primitive by contract tests.
- `GWZGitBackendDecision.md` should be marked superseded or amended before this
  remediation proceeds.

The current plan's WS3 refactor can improve ordering, but it does not remove the
root cause: GWZ is still composing Git internals and hoping the composition
matches user expectations.

Required plan change: add `WS0A - Git backend architecture decision` before the
module split. It should explicitly choose the mutating backend strategy and list
forbidden APIs, including `checkout_tree`, `set_target`, `set_head`,
`set_head_detached`, raw index mutation, and direct `.git` path mutation.

### P1: The planned sequencing splits files before deciding architecture

Reference: `dev-docs/GwzAuditResolutionPlan.md:177-200`

The plan says to degodify `workspace_ops/mod.rs` and `gwz-cli/src/main.rs` before
WS1 policy decisions. That is only safe if the current architectural seams are
stable. They are not.

The backend decision can change `GitBackend` signatures, error surfaces,
progress events, and test fixture strategy. The root/member boundary decision can
remove or radically change `.gitignore` sync helpers. Splitting first risks
preserving the wrong seams and making the real fix more expensive.

Recommended order:

1. Freeze or commit the existing fast-forward fix if desired.
2. Decide and document Git backend architecture: porcelain-only mutating
   operations, allowed read-only APIs, forbidden direct internals.
3. Decide root/member boundary: `.gitignore`, `.git/info/exclude`, gitlink /
   submodule, or a documented hybrid.
4. Add contract tests for the decisions.
5. Then split files along the new architecture seams.
6. Then implement behavioral fixes.

Pure moves are valuable, but not before the seam definition is stable.

### P1: `.gitignore` is treated as the member-boundary model instead of a design question

Reference: `dev-docs/GwzAuditResolutionPlan.md:70`,
`dev-docs/GwzAuditResolutionPlan.md:145-151`,
`src/workspace_ops/mod.rs:2163-2241`

The plan frames the `.gitignore` issue as "resync it after more operations."
That is too narrow. `.gitignore` is a leaky workaround:

- It mutates a user-visible, versioned file to express local materialization
  state.
- It can create its own dirty/staging failures.
- It is not an actual Git repository boundary.
- It can hide paths from root status but does not give regular Git a first-class
  model for member repositories.
- It duplicates membership data already present in GWZ artifacts.
- It creates semantic atomicity problems between manifest, lock, ignore file,
  and index staging.

The plan should stop treating F4 as "resync `.gitignore`" and replace it with a
root membership design decision.

Recommended options to evaluate:

| Option | Use case | Main tradeoff |
| --- | --- | --- |
| Committed `.gitignore` block | Simple current behavior | Dirties/version-controls generated path hiding; not a real boundary |
| `.git/info/exclude` | Local generated ignore state | Avoids user `.gitignore` churn but is local and must be regenerated |
| Gitlink/submodule boundary | Let regular Git know member roots are nested repos | Introduces submodule/gitlink semantics that GWZ must own and constrain |
| External member checkout root | Avoid nested repos inside root | Changes UX and path assumptions |

My view: committed `.gitignore` should not be the long-term model. If GWZ wants
regular Git to understand "do not recurse into these repos", gitlink/submodule
semantics are the only native boundary Git already understands. The trick is to
use that boundary deliberately, not to delegate GWZ workspace behavior to
`git submodule update`.

### P1: Submodules should be reopened as a boundary mechanism, not as the workspace model

Reference: `dev-docs/GWZVision.md:55-61`,
`dev-docs/GWZRequirements.md:411-413`,
`dev-docs/GwzAuditResolutionPlan.md:145-151`

The existing docs correctly say GWZ must not rely on Git submodules as its
workspace model. That does not automatically mean GWZ must avoid gitlinks as a
root repository boundary.

There are two different questions:

1. Should Git submodule porcelain orchestrate the GWZ workspace? I agree the
   answer should remain no.
2. Should the root repository record member paths as gitlinks/submodules so
   regular Git knows not to treat them as ordinary untracked directories? This
   is now worth a serious design pass.

Submodule/gitlink advantages:

- Root `git status` has a native concept of nested repository boundaries.
- Root `git add .` does not accidentally stage an entire member tree as regular
  files.
- Root commits can represent member commit pointers if GWZ chooses that mode.
- `.gitignore` no longer carries the burden of repository-boundary semantics.

Submodule/gitlink risks:

- Gitlink commits can duplicate or conflict with `gwz.lock.yml`.
- Regular `git submodule update`, recursive checkout, and recursive clean/fetch
  can mutate members outside GWZ policy.
- Branch work and dirty submodule state remain sharp unless GWZ surfaces them
  clearly.
- `.gitmodules` creates another artifact to keep consistent with GWZ manifest
  data.

This is still likely a better problem to solve than using `.gitignore` as the
boundary. The plan should add a design spike:

- Can GWZ use gitlinks as "do not reach into member" markers while keeping
  `gwz.yml`/`gwz.lock.yml` as the source of truth?
- Can GWZ refuse or repair states caused by `git submodule update --recursive`?
- Does GWZ track gitlink entries in root commits, or use submodule metadata only
  for root Git boundary behavior?
- How do root branch switches behave when member gitlinks change?
- What does `gwz status` report when the root gitlink and `gwz.lock.yml` differ?

### P1: Direct `git2` usage in `workspace_ops` violates the backend boundary

Reference: `src/workspace_ops/mod.rs:2229-2241`,
`dev-docs/GWZGitBackendDecision.md:10-13`

`stage_workspace_git_metadata` opens the root repository with
`git2::Repository::open`, mutates the index with `add_all`, stages `.gitignore`,
and writes the index. That is operation-layer Git mutation outside the
`GitBackend` trait. It also directly participates in the `.gitignore` failure
class.

The resolution plan does not call this out as its own finding. It should.

Required plan change:

- Add a finding for "workspace operation code directly mutates Git index through
  git2."
- Move root metadata staging behind the same Git backend policy as member
  operations.
- If the chosen strategy is Git CLI porcelain, staging should be a backend
  operation with tests around `git add workspace/`, `.gitignore` or
  `.git/info/exclude`, and failure behavior.

### P1: Q1 understates fetch as mutable Git state

Reference: `dev-docs/GwzAuditResolutionPlan.md:52`,
`dev-docs/GwzAuditResolutionPlan.md:112-113`

The plan recommends that the atomic guarantee does not cover remote-tracking
refs because they are derived and re-fetchable. That is a defensible policy only
if the command explicitly says fetch may be left behind after failure. For a
default `pull --head`, users generally experience a failed command as "nothing
changed." Advancing remote-tracking refs can affect later planning and status.

Recommended rewrite:

- Treat fetch as mutation for command-failure semantics.
- Use a non-mutating remote inspection step such as `git ls-remote` for planning
  when the command needs to know remote state before validation.
- Only update remote-tracking refs after all selected members pass local
  validation, or only under explicit `fetch-only`/partial policy.
- If fetch is allowed to persist after failure, report it as an explicit member
  outcome.

This also aligns with the "no Git internals" principle: use Git's command-level
semantics rather than assuming remote-tracking refs are harmless implementation
details.

### P2: Recovery metadata is introduced without schema/design alignment

Reference: `dev-docs/GwzAuditResolutionPlan.md:57`,
`dev-docs/GwzAuditResolutionPlan.md:130-133`,
`dev-docs/GWZDesign.md:219-220`

The plan proposes `.gwz/recovery/<operation_id>.yml`. That may be the right
answer, but `GWZDesign.md` currently says persistent operation event logs are
deferred. Recovery metadata is not just an implementation detail; it is a
persistent state model with lifecycle, cleanup, compatibility, and user-visible
repair behavior.

Required plan change:

- Add a schema and lifecycle decision for recovery records.
- Define when records are written, when they are cleared, and whether they are
  authoritative for repair.
- Clarify whether recovery records are local-only `.gwz/` state or versioned
  workspace artifacts.

### P2: The fake-backend test plan should not replace real Git contract tests

Reference: `dev-docs/GwzAuditResolutionPlan.md:99-107`,
`dev-docs/GwzAuditResolutionPlan.md:157-175`

Fake backend tests are useful for impossible or hard-to-trigger failure shapes,
but the incident happened because a plausible backend operation did not match
Git porcelain behavior. The plan needs a backend contract suite that runs
against disposable real repositories and the selected mutating backend.

Required contract tests:

- Add/delete/rename/nested-file fast-forward.
- Dirty target rejection.
- Interrupted checkout/failure behavior where feasible.
- `fetch-only` does not move local `HEAD` or worktree.
- Root/member boundary behavior under regular Git commands:
  `git status`, `git add .`, `git clean -fd`, `git checkout`, and `git pull`.

## Recommended Plan Edits

Replace the current top-level order with:

1. `WS0 - Preserve baseline`: commit or otherwise freeze the already-fixed
   incident regression so later work has a known base.
2. `WS1 - Git backend architecture`: forbid direct `.git` internals and
   low-level libgit2 mutation sequencing; choose Git porcelain or an equivalent
   backend strategy.
3. `WS2 - Root/member boundary`: decide `.gitignore` vs `.git/info/exclude` vs
   gitlink/submodule vs external checkout roots.
4. `WS3 - Contract tests`: write real Git backend and root-boundary tests.
5. `WS4 - Mechanical split`: split files after the stable seams are known.
6. `WS5 - Phased operations`: implement Validate / Mutate / Observe / Record
   using the selected backend strategy.
7. `WS6 - Structured partial/recovery`: add `Partial`, member-scoped errors, and
   recovery metadata only after its schema is specified.
8. `WS7 - CLI/status safety`: structured error output and dirty summaries.
9. `WS8 - Artifact durability`: atomic writes, duplicate snapshot guard, and
   workspace lock/manifest semantics.

## Submodules vs `.gitignore`: Position

I would reopen this design now.

`.gitignore` can keep the root visually clean, but it does not express the
thing GWZ actually needs: "this path is a separately governed Git repository;
regular root Git operations should not traverse it as ordinary content." Git has
one native concept for that boundary: a gitlink/submodule entry.

I would not make GWZ "use submodules" in the traditional sense. That would
inherit too much of the workflow that the GWZ vision explicitly rejects. But I
would evaluate a gitlink-backed member boundary where:

- `gwz.yml` and `gwz.lock.yml` remain the source of truth.
- GWZ never shells out to `git submodule update` as its workspace orchestrator.
- GWZ treats recursive submodule operations as external mutations to detect,
  report, and repair.
- Root Git sees a native boundary and does not mistake member trees for ordinary
  untracked files.
- The plan defines how root gitlink state relates to `gwz.lock.yml`: duplicate
  pointer, compatibility projection, or unsupported mismatch.

For an interim path, `.git/info/exclude` is a cleaner local workaround than
committed `.gitignore` because it avoids modifying a user-visible versioned file.
It still is not a boundary model; it is just less harmful than generated
committed ignores.

The plan should not spend effort hardening `.gitignore` until this choice is
made. That work may be thrown away.

## Bottom Line

The plan correctly identifies the mutation-order family. It should not proceed
as written because it fixes the symptoms inside the same unsafe assumptions:
libgit2/plumbing composition and `.gitignore` as a pseudo-boundary.

Make the backend and member-boundary decisions first. Then split files. Then
implement the phased operation model against the chosen architecture.


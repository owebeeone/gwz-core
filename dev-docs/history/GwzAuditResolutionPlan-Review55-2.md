# GWZ Audit Resolution Plan Review 55-2

Status: second review of `dev-docs/GwzAuditResolutionPlan.md` Revised R1
Date: 2026-06-17
Reviewer: GPT-5.5

Verdict: R1 is a substantial improvement, but revise before implementation.

The new §11 accepts the important architectural concerns: no operation-layer Git
internals, root/member boundary is a design decision, direct `git2` in
`workspace_ops` is a finding, fetch is mutation, recovery metadata needs a
schema, and real-Git contract tests are required. That is the right direction.

The remaining problems are mostly consistency and precision. §11 now supersedes
several earlier sections, but the earlier sections still read as active
instructions. That leaves two different plans in the same document.

## Findings

### P1: The old policy matrix and workstreams conflict with §11

References:

- `dev-docs/GwzAuditResolutionPlan.md:47-59`
- `dev-docs/GwzAuditResolutionPlan.md:111-139`
- `dev-docs/GwzAuditResolutionPlan.md:145-151`
- `dev-docs/GwzAuditResolutionPlan.md:291-304`

R1 correctly recasts key decisions in §11, but the old §2 and §4 entries still
say the opposite:

- Q1 still says remote-tracking refs are outside the atomic guarantee and that
  GWZ should run a fetch phase before Validate.
- WS3 still says `pull --head` should move `fetch` to a pre-Validate fetch phase.
- WS5 still says to define and write `.gwz/recovery/<op>.yml`.
- WS8 still says to resync `.gitignore` at the end of materialize/pull/clone.

§11 later supersedes those with `ls-remote` planning, no ad-hoc recovery record,
and AD2 boundary design. That is correct, but an implementer reading top-down can
still implement the old plan.

Required change: rewrite §§2–4 directly. Do not leave contradictory active text
and rely on §11 to override it. The plan should have one policy table, one
finding register, and one workstream sequence.

### P1: F18 is accepted but not integrated into the finding register or workstreams

References:

- `dev-docs/GwzAuditResolutionPlan.md:66-85`
- `dev-docs/GwzAuditResolutionPlan.md:282-289`
- `dev-docs/GwzAuditResolutionPlan.md:315-325`

§11 adds F18: `workspace_ops` mutates the root Git index via direct `git2`.
That is a real P1. But the consolidated finding register still ends at F17, and
the original workstreams do not include F18. Only the revised sequencing mentions
it.

Required change:

- Add F18 to the main finding register.
- Add it to the test matrix.
- Add a real workstream entry with test-first acceptance criteria.
- Decide whether F18 is part of AD1 implementation or its own prerequisite.

Suggested acceptance:

- No production module outside `src/git/*` imports or names `git2`.
- Root metadata staging is expressed as a `GitBackend` semantic operation.
- Contract tests cover staging workspace metadata for current metadata mode and
  for the AD2 interim/final boundary mode.

### P1: AD1 is directionally right, but "contract-proven porcelain-grade" is underspecified

Reference: `dev-docs/GwzAuditResolutionPlan.md:249-269`

R1 is right to push back on "porcelain-only CLI" as a foregone conclusion.
Subprocess Git has costs: progress parsing, Git-version dependency, concurrency
handling, and structured errors. Keeping libgit2 behind a strict backend
boundary can be acceptable.

The current AD1 wording still needs sharper contracts. "Contract-proven against
the equivalent porcelain command" is not enough unless the primitive contract
defines failure semantics, final-state checks, and what lower-level operations
are forbidden even inside the backend.

Contract tests cannot prove crash safety or all interleavings. They can prove
specific success/failure behaviors. The backend contract should therefore say:

- Operation code only calls semantic backend primitives, never ref/index steps.
- Each mutating primitive must document preconditions, mutations, final observed
  state, and failure state.
- The backend primitive itself must observe final `HEAD`/index/worktree state
  before returning success.
- Failure-injection tests are required where possible, not only happy-path
  comparisons with porcelain.
- If a libgit2 primitive cannot provide the contract without hand-sequencing raw
  ref/index/worktree steps, that primitive falls back to `git` CLI.

This keeps the R1 synthesis but makes it enforceable.

### P1: Bare gitlink boundary is a good spike, not yet a recommendation

Reference: `dev-docs/GwzAuditResolutionPlan.md:271-280`

AD2 is much better than "resync `.gitignore`." `.git/info/exclude` as interim is
also pragmatic.

The bare-gitlink option should remain a spike until its semantics are proven
across normal Git workflows. The plan correctly names the main cost: gitlinks
duplicate volatile commit OIDs and can create stale durable root state. That cost
is large enough that the plan should not imply bare gitlink is the likely
destination yet.

Required spike matrix:

- Root `git status` with clean member, dirty member, untracked files inside
  member, and member HEAD different from `gwz.lock.yml`.
- Root `git add .`, `git reset`, `git clean -fd`, `git checkout`, `git switch`,
  `git merge`, and clone/checkout of root history containing gitlinks.
- Root commit behavior when gitlinks are index-only vs committed tree entries.
- Root branch switch where gitlink OID changes but member worktree has local
  changes.
- `gwz status` when root gitlink, lock, and live member HEAD disagree.
- Interaction with `ignoreSubmodules` and user global Git config.
- What happens when ordinary Git tools see gitlinks without `.gitmodules`.

The important design question is whether gitlink is a committed projection,
index-only projection, or rejected for v0. Do not implement it as "same pattern
as `.gitignore`" until that is decided.

### P1: Revised sequencing puts contract tests before F18, but F18 may be needed to define the contracts

Reference: `dev-docs/GwzAuditResolutionPlan.md:315-325`

The sequence is:

1. AD1/AD2/Q6 decisions.
2. WS-contract.
3. F18.

That is plausible for red tests, but the plan should be explicit. Some AD2
contract tests will require a root metadata staging primitive, which is exactly
what F18 moves behind `GitBackend`. If WS-contract means "write failing tests
against the intended backend API," it should say so. If it means "green contract
suite," F18 likely has to happen first or inside that workstream.

Recommended sequencing:

1. AD1/AD2/Q6 paper decisions.
2. Define backend semantic API, including root metadata/boundary staging.
3. Add failing real-Git contract tests.
4. Implement F18 behind `GitBackend`.
5. Make the contract suite green.
6. Split `workspace_ops`.

### P2: Starting state and F0/WS0 are stale

References:

- `dev-docs/GwzAuditResolutionPlan.md:10-12`
- `dev-docs/GwzAuditResolutionPlan.md:63-68`
- `dev-docs/GwzAuditResolutionPlan.md:92-95`
- `dev-docs/GwzAuditResolutionPlan.md:317`

The top of the plan still says `src/git/mod.rs` is dirty and F0 is fixed but
uncommitted. §11 says WS0 is done at `79d23c7`. Current local history also shows
`79d23c7` followed by `ae5e9b2`.

Required change: update the starting state and F0 row. Otherwise the plan
continues to tell implementers to commit work that is already committed.

### P2: `main.rs` split first is acceptable, but it needs an explicit proof gate

Reference: `dev-docs/GwzAuditResolutionPlan.md:317-319`

I agree `gwz-cli/src/main.rs` is mostly architecture-independent: CLI parsing and
rendering should not depend on AD1/AD2. Splitting it first is reasonable.

The plan should still define the split acceptance gate:

- no behavior changes
- CLI tests green
- output golden tests unchanged
- no policy parsing changes
- no drive-by changes to JSON/JSONL error behavior before WS7

This keeps the split from accidentally absorbing audit fixes or output changes.

### P2: Q6 remains too implementation-shaped

References:

- `dev-docs/GwzAuditResolutionPlan.md:59`
- `dev-docs/GwzAuditResolutionPlan.md:132-135`
- `dev-docs/GwzAuditResolutionPlan.md:300-304`

§11 correctly says no ad-hoc `.gwz/recovery/<op>.yml`, but the earlier Q6 and
WS5 still prescribe exactly that path. After R1, Q6 should be rewritten as a
schema question:

- Is recovery metadata needed for v0, or should partial mutation be rejected
  until recovery is designed?
- If needed, is it an operation journal, recovery record, or event log?
- Is it local-only `.gwz/` state or versioned workspace state?
- What command lists/repairs/clears it?
- How does it interact with structured JSON/JSONL output?

## What Improved

R1 fixed the most important architectural gap from the first review:

- It no longer assumes `.gitignore` is the member-boundary answer.
- It recognizes direct `git2` use in `workspace_ops` as a backend-boundary
  violation.
- It treats fetch as mutation and proposes non-mutating remote inspection for
  planning.
- It adds real-Git backend contract tests.
- It postpones `workspace_ops` splitting until AD1/AD2 are decided.

Those are the right corrections. The plan now needs cleanup so the old text does
not compete with §11.

## Recommended Edit

Convert §11 from an addendum into the body of the plan:

1. Replace §2 with the AD1/AD2/Q6 policy table.
2. Replace F4/F10 entries with their superseded status and add F18.
3. Rewrite WS3/WS5/WS8 to remove pre-Validate fetch, ad-hoc recovery path, and
   `.gitignore` resync as active work.
4. Replace §6 with the revised sequencing instead of keeping the old order.
5. Add the backend/root-boundary contract test matrix directly to §5.

After that cleanup, the plan is good enough to drive implementation.


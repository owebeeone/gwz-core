# GWZ no Git subprocess fallback plan

Date: 2026-09-20. Status: **accepted for planning at core
`bf9446762a7c51358679ed04e147aa51dedfb2bc`, root
`57a0aba0a808417cb4c72a796ddb8926ce85179b`, after
[Consistency](../../dev-docs/GwzNoFallbackPlan-ReviewConsistency-1.md) and
[Safety](../../dev-docs/GwzNoFallbackPlan-ReviewSafety-1.md) reported GO.
This accepts the plan only; implementation remains paused.**

The two initial P2 findings were closed in one merged documentation remediation.
No interface freeze, member provisioning, dependency switch or publication is
claimed. This acceptance annotation does not change the reviewed plan body.

## 1. Objective and scope

Remove gwz-core's use of the Git executable to implement supported product
operations, preserving their required behavior. Deliver the safe per-remote
transport API needed by the existing remote transport project alongside that
work. These are four independently reviewable lanes, not four sequential phases.

This document plans the work; it does not report repository provisioning,
dependency activation, publication, or implementation as completed.

Inputs:

- [Gap inventory](GwzLibgit2Gaps.md): four subprocess cases. Commit and tag
  become one lane here; the per-remote API is an additional library work item.
- [Remote transport plan](GwzRemoteTransportPlan.md) and
  [design](GwzRemoteTransportDesign.md): retain their transport boundaries.
- [Native binding qualification](GwzRemoteTransportNativeBinding.md): accepted
  local prototype and tests, not an activated production dependency.
- [Core design](GWZDesign.md) and [requirements](GWZRequirements.md): update
  their affected clauses before implementing changed product behavior.

The contract is that core does not directly, or through a library/helper wrapper,
delegate its Git operations to the Git executable. User-configured hooks,
filters, credential helpers and signing programs may still run as external
programs; those programs can have their own Git dependency. Test fixture setup
and differential tests may use Git. Do not claim arbitrary user hooks work on
a machine without Git. The CLI's deliberate non-GWZ worktree fallback remains
outside this core plan and must be named when describing the product guarantee.

No new CLI/core carrier or framing protocol is introduced. gwz-transport remains
discrete-message stream emulation; the communication layer is supplied elsewhere.
No wholesale Git engine replacement or C libgit2 fork is assumed.

## 2. Preparation and shared contract

### P0 — Register the Rust binding fork as a workspace member

Use the existing user-created fork `git@github.com:owebeeone/git2-rs.git`.
The earlier supplied URL is authoritative; confirm it rather than creating
another repository if a later spelling differs.

1. Inspect workspace membership and any existing local checkout first.
2. Use `gwz repo clone` or `gwz repo add`, as appropriate, to make the checkout
   a member. Consult current command help. Never hand-edit `gwz.conf`.
3. Record the upstream base, fork revision, bundled libgit2 submodule revision,
   package identities and supported build features. Start from the currently
   qualified dependency baseline; do not combine this work with an upgrade.
4. Read the member's contribution/build instructions. Define ownership for
   Cargo manifests, lockfiles and dependency activation before parallel work.
5. Keep membership separate from activation: adding the checkout does not
   change production Cargo resolution. A C fix, if selected by lane 1, requires
   an explicit reproducible C source distribution strategy as well.

Deliverable: registered member and a recorded dependency baseline. Use GWZ for
workspace status, staging and commits. Publication and upstream submissions
are later explicit actions, not implied by local preparation.

### P1 — Freeze compatibility and verification boundaries

- Refresh the subprocess inventory, including indirect wrappers, against the
  implementation at lane start. Record source revisions and exact call sites.
- Define the supported operation/options/configuration matrix and the Git
  version(s) used as the compatibility oracle. Preserve existing requirements;
  do not obtain a green result by silently narrowing supported behavior.
- Identify and update the core design's last-resort fallback policy and the
  AD1 decisions before the corresponding production replacements land.
- Define shared test conventions: fixture construction may use Git, operation
  execution must also succeed with Git unavailable, and deterministic parity
  comparisons run separately against the reference Git implementation.
- Record supported object formats and platforms. Test required SHA-1/SHA-256
  configurations rather than assuming a patch preserves all feature builds.
- Read workspace `EVIDENCE.md` before creating experimental evidence. Keep
  public regression tests public and campaign data in its designated location.

The old inventory's staged Git probing and streaming `rev-list` are not required
milestones for this plan. Existing paths remain until their replacements meet
acceptance; each completed replacement removes its subprocess route entirely.

### P2 — Review the lead interface, ownership and scope checkpoint

Before any lane implementation, including new test runners or fixtures, the
integrator must file and obtain review of a lead checkpoint. Read-only inspection
and design drafting may proceed before it. Record preparation P0/P1 as separate
packages too: membership is integrator-only through GWZ, P1 is inspection/docs,
and both have zero production-code and protocol-change budgets.

The checkpoint must freeze the exact shared API signatures, vocabulary,
visibility and compatibility obligations; expand the ownership table below to
individual paths; and state integration order and the review tier for each
bounded package. Every package needs explicit numeric ceilings for production
additions, production moves, tests/tools/docs, file count and protocol changes
(zero protocol change for this plan). No TBD ceiling or unassigned shared path
permits implementation. Set grounded ceilings from the inspected baseline and
chosen lane designs; this program plan does not guess their implementation size.

Initial ownership, to be made exact in that checkpoint:

| Surface | Sole writer | Other lanes |
|---|---|---|
| Workspace membership/pins via GWZ; all production Cargo manifests and lockfiles; package metadata, native submodule pins and release recipes | Integrator | Propose changes; never edit shared inputs independently |
| Shared GitBackend contracts/types/module wiring and all cross-lane signatures | Integrator | Call existing APIs; request a reviewed contract handoff for changes |
| `gwz-core/src/git/gitbackend/transport.rs` and lane-local import tests/helpers | Lane 1 | Call-only |
| Selected libgit2 C fix, if chosen, in enumerated C source/test files | Lane 1 | Lane 2 calls it; integrator owns source/submodule pin |
| Fork `src/remote_callbacks.rs`, `src/transport.rs` and per-remote binding tests | Lane 2 | Call-only; no C or package-metadata ownership |
| `gwz-core/src/git/gitbackend/repository.rs`, `refs.rs` and enumerated commit/tag helpers/tests | Lane 3 | Call-only |
| `gwz-core/src/operation/commit_log/` and enumerated pathspec/history helpers/tests | Lane 4 | Call-only |
| Shared qualification harnesses, controlling docs, test module wiring and unassigned paths | Integrator | Changes require explicit assignment before editing |

Integration order: freeze shared boundaries first; lane implementations may then
proceed independently; integrate any C source correction before qualifying a
Rust package that depends on it; perform isolated all-consumer qualification
before the separately reviewed production dependency switch. Core behavior
replacements land only after their own acceptance gates. Lanes 3/4 have no
dependency on lane 2 unless a reviewed API change explicitly establishes one.

Use dual Code/State review for shared/durable boundaries and activation, adding
Surface for any API or user-facing interface freeze. Record permitted interior
single-axis gates at the checkpoint, with escalation on P0/P1/P2. Final package
reports compare actual scope against numeric ceilings. Stop for scope review on
more than 20% growth, a new production owner, a protocol delta, or crossing
another package's files; a budget increase must first state what is descoped.

## 3. Lane ownership and dependencies

| Lane | Primary ownership | Dependency | Independently deliverable result |
|---|---|---|---|
| 1. Local fetch | Reproduction tests, local import backend; C code only if selected | P1/P2; P0 for fork investigation/patch integration | Correct local import without `git fetch` |
| 2. Per-remote API | git2-rs callback API and native binding tests | P0/P1/P2 and binding baseline | Qualified distributable API with isolated core consumption; production switch has a separate gate |
| 3. Commit and tag | Core commit/tag orchestration and tests | P1/P2; no fork dependency expected | Commit/tag behavior without `git commit` or `git tag` |
| 4. Filtered history | Core pathspec/history traversal and tests | P1/P2; no fork dependency expected | Path-filtered log without `git rev-list` |

Lanes 3 and 4 can start design while P0 is in progress, and fixture work after
P2. Lane 1 can inspect existing reproduction evidence independently and write
new reproductions after P2. Lanes 1 and 2 may share the fork
but should use separate changes and tests. One integration owner handles shared
manifests, lockfiles, workspace pins and final documentation reconciliation.
If lanes 3 or 4 discover a missing low-level API, record that dependency before
expanding lane 2; independence is an expectation to verify, not a promise.

## 4. Lane 1 — Investigate and remove local-fetch fallback

### Investigation and decision

1. Produce a minimal failing test against the pinned stock library. Put refs to
   trees, blobs, commits and annotated tags on each side separately and together.
   Establish which refs trigger `object is not a committish` and why.
2. Separate unrelated receiver refs used as negotiation hints from explicit
   requested objects. Requested non-commit objects must remain transferable.
   Missing/corrupt objects must not be swallowed as harmless non-commit refs.
3. Characterize `FETCH_HEAD` creation/truncation separately and specify the
   local import port's intended side effects and ref-update failure behavior.
4. Compare two bounded routes: a narrow libgit2 correction, or direct local
   object transfer using pack-builder/ODB APIs behind `fetch_anonymous`.
   Compare correctness, supported refspecs, maintenance, publication and cost.
5. Record the selected route and its regression evidence before implementation.
   A C fork is optional, not a prerequisite or a predetermined result.

### Implementation and acceptance

Preserve the import port and callers. Verify object closure, required ref updates,
force/conflict semantics, cancellation, and failure behavior without exposing refs
to incomplete objects. Cover empty receivers, unrelated histories, repeated
imports and relevant family preservation cases. Measure representative large
imports if selecting a different pack algorithm. Remove the error-text trigger
and `fetch_anonymous_with_git` after the replacement passes with Git unavailable.

Deliverable: investigation/route decision, regression tests, implemented route,
and an explicit result for `FETCH_HEAD` preservation. If choosing C changes,
coordinate their distributable source pin with lane 2.

## 5. Lane 2 — Finish the safe per-remote Rust API

Start with the accepted 72-line prototype in
`tests/transport_native/patches/git2-per-remote.patch` and its retained tests.
Review against the actual fork baseline; do not redesign or re-claim completed
qualification work without a reason.

1. Port the safe `RemoteCallbacks::smart_transport` factory into the member.
   The binding constructs the transport for the actual callback owner.
2. Preserve callback lifetime, owned subtransport lifetime, error propagation,
   panic containment and coexistence with other callbacks/native transports.
   Document that reconnect can retain an existing transport; a new operation
   route uses a fresh Remote rather than assuming callback replacement reroutes it.
3. Run existing named/anonymous/clone, fetch/push, nested/concurrent route and
   destruction tests against the fork. Keep process-global registration out of
   the product implementation.
4. Select and qualify a distributable dependency strategy. A local path patch
   or test-only Cargo override is not release availability. Account for the
   existing package publication workflow, feature set and single native library
   linkage. Coordinate production manifests/lockfiles through the integrator.
5. Prove core consumption in isolated candidate builds and rerun the binding/
   adapter gates. Document exact revisions and reproducible builds outside this
   workspace. Keep production manifests and locks on the existing dependency
   until the activation gate below passes.

Before any production manifest/lock switch, the integrator must enumerate all
direct consumers (at least core, repo-inspect, local-testrepo and CLI's git2
dev-dependency), their features, package versions/sources and native library pins.
Qualify that exact proposed manifest/lock set in isolated candidate copies:
fresh workspace, standalone-core and CLI builds on every supported native
platform and required object format must resolve the same qualified Rust package
and one libgit2 native library, preserving each consumer's feature requirements.
Reproduce standalone release packaging without sibling checkouts or root-only
Cargo patches. Carry forward the accepted adapter-foundation package provenance
and tests rather than treating the fork checkout as equivalent evidence.

Production dependency activation is a separate reviewed change after those
distribution, consumer, native-link and platform gates pass. Its candidate must
match the qualified inputs; any source/pin/feature change requires relevant
requalification. A publication or host-local test result alone cannot pass it.
Missing platform evidence leaves activation pending, without blocking the other
lanes. This does not activate SSH/message/pool production routing.

No C asynchronous/resumable API is required by the stream model: the controlled
Git-facing adapter blocks while an independent worker moves messages. Test that
back pressure progresses and cancellation wakes blocked operations; do not leak
`WouldBlock` into libgit2 as an ordinary retry signal.

Independent deliverable: supported Rust API, qualified distribution candidate and
isolated core binding consumption. Production dependency integration follows the
separate activation gate. Actual SSH pumping, pooling activation and all-network-verb rollout
remain governed by the remote transport plan; this lane alone does not finish it.

## 6. Lane 3 — Commit and tag without Git command execution

Design one reusable Rust orchestration layer over existing git2 primitives,
initially within core unless a concrete reuse need justifies another crate.
Keep command policy out of the thin FFI binding.

1. Specify identity/configuration/environment precedence, date handling,
   message cleanup, hook timing, signing, staging and ref/index failure semantics.
2. Implement identity resolution and hooks with native parity fixtures:
   `core.hooksPath`, arguments, working directory, environment, message edits,
   hook-induced index changes, exit codes and pre/post-update failure behavior.
3. Implement tracked staging for `-a`, including deletions and applicable filters,
   without accidentally staging unrelated untracked files or losing staged work.
4. Implement OpenPGP, SSH and x509 signing using configured signer programs.
   Test the unsigned payload and verify resulting signatures. Commit and annotated
   tag signature formats need distinct handling; share signer infrastructure.
5. Implement lightweight, annotated and signed tags with existing API semantics,
   configuration-driven signing, safe ref updates and appropriate reflogs.
6. Switch the commit/tag backend methods and remove their Git subprocess routes.

Correct the inventory during this work: lightweight tags have no tagger identity;
ordinary commit hooks do not run for tags. Specify reference-transaction hook
behavior separately. Do not silently broaden currently supported tag targets.

Review merge commits, configuration commits and stash creation as separate
operation policies. Shared identity/signing/hook machinery must not automatically
apply ordinary commit hooks or signing to every internally created commit.
Record required behavior changes in design/requirements before making them.
Unrelated policy improvements may be separately tracked without blocking removal
of the two existing subprocess paths.

Acceptance includes parity for successful results and observable failure effects:
hook rejection, edited messages/indexes, signing failures, empty commits, unborn
HEAD, concurrent ref changes and interrupted operations. Use independent signature
verification where signature bytes are nondeterministic. Exercise supported
platforms and linked-worktree behavior. Tests with controlled helpers must pass
with Git unavailable during the product operation.

## 7. Lane 4 — Path-filtered history without rev-list

1. Freeze the existing log contract, including revision pushes/hides, ordering,
   first-parent behavior, pagination and workspace pathspec routing (L-RNG).
2. Evaluate reusable Rust pathspec components, including `gix-pathspec`, against
   the full required magic/exclusion/attribute behavior and licensing/dependency
   constraints. Reuse is a candidate, not assumed Git parity.
3. Design path-sensitive traversal and Git's required history simplification.
   Filtering an ordinary revwalk's output is insufficient: path equality affects
   which merge parents are traversed. Implement only the documented supported
   modes, without changing existing semantics.
4. Implement incremental traversal with bounded/cancellable work and appropriate
   caching. Avoid restarting the whole history walk for each returned commit.
5. Replace `PathWalk`'s subprocess path after exact ordered-sequence parity passes.
   Preserve read-only behavior; do not introduce lazy fetch or repository writes.

Acceptance: existing fixtures plus merges, octopus merges, empty/root commits,
deletions, renames as currently observed, exclusions, magic forms, first-parent,
revision ranges and pagination. Generate randomized commit graphs and pathspecs
with a recorded seed, generator version and replay command; compare complete
ordered commit sequences to native Git. Preserve a minimal deterministic fixture
for each corrected failure. Check scaling on large histories, not only correctness
on small graphs. Product traversal must run with Git unavailable.

## 8. Integration, review and completion

After P2, each lane starts with failing regression/compatibility tests, then implementation,
green tests and refactoring. Review lane-specific design decisions and final
changes independently; no lane must wait for all others to finish. Reuse qualified
transport tests and retained review context where applicable. Historical GO for
the binding prototype does not qualify new packaging or production activation.

The integration owner:

1. Reconciles dependency changes under the lane 2 preactivation gate, then checks
   combined lane results against the supported feature/platform matrix. Aggregate
   checks do not replace the qualification required before a dependency switch.
2. Audits product subprocess launch sites and indirect Git delegation; retains a
   regression guard as well as operation-level execution tests. Removing Git from
   PATH alone is insufficient if an absolute executable path is still available.
3. Runs the supported operation matrix in a controlled environment where Git
   cannot be launched, with only explicitly required test hooks/signers/helpers.
4. Reconciles this plan, the gap inventory, AD1/core policy and remote transport
   checkpoints. States remaining external-tool requirements precisely.

Completion means all in-scope subprocess paths (including any inventory additions)
are removed, compatibility and no-Git execution gates pass, and the per-remote API
is reproducibly consumed by core through the separately qualified and reviewed
dependency switch. The broader transport program has its own remaining activation gates.

## 9. Decisions to close within the work

| Decision | Owner | Required before |
|---|---|---|
| Compatibility oracle versions and supported matrix | Integrator with lanes 3/4 | Parity implementation |
| Local-fetch C fix versus direct object transfer | Lane 1 | Replacement implementation |
| Shared interface/path ownership, package budgets and review tiers | Integrator with all lanes | P2 acceptance, before lane implementation |
| Fork/package distribution, consumer/platform matrix and any C source pin | Lane 2 with integrator | Separate production dependency activation review |
| Commit/tag and internal-commit policy details | Lane 3 | Behavior changes |
| Pathspec component and traversal design | Lane 4 | Traversal implementation |

These decisions are local gates within independent lanes, not reasons to serialize
the whole project. The next execution step, once work resumes, is P0/P1 followed
by P2 and the four lanes; creating this plan does not itself resume implementation.

# No-fallback first implementation checkpoint

Date: 2026-09-20. Status: **accepted first-package scope after Code/State GO**.
Reviewed core `dd47810ece5980cfa35017ae0dfd7a8f33701e80`, root
`d98922e03b837d030477f1d9a696fef2464b8b17`; unchanged fork/transport/taut
identities are in the [Code](../../dev-docs/GwzNoFallbackCheckpoint-ReviewCode.md)
and [State](../../dev-docs/GwzNoFallbackCheckpoint-ReviewState.md) reports.
One review-entry P2 corrected; zero open findings. No implementation accepted yet.
Authority: [accepted plan](GwzNoFallbackPlan.md) and
[preparation baseline](GwzNoFallbackPreparation.md). Operator has resumed work.
This checkpoint does not activate a new production dependency or approve a C
fork, replacement history algorithm, or new commit/tag policy.

## Scope and sequencing

Four lanes can proceed independently after acceptance. Evidence must precede
the local-fetch route decision and the substantial commit/tag/history designs.
Accordingly the first packages for lanes 1, 3 and 4 are characterization and
design inputs, not speculative replacements. Lane 2 can port the already
accepted binding on its exact qualified Rust source. Subsequent implementation
packages need their own grounded budgets and design gates; this checkpoint
does not allocate an unlimited umbrella budget to them.

P0 registered `mem_git2_rs` at fork `f42a01267a3042b26d30e9d8acf286c6c739bd8a`.
P1 inventories five product subprocess sites and the existing feature/platform
matrix. Core source baseline is `0154d36d3412d26b0a8431ece67f64abd214f5d6`;
preparation documentation descendant `4199e4589a4299b7a2ba37c6a644f6d4e53d2e8c`.
Root membership/preparation checkpoint is
`1fab19cf9e8caf4cb89122dbff89c81a8a63f4fd`. Transport and taut remain unchanged.

Order: accept this checkpoint; integrate each package's test-only module wiring;
run lane packages independently; review their exact results. A C patch selected
by lane 1 must precede requalification of any package that incorporates it.
Production dependency activation remains a later, separately reviewed change
covering every consumer and supported native platform. The other lanes need not
wait for that activation.

## Shared interfaces and invariants

Retain the existing visibility, types and signatures at their baseline paths:

```rust
// src/git/gitbackend/contract.rs: existing GitBackend methods
fn fetch_anonymous(
    &self, path: &Path, url: &str, refspecs: &[&str],
) -> ModelResult<GitFetchResult>;
fn commit(&self, path: &Path, message: &str, all: bool)
    -> ModelResult<GitCommitResult>;
fn tag_create(
    &self, path: &Path, name: &str, message: Option<&str>, signed: bool,
) -> ModelResult<GitTagResult>;
fn tag_delete(&self, path: &Path, name: &str) -> ModelResult<()>;

// crates/local-import/src/lib.rs: existing LocalTransport method
fn fetch_anonymous(
    &mut self, receiver: &Path, source: &Path, refspecs: &[String],
) -> Result<(), TransportError>;

// Accepted native-binding extension; no change from its prior qualification
pub fn smart_transport<S, F>(&mut self, rpc: bool, factory: F) -> &mut Self
where
    F: FnMut(&Remote<'_>) -> Result<S, Error> + 'a,
    S: SmartSubtransport;
```

`GitCommitResult`, `GitTagResult`, `GitFetchResult` and local-import error types
are unchanged. Existing backend delegates/fakes keep their contracts. The binding
constructs the transport for the actual callback owner, preserves errors/panics
and returns independently owned subtransports. Reconnect may retain a transport;
a fresh operation route uses a fresh Remote. No global registration is added.

History retains `RepositoryHistory`, `RepositoryMessages`, the output registry
and current request/filter/range APIs at `src/operation/commit_log/`. No signatures
or visibility in that module tree change in this first package. The entire
`LogOptions`/`LogRequest` schema, shared pathspec routing and existing CLI option
surface are read-only. No new backend abstraction is authorized here.

Before later behavior changes, the integrator must amend the affected core
design/requirements and AD1 clauses. Characterization does not supersede their
current contract, including FETCH_HEAD being outside the local-import promise.
Do not narrow currently accepted pathspec/configuration behavior for convenience.

## Exact first-package ownership

All paths below are workspace-relative. Rows list the only permitted edits;
unlisted files are read/call-only. The integrator performs shared wiring before
lane execution and then leaves those files stable while lanes run.

| Writer/package | Owned paths | Read/call-only or forbidden |
|---|---|---|
| Integrator P0/P1/P2 | GWZ-managed membership/lock/integrity artifacts via GWZ; `gwz-core/dev-docs/GwzNoFallbackPreparation.md`, `GwzNoFallbackCheckpoint.md`, `GwzNoFallbackPlan.md`; root `dev-docs/CurrentProgramCheckpoint.md` and this object's review/prompt/remediation documents | No manual `gwz.conf` edits; inventory discussion is a separate completed artifact |
| Integrator wiring | `gwz-core/src/local_clone/tests/mod.rs`, `gwz-core/src/git/gitbackend.rs`, `gwz-core/src/operation/commit_log/tests.rs` | Test-module declarations only; preserve runtime behavior and all unrelated code |
| Lane 1 L1-A | `gwz-core/src/local_clone/tests/transport_noncommit.rs`; `gwz-core/dev-docs/GwzNoFallbackLocalFetchInvestigation.md` | Existing `transport.rs`, local-import port/adapter and fixture helpers read-only; no C/package edits |
| Lane 2 L2-A | `git2-rs/src/remote_callbacks.rs`, `git2-rs/src/transport.rs`; `gwz-core/tests/transport_native/prove.py`, `test_prove.py`, `README.md`; `gwz-core/dev-docs/GwzNoFallbackBindingPort.md` | Existing seven native test cases and archive/patch pins retained; no GWZ production manifests or transport runtime edits |
| Integrator L2-A source alignment | Branch selection for `git2-rs`; `git2-rs/Cargo.toml` only | No C checkout/pin changes, package publication or unrelated upstream upgrade |
| Lane 3 L3-A | `gwz-core/src/git/gitbackend/commit_tag_characterization.rs`; `gwz-core/dev-docs/GwzNoFallbackCommitTagDesign.md` | Production commit/tag/merge/stash code and trait contracts read-only; no new options or policy |
| Lane 4 L4-A | `gwz-core/src/operation/commit_log/path_characterization.rs`; `gwz-core/dev-docs/GwzNoFallbackHistoryDesign.md` | Existing fixtures, request/filter/merge/routing and CLI/protocol files read-only; no new dependency |

New child modules can reuse parent-visible helpers. If existing privacy prevents
reuse, stop for an integrator-owned bounded handoff instead of copying a parallel
harness or widening a public interface. Helpers can instead keep an initial
package documentation-only when it accurately records unresolved test design;
that is not completion of its characterization gate.

Every production Cargo manifest/lock, package identity/native pin and shared
contract/type is owned by the integrator. No first-package manifest change is
allowed except the explicit L2-A isolated fork source alignment below. A required
cross-lane contract change must be designed/reviewed before implementation.

## Numeric ceilings and review tiers

Ceilings are added/changed logical lines, with moved production lines counted
separately. Imported upstream history and unchanged reused fixtures are recorded
as baseline, not charged as newly implemented behavior. File counts include
documentation but exclude filed reviewer testimony/prompts. No protocol delta
or new production runtime owner is permitted in any first package.

| Package | Production additions/changes | Production moves | Test lines | Tool lines | Doc lines | Files | Review |
|---|---:|---:|---:|---:|---:|---:|---|
| P0/P1 preparation | 0 | 0 | 0 | 0 | 450 | 8 | This dual checkpoint review |
| P2 checkpoint/shared test wiring | 0 runtime; at most 24 test-wiring lines | 0 | 24 | 0 | 300 | 5 | Dual Code/State before wiring or execution; wiring checked with affected lane |
| L1-A local fetch characterization | 0 | 0 | 400 | 0 | 180 | 2 | State; escalate on P0/P1/P2 |
| L2-A qualified-source binding port | 150 Rust + 8 manifest | 0 | 100 | 120 | 180 | 7 | Code/State and Surface for any changed public fixture input |
| L3-A commit/tag characterization/design | 0 | 0 | 500 | 0 | 250 | 2 | State; escalate on P0/P1/P2 |
| L4-A history characterization/design | 0 | 0 | 350 | 0 | 250 | 2 | Code; escalate on P0/P1/P2 |

The accepted binding prototype adds 72 Rust lines across two files, so its
150-line ceiling includes modest integration allowance. Other first packages
reuse existing test targets and limit growth to one cohesive test module plus
a design/evidence document. Split rather than grow a crowded module; additional
paths require reviewed scope adjustment. Stop at >120% of any ceiling, a new
owner, protocol change, or unapproved file crossing. Record actuals at closure.
Any ceiling increase first identifies what is descoped. Later replacement
packages have no permission to start until their numeric budgets are accepted.

## Lane execution and evidence

### L1-A: reproduce before choosing a local-transfer route

Use pinned libgit2 1.9.7, not the newly cloned member's older path dependency.
Compare the native local fetch directly with the backend route so Git fallback
cannot hide the native defect. Fixture objects may be made with git2; Git may
be an oracle, but identify each call and do not claim a no-Git replacement yet.

Cover explicit requested commit/tree/blob/tag objects, unrelated receiver refs
of those types, each side separately and both together, empty/unrelated histories
and repeated imports. Record native error code/class/message; missing/corrupt
objects must remain distinguishable from a non-commit negotiation hint. Observe
objects, destination refs, remotes/tracking refs and FETCH_HEAD on success/failure.
Characterize multi-refspec partial publication before promising atomic updates.
Use the existing Tier B fixture and regression target. Record omitted matrix
rows explicitly if more scope is needed; they remain prerequisites to route
selection, not silently deferred replacement acceptance.

The registry source suggests receiver revwalk errors may be compared against an
error-class constant instead of the actual return code. This is a hypothesis,
not a finding closed by inspection. The report compares a minimal C correction
with direct pack/ODB transfer and names the evidence needed for either.

### L2-A: exact Rust release source, existing native baseline

Create a local `codex/` branch in the registered member from
`dffaf272eb0e62ac15b74283c4e488252db9afc3` (git2 0.21.0). Preserve the cloned
upstream branch; do not reset it or pull unrelated changes. The integrator changes
only the root git2 dependency on libgit2-sys to registry `=0.18.8`, removing its
path edge. This pairs the exact released Rust source with the already-qualified
sys 1.9.7 source; the old C submodule is not built or repinned for this package.

First verify the two unpatched files against accepted archive hashes. The
existing safe-method compile-red remains valid evidence; reproduce refusal on
the unpatched source if the fixture path changes. Apply the accepted patch and
verify resulting hashes. Any necessary delta beyond those bytes is explicit
and re-reviewed. New/modified conditional sections use enclosing modules or
cfg_if; preexisting upstream conditional-style debt is not claimed migrated.

Extend the existing `prove.py` with a mutually exclusive `--git2-source` input
alongside `--git2-archive`; archive behavior remains supported. Source mode must
check expected binding hashes and native dependency selection, operate on an
isolated copy, and run all seven existing native tests with locked graph checks.
Do not modify Cargo registry caches. Reject a member checkout containing unrelated
source drift: compare source against the exact release tree plus the declared
binding/manifest changes. Prove rejection as well as positive admission.

This gives an unpublished member-backed candidate only. Package rename,
distributable release archive, all-consumer/platform qualification and production
activation remain subsequent integrator-owned packages. A source-mode pass is
not a crates.io availability or native multi-platform claim.

### L3-A: freeze observable commit/tag behavior

Characterize `commit(message, all)`, `tag_create(name, message, signed)` and
`tag_delete(name)`, including HEAD/index/ref effects on errors. Cover ordinary
commit versus tracked-only `-a`, hooks and message edits, identity precedence,
lightweight/annotated/config-signed tags and deletion/reference-hook behavior.
Use isolated repository configuration and child-process environment where needed;
do not mutate process-wide environment in concurrent tests or use personal keys.

Record signer format/payload/error obligations and missing fixture coverage for
OpenPGP/SSH/x509. A no-message tag call cannot bypass configured signing by being
assumed lightweight. No ordinary commit hooks are applied indiscriminately to
tags, merge/config commits or stash. The design proposes one reusable internal
orchestration owner and operation-specific policy with a failure-effects matrix.
Existing Git subprocesses remain until a separately reviewed replacement passes
the full accepted compatibility matrix and denied-Git execution gate.

### L4-A: preserve exact current history semantics

Reuse existing deterministic log fixtures/oracle helpers. Characterize remaining
attribute magic, merge simplification and ordered range/first-parent cases needed
to choose a pathspec/traversal design. Existing root/member routing, long/short
exclusions, shallow and no-lazy-fetch tests remain authoritative. No `--follow`,
new sort order or other unexposed Git option is introduced. Distinguish native
ordering from timestamp sorting and path-sensitive traversal from post-filtering.

The design evaluates reusable pathspec code without adding a dependency in this
package. Name a gap before introducing randomized generation; any later generator
must record seed/version/replay inputs and reuse the existing test target. No
streaming Git-process intermediate is an acceptance milestone for this lane.

## Verification, closure and next packages

Use Rust 1.95 and the existing core test runner/targets. Execute focused tests
for touched scopes; add broad checks only for new failures/concerns or required
repository gates. Format changed Rust and run source/syntax checks on newly
modified conditional/control-flow sections, including disabled branches.
Test fixture execution of Git is allowed and must not be confused with product
no-Git execution. Native platforms/object formats are recorded as executed or
pending; this host cannot supply the entire release matrix.

Public regressions and required runners remain public. Campaign-only runs follow
EVIDENCE.md, with builds outside the evidence member. Reports name exact source
revisions, observed results, unexecuted rows and actual budget use. No hidden
fallback removal, dependency activation or production speedup is claimed until
the corresponding later implementation gate passes.

Independent Code and State reviewers must both report GO on this exact P2
shared-boundary object before wiring, fixtures or code start. Consistency/Safety
document reviews are additional evidence and do not substitute for that gate.
The shared runtime interfaces are unchanged from accepted contracts; there is
no new CLI/protocol surface frozen here. Lane 2's later source-input/public
documentation change receives its named Surface review. After first-package
evidence, choose and review each replacement design, ownership and budgets,
then implement it without reopening unrelated lanes or broadening the product.

## Bounded test-wiring handoff

L4-A uses private helpers in `commit_log/tests.rs`. Integrator wiring is therefore
a child module declaration in that existing test file, replacing the originally
listed `commit_log/mod.rs` wiring path. No helper visibility, production module,
public interface, file count or wiring budget changes. The lane still owns only
its new test module and report. This follows the checkpoint's privacy handoff
rule; the affected L4 Code review must inspect these two wiring lines.

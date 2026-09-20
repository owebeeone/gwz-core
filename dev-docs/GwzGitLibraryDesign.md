# Single-repository Rust Git library

Date: 2026-09-20. Status: **design and G0 API accepted 2026-09-21** at core
`3efc1a79a1e5044b6e2495ed392a5c43d8f90b64`, root
`bba620ed7806628cdde26254261043eb9266b0f9`, after retained
[Code](../../dev-docs/GwzGitLibraryDesign-ReviewCode-1.md),
[State](../../dev-docs/GwzGitLibraryDesign-ReviewState-1.md) and
[Surface](../../dev-docs/GwzGitLibraryDesign-ReviewSurface-1.md) GO.
One P2 Send-contract ambiguity closed in one remediation; no open findings.
This annotation changes no reviewed contract. No implementation or production
activation. Selected local package/repository name: `gwz-git`, Rust import
`gwz_git`; not a published identity.

## Decision and authority

Keep the qualified libgit2 1.9.7 correction and safe per-remote git2-rs binding
small. Own the additional Git behavior in a separate, enduring Rust library
above git2-rs. New upstream APIs can replace internals later without moving
workspace policy into a fork or requiring GWZ callers to adopt native types.

This refines the operator-direction section of [NoFallbackPlan](GwzNoFallbackPlan.md).
It preserves that plan's product semantics and no-Git-subprocess contract,
[Preparation](GwzNoFallbackPreparation.md) compatibility gates,
[NativeFix](GwzNoFallbackNativeFix.md) accepted N1/N2, and the transport design.
The [API guide](GwzGitLibraryApi.md) is the complete proposed G0 public surface.
Acceptance freezes G0 and the ownership map below, not future operation APIs.
No CLI/core messages, flags, physical carrier, or product behavior change here.
Overall GWZDesign/GWZRequirements remain authoritative for product behavior;
affected clauses must be updated before any subsequent behavior expansion.

## Ownership

| Owner | Responsibility |
| --- | --- |
| libgit2 | Object database, native Git algorithms and native transports; narrow bug fixes |
| git2-rs | Safe native binding, including the accepted per-remote factory; narrow missing bindings |
| gwz-git | Supported single-repository Git behavior, owned data/errors, later commit/tag orchestration and path-history traversal |
| gwz-core | Member selection, workspace locks, markers, snapshots, ordering, fan-out, recovery, protocol lowering and result aggregation |
| Current endpoint hosts / gwz-transport | Credentials and endpoint placement / discrete-message stream emulation and pooling, respectively |

The existing `GitRepository`/`GitBackend` trait stays in core. Its contracts
include preservation and workspace recovery, so moving that trait wholesale
would export the wrong abstraction. Core adapts library results into existing
`ModelResult` and operation responses. Existing direct git2 users coexist during
migration; G0 does not attempt to wrap every native method or move all backend code.

Library dependencies must not include core, CLI, taut, workspace contracts or
member configuration. No public native git2 types, raw pointers or native handle
escape hatch in G0. Native object lifetimes stay private. The library does not
choose repositories from a workspace or discover a parent when given a bad path.

## Placement and dependency bootstrap

Create a sibling member `gwz-git/` with its own Git repository and standalone
Cargo workspace, Rust 1.95 / edition 2024, initially `publish = false`.
Provision through `gwz repo create` (consult installed help), not mkdir/init or
hand-edited membership. No remote creation or publication is part of G0.
The integrator owns membership and root Cargo exclusion; library work must not
change core or CLI dependency graphs. This sibling boundary allows separate
reuse/versioning, unlike nesting another workspace-policy crate under core.

Initial prepared-workspace dependency:
`git2 = { path = "../git2-rs", version = "=0.21.0", default-features = false,
features = ["unstable-sha256", "vendored-libgit2"] }`.
No network feature is needed for G0. Own and commit the library Cargo.lock.
The selected source is Rust fork `4c1caabbce7d56426c763dd94114052302b23e4c`,
sys 0.18.8+1.9.7 and native C
`b172e3d187a4b6866fd9f696f40a1b8e7f56d348` (accepted N2 composition).
The path declaration is not a source pin: G0 acceptance must verify that exact
source and nested submodule before and after tests, run the existing
`tests/transport_native/prove.py --git2-source ../git2-rs` from core, and assert
vendored runtime libgit2 1.9.7 in the library tests. The existing proof checks
source bytes against Git objects; do not replace it with a HEAD-only check.

Both forks remain unpublished. G0 is reproducible only in the prepared workspace,
not a clean remote-only checkout or crates.io install. Publication/source
distribution, clean reconstruction, all consumers/features and native platforms
remain separate gates before any production dependency switch. Cargo patches in
a dependency do not propagate to consumer workspace roots: the integrator must
later choose one consistent git2/sys source for every consuming root and verify
no duplicate native `links` provider. Do not silently select registry sys or
system libgit2 to make a consumer build succeed.

## G0: read-only foundation

Only implement the signatures and semantics in the API guide: explicit
repository opening, repository paths/format, full object IDs and reading one
commit into owned data. This is useful groundwork for both commit/tag results
and history entries; it makes no claim to eliminate a fallback yet.

`Repository` owns one native handle and is `Send`, not Clone or Sync. Ownership
may move between workers for sequential use; shared concurrent access to the
handle is prohibited. Construct, use and drop need not occur on the same worker.
Run synchronous calls on a suitable worker. Owned results can outlive it. Subsequent mutation APIs
will require exclusive handle access, but that will not imply cross-process
locking or an atomic multi-repository transaction. Borrowed native cursors stay
within a repository borrow; do not build self-referential owning revwalk structs.

Open with native NO_SEARCH, without FROM_ENV or parent discovery. Accept the
repository's worktree root, Git directory, linked-worktree root or bare root.
Follow normal filesystem indirection; this is not a hostile-path sandbox. Do
not create a repository, update refs/index/worktree, alter process environment,
change current directory, or change global native settings. Ordinary native
read configuration remains available; G0 does not freeze future mutation config
precedence or bypass native repository ownership checks. No subprocess, helper,
hook, credential prompt, lazy fetch or network access on the G0 call path.

Reading a commit requires a full object ID of the repository format; no prefix,
revision expression, tag peeling or implicit HEAD. Preserve parent order, raw
message bytes (including leading newlines), raw identity bytes, numeric times
and offset minutes, and the optional raw encoding header. This is structured
data, not a promise to round-trip every raw commit header/signature. Native
parse errors remain errors. Missing parent/tree objects do not prevent returning
their IDs if the commit itself parses; reads do not recursively validate a DAG.

Errors own their data and separate library kind from native diagnostic code and
class. A code is never compared to a class (the original native-fetch bug).
No missing/corrupt object becomes an empty result. No error string is a machine
contract. Public enums are non-exhaustive; allocation failure follows Rust's
ordinary allocation model, not a new fallible-allocation guarantee.

## Future operation boundaries — requirements, not frozen signatures

### Commit and tag (L3)

Library owns supported ordinary commit/tag mechanics: effective config and
identity resolution, tracked-only staging, message preparation/cleanup, hooks,
signing, object creation and ref publication. Core retains member-first/root-last
ordering, dry-run, no-op/idempotence policy, lock/marker refresh and artifact
staging. Existing merge-resolution/config/stash commits keep their distinct
policy; sharing primitives must not apply ordinary commit hooks to them.

Configured hooks, signers, editors and filters remain permitted child programs
under NoFallbackPlan. No generic `run_git` escape hatch or disguised Git
subprocess fallback. Execute helpers where the repository lives with explicitly
constructed per-child context; never mutate process-global environment/cwd to
simulate Git. Endpoint trust and credential placement remain separate.
Do not freeze a public generic process-runner trait before the operation needs
are characterized; internal test seams suffice initially.

[L3 characterization](GwzNoFallbackCommitTagDesign.md) is the starting evidence.
Before L3 implementation freeze: identity/config/env/date precedence, index and
filter parity, message/editor behavior, all supported signer formats, hook
ordering/context, ref-hook phases, CAS/reflogs and interruption aftermath.
Successful OpenPGP/SSH/x509 signing is not yet proved. Signed tag buffers may
need one safe git2-rs wrapper over existing `git_tag_create_frombuffer`; that is
a separately reviewed capability addition, not grounds for a larger C fork.

Results must distinguish failure before publication from successful publication
with advisory post-hook failure and indeterminate verification. Hooks can have
external effects and index updates can precede failure: no blanket rollback,
blind retry or unchanged-repository promise. Native ref transactions update
refs individually; never describe them as atomic across refs. Prove admitted
physical primitives on target platforms before freezing durable transitions.

### History filtering (L4)

Core keeps request/range parsing, revision resolution and ordered push/hide
lowering, member routing and pathspec rerooting, author/time/grep/no-merges
filters, cross-repository ordering/coalescing, pagination and wire output.
Library receives resolved IDs, already repository-relative pathspec strings
with their full magic preserved, and first-parent selection. It owns incremental
single-repo traversal, pathspec evaluation, TREESAME simplification and native
intra-repository order. Emitted parents remain the actual commit parents even
where simplification rewrites traversal edges. An ordinary revwalk followed by
a changed-path predicate is not equivalent.

Proposed cursor shape is a borrow of Repository, returning an owned commit or
an explicit error/EOF; exact signatures wait for the L4 design. Cancellation
must be distinct from EOF. Core currently has no log cancellation carrier, so
no new cancellation request is implied here. Core retains per-target degradation
and strict-mode policy; library errors must not emit workspace events.

[L4 characterization](GwzNoFallbackHistoryDesign.md) identified lost worktree
context for attribute pathspecs. Before freeze, establish explicit attribute
source semantics for worktree/bare/member cases, shallow/promisor/missing-object
behavior, all admitted magic combinations, complex merge ordering, both object
formats, cursor termination and resource bounds. No implicit promisor fetch.

### Fetch and remote transport (L1/L2)

N1/N2 remain accepted. Keep the current native local-peer URL admission,
explicit refspec scope and no-network/no-credential behavior until a separately
reviewed migration. Before fallback removal: harden declared/actual tag target
type consistency, characterize missing targets, FETCH_HEAD/ref publication,
cancellation and partial outcomes. Successful happy-path fetch is insufficient.

The eventual library remote adapter uses the accepted per-remote capability,
never global transport registration. Endpoint hosts supply transport and
credentials; gwz-transport owns messages/streams/pool. Current blocking adapter
must run away from the independent message/timer pump. No SSH pool, endpoint
trust, framing or CLI/core carrier moves into this library. G0 freezes no new
stream trait or remote callback signature. Transport activation retains its own
reviewed program, independent of no-fallback packaging.

## Packages, ownership, budgets and review

G0 is the next implementation package after this design's acceptance:

- Integrator: member creation, root Cargo exclusion, source/lock selection,
  checkpoint and dependency proof. No production dependency switch.
- Library implementer: `gwz-git/{Cargo.toml,Cargo.lock,AGENTS.md,README.md,LICENSE}`,
  `src/{lib.rs,object_id.rs,error.rs,repository.rs,commit.rs}` and
  `tests/{foundation.rs,native_baseline.rs}`. Use repository policy for license;
  inherit GWZ's license rather than inventing a new licensing policy.
- Ceiling: 600 production lines, 12 manually maintained library files (lockfile
  generated/excluded); root exclusion and GWZ-generated membership owned only
  by integrator. Test size is measured, not a reason to omit required cases.
  Stop/re-scope for new owner, public method, process execution, network/wire
  delta, ownership crossing or >120% growth. Cut scope before raising ceilings.
- G0 design/API: retained Code + State + Surface review. G0 implementation:
  Code + State at first aggregate gate, Surface verifies API docs/examples.
  Later interior checkpoints may use a recorded single axis; future operation
  freezes, durable kernels and activation remain dual (plus Surface for APIs).

Tests first: full SHA1/SHA256 ID parse/display/round-trip and mixed-format reject;
normal/bare/linked/unborn opens; nested/missing-path non-discovery; exact raw
message/identity/encoding/time/parent-order extraction; missing object, tag/blob
passed as commit and malformed commit errors; missing parent references; owned
record surviving Repository drop. Verify unchanged refs/index/worktree and
ambient cwd/environment. Fixtures may use Git; runtime calls must work with Git
unavailable. Use existing test tools, not a new Monte Carlo framework for simple
parsing. Require a positive compile-time `Repository: Send` assertion and
compile-fail `Repository: Sync` and `Repository: Clone` checks, plus sequential
ownership transfer to another worker followed by read/drop. Qualify concurrent
SHA-256 reads through independent handles as a native-baseline regression: the
pinned 1.9.7 source includes the builtin hash thread-safety fix despite the stale
warning in the Rust manifest. No new C patch is needed for that resolved defect.

Run format/check/test and clippy on the selected Rust baseline with locked
dependencies, plus native source proof. Cover macOS arm64/x86, Linux arm64/x86
and Windows x86_64 MSVC per Preparation; unexecuted rows are explicitly pending,
not inferred from another OS. One host can accept an unpublished G0 candidate;
all-platform qualification gates production activation. No platform-specific
durable primitive is frozen by read-only G0.

After G0, L1 hardening, L3 characterization/design and L4 characterization/design
can proceed independently with disjoint file ownership. Each next package needs
its own exact API/paths/ceiling/review checkpoint; none is authorized by a vague
“move the remaining backend” instruction. L2 adapter and all-consumer packaging
follow their separate gates. This ownership map supersedes only the *future*
placement of single-repo behavior in the old no-fallback plan/checkpoint; accepted
first-package code and current file ownership remain unchanged until explicitly
migrated. No existing subprocess is removed by accepting this design.

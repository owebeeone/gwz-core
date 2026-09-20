# gwz-git next-package scope

Date: 2026-09-21. Status: **scope accepted after retained Code/State GO**.
Implementation starts only after that gate passes. The controlling records are [GwzGitLibraryDesign](GwzGitLibraryDesign.md),
[GwzGitLibraryApi](GwzGitLibraryApi.md), and the accepted [G0 record](GwzGitLibraryG0.md).
G0 is a read-only `gwz-git` foundation. It does not freeze operation APIs,
remove a subprocess route, activate a consumer, or qualify an unpublished
candidate beyond macOS arm64.

The observed baseline is Rust 1.95 on `aarch64-apple-darwin`, Git 2.52.0, and
the accepted vendored libgit2 1.9.7 composition: git2-rs
`ce78628308e11b4e8901d5061602619109bce21a` and C
`b172e3d187a4b6866fd9f696f40a1b8e7f56d348`. The existing physical backend still
uses `std::process::Command` at `gwz-core/src/git/gitbackend/repository.rs:291-332`
for `git -C <path> commit [-a] -m <message>`, and at
`gwz-core/src/git/gitbackend/refs.rs:164-217,232-263` for tag create/delete.
The path history cursor still invokes `git --git-dir ... rev-list` at
`gwz-core/src/operation/commit_log/mod.rs:311-350`.

These packages characterize those routes and their native Git oracle. They do
not change product behavior, shared contracts, CLI options, protocol types,
manifests, locks, or the `gwz-git` public surface. Git used by a fixture or
oracle is evidence only; a later replacement must satisfy the no-Git runtime
gate separately.

## Package and ownership table

| Package | Owned paths | Bound | Review gate |
|---|---|---:|---|
| C1 commit/tag identity and message | New `gwz-core/src/git/gitbackend/commit_tag_identity_characterization.rs`; append evidence to `gwz-core/dev-docs/GwzNoFallbackCommitTagDesign.md` (≤150 added doc lines); one integrator module declaration inside the existing `cfg_if!` test boundary in `src/git/gitbackend.rs` | ≤400 added test lines; 0 production; 0 tool; 2 lane files plus bounded wiring | State; escalate P0–P2 |
| H1 history attribute/worktree context | Existing `gwz-core/src/operation/commit_log/path_characterization.rs`; append evidence to `gwz-core/dev-docs/GwzNoFallbackHistoryDesign.md` (≤150 added doc lines); no new wiring | ≤400 added test lines; 0 production; 0 tool; 2 existing lane files | Code; escalate P0–P2 |
| Q1 qualification plan | Future `gwz-core/dev-docs/GwzGitLibraryQualification.md` | ≤180 document lines; 0 runtime/tool/test | Code/source-readiness; escalate P0–P2 |

The integrator owns only the C1 declaration. It must be a module addition in
the existing braced `cfg_if::cfg_if!` test section, with no standalone
conditional attribute reassociation. H1 reuses the already-wired
`path_characterization.rs`; it must not alter helper visibility or add a second
test-module declaration. No package may add a production import, method, error,
option, dependency, or native type.

## C1 — commit/tag identity, configuration, and message evidence

The existing `commit_tag_characterization.rs` is the accepted baseline: index
versus `all`, prepare/pre/post hook outcomes, lightweight versus annotated tag
forms, delete-hook observation, and configured OpenPGP failure/no-message
behavior are already observed there. C1 adds only the omitted rows below; it
does not reclassify those observations or choose an internal implementation.

Characterize these exact rows, with signing disabled and an empty hooks path:

* C-ID: an isolated global identity is overridden by repository identity;
  `GIT_AUTHOR_NAME/EMAIL` and `GIT_COMMITTER_NAME/EMAIL` override repository
  identity independently. Assert both stored signatures, not command output.
* C-DATE: distinct fixed author/committer timestamp and timezone environment
  values survive into the commit. Repeat annotated tag creation to observe
  tagger identity/date source, then contrast a lightweight tag's object kind.
* C-MSG: `-m` message bytes under default, `commit.cleanup=strip` and
  `commit.cleanup=verbatim`, including blank lines, trailing whitespace and
  comment lines. No editor should be required for an explicit nonempty message.
* C-EMPTY: empty/whitespace message rejection under default cleanup, with
  unchanged HEAD and staged blob/index content. Assert backend error category;
  record any message-file side effect without promising blanket rollback.

Use fresh repositories or staged tracked edits so an unrelated empty-tree
rejection cannot substitute for message rejection. Snapshot relevant refs,
index/worktree bytes and produced object data before/after each failure. Global
config means a fixture-owned file selected in the child, never user config.
Operation-provided identity is pending because the existing contract has no
such arguments. New hook/editor/signing/filter cases, ref-hook/reflog phases,
races, interruption, SHA-256 mutation and successful signer formats remain
explicit later packages. Existing L3-A hook/signing evidence is not rerun or
expanded here. This narrowing keeps C1 inside its small evidence budget.

Fixtures use native `git2` construction where practical and run the current
backend/oracle in a self-reexecuted child. The child clears ambient
`GIT_*`, `GPG_*`, editor, HOME/config, cwd-sensitive, and unrelated variables;
it sets only a known `PATH`, marker, and case-specific variables. Repository
config pins identity, an empty hooks path, disabled signing, and auto-GC.
Fixture config and any helper files live outside the worktree;
on Windows, use unique `D:/gwz-tests/<name>` roots. The child must assert that
the exact selected test ran, not accept zero tests. No process-global env/cwd
mutation, user keyring, or personal key is permitted.

Acceptance is a deterministic row table with Git 2.52.0 output, return/error
category, object/ref/index/worktree/reflog aftermath, and fixture environment.
It is a State checkpoint only: no public API or mutation policy is frozen.

## H1 — history attribute and worktree context

Extend the existing `path_characterization.rs` module by at most 400 lines,
using its native Git oracle, `build_attr_path_history`, and `super::*` fixture
helpers. Add direct member and bare-repository rows only:

* H-MEMBER: a direct member path from workspace root, set/unset/value/unspecified
  attribute forms; assert exact selected target and rerooted magic envelope,
  native ordered IDs and current cursor IDs separately.
* H-BARE: a bare member cloned from the same committed-attributes fixture;
  compare positive and unspecified attribute results with a native bare oracle.
* H-INFO: the bare member's `info/attributes` overrides, repeating positive and
  unspecified forms and asserting the exact changed selection.

Read-only snapshots cover refs, object/index state and attribute metadata after
fixture construction. All remote URLs, if any, are fixture-local. Bare/member
results are observations, including refusal if that is what the backend does.
No new top/exclusion, member-cwd, shallow/promisor/missing-object, ordering or
SHA-256 traversal rows are implied; these remain explicit later evidence.

Record that direct member rows do not prove member fan-out: current routing can
synthesize `.` and lose magic, so fan-out remains a separate unresolved issue.
Do not add traversal, cursor, cancellation, ordering, or pathspec API. H1 is a
Code characterization checkpoint comparing exact oracle sequences and errors;
it does not select `gix-pathspec` or any replacement walk.

Use the same hermetic child rules as C1, including exact-test execution and
Windows fixture roots. Keep deterministic fixtures and fixed replay data; do
not add a generator or broaden the existing history option inventory.

## Q1 — qualification and activation-readiness plan

The future qualification document must refresh, without changing manifests:

* every consumer and lock source: core `git2 0.21` with `https`, `ssh`, and
  `unstable-sha256`, independent repo-inspect/local-testrepo/CLI consumers,
  direct `libgit2-sys = 0.18.8`, `gwz-git`'s local fork with vendored
  libgit2 and SHA-256 features, and the indirect `gwz-py` core consumer with
  its independent `Cargo.toml`/`Cargo.lock`. Pin Python revision
  `d07d55dacb1725d9306be9c04d157ac29a78e000`; its current graph resolves
  git2 0.21.0 and sys 0.18.5+1.9.4. Resolve/inspect that graph independently
  and include it in provider uniqueness and native-platform/package gates;
* distinguish root-workspace CLI resolution from the CLI member's standalone
  lock. Pin CLI `7db07bbdefd2897c07fd0f9e550bf032bd8b1314`; its standalone
  lock also records sys 0.18.5+1.9.4. Enumerate all active members from GWZ and
  inspect direct and indirect dependencies, documenting nonconsumers too.
  A differing or unqualified consumer keeps activation pending;
* exact source-byte proof using
  `python3 tests/transport_native/prove.py --git2-source ../git2-rs`, before
  and after tests, plus lock/provider uniqueness and runtime libgit2 version;
* Rust 1.95 locked offline builds and both SHA-1/SHA-256 rows for Windows
  x86_64 MSVC, macOS aarch64/x86_64, and Linux GNU aarch64/x86_64. Unexecuted
  rows remain pending. The source-inspected proof path normalization concern on Windows needs
  a targeted admission regression and correction before qualification, not an excuse to infer parity;
* clean remote-only reconstruction, archive/source modes, package licensing,
  and the no-duplicate-native-`links` provider check;
* a separate all-consumer, all-feature, native-platform, publication, and
  production-activation gate. No Q1 result may claim the current production
  graph uses the corrected fork or remove the existing fallback.

## Stop conditions and review order

Stop and re-scope on any new public method/type, CLI/option/protocol delta,
production byte, dependency/lock change, process-global mutation, network
access, personal signer/key use, unbounded fixture growth, or a claim that
another OS/format is covered by macOS evidence. Treat a disagreement with
Git as a characterization finding requiring a precise reproduction, not an
automatic product defect or policy change.

First accept this consolidated scope through Code+State. Then run C1 State,
H1 Code, and Q1 Code/source audit against committed results. Surface review is unnecessary while
no public API or documentation example changes; any proposed API, durable
mutation kernel, production switch, or fallback removal requires a new design,
Surface/API review, and its own implementation gate.

This scope owns at most 200 document lines in this file, the three package
paths/ceilings above, one test-only wiring line and root checkpoint/review
records. Integrator owns all commits and the root checkpoint. Production APIs
remain read-only. Evidence is public regression code plus concise reports;
any campaign-only raw runs follow EVIDENCE.md in the private evidence member.

Accepted at core `6586768396886fe1aeb1371bbd3064377cfa70ec`, root
`3589aa05092abb7c38d90747384954565510301f` after retained
[Code](../../dev-docs/GwzGitLibraryNextPackages-ReviewCode-1.md) and
[State](../../dev-docs/GwzGitLibraryNextPackages-ReviewState-1.md) GO.
One scope P2 (omitted Python graph) closed in one text-only remediation.
This accepts package execution only; results require their named reviews.

C1/H1/Q1 results accepted at core `deba48c93a04e6aaf0bab36066b12d1547d5469e`,
root `9fc664de8389ea334f36bc41135cd59448893e05`, after scoped
[State C1](../../dev-docs/GwzGitLibraryEvidence-ReviewState.md) and
[Code H1/Q1](../../dev-docs/GwzGitLibraryEvidence-ReviewCode.md) GO.
Two nonblocking P3 wording/precision findings receive owner corrections in
the evidence reports; no runtime delta or blocking implementation finding.

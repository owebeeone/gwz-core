# Native local-fetch correction and isolated Rust integration

Date: 2026-09-20. Status: **DRAFT; implementation requires Code/State GO**.
Authority: [plan](GwzNoFallbackPlan.md), [first checkpoint](GwzNoFallbackCheckpoint.md),
[local investigation](GwzNoFallbackLocalFetchInvestigation.md), and
[binding port](GwzNoFallbackBindingPort.md). This is the separately budgeted
successor to L1-A/L2-A. It does not authorize production activation.

## Intended behavior

A receiver ref naming a tree/blob or a tag ultimately naming a tree/blob is not
a commit-negotiation hint. Local fetch must skip it even when the named object
exists in the source. Missing source-side receiver hints remain ignored as today.
Wanted objects still transfer, including explicitly requested non-commit objects.
Other lookup, parse, allocation, pack and publication failures still propagate.

The defect is a return code compared with the error class `GIT_ERROR_INVALID`
in `foreach_reference_cb`. Replace that comparison with the applicable negative
return codes from `git_revwalk_hide`: `GIT_EINVALIDSPEC` and, if needed for tags,
`GIT_EPEEL`. Verify their origin through object peeling before selecting the final
condition. Do not suppress errors by broad class or message matching. Preserve
existing missing-object behavior; this package makes no stricter missing-tag-
target guarantee than stock libgit2.

No change to FETCH_HEAD semantics, ref publication atomicity, wanted-ref policy,
GitBackend interfaces, CLI/core messages, SSH, credentials, or pooling. No removal
of production fallbacks. Those remain subsequent accepted-plan gates.

## Packages and ownership

The lane owner owns these sequential packages; helpers may draft owned test files.
All unlisted runtime files remain read-only. No pushes, PR publication or tags.

**N1 — native correction.** Create `codex/local-fetch-noncommit` from C fork main
`0551dfd4ad989b6a3d5683c0d4cf326c6efef929`; modify only
`libgit2/src/libgit2/transports/local.c` and
`libgit2/tests/libgit2/network/fetchlocal.c`. Reuse Clar and its sandbox/cleanup.
Tests must force an actual transfer with a wanted commit absent in the receiver,
while a receiver hint's object is present at the source. Cover direct tree/blob,
annotated non-commit tags, commit/tag-to-commit controls, receiver-only object,
explicit wanted non-commit objects and a genuine malformed-object error. Check
requested destination OIDs and object availability, plus unchanged hint refs.
Run regression red before production correction, then the fetchlocal suite and
normal C suite. Record environmental failures separately from pass claims.

Backport the exact two-file correction onto a separate
`codex/local-fetch-noncommit-1.9.7` branch from
`49e408b3208bc3093757a1c2db938d3590f3f412`; repeat the focused and normal suites.
Source import is not an upgrade of the qualified native baseline. Modified C
control bodies have braces. Existing unrelated style is not migrated.

**N2 — isolated Rust/native alignment.** On the existing Rust candidate branch,
import `libgit2-sys` verbatim from upstream git2-rs commit
`6c93812dbc1c34aef6e6464a645545b4a4299807` (published 0.18.8+1.9.7).
Retain the 0.21.0 Rust release and the reviewed two-file binding extension.
Owned files: `git2-rs/libgit2-sys/{CHANGELOG.md,Cargo.toml,build.rs,lib.rs,libgit2}`,
`git2-rs/Cargo.toml`, `git2-rs/.gitmodules`. Root manifest selects exact0.18.8
by path; C submodule URL becomes the operator's fork and gitlink becomes the
reviewed N1 backport. The sibling C member and nested submodule are separate
checkouts. No symlink, custom native build layout or product manifest edit.
Unchanged imported upstream conditional sections are baseline debt, not a claim
of completing the workspace's conditional-scope migration.

Extend only the existing proof in `gwz-core/tests/transport_native/`:
`prove.py`, `test_prove.py`, `README.md`, `Cargo.toml`, `binding-pin.json`, and
new `tests/local_fetch.rs`. Preserve archive qualification against registry sys.
Source qualification admits the release tree plus the existing binding hashes,
exact upstream sys tree, the explicitly changed manifest/submodule metadata,
and the exact patched C commit. Verify modes, paths and bytes from Git objects,
including native submodule contents; copy verified bytes to an isolated directory
before Cargo. Do not trust export-ignore, dirty checkout metadata or system C.
The updated lock guard allows only the recorded git2 and sys provenance changes;
versions, dependencies, features and all unrelated package entries remain pinned.
The source fixture forces vendored libgit2. The Rust local-fetch regression
runs against the patched source; stock/archive mode must continue to characterize
its known failure rather than assert success. Existing seven native binding tests
remain unchanged and pass in both modes.

Unpublished C commits may be materialized from the sibling checkout locally;
report that dependency explicitly. A clean remote-only clone is not qualified
until the forks are published in a later authorized step. Print/record C and sys
source identities. Test admission rejects changed C source, absent/mismatched
submodule, wrong sys version/source and unrelated dependency drift. Public
proof inputs retain documented defaults and lifecycle; changed fixture mode gets
Surface review. No new public product command/API is introduced.

## Budgets and gates

| Package | New/changed production lines | Test lines | Tool lines | Doc lines | Files |
|---|---:|---:|---:|---:|---:|
| N1, per C branch | 12 | 260 | 0 | 0 | 2 |
| N2 integration | 12 manifest/metadata | 220 | 220 | 220 | 14 |

N2's exact upstream sys import (currently115 insertions/21 deletions, five paths)
is recorded separately as baseline alignment; native gitlink is not a copied
source fork. No unrelated upstream import. No production owner or protocol delta.
Stop/re-scope on new ownership, >120% growth or an unlisted changed path.
This document and root checkpoint/review artifacts are owner bookkeeping outside
code budgets. N1 correction must be qualified before N2 incorporates it.

Design gate: retained Code/State peer-blind reviews of the committed document.
Acceptance: retained Code/State on the settled implementation tuple; Surface on
changed proof help/README. Interior N1 evidence may proceed into N2 after green
native gates under this frozen scope; final acceptance covers both exact branches
and the source/native identity. Any P0/P1/P2 blocks; merged remediation, max two
architectural rounds. Record local platform/toolchain, actual executed results,
source import identity and budget actuals. This is local isolated qualification,
not all-consumer, five-platform, release or production activation evidence.

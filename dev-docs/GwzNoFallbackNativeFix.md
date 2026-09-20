# Native local-fetch correction and isolated Rust integration

Date: 2026-09-20. Status: **design accepted at core `e45025d622c0d5993d4daba609a68d1cee463c45`,
root `179231adbab23144b20b62428ca56525edc7c9c3`, after retained Code/State GO**.
Reports: root `GwzNoFallbackNativeFix-DesignReview{Code,State}-1.md`.
One P2 scope-claim correction closed; P3 fetch-visible oracle corrected below.
Implementation **accepted** at core `01d6f6624472620c215693243f7ac3865aeb31a4`,
root `62c2f122c28ededdefb7af32058f4b058b018dcb`, Rust
`4c1caabbce7d56426c763dd94114052302b23e4c`, C backport
`b172e3d187a4b6866fd9f696f40a1b8e7f56d348`, upstream-facing C
`fe618d0f5de9e506b9714643afc42d2fcba6e984` after retained Code/State/Surface GO.
Reports: root `GwzNoFallbackNativeFix-Review{Code,State,Surface}.md`.
This accepts the bounded native correction and isolated integration only.
Authority: [plan](GwzNoFallbackPlan.md), [first checkpoint](GwzNoFallbackCheckpoint.md),
[local investigation](GwzNoFallbackLocalFetchInvestigation.md), and
[binding port](GwzNoFallbackBindingPort.md). This is the separately budgeted
successor to L1-A/L2-A. It does not authorize production activation.

## Intended behavior

A receiver ref naming a tree/blob or a tag ultimately naming a tree/blob is not
a commit-negotiation hint. Local fetch must skip it even when the named object
exists in the source. Missing source-side receiver hints remain ignored as today.
Wanted objects still transfer, including explicitly requested non-commit objects.
Errors other than the existing `GIT_ENOTFOUND` suppression and the two newly
recognized noncommittish codes continue to propagate.

The defect is a return code compared with the error class `GIT_ERROR_INVALID`
in `foreach_reference_cb`. Replace that comparison with the applicable negative
return codes from `git_revwalk_hide`: `GIT_EINVALIDSPEC` and, if needed for tags,
`GIT_EPEEL`. Verify their origin through object peeling before selecting the final
condition. Do not suppress errors by broad class or message matching. Preserve
existing missing-object behavior; this package makes no stricter missing-tag-
target guarantee than stock libgit2. `GIT_ENOTFOUND` also represents a parsed
tag declaring the wrong target type; that pre-existing suppression is preserved
and explicitly characterized here. N1 does not claim comprehensive malformed-
object rejection. Separate type-consistency hardening remains a prerequisite
to deciding production fallback removal, not part of this upstream bug fix.

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
explicit wanted non-commit objects and a syntax-malformed tag error (parser `GIT_EINVALID`, fetch-visible
`GIT_ERROR` with tag error detail). Also characterize a parsed
tag with mismatched declared/actual target type and a missing tag target: both
retain stock suppression; record this limitation without calling it corrected. Check
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
`git2-rs/Cargo.toml`, `git2-rs/.gitmodules`. Root manifest selects exact 0.18.8
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

N2's exact upstream sys import (currently 115 insertions/21 deletions, five paths)
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

## Execution evidence (accepted tuple above)

Host: macOS arm64; Apple Clang via CMake 4.3.3, Rust 1.95.0, Python 3.10.15.
N1 upstream-facing commit: `fe618d0f5de9e506b9714643afc42d2fcba6e984`, branch
`codex/local-fetch-noncommit`, parent `0551dfd4ad989b6a3d5683c0d4cf326c6efef929`.
Production delta is one replaced condition; 243 added Clar test lines.
Neutral fixtures exercise shared tree/blob/annotated hints, commit controls,
receiver-only objects, explicit tree/blob/tag wants, syntax-malformed failure
and preserved mismatched/missing-target behavior. All hint refs remain unchanged;
syntax failure leaves the wanted destination absent.

C commands use an external build directory:

```sh
cmake -S libgit2 -B /tmp/gwz-libgit2-main-build -DBUILD_TESTS=ON \
  -DBUILD_CLI=OFF -DUSE_SSH=OFF -DUSE_HTTPS=SecureTransport -DBUILD_SHARED_LIBS=OFF
cmake --build /tmp/gwz-libgit2-main-build -j8
/tmp/gwz-libgit2-main-build/libgit2_tests -snetwork::fetchlocal
ctest --test-dir /tmp/gwz-libgit2-main-build -R '^offline$' --output-on-failure
```

Unmodified main offline suite passed (145.72s); patched focused suite and normal
offline suite passed (145.88s). The initial HTTPS=OFF configuration was invalid
with the default NTLM crypto choice; the recorded SecureTransport configuration
builds successfully. No source workaround was made for that configuration error.
The standalone Rust archive proof passes eight tests, including the original
seven binding tests and a four-row characterization that still expects stock
1.9.7's direct and annotated noncommit-hint errors. Ten Python admission/lock
guards pass. These results do not yet qualify the patched source integration.

N1 backport commit: `b172e3d187a4b6866fd9f696f40a1b8e7f56d348`, branch
`codex/local-fetch-noncommit-1.9.7`, parent `49e408b3208bc3093757a1c2db938d3590f3f412`.
Added/deleted lines match the upstream-facing patch exactly (only base blob IDs
and context offsets differ). Final tests applied before the production fix on
1.9.7 failed twice: receiver hints returned `-19` (EPEEL), explicit noncommit
wants with a shared tree hint returned `-12` (EINVALIDSPEC). Applying the one-line
condition correction made the complete fetchlocal suite pass. Build configuration
is identical to main, using `/tmp/gwz-libgit2-197-build`.

### Baseline comparison raised during implementation

Operator asked about main versus 1.9.7. They are divergent development/maintenance
lines (148 commits unique to 1.9.7, 548 unique to the checked-out main); do not
interpret main's still-1.9.0 version macro as its API compatibility level.
Comparing `src/libgit2` and `include/git2` gives 160 changed files, 6219 insertions
and 2353 deletions. Main adds pathspec-filtered revwalk, richer commit/create/amend
and signing callbacks, reftable, and unconditional SHA256 with changed OID APIs.
It also avoids creating nonexistent FETCH_HEAD when update is suppressed, while
retaining the receiver-hint bug fixed here. These are source observations, not
claims of Git semantic parity or compatibility with our Rust bindings.
Assess main's commit/history APIs before implementing the remaining lanes; this
package retains 1.9.7 solely for the already-qualified isolated integration.
No baseline upgrade is selected by that comparison.

The patched 1.9.7 normal offline C suite also passed (145.25s).
N2 Rust candidate is `4c1caabbce7d56426c763dd94114052302b23e4c`; the exact four
upstream sys files were imported from `6c93812dbc1c34aef6e6464a645545b4a4299807`.
The sys gitlink now selects `b172e3d187a4b6866fd9f696f40a1b8e7f56d348` and the
parent path dependency selects exact 0.18.8. Initialization used the sibling C
checkout for unpublished objects; submodule metadata and origin are synchronized
to `https://github.com/owebeeone/libgit2`. This is an unpublished local candidate,
not proof that a fresh remote-only checkout can reproduce it today.

Final source proof passes eight native tests, including the four-row fixed fetch
matrix and all unchanged binding tests. Its output identifies exact sys/C pins;
Cargo replaces only git2/sys provenance while the lock guard preserves versions
and every other dependency. Both source and archive builds assert vendored C and
native 1.9.7. Ten Python guards pass; rustfmt and changed-range whitespace checks
pass. Git-object reads are batched, with checked object headers/lengths/modes and
exact checkout file admission before building the isolated copy.

N2 actuals: two manifest/metadata lines plus native gitlink; upstream sys import
114 additions/20 deletions across four ordinary files (the fifth imported path
is the separately patched C gitlink); proof 136 added/deleted tool lines,
96 Python test changes plus 123 Rust test lines, 36 README changes, four pin lines.
N2 touches 13 files across core/fork excluding this document and owner reports.
Both packages remain within the accepted 120% stop boundary; no unlisted runtime
path or production owner changed. Existing upstream conditional sections remain
baseline debt. Production manifests, fallback branches, CLI/core and transport
runtime remain unchanged. No PR, push, tag or release was published.

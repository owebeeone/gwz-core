# gwz-git G0 implementation checkpoint

Date: 2026-09-21. Status: **implemented local candidate; acceptance review pending**.
Controlling [design](GwzGitLibraryDesign.md) and [API](GwzGitLibraryApi.md)
were accepted at core `3efc1a79a1e5044b6e2495ed392a5c43d8f90b64`.

## Scope and composition

G0 implements only the explicit repository/read-commit foundation. No core or
CLI dependency switch, fallback removal, transport activation or publication.
The integrator provisioned local member `mem_gwz_git` at `gwz-git` through
`gwz repo create gwz-git`. It has no remote. Root Cargo excludes this independent
Cargo workspace; its path dependency uses the accepted fork with network
features disabled, unstable SHA-256 enabled and vendored native code required.

Required source identities, verified before implementation testing:

| Source | Identity |
| --- | --- |
| git2-rs | `4c1caabbce7d56426c763dd94114052302b23e4c` |
| libgit2-sys | 0.18.8+1.9.7, qualified N2 composition |
| C sibling and nested submodule | `b172e3d187a4b6866fd9f696f40a1b8e7f56d348` |

The path manifest is not an immutable source pin. Source-byte qualification
must pass both before and after library tests. Lockfile/source graph inspection
and the library's runtime native-version assertion complement that proof.
No system-native or alternate registry provider is admitted.

## Verification record

From `gwz-core`, before G0 implementation tests:

```sh
python3 tests/transport_native/prove.py --git2-source ../git2-rs
```

Passed exact source-byte admission and all eight native binding/fetch tests.
The qualified composition remains the reviewed N2 source, with libgit2 1.9.7
vendored. The same proof passed again after library tests (eight tests), with
unchanged Rust/sys/C source identities and byte admission.

Executed library commands (from `gwz-git`, build products outside the member):

```sh
CARGO_TARGET_DIR=/tmp/gwz-git-g0-target cargo +1.95.0 fmt --check
CARGO_TARGET_DIR=/tmp/gwz-git-g0-target cargo +1.95.0 check --locked --all-targets
CARGO_TARGET_DIR=/tmp/gwz-git-g0-target cargo +1.95.0 test --locked
CARGO_TARGET_DIR=/tmp/gwz-git-g0-target cargo +1.95.0 clippy --locked --all-targets -- -D warnings
```

All commands passed on macOS arm64. Eleven integration tests passed (four
foundation, seven native-baseline), plus one compiled usage example and two
compile-fail ownership doctests. No ignored tests. The initial test-first run
failed with unresolved `gwz_git` before source implementation; initial locked
run required generation of the new lockfile. Draft implementation corrections
included exact ID error classification, native signature parsing and use of raw
ODB bytes to preserve embedded NUL in messages. An interim owner compile caught
an incomplete private error-constructor edit; the final check and clippy pass.

Owner audit strengthened the same tests with malformed/full-width ID cases,
missing-object code/class/message retention, explicit Git-directory opening,
hostile ambient GIT_DIR in the no-Git child, full environment comparison and
use of records after cross-worker repository destruction. These passed without
further product code changes. Source uses no conditional attributes; Rust
format/check/clippy cover both ordinary cfg! branch bodies, including Windows
fixture-path selection. No legacy native-source style migration is claimed.

Cargo metadata and lock inspection show exactly one local git2 0.21.0 and local
sys 0.18.8+1.9.7 provider. Selected features are git2 `unstable-sha256` /
`vendored-libgit2` and sys `unstable-sha256` / `vendored`; no SSH/HTTPS feature.
The native-baseline test independently asserts runtime `(1, 9, 7)` and vendored.

Product regression fixtures and tests live in the library. This package creates
no raw evidence campaign or bespoke harness. The implementation is 326 Rust
source lines in five cohesive files, within the 600-line ceiling; 11 maintained
library files plus generated Cargo.lock, within the 12-file ceiling. Tests total
568 Rust lines. Root changes are generated membership plus one exclusion;
core changes are this evidence record and API-guide implementation status only.

Native execution available here is macOS arm64 only. macOS x86_64, Linux arm64
and x86_64, and Windows x86_64 MSVC remain pending; local G0 candidate acceptance
must not be described as all-platform or production qualification. Both fork
commits are unpublished; remote-only reconstruction is still unavailable.

## Remediation verification — 2026-09-21

Initial Code review found two P2 defects and one P3 contract excess; State and
Surface returned GO. The [bounded scope amendment](../../dev-docs/GwzGitLibraryG0-RemPlan.md)
received retained Code/State GO before the correction. Earlier measurements
above describe the original candidate, not this superseding composition.

New Rust source: `ce78628308e11b4e8901d5061602619109bce21a`, descending
from the prior pin and changing only `src/error.rs` (24 added / 45 deleted lines,
including tests). Existing per-remote files and C gitlink are byte-identical.
The raw getter now preserves every stored native class and callback replay uses
that value; the safe enum conversion is unchanged. C remains `b172e3d187a4b6866fd9f696f40a1b8e7f56d348`.

Library tree/ordered-parent fields now come from validated raw ODB headers,
independent of shallow/graft traversal rewriting. Native validation/signatures
are retained. Error/NativeDiagnostic expose only their accepted traits.

The new shallow/graft and malformed-graft tests failed on the old source:
empty parents instead of two stored parents; class 0 instead of native class 36.
They pass after correction for SHA-1/SHA-256, preserving traversal metadata.
Full fmt/check/test/clippy pass on macOS arm64: **13 integration tests** (4 + 9),
**7 documentation checks** (one compiled usage example, six compile-fail trait
checks), none ignored. Corrected totals: **367 source lines, 652 test lines**,
11 maintained files plus generated lock. No conditional attributes introduced.

Updated three-file patch and hashes passed the unchanged qualification runner:
**9 native tests before and after library checks**, **9 archive-mode tests**,
and **10 Python admission guards**. Archive mode retains stock C and its
expected local-fetch failure characterization; source mode proves the C fix.
The raw-class/callback-replay unit test passed in a verified isolated fork copy
for classes 0, 1, 34, 35, 36, 12345 and -1. Its first offline attempt lacked the
upstream workspace's curl dependency; the isolated retry fetched dependencies
and passed (one test, 227 filtered). This supplemental unit run does not replace
the locked source/archive proofs or change any member manifest/lock.

Final metadata still shows exactly one local git2 0.21.0 and sys 0.18.8+1.9.7,
the same vendored SHA-256 features, no SSH/HTTPS. Library/core/fork manifests and
locks are unchanged by remediation. Native C identity, source admission runner,
and production activation remain unchanged. All other native-platform and
publication gates above remain pending.

## Review and next boundary

Corrected implementation awaits retained Code/State re-verdict, plus Surface
on revised qualification instructions. Original Code findings remain open until
that reviewer verifies closure. G0's 600 production-line / 12 maintained-file
budget remains controlling. Later L1 hardening and L3/L4 characterization/design
retain separate packages; G0 does not freeze their operation APIs.

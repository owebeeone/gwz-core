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

## Review and next boundary

Settle the implementation tuple, then retained Code/State plus Surface on README
and API examples. Blocking findings require original-reviewer closure. G0's
600 production-line / 12 maintained-library-file budget remains controlling.
Later L1 hardening and L3/L4 characterization/design retain separate packages;
this checkpoint does not freeze their operation APIs.

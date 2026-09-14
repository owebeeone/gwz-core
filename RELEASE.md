# Releasing gwz-core

gwz-core is the library + protocol crate. It depends on nothing else in this repo
set, so it has **no release branch** — there is no dev-vs-release dependency-source
split to manage. **Release tags are cut directly off `main`.**

## Release script

`scripts/release.py` automates the cut on `main` for a given tag `vX.Y.Z`:

1. Gate the tree: `python protocol/regen.py --check`, `cargo fmt --check`,
   `python scripts/run_tests.py`, `cargo clippy` (same bar as CI). If rustfmt fails,
   run `cargo fmt` and commit the formatting changes.
2. Bump `version` in `Cargo.toml`, commit `chore(release): gwz-core X.Y.Z`.
3. Tag that commit `vX.Y.Z` (lightweight; never moves an existing tag).

In the same commit as the product version bump, the script also advances the
internal `0.0.N` line that the fourteen crates under `crates/` share — their own
`[package].version` and every internal dependency edge in gwz-core's manifest and
in theirs (see [dev-docs/GwzCratesIoPlan.md](dev-docs/GwzCratesIoPlan.md) D2). On the
exact commit it is about to tag it then runs the crate-version lockstep gate against
that tag and packages every published crate, so a later `--no-verify` upload rests on
a package cargo has really assembled; publishing to crates.io itself happens in CI
(plan D5, Phase 2), never in this script.

Requires a clean working tree — land feature work first. If the protocol schema
(`protocol/gwz.taut.py`) changed, regenerate and commit **before** running the script:

```bash
python protocol/regen.py    # writes src/protocol/generated.rs, src/cbor.rs, protocol/corpus/
python protocol/regen.py --check   # CI-style: verify only, no writes
```

Never hand-edit generated protocol output.

```bash
python scripts/release.py vX.Y.Z              # verify + bump + commit + tag (no push)
python scripts/release.py vX.Y.Z --push       # also push main + tag to origin
```

## Release order

**Always release gwz-core before gwz-cli, and let its crates.io publish job finish first.**
gwz-cli's `release` branch pins `gwz-core = "=X.Y.Z"` from crates.io, not the git tag:
[gwz-cli/scripts/release.py](../gwz-cli/scripts/release.py) checks that the gwz-core tag exists
(its parity tests read gwz-core's fixtures from a clone at the tag) and then waits for gwz-core
`X.Y.Z` on crates.io before it reconciles the `release` branch.

A full release goes out in this order:

1. **taut-shape**, only when gwz-core needs a change from it. gwz-core depends on `taut-shape`
   from crates.io, so that change is released from taut-shape-rs first.
2. **gwz-core**: land on `main`, run `python scripts/release.py vX.Y.Z --push`, then publish the
   GitHub release for `vX.Y.Z`, which runs `.github/workflows/release.yml`.
3. **gwz-core's crates.io publish job** in that run must finish (see below).
4. **gwz-cli**: `python scripts/release.py vX.Y.Z --push` in gwz-cli, then its GitHub release,
   which builds the binaries and publishes the `gwz` crate (see
   [gwz-cli/RELEASE.md](../gwz-cli/RELEASE.md)).
5. **gwz-py** at the same tag (see [gwz-py/RELEASE.md](../gwz-py/RELEASE.md)). Its `release`
   branch still pins gwz-core by git tag.

## The crates.io publish job

The `publish` job of `.github/workflows/release.yml` publishes to crates.io; nothing publishes
from a laptop. The Linux verification job (`verify`) gates it, and the Windows leg does not. It
runs `scripts/publish_crates.py`, which checks the tag against every manifest version and then
publishes the thirteen published internal crates and gwz-core, in the dependency order derived
from the manifests. It skips a version crates.io already holds, and waits until each new version
is visible before it publishes the next crate. When crates.io refuses a new crate name because
too many were published in a short period, it waits a little over ten minutes and tries that
crate once more. It authenticates only by Trusted Publishing: each crate has a trusted publisher
for this repository, workflow `release.yml` and environment `crates-io`, and is set to
trusted-publishing-only on crates.io, and the environment keeps no token secret. Any other
publish error stops the job at that crate. Retry a partial run by dispatching `release.yml` with
the tag (Run workflow, `tag` = `vX.Y.Z`); the crates already published are skipped.

The internal crates are published so that gwz-core can be built from crates.io. Publishing does
not stabilise their API: each is internal to GWZ, versioned in lockstep, with no compatibility
promise beyond gwz-core's own; depend on `gwz-core`.

A gwz-core built from crates.io reports `revision=unavailable dirty=unknown` in its build
provenance (the `core` line of `gwz --build-info`), with a digest of the packaged sources. A
build from git reports the commit, whether the checkout was dirty, and a digest of the
checkout's sources, so the two digests differ even for the same commit (plan D4).

## Manual process

If you prefer not to use the script, the steps are the same:

1. Land all changes on `main`; ensure green (`cargo fmt --check`, `python scripts/run_tests.py`,
   `cargo clippy`). Regenerate protocol output when the schema changed (see above).
2. Bump `version` in `Cargo.toml` (semver; an additive protocol/API change is a minor bump).
3. Commit, then tag that commit: `git tag vX.Y.Z` (tags are **off `main`**).
4. Push `main` and the tag.

## Downstream

**gwz-cli** pins the gwz-core release published on crates.io, `gwz-core = "=X.Y.Z"`, on its
`release` branch, and its release script waits for that version on crates.io before it
reconciles the branch; see [gwz-cli/RELEASE.md](../gwz-cli/RELEASE.md). **gwz-py** still pins
gwz-core by git tag on its `release` branch; see [gwz-py/RELEASE.md](../gwz-py/RELEASE.md).

## Slow compiler probes are manual-only

Release scripts and automatic workflows do not run the source-mutation compiler
suites. To investigate a change to architecture enforcement explicitly, run:

```sh
python scripts/run_compiler_tests.py          # both suites
python scripts/run_compiler_tests.py boundary
python scripts/run_compiler_tests.py privacy
```

GitHub Actions also has **Architecture compiler probes (manual)**, triggered only
with Run workflow. These tests are not a release prerequisite. The ordinary
source scan, Rust compilation, behavior tests and Clippy remain in the release
path. Do not add the mutation suites back to release or push/PR workflows.

### Repository factory migration

Use `python scripts/run_tests.py` for the normal suite. It runs the migrated
repository tests and root matrices with fake Git, then the remaining tests with
native Git in a separate process. The small repository contracts run in both
modes routinely. `python scripts/run_tests.py --compare` runs the complete
converted group against both backends explicitly.

For a focused native test, use `GWZ_TEST_GIT=real cargo test --locked --lib FILTER`
(on PowerShell, set `$env:GWZ_TEST_GIT = 'real'` first). An unset selector chooses
fake Git, so unconverted unit fixtures must use the native process while this
migration is in progress. Integration tests always compile the production
factory and cannot select fake Git.

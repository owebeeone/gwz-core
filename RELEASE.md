# Releasing gwz-core

gwz-core is the library + protocol crate. It depends on nothing else in this repo
set, so it has **no release branch** — there is no dev-vs-release dependency-source
split to manage. **Release tags are cut directly off `main`.**

## Process

1. Land all changes on `main`; ensure green (`cargo test`, `cargo clippy`, and — if the
   protocol changed — regenerate and check the corpus; see `dev-docs/GWZGitlinkPlan.md`
   notes / the taut workflow).
2. Bump `version` in `Cargo.toml` (semver; an additive protocol/API change is a minor bump).
3. Commit, then tag that commit: `git tag vX.Y.Z` (tags are **off `main`**).
4. Push `main` and the tag.

## Downstream

**gwz-cli** pins a gwz-core release by tag and builds against it on its `release` branch.
After you publish a new gwz-core tag, bump gwz-cli's `release` branch to pin it — see
[gwz-cli/RELEASE.md](../gwz-cli/RELEASE.md).

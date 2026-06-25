# Releasing gwz-core

gwz-core is the library + protocol crate. It depends on nothing else in this repo
set, so it has **no release branch** — there is no dev-vs-release dependency-source
split to manage. **Release tags are cut directly off `main`.**

## Release script

`scripts/release.py` automates the cut on `main` for a given tag `vX.Y.Z`:

1. Gate the tree: `python protocol/regen.py --check`, `cargo fmt --check`,
   `cargo test --locked`, `cargo clippy` (same bar as CI). If rustfmt fails,
   run `cargo fmt` and commit the formatting changes.
2. Bump `version` in `Cargo.toml`, commit `chore(release): gwz-core X.Y.Z`.
3. Tag that commit `vX.Y.Z` (lightweight; never moves an existing tag).

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

## Order relative to gwz-cli

**Always release gwz-core before gwz-cli.** gwz-cli's `release` branch pins gwz-core by
git tag; [gwz-cli/scripts/release.py](../gwz-cli/scripts/release.py) verifies that tag
exists on the gwz-core remote before it reconciles the `release` branch. If you run the
gwz-cli script first, it will fail until the gwz-core tag is pushed.

Typical sequence when both crates need a release:

1. **gwz-core** — land on `main`, then `python scripts/release.py vX.Y.Z --push`
2. **gwz-cli** — `python scripts/release.py vX.Y.Z --push` (same gwz-core tag; gwz-cli
   version is independent — see [gwz-cli/RELEASE.md](../gwz-cli/RELEASE.md))

## Manual process

If you prefer not to use the script, the steps are the same:

1. Land all changes on `main`; ensure green (`cargo fmt --check`, `cargo test`,
   `cargo clippy`). Regenerate protocol output when the schema changed (see above).
2. Bump `version` in `Cargo.toml` (semver; an additive protocol/API change is a minor bump).
3. Commit, then tag that commit: `git tag vX.Y.Z` (tags are **off `main`**).
4. Push `main` and the tag.

## Downstream

**gwz-cli** pins a gwz-core release by tag and builds against it on its `release` branch.
After you publish a new gwz-core tag, bump gwz-cli's `release` branch to pin it — see
[gwz-cli/RELEASE.md](../gwz-cli/RELEASE.md).

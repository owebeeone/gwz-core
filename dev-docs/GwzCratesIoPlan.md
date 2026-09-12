# GWZ crates.io Publication Plan

Status: **ADOPTED 2026-09-13** by the operator, without a separate review
round, from the draft of 2026-09-12 revised the same day after checking what
uv and ruff do and reading the bootstrap how-to. Written by Fable at the
operator's request ("we're going to publish them all"). No manifests, scripts
or workflows had changed at adoption. Implementation is chartered for
non-Fable agents (Opus builders) under the usual review loop.

## Goal

Publish `gwz-core`, thirteen of its fourteen internal crates (the dev-only
test fixtures crate stays unpublished) and the `gwz` command-line crate to
crates.io as part of the ordinary release, so that
`cargo install gwz` and `cargo add gwz-core` work without a git URL, and so
that the two placeholder versions the operator reserved today
(`0.0.0-bootstrap.1` for `gwz` and `gwz-core`) are superseded by real ones.
The GitHub releases, the installers and the PyPI wheels keep working exactly
as they do now; crates.io is an additional channel, not a replacement.

## 1. Facts this plan rests on

crates.io state on 2026-09-12:

- `gwz` and `gwz-core` exist as `0.0.0-bootstrap.1`, 1.3 KB placeholders
  published by owebeeone this morning, not yanked. How that was done, from
  an isolated `.github/bootstrap-crate/` package through a manual workflow
  with a seven-day token, is recorded in
  `gwz-dev/dev-docs/CrateBootstrapHowTo.md`; the two repository secrets it
  used were deleted after publication and no cargo credentials file exists
  on the operator's Mac.
- Trusted Publishing (OIDC from GitHub Actions, `rust-lang/crates-io-auth-action`)
  works only for a crate that already exists: the how-to records that the
  first publication of a new name needs an API token with permission to
  publish new crates. After the first version, a trusted publisher is
  configured per crate on crates.io and no token is needed again.
- What uv and ruff do with the same shape of workspace: every crate
  publishes. The product crates carry the product version (`uv` 0.12.13,
  `ruff` 0.16.7) while the internals share their own lockstep line
  (`uv-*` at 0.0.80, `ruff_*` at 0.0.13), each described as "an internal
  component crate". Their manifests pin
  `{ version = "0.0.80", path = "crates/..." }`. Publishing is a
  cargo-dist `publish-jobs` sub-workflow authenticated by Trusted
  Publishing, running a script that walks the workspace in dependency order
  with `--no-verify` (the dry run happens elsewhere in CI), on nightly only
  to raise the per-crate index wait to ten minutes because crates.io
  indexing "has been known to lag long enough to exceed" the sixty-second
  default. `taut-shape` is published
  at 0.9.1 (2026-08-26) by the same owner from `taut-shape-rs`, through a
  workflow that tests, runs `cargo package --locked`, checks tag and version
  agree, skips a version that is already present, and publishes with a
  `CARGO_REGISTRY_TOKEN` secret behind a `crates-io` GitHub environment.
- All fourteen internal names are free: `gwz-repo-contract`,
  `gwz-copy-contract`, `gwz-family-model`, `gwz-family-store-contract`,
  `gwz-work-detector`, `gwz-history-check`, `gwz-repo-factory`,
  `gwz-repo-inspect`, `gwz-refcopy`, `gwz-local-testrepo`,
  `gwz-family-store`, `gwz-local-import`, `gwz-workspace-install`,
  `gwz-local-disposal`.

crates.io rules that bind this plan:

- A published crate may not depend on a git source, nor on a path dependency
  without a `version`; the `path` may stay for local builds, the registry
  uses the version.
- Every crate is built in isolation at publish time against registry
  dependencies, so crates publish in dependency order and each must compile
  on its own. `cargo publish` waits for the index to show a crate before the
  next dependent can resolve it (cargo 1.66 and later).
- The uploaded package is limited to 10 MB. A publish is permanent; a
  version can only be yanked, never replaced.
- Publishing is rate limited per account, with defaults taken from the
  crates.io source: a brand-new crate name gets a burst of 5, then one more
  every 10 minutes; a new version of an existing crate gets a burst of 30,
  then one more every minute. Over the first limit the API answers "You have
  published too many new crates in a short period of time". Overrides exist
  and are requested by email, but this plan does not ask for one: the first
  publish waits instead (D5), and every later release is on the second,
  generous limit.

The gwz-core repository at 1.0.11:

- Fourteen crates under `crates/`, all at version 0.1.0, all
  `publish = false`, all GPL-2.0-only, edition 2024, rust-version 1.95, each
  with a description; none has a `repository` field, a README or keywords.
  Thirteen are runtime dependencies of gwz-core; `gwz-local-testrepo` is a
  path-only dev-dependency, which cargo drops from a published package.
- Dependency layers, which fix the publish order:
  1. `gwz-repo-contract`, `gwz-copy-contract`, `gwz-family-model` (no
     internal dependencies);
  2. `gwz-family-store-contract` (family-model); `gwz-work-detector`,
     `gwz-history-check`, `gwz-repo-factory`, `gwz-repo-inspect`,
     `gwz-local-testrepo` (repo-contract); `gwz-refcopy` (copy-contract);
  3. `gwz-family-store` (family-model, family-store-contract);
     `gwz-local-import` (family-model, repo-contract);
     `gwz-workspace-install` (copy-contract, family-model,
     family-store-contract, repo-contract);
  4. `gwz-local-disposal` (family-model, family-store-contract,
     repo-contract, work-detector);
  5. `gwz-core`; then `gwz`.
- gwz-core depends on `taut-shape` by git revision `7fd171b`, which is an
  ancestor of the published 0.9.1 tag (`70110e2`), not the tag itself.
  taut-shape 0.9.2, released 2026-09-12, differs from 0.9.1 only by
  publishing the crate README.
- The build script (`build.rs` with `build_support/provenance.rs`) records a
  git revision only when the manifest directory is its own repository, and
  otherwise emits `revision=unavailable dirty=unknown` with a digest of the
  source files present, so a registry package builds and reports honest
  provenance. `build_support/` must therefore be inside every package that
  uses it.
- The tracked tree is about 14 MB uncompressed: `src/` 9.9 MB, `scripts/`
  4.0 MB, `protocol/` 1.7 MB (excluding its virtual environment),
  `dev-docs/` 0.7 MB, `docs/` 0.2 MB. There is no `include` or `exclude`
  list today, so a package would carry all of it.
- No `[features]` in gwz-core or gwz-cli. gwz-core's release script bumps
  `Cargo.toml`, `BUILD.bazel` and `Cargo.lock` only.
- gwz-cli's `release` branch pins gwz-core by git tag; its release script
  verifies that tag on the gwz-core remote, regenerates the lock, and
  asserts the lock pins gwz-core through that git tag. Its parity tests read
  gwz-core fixtures through a sibling checkout the script clones at the tag.
- gwz-py's `release` branch also pins gwz-core by git tag; its publish
  workflow asserts that pin, and its provenance parity test compares the
  native extension's core provenance with the CLI's.

## 2. What is not confirmed

- U1. That the publish job's wait-and-retry across thirteen new names,
  about eighty minutes at the default limit, sits comfortably inside the
  job's `timeout-minutes` and the runner's six-hour ceiling (S2.2 measures
  it).
- U2. That gwz-core's package stays under 10 MB compressed once an
  `include` list is in place (measured in S1.4).
- U3. That gwz-core behaves identically on `taut-shape` 0.9.2 from the
  registry as on the pinned ancestor revision (the full suite in S1.3
  decides).
- U4. Whether stable cargo's sixty-second index wait is enough between
  crates on this account, or whether the publish loop must poll the index
  itself (uv needed nightly's `-Zpublish-timeout` at ten minutes).
- U5. Whether `cargo package` can verify an internal crate before its
  dependencies exist on the registry (it cannot build against unpublished
  versions; S1.4 uses `--no-verify` locally and relies on publish-time
  verification the first time).
- U6. Whether docs.rs builds each crate cleanly (the build script runs `git`
  and tolerates its absence; docs.rs has no network for `taut-shape` beyond
  the registry, which is fine).
- U7. Whether crates.io's ten-minute refill counts from the last successful
  publish or from the last rejected attempt; the loop assumes the former and
  waits a little longer than ten minutes so either answer works.

## 3. Scope decisions (proposed; the operator confirms or changes them at S0.1)

- D1. **Fifteen crates publish**: the thirteen runtime internals, gwz-core
  and gwz. `gwz-local-testrepo` keeps `publish = false`: it is a path-only
  dev-dependency that cargo drops from the published gwz-core, nothing
  downstream can use it, and publishing it would cost a first-publish slot
  and metadata to maintain. It still tracks the internal version line (D2)
  so the bump stays uniform, and publishing it later is a one-line change.
- D2. **Two lockstep lines, as uv and ruff do.** gwz-core and gwz carry the
  product version (1.0.12 next). The fourteen internals share their own
  `0.0.N` line, starting at 0.0.1 and incremented by one at every gwz-core
  release, each described as "an internal component crate of GWZ". Under
  cargo's rules every 0.0.x version is incompatible with every other, so a
  caret dependency `{ path = "crates/x", version = "0.0.1" }` resolves
  exactly one version without `=` pins, and the number itself tells a reader
  the crate is not a supported API. The release script bumps the product
  version in two manifests and the internal line in fourteen, plus every
  internal dependency edge. Alternatives rejected: giving the internals the
  product version, which would make `gwz-family-model 1.0.12` look like a
  stable 1.x API; and independent versions bumped by hand when a crate
  changes, which is where mistakes come from.
- D3. **taut-shape from the registry.** gwz-core depends on
  `taut-shape = "0.9.2"`; future changes gwz-core needs from taut-shape go
  through a taut-shape-rs release first, so the release order becomes
  taut-shape, then gwz-core, then gwz. The pinned ancestor revision is
  retired once the suite passes on 0.9.2 (U3).
- D4. **Explicit `include` lists.** Each package carries only what a
  downstream build needs: `Cargo.toml`, `src/`, `build.rs` and
  `build_support/` where present, `LICENSE`, `README.md`, and for gwz-core
  `protocol/gwz.taut.py` because the provenance digest hashes it when it is
  present. Tests, dev-docs, docs, scripts, corpus and evidence stay in git.
  The provenance digest of a registry build therefore differs from a git
  build of the same commit; that is expected and documented.
- D5. **Publishing runs in CI, never from a laptop.** A `publish` job in
  gwz-core's `release.yml`, after the Linux verification job succeeds, gated
  by the `crates-io` environment, checks that the tag and every manifest
  version agree, then publishes the fourteen crates of this repository in
  dependency order with `cargo publish --locked --no-verify` (the packaging
  dry run has already happened in the verification job), polling the
  crates.io index until each crate is visible before publishing the next
  (U4), and skipping any version already present so a retry is idempotent.
  When crates.io answers that too many new crates were published in a short
  period, the loop waits ten minutes and retries the same crate, at most
  once per crate still to publish, so the first run needs no rate-limit
  increase and no human pacing; any other publish error fails the job.
  `workflow_dispatch` with a tag retries a partial run. Authentication is
  Trusted Publishing through `rust-lang/crates-io-auth-action`, except for
  the first publication of a name, which crates.io only accepts with a
  token: the job reads a `CARGO_REGISTRY_TOKEN` environment secret when one
  exists and the OIDC token otherwise. That secret exists only for the S2.2
  rehearsal that creates the thirteen names, and S2.3 deletes it once every
  crate has a trusted publisher. The local release scripts gain packaging
  gates only (S1.5), no publish step.
- D6. **gwz publishes from its `release` branch on the registry core.** The
  `release` branch's dependency becomes `gwz-core = "=X.Y.Z"` from crates.io
  instead of the git tag; the release script's remote-tag check becomes a
  registry-version check that waits for the index, and its lock assertion
  checks a registry source at that version. The parity tests keep reading
  fixtures from the sibling clone at the tag. `cargo install gwz` becomes a
  documented install path next to the installer script.
- D7. **gwz-py stays on the git-tag pin.** Its publish workflow and its
  provenance parity test assume a git-built core; moving it to the registry
  is an open item (O1), not part of this plan.
- D8. **Publishing does not stabilise the internals' API.** Each internal
  crate's README and description say it is internal to GWZ, versioned in
  lockstep, with no compatibility promise beyond gwz-core's own; depend on
  `gwz-core`.
- D9. **Placeholders are yanked after the first real publish**, not before,
  so the names never sit unowned and `cargo install gwz` never resolves to
  the stub once 1.0.12 exists.

## 4. Phases

Foundational work first. Phase 1 makes the tree publishable without touching
the network; Phase 2 adds the CI publisher; Phase 3 moves the CLI; Phase 4
is the first release through the new path. Budgets are aspirational targets,
not limits.

### Phase 0: adoption (milestone: the plan is reviewed and adopted)

- **S0.1: review.** The operator confirmed D1 to D9 on 2026-09-13 and
  declined a review round. Output: this file's
  status line updated with a dated adoption note.

### Phase 1: a publishable tree (milestone: every crate packages locally, versions are in lockstep, and the suite is green on registry taut-shape)

- **S1.1: crate metadata** *(gwz-core, thirteen manifests plus thirteen
  short READMEs; ~330 lines)*. Remove `publish = false` from the thirteen
  published internals (the fixtures crate keeps it); add
  `repository = "https://github.com/owebeeone/gwz-core"`, `readme`, and
  `keywords`; write a README per crate (ten to twenty lines: what it is,
  the D8 sentence, a pointer to gwz-core). Version each at 0.0.1 and set the
  description to "an internal component crate of GWZ: ..." (D2).
- **S1.2: versioned dependency edges and the lockstep check** *(gwz-core
  manifests and `scripts/checks/check_crate_versions.py`; ~60 manifest lines
  plus ~150 lines of script and test)*. Every internal dependency line in
  gwz-core and in the internals gains `version = "0.0.1"` beside its
  `path`. The check script asserts: gwz-core's version matches the release
  tag, the fourteen internal versions are equal and on the `0.0.N` line,
  every internal edge names that version, no git dependencies anywhere,
  every published package has `repository`, `readme`, `license` and
  `description`, and `gwz-local-testrepo` alone keeps `publish = false`. It
  runs in CI and in the release script (S1.5). A test
  exercises it against a fixture tree with each violation.
- **S1.3: taut-shape from the registry** *(gwz-core `Cargo.toml`, one line,
  plus a full suite run)*. Replace the git revision with `"0.9.2"`, refresh
  the lock, run `python scripts/run_tests.py`, clippy and the regen check.
  Record the result (U3). If anything differs, the fix goes to taut-shape-rs
  as a 0.9.2 release first (D3).
- **S1.4: package hygiene** *(gwz-core and the internals; ~80 lines of
  `include` lists plus a size measurement)*. Add the D4 `include` lists;
  run `cargo package --list -p <crate>` and `cargo package --no-verify
  --locked -p <crate>` for all fourteen in dependency order (U5), record each
  package's compressed size and confirm gwz-core is under 10 MB (U2); confirm
  `build_support/provenance.rs` and `protocol/gwz.taut.py` are inside the
  gwz-core package and that a build of the extracted package reports
  `revision=unavailable`.
- **S1.5: release script gates** *(gwz-core `scripts/release.py`; ~120
  lines plus tests)*. The bump step writes the product version into
  gwz-core's manifest, increments the internal `0.0.N` line in the fourteen
  manifests and every internal dependency edge, then `BUILD.bazel` and the
  lock as today;
  `run_gates` adds the S1.2 check and the S1.4 packaging pass; the commit
  message stays `chore(release): gwz-core X.Y.Z`. The script never
  publishes (D5).

### Phase 2: the CI publisher (milestone: publishing a gwz-core GitHub release publishes fourteen crates in order, idempotently)

- **S2.1: the publish job** *(gwz-core `.github/workflows/release.yml`;
  ~120 lines)*. A `publish` job with `needs: verify` restricted to the Linux
  matrix leg, `environment: crates-io`, `permissions: contents: read`. Steps:
  check out the tag; assert the tag and all fourteen published versions agree (S1.2
  check); for each crate in the section 1 order, query
  `https://crates.io/api/v1/crates/<name>/<version>` and skip when present,
  else `cargo publish -p <name> --locked --no-verify`, then poll the index
  until the version resolves before moving on; on the new-crate rate-limit
  response, sleep ten minutes and retry that crate (D5); fail the job on any
  other publish error so the retry starts where it stopped. The job sets
  `timeout-minutes: 240` so the first run's waits fit. `workflow_dispatch`
  with a tag reuses the same job for retries. Authentication per D5:
  `rust-lang/crates-io-auth-action` with `id-token: write`, overridden by
  the `CARGO_REGISTRY_TOKEN` environment secret when it is set. The
  verification job gains the `cargo package --no-verify --locked` pass from
  S1.4 so `--no-verify` at publish time is honest.
- **S2.2: first publication and rehearsal** *(operator plus evidence)*.
  The operator creates a crates.io token scoped to publishing new crates with a seven-day expiry, exactly as the
  bootstrap how-to describes, and stores it as the `CARGO_REGISTRY_TOKEN`
  secret of the `crates-io` environment on gwz-core only. Run the job by
  `workflow_dispatch` against a throwaway pre-release tag such as
  `v1.0.12-rc.1` on the tree from Phase 1 (the core release script already
  accepts `vX.Y.Z-rc.N`; the internals go out at 0.0.1). This creates the
  thirteen names and publishes a gwz-core pre-release, which is the point:
  it proves the order, the index waits, the rate-limit waits and the skip
  logic on versions nobody will depend on. Expect about eighty minutes: five
  names publish at once and the other eight one every ten minutes at
  crates.io's default limit (U1). Record the run and timings in
  `gwz-core/dev-docs/GwzCratesIo-Rehearsal-YYYYMMDD.md`.
- **S2.3: trusted publishers, and the token retired** *(operator; ~15
  minutes of clicking)*. On crates.io, configure a trusted publisher for all
  fifteen crates (owner `owebeeone`, repository `gwz-core` and workflow
  `release.yml` for fourteen, repository `gwz-cli` and its publish workflow
  for `gwz`, environment `crates-io`), then delete the `CARGO_REGISTRY_TOKEN`
  secret and revoke the token on crates.io. From here on nothing publishes
  with a token, and the how-to gains a closing note saying so.

### Phase 3: the CLI on the registry core (milestone: `cargo install gwz` installs the released CLI)

- **S3.1: the release branch and script** *(gwz-cli `Cargo.toml` on
  `release`, `scripts/release.py`, `tests/release_script.rs`; ~150 lines)*.
  The `release` branch's dependency becomes `gwz-core = "=X.Y.Z"`;
  `reconcile_cargo_toml` writes the version instead of the tag;
  `verify_remote_tag` becomes a registry check that polls the crates.io
  index until gwz-core X.Y.Z resolves; `verify_locked_git_pin` becomes
  `verify_locked_registry_pin` (source `registry+https://github.com/rust-lang/crates.io-index`,
  exact version). The sibling core clone for fixtures stays as it is. The
  merge gotcha in `RELEASE.md` is rewritten for the new line.
- **S3.2: the gwz publish job** *(gwz-cli `.github/workflows/` and
  `dist-workspace.toml`; ~100 lines)*. The way uv does it: a
  `publish-crate.yml` sub-workflow on `workflow_call`, listed in
  `dist-workspace.toml` as `publish-jobs = ["./publish-crate"]`, so dist's
  own release run calls it after the binaries are built. It authenticates
  with Trusted Publishing (the `gwz` name already exists, so no token is
  ever needed here), waits for gwz-core's version to be in the index, and
  publishes the single crate with the same skip-if-present logic. The
  dist-generated `release.yml` is hand-constrained; regenerating it with
  `dist generate` to pick up the publish job must preserve those
  constraints, which the step verifies by diff.
- **S3.3: documentation** *(gwz-core `RELEASE.md`, gwz-cli `RELEASE.md`,
  `docs/QuickStart.md`, gwz-dev `AGENTS_GWZ.md` and `README.md`; ~80
  lines)*. The release order taut-shape, gwz-core, gwz, and gwz-py; what
  the publish jobs do and how to retry; `cargo install gwz` beside the
  installer script; the D8 sentence for the internals; the provenance
  difference between registry and git builds (D4).

### Phase 4: the first registry release (milestone: 1.0.12 on crates.io end to end)

- **S4.1: cut 1.0.12** through the extended scripts in the documented order:
  gwz-core (script, GitHub release, verification, publish job), gwz-cli
  (script on the registry core, GitHub release, dist builds, publish job),
  gwz-py unchanged on the git tag. The operator watches the publish jobs;
  a partial publish is resumed by `workflow_dispatch`, never by hand from a
  laptop.
- **S4.2: yank the placeholders** *(operator, two commands)*.
  `cargo yank --version 0.0.0-bootstrap.1 gwz-core` and the same for `gwz`,
  after S4.1's versions are visible (D9). Also yank the S2.2 pre-release
  versions.
- **S4.3: evidence** *(gwz-core `dev-docs/GwzCratesIo-Release-YYYYMMDD.md`;
  ~60 lines)*. Versions, run ids, timings, the rate-limit outcome, and a
  fresh-machine check of `cargo install gwz` and `cargo add gwz-core` in an
  empty project.

## 5. Step dependency sketch

```
S0.1 -> { S1.1, S1.3 } -> S1.2 -> S1.4 -> S1.5 -> S2.1 -> S2.2 -> S2.3 -> S3.1 -> S3.2 -> S3.3 -> S4.1 -> S4.2 -> S4.3
```

S1.1 and S1.3 are independent and can be picked up by different agents;
S3.3 can be drafted alongside S3.1 and S3.2 and finished after them. Nothing
in Phase 3 starts before S2.3 has retired the token, because the CLI's
release branch cannot resolve a registry gwz-core that does not exist and
its publish job is token-free by design.

## 6. Open items

- O1. gwz-py on the registry core (D7): its publish workflow's pin
  assertion and its provenance parity test would both change; worth its own
  step once Phase 4 has landed, if uniformity matters more than the
  git-built provenance the wheels carry today.
- O2. Bazel: `BUILD.bazel` carries the gwz-core version today; whether the
  internals need matching Bazel version fields is decided in S1.5.
- O3. Whether crates.io categories and keywords are worth choosing for the
  internals, given D8 says nobody should depend on them directly.
- O4. docs.rs (U6): if a crate fails to build there, the fix is
  `[package.metadata.docs.rs]` settings, not code.
- O5. The bootstrap packages under `.github/bootstrap-crate/` in gwz-core
  and gwz-cli, and their manual workflows, become dead once S4.2 yanks the
  placeholder versions; the how-to says to keep the published versions as
  history, and the directories can go in the same release that yanks them.
- O6. The how-to's procedure is the right one if the operator wants the
  thirteen names held before Phase 2 is ready; this plan does not do that,
  because S2.2 creates them with real 0.0.1 pre-release content and the
  names are obscure enough that squatting is not a live risk.

## 7. Adoption trail

- 2026-09-12: drafted (Fable), after confirming the crates.io state, the
  fourteen crates' metadata and dependency edges, the taut-shape pin, and
  the build provenance behaviour without git metadata.
- 2026-09-12, later: revised after checking uv's and ruff's published
  crates and workflows and reading `CrateBootstrapHowTo.md`: D2 moves the
  internals to their own `0.0.N` line; D5 and S2.1 use Trusted Publishing
  with a token only for the first publication of new names, `--no-verify`
  after a CI dry run, and index polling; new S2.3 configures trusted
  publishers and retires the token; S3.2 uses dist's `publish-jobs`; U4, U7,
  O5 and O6 replaced.
- 2026-09-13: the operator chose to let the first publish wait out
  crates.io's new-crate limit rather than request an increase; the limits
  are stated precisely in section 1, D5 and S2.1 carry the wait-and-retry
  loop, S2.2 states the expected duration, and U1, U7 and S0.1 no longer
  mention a request.
- 2026-09-13: D1 set to fifteen crates at the operator's decision; the
  fixtures crate stays unpublished but tracks the internal version line.
  Awaiting S0.1.
- 2026-09-13: ADOPTED by the operator; no review round. Phase 1 begins with S1.1.
- 2026-09-13: S1.3 done: gwz-core on taut-shape 0.9.2 from crates.io; full suite 2123 passed, 0 failed; outer workspace patched to the sibling checkout (U3 answered).
- 2026-09-13: S1.2 done: 32 internal dependency edges versioned at 0.0.1; check_crate_versions.py gates the lockstep line in run_tests.py and CI; REQ-165 amended.
- 2026-09-13: S1.4 done: include lists on gwz-core and the thirteen published
  internals, every pattern anchored with a leading `/` because an `include`
  list overrides `.gitignore` and an unanchored `README.md`/`LICENSE` matched
  at any depth; gwz-core packages at 1.7 MiB compressed (8.5 MiB, 735 files);
  U2 answered. U5 answered harder than expected: `cargo package --no-verify`
  still resolves the published manifest against the registry, so a per-crate
  pass fails for every internal that has an internal edge; `cargo package
  --workspace --no-verify --locked` co-packages all fifteen (cargo 1.95) and
  is what the gates use.
- 2026-09-13: S1.5 done: release.py advances the internal line, runs
  check_crate_versions.py with the tag and packages every published crate
  before tagging; publish order derived from the manifests. The packaging pass
  is `cargo package --workspace --no-verify --locked` rather than one
  `-p <crate>` per crate, for the U5 reason S1.4 measured; the publish order
  decides which archives must exist when it finishes.

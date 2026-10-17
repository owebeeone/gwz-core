# GWZ 1.1.0 Phase 8 readiness: what the release needs on crates.io, and in what order it runs

Status: **READINESS NOTE, 2026-10-17.** It decides nothing and authorizes no commit, push, tag or publish. It is a read-only audit plus one dry run in a scratch copy. The choices that need the operator are in section 6.

**What this answers.** `GwzV110RemainingPlan.md` lists Phase 8 as "gwz-transport and gwz-git still `publish = false`; tags; publish jobs; WPP 6.5" (row at line 37) and D5 (line 103). This note lists everything the publish path actually needs, from the manifests and scripts at the tuple below, and it corrects that row (git2-rs's two crates are also unpublished, and gwz-git is not on any product's path).

**Tuple read.** root `081ea966`, gwz-core `965d884a`, gwz-cli `279bf85`, gwz-py `8e8446b`, gwz-transport `a27817b`, gwz-sspi `75c1b03` (tag `v0.1.1`), git2-rs `d13951f`, gwz-git `a9d7ee0`. Lane `rel8`. crates.io was queried on 2026-10-17. git2-rs, gwz-git and libgit2 were only read (`git status` clean in both afterwards).

**Controlling text.** The 1.1.0 plan's Phase 8 (`GwzV110Plan.md`, "Phase 8 - v1.1.0 release", steps 1 to 7) as the release plan adopts it (`GwzTransportReleasePlan.md`, Phase 10, line 447), as amendment 2 §3.12 and §3.21 (a DRAFT, not reviewed) change it. Phase 8 of the 1.1.0 plan is "Phase 10" in the release plan; this note says Phase 8 throughout.

## 0. The short version

1. **gwz-core cannot be published today, and not only because of gwz-transport.** `cargo package --workspace` stops at `gwz-repo-inspect` because `gwz-git2 =0.21.0` is not on crates.io (only `0.0.0-bootstrap.1`). gwz-core's own manifest then names `gwz-sspi` and `gwz-transport` with no version. `release.py` refuses outright on four external path dependencies (section 1).
2. **The git2-rs fork must be on crates.io: `gwz-libgit2-sys` and `gwz-git2`, both `publish = false` today.** That is a hard requirement of gwz-core, gwz-cli (a dev-dependency) and the gwz-py wheel (through gwz-core). **gwz-git is not required by any product crate**; only the plan's step 4 publishes it (section 2). Both git2-rs decisions go to the operator; this note proposes no edit to either repository.
3. **gwz-transport is nearly ready**: `cargo package --list` and `cargo publish --dry-run` pass in a scratch copy with the guard lifted (88 files, 882.7 KiB, 167.1 KiB compressed). What remains is one reviewed guard-lift commit and a README that still says "unpublished" (section 3).
4. **Four gwz-core internal crate names do not exist on crates.io** (`gwz-ids`, `gwz-session-channel`, `gwz-session-contract`, `gwz-session-host`). A new name needs a token or a bootstrap placeholder before a trusted publisher can exist; TR3.3's exit says this is recorded before Phase 8, and nothing records it (section 4).
5. **The release order is a DAG with four registry events before gwz-core** (gwz-sspi, the two fork crates, gwz-transport) (section 4). Four manifest and script changes are owed first (section 6, OQ-P8-3 to OQ-P8-8).

## 1. The publish dependency graph

Rule: `cargo publish` strips `path` and keeps `version`; a dependency with a `path` and no `version` is refused ("all dependencies must have a version requirement specified when packaging"), in every table including `[target.'cfg(..)']` ones, so the candidate tables count today. A dependency that keeps a `version` (a dev-dependency included) must resolve on crates.io, which I confirmed with a probe crate (a path plus `=0.21.0` dev-dependency on `gwz-git2` fails to select a version, candidate `0.0.0-bootstrap.1`).

### 1.1 What gwz-core's published manifest names

| Dependency | Where | Form | On crates.io now | Needed before gwz-core publishes |
|---|---|---|---|---|
| 13 internal crates (`gwz-copy-contract` ... `gwz-workspace-install`) | `gwz-core/Cargo.toml:59-71` | path + version `0.0.8` | yes, `0.0.8` | new `0.0.9` line, published by the core release job |
| `gwz-ids`, `gwz-session-channel`, `gwz-session-contract`, `gwz-session-host` | `Cargo.toml:74`, `:77-79` | path + version `0.0.8` | **no, the names do not exist** | first publish of four new names (section 4) |
| `git2` = package `gwz-git2` | `Cargo.toml:89` | **path + `=0.21.0`** | placeholder `0.0.0-bootstrap.1` only | **`gwz-git2 0.21.0`** |
| `taut-shape` | `Cargo.toml:90` | registry `0.10.0` | yes | nothing |
| `gwz-transport` | `Cargo.toml:126`, in `[target.'cfg(gwz_transport_candidate)'.dependencies]` | **path only, no version** | placeholder only | `gwz-transport 0.1.0`, and a version on the edge |
| `gwz-sspi` | `Cargo.toml:127`, same table | **path only, no version** | `0.1.0`, `0.1.1` | `gwz-sspi 0.1.2` (D31), and a version on the edge |
| `libgit2-sys` = package `gwz-libgit2-sys` | `Cargo.toml:213`, `[dev-dependencies]` | path + `=0.18.8` | placeholder only | **`gwz-libgit2-sys 0.18.8`** (the versioned dev-dependency is kept in the published manifest) |
| `ssh2`, `libssh2-sys`, `tokio`, `hyper`, `native-tls`, `openssl` and the other transport crates | `Cargo.toml:128-139`, `:201-210` | registry, exact pins | yes | nothing |
| `windows-sys`, `libc`, `rustix`, `cap-std` and the rest | `Cargo.toml:80-115` | registry | yes | nothing |

The internal crates have their own edges to the fork: `crates/repo-inspect/Cargo.toml:28` (`gwz-git2`, path + `=0.21.0`, a normal dependency, so `gwz-repo-inspect` cannot publish before `gwz-git2`) and `crates/local-testrepo/Cargo.toml:42` (`publish = false`, never packaged). The 1.0.17 manifests used upstream `git2 0.21` and `libgit2-sys =0.18.8`; the fork rename is why the registry path broke.

**Evidence (run in a scratch copy under `build-targets`, now deleted; `--no-verify` so nothing compiled):**

- `cargo package --workspace --no-verify --locked` packaged 13 crates, then stopped at `gwz-repo-inspect`: `failed to select a version for the requirement gwz-git2 = "=0.21.0"; candidate versions found which didn't match: 0.0.0-bootstrap.1`.
- `cargo package -p gwz-core --no-verify --locked`: `dependency gwz-sspi does not specify a version`.
- `scripts/release.py`'s own gate, called directly (`assert_no_external_path_dependencies`, `scripts/release.py:244`, run at `:788`), refuses with four lines: `dependencies.git2 -> ../git2-rs`, `dev-dependencies.libgit2-sys -> ../git2-rs/libgit2-sys`, and the two `gwz-transport`/`gwz-sspi` candidate-table edges. **`gwz-core/scripts/release.py v1.1.0` fails at its first gate until the manifest form is decided** (OQ-P8-3).

### 1.2 What gwz-cli and gwz-py name

| Crate | Dependency | Where | Form | Needs |
|---|---|---|---|---|
| gwz-cli (`gwz`) | `gwz-core` | `gwz-cli/Cargo.toml:46` | path on `main`; `release` branch pin `=X.Y.Z` (release.py) | gwz-core 1.1.0 |
| gwz-cli | `gwz-sspi` | `Cargo.toml:42` | path + `=0.1.1`; release.py rewrites to `"=0.1.1"` (`scripts/release.py:321-328`) | `gwz-sspi 0.1.2` and a changed script (OQ-P8-7) |
| gwz-cli | `git2` = `gwz-git2` | `Cargo.toml:50`, `[dev-dependencies]` | **path + `=0.21.0`; nothing in `release.py` reconciles it** | `gwz-git2 0.21.0` on crates.io, and a release-branch form (OQ-P8-8) |
| gwz-py (wheel) | `gwz-core`, `gwz-sspi` | `gwz-py/Cargo.toml:20`, `:18` | path on `main`; pins on `release` (`scripts/release.py:288-295`, hard-coded `=0.1.1`) | as above |
| gwz-py | the fork, `gwz-transport` | none directly | through gwz-core's published manifest | `RELEASE.md`'s pin table already says "the versions the published gwz-core X.Y.Z requires" (line 43) |

`gwz-py/RELEASE.md:15-17` still says SSPI "is currently unpublished and `publish=false`"; gwz-sspi 0.1.0 and 0.1.1 are published. Stale text, to fix with the pin change.

### 1.3 The graph, as publish edges

```text
gwz-libgit2-sys 0.18.8+1.9.7 ── gwz-git2 0.21.0 ──┬── gwz-repo-inspect 0.0.9 ──┐
                                                  ├── gwz-core 1.1.0           │
                                                  ├── gwz (cli) dev-dependency │
                                                  └── gwz-git 0.1.0 (no product depends on it)
gwz-sspi 0.1.2 ───────────────────────────────────┼── gwz-core, gwz (cli), gwz-py
gwz-transport 0.1.0 ──────────────────────────────┴── gwz-core
17 internal crates 0.0.9 (4 new names) ──── gwz-core 1.1.0 ──── gwz (cli) 1.1.0 ──(tag, tests)── gwz-py 1.1.0
taut-shape 0.10.0 is already published.
```

Publish order of the 18 gwz-core crates is derived by `scripts/checks/check_crate_versions.py --print-publish-order` (17 internals, then gwz-core); I ran it.

## 2. The gwz-git and git2-rs question

**What gwz-core depends on.** The fork, by path, as package `gwz-git2` (`Cargo.toml:89`; Rust crate name stays `git2`) and, for tests, `gwz-libgit2-sys` (`Cargo.toml:213`). It does **not** depend on gwz-git at all. A grep over all five product repositories and their locks finds `gwz-git` only in the root workspace's `exclude` list (root `Cargo.toml:27`) and one probe note (`gwz-core/tests/transport_native/consumer_probe.md`). `gwz-git` (`Cargo.toml:7`, `publish = false`) depends on the fork, not the other way round.

**Why vendoring is not an alternative.** The fork is not a rename of upstream: its history carries GWZ's per-remote smart transport binding (`e883be3`), the native local-fetch correction that `Cargo.toml:82-88` says the family import relies on alone, and a vendored libgit2 at `b172e3d18` (the submodule gitlink and `libgit2/` HEAD agree, and 1.1.0 plan Phase 8 step 1 makes that hash the pre-push check). Returning to upstream `git2 0.21` and `libgit2-sys 0.18.8`, as 1.0.17 had, drops those. A sys crate also cannot be carried inside another package (`links = "git2"` and its `build.rs`). So the publishing route is the plan's: publish the fork under its two `gwz-*` names.

**Exactly what is needed (all in repositories this lane may not touch):**

| Item | State read | Needed |
|---|---|---|
| `gwz-libgit2-sys` | `git2-rs/libgit2-sys/Cargo.toml:10` `publish = false`; version `0.18.8+1.9.7`; `exclude` list present | lift the guard; publish before `gwz-git2` |
| `gwz-git2` | `git2-rs/Cargo.toml:12` `publish = false`; version `0.21.0` (gwz-core requires `=0.21.0` exactly); depends on the sys crate by path + `=0.18.8` | lift the guard; publish second |
| Package size | `cargo package --list --locked -p gwz-libgit2-sys` (list only, target redirected): 683 files, 8.3 MiB uncompressed | well under crates.io's 10 MiB compressed limit; a real `cargo publish --dry-run` has not been run (it needs the guard lifted, which is an edit to that repo) |
| Release tooling | no `gearu.toml` in git2-rs (`gearu plan` answers "no gearu.toml found"); `.github/workflows/` holds `bootstrap-gwz-git2.yml`, `bootstrap-gwz-libgit2-sys.yml` and upstream's `publish.yml`, all referring to registry tokens | 1.1.0 plan S2.2: one `gearu.toml` and one tag for both packages; a `release.yml` that publishes the sys crate, waits for the index, then the API crate; TR3.2's removal of the three stray workflows |
| Trusted publishers | not readable from the repository | a trusted publisher on crates.io for each of the two names (TR3.3 exit); unknown |
| Pre-push check | `libgit2` is at `b172e3d187a4b6866fd9f696f40a1b8e7f56d348` in this lane | step 1's check passes today and must be re-run on the release commit |

**gwz-git.** Nothing the product ships needs it. The 1.1.0 plan (step 4, `GwzV110Plan.md` lines 385-389) still publishes it after the API crate, with `gearu release <version> --dependency-tag ...`. Its state: `publish = false`, no `gearu.toml` (S2.2 listed one), only `bootstrap-crate.yml`. Leaving it out of 1.1.0 is a scope change to Phase 8 step 4 and is the operator's (OQ-P8-2). The RemainingPlan row should say "gwz-libgit2-sys, gwz-git2, gwz-transport and (if kept) gwz-git".

**The decision for the operator (OQ-P8-1):** who prepares git2-rs for publishing (the guard lifts, `gearu.toml`, `release.yml`, the trusted publishers). This note does not propose, and the lane does not make, those edits.

## 3. gwz-transport readiness

Dry run: a scratch copy (no `.git`, no `target`) under `build-targets/rel8-pkg`, `publish = false` deleted from the copy's `Cargo.toml`, `cargo +1.95.0`, `CARGO_TARGET_DIR` under `build-targets`. The lane's checkout was not touched; the scratch directory is deleted.

| Command | Result |
|---|---|
| `cargo package --list --allow-dirty` | 88 files: `Cargo.toml`, `Cargo.toml.orig`, `Cargo.lock`, `LICENSE`, `LICENSE-taut`, `README.md`, 3 `protocol/` files, 5 `scripts/` files, 35 `src/` files, 39 `tests/` files |
| `cargo publish --dry-run --allow-dirty --locked` | passed: packaged 88 files, 882.7 KiB (167.1 KiB compressed); verified (compiled the packaged lib, 7.7 s); `warning: aborting upload due to dry run` is the only warning |
| `cargo package --allow-dirty --locked` | passed |

The 2026-10-05 note said 81 files; the tree has grown since. The `--allow-dirty` is only because the scratch copy has no VCS; `gearu`'s exact stage runs `cargo +1.95.0 package --locked` on the committed release.

What lifting `publish = false` needs, item by item:

| Item | State | Needed |
|---|---|---|
| Guard | `gwz-transport/Cargo.toml:11` `publish = false` | delete the line, in a reviewed change (1.1.0 plan S2.1 and amendment 2 §3.21 part B) |
| The release check | `scripts/release_checks.py:43-45` refuses while `publish` is false; `scripts/test_release_checks.py:40` and `:70` pin both refusals | no change to the check; the two refusal tests build their own manifest with `publish = false` and stay valid. Add a test that the real manifest is open (`gearu plan` does not run checks) |
| Version | `0.1.0` (`Cargo.toml:3`); `gearu plan 0.1.0` accepts it, "Changes: none" | keep (S2.2: first versions stay unless `gearu plan` requires a bump). Open: with no version change, which commit does `gearu release 0.1.0` tag, and does it make an empty commit? Check on the first real run (OQ-P8-6) |
| License | `license = "GPL-2.0-only"`, `LICENSE` and `LICENSE-taut` both in `include` | none |
| Metadata | `repository`, `description`, `readme`, `edition`, `rust-version` present; no `keywords` or `categories` | optional; the other crates carry both |
| README | `README.md:4` "This package is unpublished; SSH/HTTPS adapters are not enabled." and `:334` "The package is not published." | **must change**: they become the crates.io page. In scope of the guard-lift change |
| `include` | `Cargo.toml:10`, anchored, ships `tests/**/*.rs`, `scripts/*.py`, `protocol/*` | works as it stands; whether 39 test files should ship is a taste call, not a gate |
| Dependencies | `[dependencies]` is empty (`Cargo.toml:16`) | none; this is why `gearu.toml` regenerates the lock offline |
| Toolchain | rustfmt 1.96.0 for the generators, 1.95.0 for builds | `RELEASE.md` "release interpreter, once per machine": create `.release-venv` with `taut-proto==0.10.0` and both toolchains (operator action) |
| Release workflow | `.github/workflows/release.yml`: on `release.published`, reruns both stages on the tag, then `rust-lang/crates-io-auth-action@v1` and `cargo publish --locked`, no environment, no token fallback | ready |
| Trusted publisher | configured 2026-10-05: owner `owebeeone` (ID 366621), repository `gwz-transport`, workflow `release.yml`, no environment (checkpoint line 453; `RELEASE.md`) | cannot be read from the repository; confirm it still stands before the tag |
| Stray publish path | `.github/workflows/bootstrap-crate.yml` still refers to `CRATES_IO_BOOTSTRAP_TOKEN` | TR3.2 removes it with a workflow-text test; it waits on TR3.3's registry steps |
| gearu config | `gearu.toml` present; `gearu plan 0.1.0` passes (section 5) | none |
| Pin consumers | gwz-core's `.github/gwz-transport.commit` is `a27817b9`, the current `main` HEAD; its candidate generators pin the IR digest | after the release, the pin must name the commit the `v0.1.0` tag points at (section 4) |

## 4. The release order, with every gate

The order is the 1.1.0 plan's Phase 8 with amendment 2's insertions. Tags below are the repository's own; the three product tags are `v1.1.0`. The operator runs the commands (Phase 8 preamble); the stop rule applies throughout: a failure after a push, a GitHub Release or a publish stops the train, records the artifact IDs, and resumes only with a new patch or release-candidate version. A tag is never moved.

### 4.1 Gates before any tag

| Gate | State (2026-10-17) |
|---|---|
| S5.4 defaults note (B7), WPP 6.3 / S5.5 repeat (B8), **S5.6 cell table (B9)** | not started; an unsupported in-scope cell blocks S7.5 and Phase 8 |
| S7.1 (C3b, C4a, C4b), S7.2 (C5), S7.3 + WPP 6.4 (D1), S7.4 (D2) | not started (C3b waits on D31) |
| **S7.5** activation and release review: dual plus Surface | not started |
| **WPP 5.5** gwz-sspi activation review: dual (release gate; OQ14 (b)). Runs after 5.6, 6.2, 6.3 | not started. gwz-sspi 0.1.0/0.1.1 were published on the operator's decision B without it |
| D31: gwz-sspi 0.1.2 reviewed, tagged, published; pins moved in gwz-core, gwz-cli, gwz-py | lane `sspi012` |
| TR3.3 registry exit: the fourteen names visible with trusted publishers, "recorded in the checkpoint before Phase 10" (amendment 2, TR3.3) | **not recorded**; the four new internal names do not exist; the fork names' publishers are unrecorded |
| TR3.2: stray publish paths removed, bootstrap token revoked | open; waits on TR3.3 |
| TR3.4 gwz-py registry pins | done (gwz-py `94ebeb6` per checkpoint); the `=0.1.1` literal needs `0.1.2` |
| Windows precondition: `windows-matrix.yml` dispatched on the gwz-core release commit and passing, run recorded in the checkpoint | **WPP 6.5**, before the gwz-core tag |
| **WPP 6.5 / Phase 8 release-gate dual review** ("the Phase 10 review"); depends on 6.4, 5.5, 5.6 | not started |
| Manifest decisions OQ-P8-3 (gwz-core edge form), OQ-P8-7, OQ-P8-8 and their script changes | open |

### 4.2 The steps

| # | Repository | Command | Tag | Waits for | Publishes |
|---|---|---|---|---|---|
| 1 | gwz-sspi | `gearu release 0.1.2 --push --github-release` | `v0.1.2` | D31 review, 5.5 | `gwz-sspi 0.1.2` (`release.yml`, Trusted Publishing) |
| 2 | git2-rs | `gearu release <v> --push --github-release` (a `gearu.toml` does not exist yet) | one tag for both | the libgit2 hash check; OQ-P8-1 | `gwz-libgit2-sys 0.18.8+1.9.7`, then `gwz-git2 0.21.0` |
| 3 | gwz-transport | `gearu release 0.1.0 --push --github-release` | `v0.1.0` | the reviewed guard-lift commit; `.release-venv` | `gwz-transport 0.1.0` |
| 4 | gwz-git (only if OQ-P8-2 keeps it) | `gearu release <v> --dependency-tag ... --push --github-release` | `v0.1.x` | step 2 visible | `gwz-git` |
| 5 | gwz-core | `scripts/release.py v1.1.0 --push`, then the GitHub Release | `v1.1.0` (lightweight, off `main`) | steps 1, 2, 3 visible on crates.io; 6.5's `windows-matrix.yml` run; S7.5 GO | 17 internal crates at `0.0.9` (4 new names) and `gwz-core 1.1.0`, by `release.yml` |
| 6 | gwz-cli | `scripts/release.py v1.1.0 --push`, then the GitHub Release | `v1.1.0` (off `release`) | step 5's publish job; **`gwz-sspi 0.1.2` visible** | dist binaries (5 targets); `gwz 1.1.0` by `publish-crate.yml` |
| 7 | gwz-py | `scripts/release.py v1.1.0 --push`, then the GitHub Release | `v1.1.0` (off `release`) | steps 5 and 6 (the cli tag is read by its tests) | wheels and sdist to PyPI by `publish.yml` |
| 8 | all | the post-release check, WPP 6.5 | none | steps 5 to 7 | nothing: `cargo install gwz --version 1.1.0 --locked` and a Python install from the registries alone on macOS ARM64, Linux x86-64 and dabeest; `server` start, status, stop; one SSH and one HTTPS command and one gwz-py operation on the transport route; the 1.2.0 surfaces absent; the dabeest-only rows; every server the check starts is stopped |

Steps 1 to 3 do not depend on one another and can run in any order or together (steps 1 and 3 have no dependencies of their own; step 2's two crates are ordered inside it). Step 4 is the only step that can follow step 2 alone.

### 4.3 What moves with the release (D27 and the `.github` pins)

- **gwz-core's manifest** takes the four external edges' versions: `gwz-git2 =0.21.0` (already there), `gwz-transport =0.1.0`, `gwz-sspi =0.1.2`, and the dev-dependency `gwz-libgit2-sys =0.18.8` (already there). Whether they stay dual-form on `main` or are version-only in the release commit is OQ-P8-3.
- **D27 (Bazel).** `@gwz_core_crates` stays unsupported while any sibling path edge remains in the lock that `from_cargo` splices (D27's own words: it "cannot read the sibling path dependencies"). The registry pins of this step restore it only if the edges are version-only in the tree Bazel reads (OQ-P8-3 (b), (c) or (d)); dual-form edges keep the path and leave D27 in force. After the release, `bazel build //gwz-core/...` is the check. MODULE.bazel and the Bazel lock are root files, outside this lane.
- **`.github/git2-rs.commit`, `.github/gwz-transport.commit`, `.github/gwz-sspi.commit`** (gwz-core; the header of each states the bump rule) name the commits CI checks out beside gwz-core. After the release each must equal the commit its published tag points at, so the tested bytes are the published bytes: `gwz-transport.commit` = `v0.1.0`'s commit, `gwz-sspi.commit` = `v0.1.2`'s (today `75c1b032` = `v0.1.1`), `git2-rs.commit` = the git2-rs tag's commit (today `d13951f`, untagged). `.github/gwz-py.commit` (the candidate tests' Python codec) is unrelated to the publish path. Once the edges are version-only, `release.yml`'s three sibling checkout steps (`verify`, `verify-windows`, `publish`) have nothing to check out and can go; the candidate and Windows-matrix workflows keep theirs until S7.1 retires the switch.
- **Locks.** After the release, as for 1.0.13: root `Cargo.lock`, gwz-py `Cargo.lock`, gwz-cli's own `Cargo.lock`; each moves by the product line plus the 17 internals now.

### 4.4 How earlier releases were cut (gwz-cli `docs/Releases.md`, gwz-core/gwz-cli/gwz-py `RELEASE.md`)

Releases from 1.0.12 on follow the shape step 5 to 7 has: core `release.py` with the gates (about 12 minutes with the suite, 4 with `--no-test` when the suite had just passed), the GitHub Release (Linux `verify` gates the publish job; Windows `verify-windows` does not), the cli script waits on crates.io for the core version (default 900 s, `--registry-timeout`), cli GitHub Release (dist, then `custom-publish-crate`), py script, py GitHub Release (PyPI). `docs/Releases.md` itself has per-release notes only for 1.0.17 and "Catching up: 1.0.14 and 1.0.16"; the process details above are in the three `RELEASE.md` files and the operator's process notes. Differences this time: four registry events before step 5, four new crate names in step 5 (a new-name burst of 5 per ten minutes is within the limit), a Windows matrix run before the core tag, and the post-release check.

## 5. Per-repo release tooling, dry runs

| Repo | Tool | Command run | Result |
|---|---|---|---|
| gwz-transport | gearu | `gearu plan 0.1.0` (read-only; reads `ls-remote`, creates nothing) | OK: branch `main` at `a27817b9`, no dependencies, no version change; check `release_checks.py --version {version}` listed. `release_checks.py` itself would refuse (publish = false) |
| gwz-sspi | gearu | `gearu plan 0.1.2` | OK: `Cargo.toml 0.1.1 -> 0.1.2`; check `release_checks.py`. Reflects the lane tree; lane `sspi012` owns the real change |
| gwz-core | `scripts/release.py` | no `gearu.toml` (by operator decision 2026-10-05, `gearu plan` answers so); `release.py` has no dry-run flag, so not run end to end. Its first gate was called alone | **refuses**: four external path dependencies (section 1.1) |
| gwz-cli | `scripts/release.py` | no `gearu.toml`; no dry-run flag; read | hard-codes `gwz-sspi` to `=0.1.1` (`:321-328`), has no rule for the `git2` dev-dependency (`Cargo.toml:50`), waits on gwz-core only (`REGISTRY_API`, `:73`, polled at `:194`; `publish-crate.yml` also waits on gwz-core only) |
| gwz-py | `scripts/release.py` | no `gearu.toml`; no dry-run flag; read | hard-codes `gwz-sspi` to `=0.1.1` (`:288-295`); the registry-pin check refuses any path dependency |
| git2-rs, gwz-git | gearu | `gearu plan 0.1.0` | `no gearu.toml found`: S2.2 was not done for either |

The three `release.py` scripts commit and tag locally, so none was run.

## 6. Open items for the operator

Numbered so the checkpoint can cite them. Each is a question or a finding; none is decided here.

- **OQ-P8-1. Preparing git2-rs for publication.** The two fork crates are `publish = false`, there is no `gearu.toml`, three stray token workflows remain, and the trusted publishers are unrecorded. The lane may not edit the repository. Who does it, and when (before step 2)? A `cargo publish --dry-run` of both crates has not been possible without editing the repository; size says it will fit.
- **OQ-P8-2. Is gwz-git in 1.1.0?** No product crate depends on it. Plan step 4 publishes it; keeping it needs a `gearu.toml`, a release workflow and a trusted publisher, dropping it needs an amendment to step 4 and to S2.1 to S2.3's name list.
- **OQ-P8-3. The form of gwz-core's external edges at release.** `release.py:244-262` refuses any path dependency outside the repository, and gwz-core has no `release` branch (`RELEASE.md`: tags are cut off `main`). Options: (a) keep dual-form edges (`path` + `version`) on `main`, as the internal crates do, and change the check to require a version beside each external path; (b) the release commit rewrites the four edges to version-only (the plan's wording: "The release commit pins `gwz-transport` and the new `git2` API crate by the versions just published") and a following commit restores the paths; (c) give gwz-core a `release` branch as gwz-cli and gwz-py have; (d) `main` goes version-only and development patches the siblings in with `[patch.crates-io]` (not tested here). Only (b) to (d) can end D27. Note also that `make_standalone_worktree` (`release.py:163`) builds in a temporary directory with nothing beside it, so under (a) its gates cannot resolve `../git2-rs` either.
- **OQ-P8-4. The four new internal names.** `gwz-ids`, `gwz-session-channel`, `gwz-session-contract` and `gwz-session-host` do not exist on crates.io. Amendment 2 TR3.3 says they publish real versions through gwz-core's release workflow, but a first publish needs a token (the comments of `release.yml`'s `publish` job) and a trusted publisher can only follow. Options: bootstrap placeholders and trusted publishers first, as for the other names; or a one-time token in the `crates-io` environment for step 5 (the workflow still prefers it when set, which TR3.2 plans to remove). The operator mints any token; no token comes through chat.
- **OQ-P8-5. TR3.3's exit is not recorded.** The checkpoint lists no trusted-publisher state for `gwz-git2`, `gwz-libgit2-sys`, `gwz-git`, or the fourteen names generally.
- **OQ-P8-6. gwz-transport's guard-lift change.** It needs the README rewrite (section 3), a review route (a dual, or the 6.5 review), and a check of what `gearu release 0.1.0` tags when the version does not change. The operator-side step is the `.release-venv`. The `tests/` files are shipped; say if that is not wanted.
- **OQ-P8-7. gwz-sspi 0.1.2 in the product pins.** `gwz-cli/scripts/release.py:321-328` and `gwz-py/scripts/release.py:288-295` accept only `=0.1.1`; `gwz-py/RELEASE.md:15-17` and `:40` carry stale "unpublished" text; gwz-core's `Cargo.toml:127` has no version. Neither cli script nor `publish-crate.yml` waits for gwz-sspi on the registry (they wait for gwz-core only), so step 6 relies on step 1 having finished. Lane `sspi012` is on the pins; the waits are not in its brief as far as I read it.
- **OQ-P8-8. gwz-cli's `git2` dev-dependency on the release branch.** `release.py` does not reconcile `Cargo.toml:50`, so the standalone worktree cannot resolve `../git2-rs`, and `verify_locked_registry_pin` requires every `gwz-*` package in the lock to come from crates.io. Either it becomes version-only (needs `gwz-git2 0.21.0` published, which it will be) or the tests change; `release.py` and `test_release.py` need the rule either way.
- **OQ-P8-9. The sibling checkouts and `.commit` pins** (section 4.3): which workflows keep them after the edges change, and who moves each pin to its tag's commit.
- **OQ-P8-10. Amendment 2 §3.21 and §3.22 are DRAFTs; revisions 3 to 8 are unreviewed** (OQ-D). Step 5a's wording ("reviewed activation") is overtaken by decision B and by D31; the order in section 4.2 follows the code, not §3.21's sentence. The remaining plan's OQ-D answer (fold into S7.5) covers it if the operator keeps it.
- **OQ-P8-11. Housekeeping from before:** yank `gwz-sspi 0.0.0-bootstrap.1` and the other placeholders when their names publish; remove the `.github/bootstrap-crate` packages (TR3.2).

Findings that are not open questions: the RemainingPlan's Phase 8 row understates the unpublished set (section 2); the live 1.1.0 plan text for step 5 says gwz-core pins "the new `git2` API crate", which is `gwz-git2` here.

## 7. What I ran and read, for reproduction

- crates.io API queries (2026-10-17): `gwz-git2`, `gwz-libgit2-sys`, `gwz-git`, `gwz-transport` have only `0.0.0-bootstrap.1`; `gwz-sspi` has `0.1.1`, `0.1.0`, `0.0.0-bootstrap.1`; `gwz-core` and `gwz` have `1.0.17` as latest; `gwz-ids`, `gwz-session-channel`, `gwz-session-contract`, `gwz-session-host` do not exist; the thirteen older internals exist at `0.0.8`; `taut-shape 0.10.0` exists.
- `gearu plan 0.1.0` in gwz-transport and `gearu plan 0.1.2` in gwz-sspi; `gearu plan` in gwz-core, gwz-cli, gwz-py, git2-rs and gwz-git (no `gearu.toml`).
- Scratch copies under `build-targets/rel8-pkg` (deleted): gwz-transport without its guard (list, publish dry run, package), gwz-core without `target`, `.git`, tests and docs beside symlinks to the lane's git2-rs and gwz-sspi (`cargo package --workspace --no-verify --locked`), and a two-file probe crate for the dev-dependency rule.
- Called `release.assert_no_external_path_dependencies()` from gwz-core with `python3.13 -B`.
- `cargo package --list --locked --allow-dirty -p gwz-libgit2-sys` and `-p gwz-git2` in git2-rs with `CARGO_TARGET_DIR` redirected (lists only); no file in git2-rs or gwz-git was written.
- Not run: any `release.py`, any `gearu release`, any tag or publish, the gwz-core or gwz-cli packaging with a verify build (it cannot succeed before the registry events above), a real dry run of the fork crates.
- Hypotheses not tested: a grep found no `include_str!` or `include_bytes!` in the non-test sources reaching outside the packaged `include` set, but the first verify build of the packaged gwz-core is the proof; option (d) of OQ-P8-3.

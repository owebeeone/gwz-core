# GWZ 1.0.12: the first crates.io release

Steps S4.1 and S4.3 of [GwzCratesIoPlan.md](GwzCratesIoPlan.md). Cut and published
on 2026-09-14; all times are UTC.

**Outcome.** GWZ 1.0.12 is released through every channel. gwz-core, its thirteen
internal crates and the `gwz` CLI crate are on crates.io, published only by CI through
Trusted Publishing. The CLI binaries and installers are on the gwz-cli GitHub release,
and the Python package is on PyPI with five wheels and the source distribution. `cargo install gwz` and `cargo add gwz-core`
work from crates.io alone. Command behaviour is unchanged from 1.0.11.

## 1. Order and results

| Step | Where | Result |
|---|---|---|
| gwz-core full suite on pre-bump commit `c88bb1d` | local macOS | 2126 passed, 0 failed, 1 ignored |
| gwz-core release script | local | commit `2e240ae`, tag `v1.0.12`; gwz-core 1.0.12, internals 0.0.3 |
| gwz-core release run [34796327818](https://github.com/owebeeone/gwz-core/actions/runs/34796327818) | GitHub | Linux verification passed 01:34:05 to 01:41:49; publish job 01:41:53 to 01:42:42; Windows verification passed 01:34:06 to 01:56:01 with 2086 tests, confirming the line-ending fix |
| gwz-cli release script | local | waited 440 s for gwz-core on crates.io; 275 tests passed; `gwz-1.0.12.crate` packaged and verified; commit `8d02eb3` on `release`, tag `v1.0.12` |
| gwz-cli release run [34796914602](https://github.com/owebeeone/gwz-cli/actions/runs/34796914602) | GitHub | five target builds, global artifacts and host passed; `gwz` crate published at 01:53:20 |
| gwz-cli docs run [34796914467](https://github.com/owebeeone/gwz-cli/actions/runs/34796914467) | GitHub | site deployed from the tag |
| gwz-py release script | local | protocol checks, `cargo check`, 845 tests, wheel smoke test; commit `a6f1583` on `release`, tag `v1.0.12` |
| gwz-py publish run [34797197269](https://github.com/owebeeone/gwz-py/actions/runs/34797197269) | GitHub | wheels built on five platforms; the publish job uploaded them with the source distribution and PyPI attestations, 02:11:56 to 02:12:05 |

How each script ran:

- **gwz-core:** `scripts/release.py v1.0.12 --push --no-test`, with incremental
  compilation and debug information off to fit the disk. The full suite had just passed
  locally on the same pre-bump commit, and the hosted release run repeats it at the tag
  on Linux, where it gates publishing, and on Windows. The script still ran the
  regeneration, format, lock, lockstep, boundary and clippy gates, and before tagging it
  ran the lockstep gate against the tag and packaged the workspace.
- **gwz-cli:** `scripts/release.py v1.0.12 --push --registry-timeout 3000`, started
  while gwz-core's verification was still running, so its registry wait absorbed that
  time. It migrated the `release` branch from the git pin to `gwz-core = "=1.0.12"`.
- **gwz-py:** `scripts/release.py v1.0.12 --push`, with every build sharing one
  temporary `CARGO_TARGET_DIR`. That directory peaked at 1.4 GB and free disk never
  fell below 8.8 GB. The test CLI was built from gwz-cli at the tag, linking gwz-core
  from crates.io, so the provenance check passed under the version and build kind rule
  (plan D7 and S3.4).

## 2. crates.io publications

gwz-core's publish job published fourteen crates in dependency order in 49 seconds.
They are updates to existing names, so crates.io's larger update limit applied and
there were no rate-limit waits. Each crate was visible on the first index poll.

| Crate | Version | Published | By |
|---|---|---|---|
| gwz-copy-contract | 0.0.3 | 01:42:07 | GitHub run 34796327818 |
| gwz-family-model | 0.0.3 | 01:42:09 | GitHub run 34796327818 |
| gwz-family-store-contract | 0.0.3 | 01:42:11 | GitHub run 34796327818 |
| gwz-family-store | 0.0.3 | 01:42:14 | GitHub run 34796327818 |
| gwz-refcopy | 0.0.3 | 01:42:16 | GitHub run 34796327818 |
| gwz-repo-contract | 0.0.3 | 01:42:19 | GitHub run 34796327818 |
| gwz-history-check | 0.0.3 | 01:42:20 | GitHub run 34796327818 |
| gwz-local-import | 0.0.3 | 01:42:23 | GitHub run 34796327818 |
| gwz-repo-factory | 0.0.3 | 01:42:26 | GitHub run 34796327818 |
| gwz-repo-inspect | 0.0.3 | 01:42:28 | GitHub run 34796327818 |
| gwz-work-detector | 0.0.3 | 01:42:30 | GitHub run 34796327818 |
| gwz-local-disposal | 0.0.3 | 01:42:32 | GitHub run 34796327818 |
| gwz-workspace-install | 0.0.3 | 01:42:34 | GitHub run 34796327818 |
| gwz-core | 1.0.12 | 01:42:38 | GitHub run 34796327818 |
| gwz | 1.0.12 | 01:53:20 | GitHub run 34796914602 |

crates.io records every one as published by GitHub through Trusted Publishing. A
publish succeeds only for a crate whose own configuration matches, so all fifteen
trusted publisher entries are now proven. The `gwz` publish job confirmed the manifest
pins `gwz-core = "=1.0.12"`, found gwz-core 1.0.12 on crates.io and `gwz` 1.0.12 absent,
authenticated, and published with cargo's verification build on.

At 01:56 docs.rs had built all fifteen crates, and every crates.io page served its
README.

## 3. Proof from crates.io alone

- **Library.** In a new Cargo project, `cargo add gwz-core` chose 1.0.12. The lock held
  fourteen gwz crates, all from crates.io, with no git sources. `cargo check` of a
  function returning `gwz_core::VERSION` passed in 21 seconds.
- **CLI.** `cargo install gwz --version 1.0.12 --locked` into a throwaway root took
  55 seconds. The binary reports `gwz 1.0.12`, and `--build-info` shows the CLI and
  the core both as crates.io builds: `revision=unavailable dirty=unknown`, each with a
  source digest and `build=cargo`.

Both proofs ran on the operator's Mac with its existing Cargo download cache, not on a
clean machine. Cached downloads are checked against crates.io checksums, but a C
compiler and OpenSSL were already installed.

## 4. Bookkeeping

The workspace root lock records gwz-core `2e240ae`. The root and gwz-py `Cargo.lock`
files were refreshed to gwz-core 1.0.12 and the 0.0.3 internal line (plan O7). All of
it is pushed.

## 5. Remaining for the operator (S4.2)

Yank the versions that were never meant to be used, from each version's page on
crates.io or with a token that has yank rights:

- `gwz` 0.0.0-bootstrap.1 and `gwz-core` 0.0.0-bootstrap.1, the name placeholders;
- `gwz-core` 1.0.12-rc.1 and the thirteen internal crates at 0.0.2, the rehearsal.

After the yanks, the bootstrap packages under `.github/bootstrap-crate/` and their
manual workflows in gwz-core and gwz-cli can be removed (plan O5).

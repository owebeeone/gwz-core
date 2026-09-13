# GWZ crates.io rehearsal: 1.0.12-rc.1

Step S2.2 of [GwzCratesIoPlan.md](GwzCratesIoPlan.md). Run on 2026-09-12 (UTC),
recorded 2026-09-14.

**Question.** Does the CI publisher create the thirteen new internal crate names
and publish a gwz-core pre-release in dependency order, surviving crates.io's
new-crate rate limit, with nobody pacing it?

**Outcome.** Yes. Fourteen crates published in one run of 73 minutes, with seven
automatic rate-limit waits and no manual retry. The Windows verification leg
failed on a test-only line-ending bug; by design (plan D5) it did not gate
publication, and it is fixed in gwz-core `7b3831c`.

## 1. Inputs

- **Release commit.** gwz-core `00f31e2`, `chore(release): gwz-core 1.0.12-rc.1`,
  cut locally with `scripts/release.py v1.0.12-rc.1 --push --no-test`. No Rust
  source had changed since the S1.3 full suite, so `--no-test` followed the 1.0.10
  and 1.0.11 precedent; the script still ran the regeneration, format, lock,
  lockstep, boundary and clippy gates, and on the exact commit before tagging it
  ran the lockstep gate against the tag and
  `cargo package --workspace --no-verify --locked`. The lightweight tag
  `v1.0.12-rc.1` and `main` were pushed atomically. Root lock record `2eb397a`.
- **Versions.** gwz-core 1.0.12-rc.1; all fourteen internal crates at 0.0.2. The
  release script advances the internal line with every product bump, so the first
  published internal version is 0.0.2, not the 0.0.1 that S2.2 anticipated.
- **Credential.** The `crates-io` environment on gwz-core, created 2026-09-12,
  held a `CARGO_REGISTRY_TOKEN` secret that the operator set at 20:54:16 UTC.
- **Local host.** 9.9 GB free before the cut. With the operator's approval,
  `gwz-core/target/debug/incremental` (16 GB) was deleted and the cut ran with
  `CARGO_INCREMENTAL=0`.

## 2. The run

`gh workflow run release.yml --repo owebeeone/gwz-core -f tag=v1.0.12-rc.1`
started run
[34718460081](https://github.com/owebeeone/gwz-core/actions/runs/34718460081).

| Job | Start (UTC) | End (UTC) | Result |
|---|---|---|---|
| Verify (ubuntu-24.04) | 20:55:00 | 21:03:17 | success |
| Verify (windows-2022) | 20:54:57 | 21:20:06 | failure, see section 5 |
| Publish to crates.io | 21:03:20 | 22:16:31 | success |

Linux verification passed 2145 tests with 1 ignored and 0 failed, across ten
result blocks. The lockstep gate reported gwz-core 1.0.12-rc.1 with fourteen
internal crates at 0.0.2 and 32 versioned internal edges.

Authentication went as planned. `rust-lang/crates-io-auth-action` answered
"No Trusted Publishing config found for repository `owebeeone/gwz-core`", which is
expected before S2.3. `continue-on-error` let the job use the environment secret
(`HAVE_TOKEN: true`).

## 3. Publisher ledger

Publication times are the `created_at` minutes crates.io records. A refused
crate waited 620 seconds and retried once.

| # | Crate | Version | Published (UTC) | Decision |
|---|---|---|---|---|
| 1 | gwz-copy-contract | 0.0.2 | 21:03 | published at once |
| 2 | gwz-family-model | 0.0.2 | 21:03 | published at once |
| 3 | gwz-family-store-contract | 0.0.2 | 21:03 | published at once |
| 4 | gwz-family-store | 0.0.2 | 21:03 | published at once |
| 5 | gwz-refcopy | 0.0.2 | 21:03 | published at once |
| 6 | gwz-repo-contract | 0.0.2 | 21:14 | refused, retry after 21:06:02; waited |
| 7 | gwz-history-check | 0.0.2 | 21:24 | refused, retry after 21:16:02; waited |
| 8 | gwz-local-import | 0.0.2 | 21:34 | refused, retry after 21:26:02; waited |
| 9 | gwz-repo-factory | 0.0.2 | 21:45 | refused, retry after 21:36:02; waited |
| 10 | gwz-repo-inspect | 0.0.2 | 21:55 | refused, retry after 21:46:02; waited |
| 11 | gwz-work-detector | 0.0.2 | 22:06 | refused, retry after 21:56:02; waited |
| 12 | gwz-local-disposal | 0.0.2 | 22:06 | published at once |
| 13 | gwz-workspace-install | 0.0.2 | 22:16 | refused, retry after 22:16:02; waited |
| 14 | gwz-core | 1.0.12-rc.1 | 22:16 | published at once |

The final ledger line read
`ok (14 published, 0 already on crates.io, of 14 crate(s); 7 rate-limit wait(s))`.
Every crate was visible on the first index poll after `cargo publish` returned.
gwz-core published without a wait because the name already existed, so the
update limit applied instead of the new-crate limit.

## 4. What the run answered

- **U1, duration.** The publish job took 73 minutes 11 seconds, under a third of
  its 240-minute timeout.
- **U4, index visibility.** Stable cargo's own wait is enough. The publisher's
  extra poll never had to wait.
- **U7, refill timing.** crates.io refills new-crate tokens on a fixed ten-minute
  grid. Every refusal named a retry time at the same seconds past a ten-minute
  mark, whenever the attempt was made. The burst covered the first five names and
  each later name took one grid token, so the grid fixed the 22:16 finish.
  Retrying at the named time instead of after a flat 620 seconds would not have
  ended the run sooner. gwz-local-disposal needed no wait because
  gwz-work-detector's retry landed just after the 22:06:02 refill.
- **U6, docs.rs.** On 2026-09-14 docs.rs reported a successful documentation
  build for all fourteen crates. Every crate page serves its README; one
  gwz-copy-contract README fetch returned HTTP 500 and three retries returned the
  page, and its package carries `README.md`.

## 5. The Windows verification failure

`tests/publish_workflow.rs`, test
`release_workflow_gates_publication_on_the_linux_verification_alone`, asserted
`RELEASE_WORKFLOW.contains("needs: verify\n")`. The Windows checkout converts the
workflow to CRLF line endings, so the literal newline never matched. The
Windows leg passed every earlier result block (2083 tests passed, 1 ignored)
and stopped at this one: 12 passed, 1 failed. The failure is in a test, and the
product code is unaffected.

Reproduced locally by converting the workflow to CRLF, which gave the same
12 passed and 1 failed. The fix compares whole lines through `lines()`, which
strips either ending; the test binary passes under CRLF and under LF. Committed
as gwz-core `7b3831c` with root `8039767`. The hosted Windows leg first runs the
fix at the next tagged release.

## 6. After the run

- The environment secret was deleted on 2026-09-14 with
  `gh secret delete CARGO_REGISTRY_TOKEN --repo owebeeone/gwz-core --env crates-io`.
  No environment secrets remain.
- Still the operator's, under S2.3: revoke the token on crates.io, and configure a
  trusted publisher for each of the fourteen crates (GitHub, owner `owebeeone`,
  repository `gwz-core`, workflow `release.yml`, environment `crates-io`). The
  `gwz` entry waits for S3.2, which decides the workflow file crates.io must trust.
- The rehearsal versions stay published until S4.2 yanks them after the real
  1.0.12.

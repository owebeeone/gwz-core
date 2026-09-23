# GWZ 1.1.0 plan

Status: **accepted at plan SHA-256 `9d49af85bd340addc1c35e6c11eefe3e4ab9f2bc2a9143b8f13f3af5a7fc8f62` after [Consistency-2](GwzV110Plan-ReviewConsistency-2.md) and [Safety-2](GwzV110Plan-ReviewSafety-2.md) reported GO; this accepts the plan text only**. The status sentence was added after that GO. Round-1 NO-GO on `a52cd7a8…` was remediated in [GwzV110Plan-RemPlan.md](GwzV110Plan-RemPlan.md). Round-2 NO-GO on `6ec8f7e7…` was remediated in [GwzV110Plan-RemPlan-1.md](GwzV110Plan-RemPlan-1.md). Writing this document does not implement, commit, tag, push, or publish.
The product release this plan leads to is **v1.1.0** of `gwz-core`, the
`gwz` CLI, and `gwz-py`. The published line today is **1.0.17**. When this
plan is accepted, amend `dev-docs/CurrentProgramCheckpoint.md` so it names
this plan and no longer describes the timeout work and Q6 as paused-only.

Authorities that stay in force:

- [GwzRemoteTransportDesign.md](GwzRemoteTransportDesign.md) and
  [GwzRemoteTransportPlan.md](GwzRemoteTransportPlan.md) Phase 6.
- [GwzRemoteTransportReleaseReadiness.md](../../dev-docs/GwzRemoteTransportReleaseReadiness.md).
  This plan is the resume of that paused gate, aimed at v1.1.0.
- The accepted timeout plan
  [GwzRemoteTransportAlphaTimeoutPlan.md](GwzRemoteTransportAlphaTimeoutPlan.md).
  This plan does not reopen the two clocks. It closes that plan's open S5.1
  and S5.2 before the release tag, including S5.2's production-graph stall
  regression. The local alpha was rebuilt before S5.1; that rebuild does not
  close S5.1. This plan amends only the timeout plan's "record the verdict
  before rebuilding" timing: the review covers the tree already rebuilt, and
  no further alpha rebuild and no v1.1.0 tag happen before that GO.
- [GwzRemoteTransportQualification.md](GwzRemoteTransportQualification.md)
  for what Batch A already ran.
- `gearu` (`https://github.com/owebeeone/gearu`) for every repository that
  still needs a release script. `gwz-core`, `gwz-cli`, and `gwz-py` keep
  their existing `scripts/release.py`.
- Review loop: `/Users/owebeeone/.claude/skills/review-loop/SKILL.md`, under
  `dev-docs/AgentProcessRules.md` as amended by
  `dev-docs/GwzProcessOptimization.md`.

## 1. Outcome

v1.1.0 ships the accepted endpoint-owned SSH and gh-only HTTPS transport,
with connection pooling, as supported behavior of the normal `gwz`,
`gwz-core`, and `gwz-py` builds. A developer `RUSTFLAGS` cfg is no longer
the only way to reach that runtime.

The release train publishes the new crates first, in dependency order, then
the three product repositories at `v1.1.0`. `gearu` prepares, tests, commits,
and tags the new crates and creates their GitHub Releases. Registry upload
stays in each repository's `release.published` workflow. Gearu does not run
`cargo publish`.

## 2. v1.1.0 scope

| In this release | Recorded as unsupported in 1.1.0 |
|---|---|
| macOS ARM64, Linux x86-64, Windows x86-64 | Linux ARM64, macOS x86-64 |
| SSH and gh-only HTTPS, both placements, one pool | iroh, a physical byte carrier, a separate-process wire |
| The two setup clocks from the accepted timeout plan, with 9 s stall and 30 s aggregate defaults per the accepted retry plan | A new timeout flag, or a changed 60 s idle default |
| Python using the same `gwz-transport` pool as Rust | A second pool implemented in Python |
| New crates for this libgit2, the forked git2 bindings, `gwz-transport`, and `gwz-git` | Publishing those forks as `libgit2-sys` or `git2` |

Windows work runs on `ssh gianni@dabeest`. The default SSH shell there is
bash (mingw). Commands are bash commands. The work root is the E: drive,
mingw path `/e/gwz-tests/<name>`. Earlier Q4 and Q6 trees under `D:/gwz-tests`
stay historical evidence. This plan does not resume them and does not write
there. Cargo config isolation from the Q4 remediation still applies: an
ancestor Cargo config, or an unreadable config, refuses the run.

Phases 4, 5, and 8 redact agent-socket paths, known_hosts bodies, and `gh`
tokens or headers from retained evidence. A secret in filed evidence fails
that step. Python-visible errors in S6.3 follow the same rule.

Narrowing Linux ARM64 and Intel Mac out of 1.1.0 is a scope decision of this
plan. It is not a silent pass of those matrix rows. Putting either platform
into the supported set later is an amendment, not a footnote on the tag.

## 3. Phases

### Phase 1 — Python transport design (milestone: an accepted design, no product code)

`gwz-py` links only `gwz-core`. `configure_transport_runtime` and
`transport_capabilities` are core protocol messages. Production core does
not depend on `gwz-transport`. Phase 6 implementation waits on this design.

- **S1.1: write the design** *(`gwz-py/dev-docs/GwzPyTransportDesign.md`;
  design only, 0 product lines)*. One pool, owned by `gwz-transport`, shared
  with the Rust host. `gwz-py` calls a Python binding of that crate for
  session and pool management. The design states the package boundary
  (bindings inside `gwz-transport`, or a separate package), what happens to
  the existing core transport messages, and how credential locality and
  gh-only HTTPS stay endpoint-owned. It does not add a Python pool, a second
  deadline policy, or a new wire message. Either boundary has a publisher in
  Phase 8: inside `gwz-transport`, that repo's release is the publisher; a
  separate package is Phase 8 step 3 and is not left implied.
  If S1.1 chooses that separate package, Phase 2 re-enters for it alone:
  S2.1 records the fifth name and owner, and S2.2 and S2.3 add its
  `gearu.toml` and publish workflow before Phase 8 step 3. `gearu plan`
  on that repo is part of the Phase 2 exit for that branch.

- **S1.2: review that design** *(review documents beside the design; 0
  product lines)*. Run the review loop on the design text only. Two
  peer-blind axes, Consistency and Safety. Remediation is capped at two
  rounds. A third architectural root stops the lane. Implementation of the
  binding does not start before both axes GO.

S1.2 cannot start before S1.1. Nothing else in this plan edits the Python
binding before that GO.

### Phase 2 — Crate identity and gearu (milestone: `gearu plan` succeeds for each new package; nothing is tagged)

Can proceed beside Phase 1. Publication waits for Phase 8.

The C tree `owebeeone/libgit2` at `b172e3d18` stays the vendored source. It
does not become a crates.io crate by itself. `git2-rs` owns the two new Rust
packages: a sys crate that vendors that tree, and a `git2` API crate that
depends on the sys crate by version. `gwz-transport` and `gwz-git` are the
other two first publishes. `gwz-git` depends on the new `git2` API crate, not
on a path to `../git2-rs`.

- **S2.1: record the four crate names and owners** *(a short section in this
  plan's companion note, or in the gearu configs once chosen; 0 runtime
  lines)*. The names are new. `libgit2-sys` and `git2` stay the rust-lang
  crates. Each package records `repository`, `license`, and `description`.
  `publish = false` remains until that package's Phase 8 release commit.

- **S2.2: install gearu guidance and `gearu.toml`** *(`gearu init` plus one
  `gearu.toml` in `git2-rs`, `gwz-transport`, and `gwz-git`; well under 500
  lines total)*. `[rust]` manifests, `cargo test --locked` and
  `cargo publish --dry-run` as checks, `tag_prefix = "v"`, `github_repo` set.
  `git2-rs` has one `gearu.toml` and one tag for both new packages. Gearu
  does not release the sys crate and the API crate as two tags of that
  repo, and it does not use `--dependency-tag` between them. Cross-repo
  edges (`gwz-git` to the API crate, and a separate binding package if S1.1
  names one) are registry dependencies: `version = "=<published>"` and no
  `git` key. Gearu may verify that the upstream tag exists. It must not
  write a git pin into the published manifest. A dependency override, when
  one is required, is `gearu release <version> --dependency-tag NAME=vX.Y.Z`
  with `NAME` the configured dependency name. `cargo publish --dry-run` on
  the release commit must accept that manifest. `gwz-transport` has no git2
  dependency. `gearu plan` for the intended first version passes on each
  repo. First versions stay the versions already in those manifests unless
  `gearu plan` requires a bump. The product tag is still `v1.1.0`; these
  crates are not required to be numbered 1.1.0.

- **S2.3: add the publish workflow** *(one `release.published` workflow per
  new repo; under 500 lines total)*. CI runs `cargo publish`. No token is
  stored in the repo. A brand-new crate name cannot take a trusted publisher
  until the name exists, as in `GwzCratesIoPlan.md`. The first publish of
  each new name uses an operator-held API token outside the repository.
  After that publish is visible, the operator configures the trusted
  publisher and later versions are tokenless. The `git2-rs` workflow
  publishes the sys crate, waits until the index shows it, then publishes
  the API crate, from the single tag. `gearu plan` does not create the
  publisher and does not run that first token publish.

### Phase 3 — Accept the open transport corrections (milestone: the timeout change and the Q6 retirement fix have GO)

Can proceed beside Phases 1 and 2. The timeout implementation is already in
the working tree and in the rebuilt local `gwz-alpha`. This phase reviews
and qualifies it. It does not retune the clocks.

- **S3.1: timeout plan S5.1** *(review note beside the timeout plan; 0
  product lines)*. Review the timeout plan's S3.1, S3.3, and S4.1 against
  its §2. Stall resets only on a completed native attempt. Idle slices do
  not. The aggregate does not reset. Interaction pauses both. Zero disables
  network deadlines and still disposes. A late result is not reused and is
  not `PermissionDenied`. `stall` and `aggregate` stay distinct. The alpha
  already rebuilt before this review does not close S5.1. Record the verdict
  before any further alpha rebuild, any further alpha claim, and the v1.1.0
  tag.

- **S3.2: Q6 aggregate review** *(review documents; 0 product lines)*.
  Retained Code and State reviewers, one aggregate gate, on the monotonic
  retirement fix, physical-cleanup accounting, the runner diagnostic
  correction, and the bounded measurement claims. Batch A results stay a
  baseline until this GO. Do not rerun suites that already passed unless a
  finding requires it.

- **S3.3: timeout plan S5.2** *(production-graph regression, then live
  fetches; 0 product lines beyond fixes a failure forces)*. After S3.1.
  This step stays open unless the production-graph regression has passed:
  one idle stage expires with reason `stall` while the aggregate is still
  ahead. A tree whose stall path is unwired fails here even if default cold
  fetches pass. Then repeat the cold live fetches at the default timeout
  and retain any setup-timeout reason. A pass with `--ssh-timeout 15`, or a
  cold fetch that succeeds only because `connect_ms` is 10,000, does not
  close this step. If the default still fails, record the retained reason
  and stop. Do not raise the aggregate inside this step. S7 and Phase 8
  wait on this step.

### Phase 4 — Windows integrated transport (milestone: SSH and HTTPS run on Windows x86-64 through the same host)

The Unix modules in `ssh_network`, `agent_socket`, `ssh_key_auth`, and
`ssh_local` are `cfg_if` unix blocks. Windows gets sibling modules inside
those same `cfg_if` blocks. A `#[cfg]` on an import is not the boundary.
The stall and aggregate clocks stay the `Control` already implemented;
Windows waits call the same `begin_slice` / `end_slice` rule.

- **S4.1: record the dabeest toolchain** *(a short evidence note; 0 product
  lines)*. Over `ssh gianni@dabeest`, in mingw bash, confirm Rust 1.95, a
  writable `/e/gwz-tests` root, and the absence of ancestor Cargo config
  before any build. Target dir stays on E:. Do not compile Windows on the
  Mac.

- **S4.2: Windows network and known_hosts** *(`ssh_network.rs` windows
  module; aspirational < 500 lines)*. TCP connect, handshake wait, and
  known_hosts reads use `Control`. An idle slice expires as `stall` while
  the aggregate is ahead, by the same tests the Unix module already has,
  compiled for Windows.

- **S4.3: Windows agent channel** *(`agent_socket.rs` windows module;
  aspirational < 500 lines)*. Speak the agent the mingw OpenSSH environment
  on dabeest actually exposes. Exact-agent support stays unclaimed until a
  fixture proves a selected identity. A missing agent is a refusal, not a
  fallback to another key store.

- **S4.4: Windows key paths and endpoint assembly** *(`ssh_key_auth.rs` and
  `ssh_local.rs` windows arms; aspirational < 500 lines)*. Home and
  known_hosts come from the endpoint environment, as on Unix. Assembly does
  no trust or agent I/O on the caller.

- **S4.5: integrated build on E:** *(expand the parent construction gates,
  then the candidate build and the existing integrated host/endpoint
  fixtures; aspirational < 500 lines of cfg wiring plus compile fixes)*.
  The sites that are `cfg(all(unix, gwz_transport_candidate))` today,
  including `transport_host`, `git::endpoint`, and the binding construction,
  become `cfg_if` unix|windows arms under `gwz_transport_candidate` in this
  step, so the Windows candidate actually contains those modules. Phase 7
  removes the candidate switch. It is not the first time those sites compile
  on Windows. SSH clone/fetch and HTTPS clone/fetch against the disposable
  fixtures pass on dabeest. A Windows run that skips the Unix-gated tests
  is not this step.

S4.2, S4.3, and S4.4 can proceed together after S4.1. S4.5 waits on all
three. S4 does not wait on the Python design.

### Phase 5 — Measurements and construction defaults (milestone: defaults chosen inside the frozen caps)

Mac and Linux rows can start after S3.2. Windows rows wait on S4.5.
Historical prototype numbers and the Batch A loopback medians are context.
They are not this exit.

- **S5.1: aggregate operations** *(measurement evidence; 0 product lines)*.
  Workspace fetch, push plus post-push reads, both placements, long-lived
  reuse, and large packs, on macOS ARM64 and Linux x86-64. Keep
  fixture setup out of the timers. Report medians and ranges. Preserve Git
  results.

- **S5.2: coalescing** *(measurement evidence; 0 product lines)*. Compare
  immediate emission with coalescing settings including 100 ms. 100 ms is
  an evaluation point. Report connect counts as well as wall time.

- **S5.3: sustained memory** *(measurement evidence; 0 product lines)*.
  Bounded memory under sustained backpressure. A process RSS sample that
  includes fixtures does not close this step.

- **S5.4: choose defaults** *(construction-default change plus a note of the
  measurements that selected them; aspirational < 500 lines)*. After S5.1,
  S5.2, and S5.3. Coalescing,
  buffer and queue sizes, wait and cleanup deadlines, and the endpoint pool
  ceilings, at or below every frozen hard cap. The 60-second idle default
  stays. Wire fields, tags, and hard ingress caps stay. A changed hard cap
  is a design amendment, not this step.

- **S5.5: repeat the applicable rows on dabeest** *(evidence; 0 product
  lines)*. After S4.5 and S5.4. The same operations the Windows host can
  run, on E:, through mingw bash. Record any row the host cannot run as
  unsupported for 1.1.0 rather than as a pass. S7 waits on this step.

- **S5.6: Design §11 sign-off** *(a cell table; 0 product lines)*. For every
  cell of Design §11, on each of the three 1.1.0 platforms, record either an
  evidence ID or an explicit unsupported mark. An unsupported cell is not
  advertised. gh-authenticated HTTPS is advertised only after a real-account
  `gh` operation. A fixture is not that proof. TLS and proxy parity are the
  same: advertise them only with the evidence Design §12 requires, or mark
  them unsupported. S7.2 cannot advertise a cell this table does not cover.
  A mark of unsupported on a cell that §2 lists as in this release fails
  S5.6 and blocks S7.5 and Phase 8, unless an accepted amendment first
  removes that cell from §1 and from the §2 "in this release" column.
  Linux ARM64, macOS x86-64, and exact-agent-until-proven may be marked
  unsupported without that amendment. S7.1 waits on this step.

### Phase 6 — Python integration (milestone: `gwz-py` uses the accepted binding)

Starts only after the Phase 1 GO. The design's package boundary decides
which repository owns S6.1. If the bindings live in `gwz-transport`, that
repo's Phase 8 release includes them and waits on this phase.

- **S6.1: the binding** *(the package the design names; aspirational < 500
  lines)*. Expose the transport session and pool operations the design
  lists. Deadlines stay the Rust clocks. No Python reimplementation of the
  pool.

- **S6.2: `gwz-py` calls that binding** *(`gwz-py` native extension, the
  Python API surface the design names, and `gwz-py/RELEASE.md`; aspirational
  < 500 lines)*. The extension links `gwz-transport` (or the binding package)
  as well as `gwz-core`. Fetch and push from Python use the same pool as the
  CLI. Existing core messages that only report capabilities stay reporters.
  Amend `RELEASE.md` and the publish workflow so every native dependency pin
  is named. The binding crate is a registry `version` pin with no `git` key
  and no sibling path. `gwz-core` on the release branch is `=1.1.0` from
  crates.io. `GwzCratesIoPlan.md` D7's git-tag-only core pin is not the
  1.1.0 form.

- **S6.3: focused tests** *(tests named by the design; aspirational < 500
  lines)*. One pool serves two Python operations against a disposable SSH
  or HTTPS fixture. A gh failure and an unsupported proxy still refuse.
  Credential material does not appear in the Python-visible errors.

### Phase 7 — Production activation (milestone: an ordinary build on the three platforms constructs the transport, and the activation review has GO)

Depends on S3.1, S3.2, S3.3, S4.5, S5.4, S5.5, S5.6, and Phase 6. The
dependency sketch is normative.

- **S7.1: ship the runtime in the normal build** *(remove the candidate
  switch from the sites Phase 4 already opened to Windows; aspirational
  < 500 lines of wiring)*. macOS, Linux x86-64, and Windows x86-64 construct
  the accepted host without a private `RUSTFLAGS` switch. This step does not
  introduce Windows into those sites; S4.5 already did. Keep each platform
  arm inside `cfg_if`. Credential locality and gh-only authenticated HTTPS
  stay as designed. Paths that 1.1.0 does not support stay native and are
  listed in the migration notes.

- **S7.2: help, migration notes, and the route ledger** *(docs and the
  ledger; aspirational < 500 lines)*. Configuration and help match the
  shipped switches. The notes state the gh-only HTTPS policy. Every
  advertised cell is one that S5.6 marked with evidence. An unsupported cell,
  a native route, or a cell with no S5.6 row fails this step. Windows
  exact-agent stays false, in capabilities and in the ledger, until a named
  fixture GO. Selected-identity Windows SSH is not advertised before that GO.

- **S7.3: clean consumer builds** *(no product feature work)*. From the
  workspace pins, build the CLI and the Python extension on macOS ARM64 and,
  on dabeest, the Windows CLI and extension. Linux x86-64 uses the existing
  CI host. A path-only developer build is not the release proof; Phase 8
  repeats the consumer build against the published crates.

- **S7.4: observation recheck** *(evidence; 0 product lines)*. Recheck
  observation attribution, the offered / authenticated / reused distinctions,
  and private-member suppression, on the activated tree. The route ledger
  does not substitute for this step.

- **S7.5: activation and release review** *(dual review documents; 0 product
  lines)*. After S7.3 and S7.4. Consistency and Safety, peer-blind, on the
  settled activated tree. Add Surface if S7.2 changed a public switch or
  help page. Phase 8 does not start without this GO. Exiting earlier phases
  does not authorize a tag or a publish.

Transport Plan Phase 6 exit rows map as follows. A row with no step is not
waived.

| Phase 6 exit row | Step |
|---|---|
| Attributable results at exact revisions | S5.1–S5.5, S3.3 |
| Complete acceptance matrix | S5.6 |
| Tuned defaults within frozen caps | S5.4, after S5.3 |
| Compatibility and package build checks | S7.3, Phase 8 |
| Aggregate review | S7.5 |
| Network-entry ledger, no silent native route | S7.2 |
| Observation attribution, offered/authenticated/reused, private-member suppression | S7.4 |
| Unsupported capabilities recorded; scope reduction is an amendment | §2 and S5.6 |
| Raw campaign data stays private | §2 evidence rule |

### Phase 8 — v1.1.0 release (milestone: the three product tags exist and their publish jobs have succeeded)

Depends on S7.5 and S2.3. The operator runs the commands. This document does
not run them. The sketch is normative: S3.3 and S5.5 are on the path here.

If any step fails after a push, a GitHub Release, or a registry publish,
stop. Do not run a later product tag. Do not claim v1.1.0 complete. Record
the published artifact IDs. Resume only with a new patch or release-candidate
version. Never move or reuse a tag.

1. **`git2-rs`, one release.** Before `--push`, `git rev-parse` of the
   vendored libgit2 tree must equal
   `b172e3d187a4b6866fd9f696f40a1b8e7f56d348`. Any other hash refuses the
   release. Then `gearu release <version> --push --github-release`. The
   workflow publishes the sys crate, waits until the index shows it, then
   publishes the API crate, from that single tag. There is no second tag
   and no `--dependency-tag` between those two packages. The first publish
   of each new name uses the operator-held token from S2.3.

2. **`gwz-transport`.** `gearu release <version> --push --github-release`.
   This can run beside step 1. If the bindings live in this repo, this step
   waits on Phase 6 and is their publisher.

3. **Separate binding package, only if S1.1 chose one.** Its own
   `gearu release <version> --push --github-release` after `gwz-transport` is
   visible, with a registry version pin and no `git` key. If S1.1 put the
   bindings inside `gwz-transport`, this step does not exist.

4. **`gwz-git`.** After the API crate is visible on crates.io:
   `gearu release <version> --dependency-tag <api-crate-name>=vX.Y.Z --push --github-release`,
   using the name configured in S2.2. The release commit's Cargo.toml edge
   is `version = "=<that version>"` with no `git` key. `cargo publish
   --dry-run` must already have accepted it.

Product repositories, each with the existing release script, tag `v1.1.0`.
Each waits until the previous product crate is visible. Pins are registry
versions, not git pins and not sibling paths.

5. `gwz-core`: `scripts/release.py v1.1.0 --push`. The release commit pins
   `gwz-transport` and the new `git2` API crate by the versions just
   published. Internal gwz-core crates bump the way the 1.0.12 script bumped
   them.
6. `gwz-cli`: `scripts/release.py v1.1.0 --push`. The `release` branch pin
   becomes `gwz-core = "=1.1.0"`.
7. `gwz-py`: `scripts/release.py v1.1.0 --push`, after `gwz-core` 1.1.0 and
   the binding's publishing step (step 2 or step 3) are on their registries.
   Wheels use the pins S6.2 wrote into `RELEASE.md`.

After every product job has succeeded: `cargo install gwz --version 1.1.0
--locked` and a Python install of `gwz` 1.1.0 succeed from the registries
alone on macOS ARM64, on Linux x86-64, and on dabeest. CI of the tag is not
that Linux proof. `gwz --version` and the Python package version both report
1.1.0. The route ledger's advertised commands work on those hosts. A fixture
smoke does not stand in for an S5.6 cell that required a real-account `gh`
operation.

## 4. Dependency sketch

```text
S1.1 ── S1.2 ── S6.1 ── S6.2 ── S6.3 ── S7.1
S2.1 ── S2.2 ── S2.3 ──────────────────────── Phase 8
S3.1 ── S3.3 ─────────────────────────────── S7.1
S3.2 ── S5.1 ── S5.2 ── S5.3 ── S5.4 ── S5.5 ── S7.1
                         S5.6 ── S7.1
S4.1 ── S4.2 ─┐
        S4.3 ─┼── S4.5 ── S5.5
        S4.4 ─┘
S7.1 ── S7.2 ── S7.3 ── S7.4 ── S7.5 ── Phase 8
```

S1 and S2 can run together. S3 can run with them. S4.2, S4.3, and S4.4 can
run together after S4.1. S5.1 can run with S4 once S3.2 has GO. S5.4 waits on
S5.1, S5.2, and S5.3. S6 waits on S1.2. S7.1 waits on S3.3, S4.5, S5.5,
S5.6, and S6. Phase 8 waits on S7.5 and S2.3.

## 5. Out of scope

- Replacing `scripts/release.py` in `gwz-core`, `gwz-cli`, or `gwz-py`
  with gearu.
- Publishing under the names `libgit2-sys` or `git2`.
- Treating Linux ARM64 or macOS x86-64 as 1.1.0 platforms.
- iroh, a physical carrier, or a separate-process transport.
- Changing the 60-second idle default, or adding a frozen hard cap. The stall
  default is 9 seconds and the aggregate default is 30 seconds, per
  `GwzRemoteTransportRetryPlan.md`.
- A Python implementation of the pool.
- Running Phase 8 because this file exists. The operator runs it after the
  phase exits above.

Candidate S6.3 clarification (2026-09-24, pending Python concurrency design
review): the [corrected Python design](../../dev-docs/GwzPyTransportConcurrencyDesign-1.md)
requires two genuinely overlapping Python operations on **one** Client and
pool, with independent results and cancellation. The current Python 1.1
session has only a local endpoint; explicit CLI placement must fail with a
typed error before credential access until a separate CLI capacity-owner
handshake is designed. This candidate does not close S6.3 or lift the Phase 6/7
NO-GO by itself.

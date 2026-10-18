# GwzV110Plan — Consistency-AXIS REVIEW

**Review object:** Uncommitted plan `/Users/owebeeone/limbo/gwz-dev/gwz-core/dev-docs/GwzV110Plan.md` at SHA-256 `a52cd7a8c25c5887c99db3c839fbf2f069855b5a874e6111e039c3f3f8320f31` (not in gwz-core HEAD). Status: draft plan text only. Verified at start and end of review with `shasum -a 256 gwz-core/dev-docs/GwzV110Plan.md` — hash unchanged.
**Baseline:** `.` `9c0008870ecd8f209611ebd99fb1598a14021da9`; `gwz-core` `102015a5ff3abc059be699f801d1412b0fd01c8a`; `gwz-cli` `ab59011db0ee00ab0c032fc23fc06b2578bf7b68`; `gwz-py` `d07d55dacb1725d9306be9c04d157ac29a78e000`; `gwz-transport` `aa40936d0805e8cb60f8027615abe20d4f2045e4`; `gwz-git` `aa77c2ce5ad0bf6b4f4b64b2d8fd75e8547c3b4c`; `git2-rs` `ce78628308e11b4e8901d5061602619109bce21a`; `libgit2` `b172e3d187a4b6866fd9f696f40a1b8e7f56d348`. Sources read from those HEADs plus the named uncommitted object; gearu CLI/docs from `/Users/owebeeone/limbo/gearu`. Out-of-scope dirty trees not inspected for verdict content.
**Date:** 2026-09-23
**Axis:** CONSISTENCY — refute fitness of the plan against its controlling graph (internal contradictions; verbatim agreement with cited contracts; satisfiability of its own steps; unstated impacts). Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on it. Filed verbatim by the lane owner.

**Verdict: NO-GO** — 0 P0, 4 P1, 5 P2, 2 P3 open. I pre-commit to GO on a revision that resolves P1-1, P1-2, P1-3, P1-4, P2-1, P2-2, P2-3, P2-4, and P2-5 as specified.

---

## 0. Evidence base

| Source | How used |
|---|---|
| Object `GwzV110Plan.md` @ `a52cd7a8…` | Full read; §§1–5, phases, sketch |
| `GwzRemoteTransportDesign.md` §10–§12 (esp. §11 matrix, §10 timeout domains) | Cited authority; exit obligations |
| `GwzRemoteTransportPlan.md` Phase 6 + exit evidence (lines 346–374) | Discharge mapping |
| `GwzRemoteTransportReleaseReadiness.md` items 1–6 | Resume gate mapping |
| `GwzRemoteTransportAlphaTimeoutPlan.md` §2, Phase 5 S5.1/S5.2, §5 | S3.1/S3.3 discharge |
| `GwzRemoteTransportQualification.md` Remaining Phase 6 (112–119); D: roots (22) | Remaining work; path history |
| `gwz-py/Cargo.toml`, `gwz-py/RELEASE.md` | Product release / pins |
| `gwz-transport/Cargo.toml`, `gwz-git/Cargo.toml` | `publish = false`; path `git2` |
| `git2-rs/Cargo.toml`, `git2-rs/libgit2-sys/Cargo.toml` | Published names `git2` / `libgit2-sys`; one repo, two packages |
| gearu `README.md`, `docs/ReleaseProcess.md`, `docs/Configuration.md`; `gearu --help` / `plan --help` / `release --help` | Command shape; no `cargo publish`; `--dependency-tag` |
| `gearu/src/gearu/release.py` dependency pin apply | Tag value written into `pin_field` |
| `gwz-core/src/lib.rs:49–52`, `git/mod.rs:4–7`, and other `cfg(all(unix, gwz_transport_candidate))` construction sites | Windows candidate construction reachability |
| `GwzCratesIoPlan.md` (facts on trusted publishing / git deps; D7) | Unstated impact / first-publish |
| `dev-docs/CurrentProgramCheckpoint.md` (timeout / Q6 pause heads) | Unstated impact |

Tuple SHAs and object hash re-checked at end: unchanged.

Deferred outcomes (unsupported Linux ARM64 / macOS x86-64; keep `scripts/release.py`; C libgit2 not a crates.io crate; 3 s / 10 s / 60 s clocks) were not filed as findings. Shape/satisfiability around those decisions remained in scope.

---

## 1. Findings

### [P1-1] Phase 8 steps 1–2 are not an executable gearu release of one `git2-rs` repo

**Location.** Object Phase 2 (lines 91–96, 104–113) and Phase 8 steps 1–2 (lines 274–279): one `gearu.toml` in `git2-rs` owning both new packages; then two sequential `gearu release <version> --push --github-release` with step 2 using `--dependency-tag` to step 1’s tag.

**Violated invariant.** Gearu prepares **one repository** per `gearu release`, creates **one** immutable tag, and uses `--dependency-tag` only to override a configured **cross-repository** dependency tag (`gearu release --help`; `docs/Configuration.md` 134–161; `docs/ReleaseProcess.md` 115–120). It does not publish package A then package B as two tagged releases of the same monorepo with an inter-package `--dependency-tag`.

**Reproduction / state sequence.** (1) S2.2 installs a single `gearu.toml` in `git2-rs` covering the sys + API manifests (matches current tree: `git2` + `libgit2-sys` under one repo). (2) Operator runs Phase 8 step 1 `gearu release … --push --github-release` for the sys crate. (3) Step 2 requires a second `gearu release` of the same repo with `--dependency-tag` pointing at step 1’s tag. Gearu either retags/rebumps the whole repo (second release) or cannot express “API package only, depending on prior same-repo sys tag” — `--dependency-tag` resolves `remote_tag_commit_at(dependency.url, tag)` on a **configured dependency URL**, not an in-tree package (`release.py` 138–147).

**Impact.** The first two links of the release train cannot be executed as written; everything that waits on those crates (gwz-git, product pins) is blocked.

**Remedy.** Rewrite steps 1–2 to one of: (a) **one** `gearu release` of `git2-rs` whose publish workflow publishes sys then API in dependency order from a single tag; or (b) **two repositories** with two `gearu.toml` files and a real cross-repo `[[dependencies]]` URL — and make S2.1/S2.2 say which. Remove same-repo `--dependency-tag` between sys and API unless (b) is chosen.

**Closure test.** Dry-run the chosen sequence against gearu’s model: `gearu plan` on the configured repo(s) with the intended manifests; show that no step requires a second tag of the same repo solely to unlock the sibling package; document the exact publish-job order for sys→API.

---

### [P1-2] Open Phase 1 binding-package boundary leaves Phase 8 step 7 unsatisfiable on one branch

**Location.** Phase 1 S1.1 (lines 69–76): package boundary open (“bindings inside `gwz-transport`, or a separate package”). Phase 6 (225–227): boundary decides S6.1 ownership. Phase 8 steps 1–4 (274–282): only sys, API, `gwz-transport`, `gwz-git`. Step 7 (292–294): waits until “the binding’s crate” is on its registry.

**Violated invariant.** A deferred choice may not leave a later mandatory release step without a publisher in the train (authority brief: deferred outcome ≠ deferred executability).

**Reproduction / state sequence.** Phase 1 GO chooses a **separate** binding package. S6.1 lands there. Phase 8 runs steps 1–4 as listed — that package is never released. Step 7’s precondition (“binding’s crate … on their registries”) never becomes true.

**Impact.** Release train dead-ends after product code exists; v1.1.0 cannot be cut on that design branch without an unstated extra publish.

**Remedy.** Either (1) constrain S1.1 so the binding is only inside a crate already in steps 1–4 (and say so), or (2) add an explicit Phase 8 step that publishes the separate binding package in dependency order, with the same wait-for-registry rule, before step 7.

**Closure test.** For each S1.1 boundary option, walk Phase 8 and show a named publish step and registry wait for every crate step 7 names.

---

### [P1-3] S2.2 gearu tag pins conflict with crates.io publication of the new Rust crates

**Location.** S2.2 (108–109): `[[dependencies]]` pins API→sys tag and `gwz-git`→API tag. Phase 8 (274–276, 282): wait until each crate is **visible on crates.io** before the next dependent; product pins use published versions (286–288).

**Violated invariant.** A crates.io package must not depend on a git source (`GwzCratesIoPlan.md` facts, lines 62–64). Gearu dependency pins write the **resolved git tag string** into `pin_field` and, when `lock_package` is used, require a **Git** `Cargo.lock` source (`docs/Configuration.md` 134–156; `release.py` 177–208; `dependency_checks.py`).

**Reproduction / state sequence.** Configure S2.2 as written with `pin_file`/`pin_key`/`pin_field = "tag"` (the documented shape). Run `gearu release` for `gwz-git` with `--dependency-tag` to the API tag. Cargo.toml gains/keeps a git+tag pin. CI `cargo publish` for `gwz-git` refuses the git dependency (or publishes a manifest that cannot resolve on the registry). The Phase 8 “visible on crates.io then next dependent” story assumes registry version edges, not gearu’s git-tag edges.

**Impact.** New-crate publish jobs fail or produce unloadable dependents; the train cannot both obey S2.2 and clear crates.io.

**Remedy.** Separate concerns: (1) gearu may **verify** remote tags exist without rewriting Cargo.toml to git pins; (2) published Cargo.toml edges must be **version-only** registry deps; (3) say how versions are bumped (manual, script, or gearu `pin_field = "version"` with a version string — not a `v…` tag dumped into `version`). Align Phase 8 waits with that model.

**Closure test.** For `gwz-git` (and API→sys if cross-repo), show a candidate `Cargo.toml` that `cargo publish --dry-run` accepts after the gearu release commit, with no `git =` on those edges.

---

### [P1-4] Trusted-publisher-only first publish of four new crate names is unsatisfiable

**Location.** S2.3 (115–120): workflow “trusted-publisher ready”, “no token in the repo”; operator “creates the crates.io trusted publisher for each new name **before** Phase 8”. Phase 8 first publishes of four new names (274–282). Authorities: `GwzCratesIoPlan.md` 32–36 (trusted publishing only after the crate exists; first new name needs an API token / bootstrap).

**Violated invariant.** You cannot attach a crates.io trusted publisher to a name that does not yet exist; first publication of a new name is outside the “tokenless trusted publishing only” path the plan mandates.

**Reproduction / state sequence.** Operator attempts to configure trusted publishers for the four S2.1 names before any version exists → crates.io has no crate to attach. Phase 8 GitHub Release fires a tokenless trusted-publish job → auth/publish fails for brand-new names. No bootstrap/placeholder step is in the plan (unlike the existing gwz bootstrap recorded in `GwzCratesIoPlan.md`).

**Impact.** First publish of the train fails; product v1.1.0 cannot depend on those crates.

**Remedy.** Add an explicit bootstrap (token or placeholder publish) **or** document a pending-publisher/bootstrap path per name before the tokenless workflow is the sole path; sequence “create trusted publisher” **after** first successful publish (as in CratesIoPlan S2.3), not before Phase 8 for nonexistent crates.

**Closure test.** For one new name, rehearse: first publish succeeds under the documented credential path; subsequent publish is trusted-publisher-only with no repo token.

---

### [P2-1] S4.5 Windows candidate integrated build contradicts S7.1’s still-unix construction gates

**Location.** S4.5 (181–186): `cfg(all(windows, gwz_transport_candidate))` is enough; SSH/HTTPS integrated fixtures must pass on dabeest. S7.1 (249–254): production wiring still targets sites that are `cfg(all(unix, gwz_transport_candidate))`. Controlling code at HEAD: `transport_host` and `git::endpoint` are behind `cfg(all(unix, gwz_transport_candidate))` (`lib.rs` 49–52; `git/mod.rs` 4–7); binding construction likewise (`transport_binding.rs` 8–9).

**Violated invariant.** A Phase 4 exit that requires Windows integrated host/endpoint fixtures cannot leave construction exclusive to `unix` until Phase 7, while Phase 7 text assumes those sites are **still** unix-only after Phase 4.

**Reproduction / state sequence.** Implement S4.2–S4.4 Windows modules inside endpoint files. Build with `RUSTFLAGS=--cfg gwz_transport_candidate` on Windows without expanding the parent `all(unix, …)` gates. `transport_host` / `endpoint` / binding runtime do not compile into the Windows candidate; S4.5’s “SSH clone/fetch and HTTPS clone/fetch … pass on dabeest” cannot run. If Phase 4 **does** expand those gates to windows, S7.1’s “still `cfg(all(unix, gwz_transport_candidate))`” is false.

**Impact.** Windows integrated qualification (ReleaseReadiness item 3; Qualification Remaining Phase 6) is scheduled in Phase 4 but unreachable as the cfg story is written.

**Remedy.** State in Phase 4 which construction/module gates must become `cfg_if` unix|windows under `gwz_transport_candidate` for S4.5; define S7.1 as removing the **candidate** requirement (ordinary build), not as the first time Windows appears in those sites — or move Windows integrated host proof to after that expansion and update S7.1 wording.

**Closure test.** On dabeest, candidate build lists `transport_host` / endpoint symbols for Windows; S4.5 fixtures run without requiring Phase 7 production (no-RUSTFLAGS) activation.

---

### [P2-2] Plan S3.1 / S3.3 do not close AlphaTimeoutPlan S5.1 / S5.2 as claimed

**Location.** Object authorities (14–17): “closes that plan’s open S5.1 and S5.2”. Object S3.1 (128–134), S3.3 (143–148). Controlling `GwzRemoteTransportAlphaTimeoutPlan.md` S5.1 (244–249), S5.2 (251–264).

**Violated invariant.** Closing steps must match the controlling plan’s gates, not a weakened paraphrase.

**Reproduction / state sequence — S5.1.** Controlling S5.1: review S3.1, S3.3, and S4.1 against §2; “Record the verdict **before rebuilding**.” Object Phase 3 intro (124–126): implementation “already in the working tree and in the rebuilt local `gwz-alpha`”; S3.1 only records verdict “before any further alpha claim and before the v1.1.0 tag.” Review-after-rebuild does not satisfy review-before-rebuild. Object S3.1 also does not name S4.1 late-result disposal as in-scope for that review.

**Reproduction / state sequence — S5.2.** Controlling S5.2 stays open unless the **S3.3 production-graph regression** has passed (“one idle stage expires with reason `stall` while the aggregate is ahead”); unwired stall path fails even if default cold fetches pass. Object S3.3 only forbids `--ssh-timeout 15` and “success only because `connect_ms` is 10,000”; it **omits** the production-graph stall gate.

**Impact.** The plan claims to close the accepted timeout plan while leaving its actual exit criteria unsatisfied — false composition of programme state before the v1.1.0 tag.

**Remedy.** Rewrite S3.1 to require the S5.1 review content (including S4.1) and either (a) rebuild-after-GO as in the timeout plan, or (b) an explicit accepted amendment of S5.1’s “before rebuilding” clause. Rewrite S3.3 to include the production-graph stall regression as a hard precondition, verbatim in spirit to timeout S5.2.

**Closure test.** Checklist: S5.1 verdict filed covering stall/aggregate/interaction/zero/late-result; S3.3 graph test red/green retained; default cold fetch evidence retained; no close on `--ssh-timeout 15` alone.

---

### [P2-3] Sustained-memory step S5.3 is outside the dependency sketch that feeds default selection

**Location.** Object S5.3 (207–209), S5.4 (211–216); §4 sketch (309): `S5.1 ── S5.2 ── S5.4 ── S5.5` (no S5.3). Controlling: Transport Plan Phase 6 (348–358) and exit evidence; ReleaseReadiness item 2 (40–44); Qualification Remaining Phase 6 (112–115) — measure sustained-memory **before** selecting defaults.

**Violated invariant.** Defaults may not be chosen while a required measurement row is optional/orphaned relative to the ordering graph the plan itself publishes.

**Reproduction / state sequence.** Agents follow §4: complete S5.1 and S5.2, skip S5.3, execute S5.4 “choose defaults.” Sketch and prose of §4 never require S5.3. Controlling Phase 6 / readiness / qualification all require sustained-memory evidence before defaults.

**Impact.** Phase 5 can “exit” with construction defaults that the controlling Phase 6 forbids; false readiness for Phase 7 activation.

**Remedy.** Insert S5.3 on every path into S5.4 in both the sketch and the prose (e.g. S5.1/S5.2/S5.3 → S5.4); state that S5.4 is blocked until S5.3 evidence exists.

**Closure test.** Dependency sketch and Phase 5 text both refuse S5.4 without a retained sustained-backpressure memory result that excludes fixture RSS as sole proof (per S5.3’s own close rule).

---

### [P2-4] Internal contradiction: whether S5.5 gates S7

**Location.** Phase 7 header (247): depends on Phases 3, 4, 5, and 6 (Phase 5 includes S5.5). Sketch (309): `… S5.4 ── S5.5 ── S7.1`. §4 prose (316–318): “S7 waits on S3, S4.5, S5.4, and S6” — **omits S5.5**.

**Violated invariant.** The plan’s dependency sketch, phase-dependency sentence, and Phase 7 header must agree.

**Reproduction / state sequence.** Implementer reads §4 prose, starts S7.1 after S5.4 without S5.5 Windows measurement rows. Sketch and “depends on Phase 5” still show S5.5 unfinished. Windows production activation can proceed without the Phase 5 Windows measurement exit the sketch requires.

**Impact.** Ambiguous Phase 7 entry; Windows measurement gate can be skipped while still claiming Phase 5 complete under one reading.

**Remedy.** Make one rule: either S7 waits on S5.5 (update prose) or S5.5 is post-default / non-blocking for S7 (update sketch and Phase 7 header) — and align with ReleaseReadiness’ Windows qualification expectation.

**Closure test.** Single dependency sentence matches the ASCII sketch and Phase 7 “Depends on …” line; no alternate reading.

---

### [P2-5] Silent drop of Transport Plan Phase 6 exit rows (observations; aggregate release review)

**Location.** Object claims Transport Plan Phase 6 as authority (10–11) and maps measurements/activation/release across Phases 5, 7, 8. Controlling Phase 6 exit evidence (366–372): among other items, “Recheck observation attribution, offered/authenticated/reused distinctions and private-member suppression”; “compatibility and package build checks, and **aggregate review**.” ReleaseReadiness item 6 (61–64): “aggregate activation/release review” before the publish sequence. Object text: no step rechecks observation attribution / offered/authenticated/reused / private-member suppression; Phase 8 (269–301) jumps to operator `gearu`/`release.py` with no aggregate activation/release review gate.

**Violated invariant.** A resume plan that cites Phase 6 may not omit required exit evidence rows without an explicit supersession list.

**Reproduction / state sequence.** Complete object Phases 5, 7, 8 as written. Observation-attribution recheck never runs. No filed aggregate activation/release review appears before tags. Transport Plan Phase 6 exit evidence remains incomplete while v1.1.0 is tagged.

**Impact.** Programme can declare rollout readiness without the observation and aggregate-review exits the controlling plan still requires.

**Remedy.** Add explicit steps (or a supersession table naming exact Phase 6 exit bullets waived/amended with authority) covering: observation attribution recheck; aggregate activation/release review before Phase 8 commands. Keep ledger/help (S7.2) but do not treat them as substitutes for those rows.

**Closure test.** Traceability table: each Phase 6 exit bullet → object step ID or dated amendment ID; no unmapped bullet.

---

### [P3-1] Unstated impact on CurrentProgramCheckpoint and pause heads

**Location.** Object presents itself as the resume of `GwzRemoteTransportReleaseReadiness.md` (12–13). Checkpoint heads (`CurrentProgramCheckpoint.md` 3–11, 44–62) still record: timeout plan accepted as **text only** (“No timeout implementation, alpha rebuild, or Q6 resume”); Q6 “paused”; resume via readiness doc.

**Violated invariant.** Implementing this plan without updating the checkpoint would leave the programme ledger contradicting live work (unstated impact on an uncited controlling ledger).

**Impact.** Later agents resume from checkpoint and re-pause or duplicate gates.

**Remedy.** Require a checkpoint amendment when this plan is accepted (status, tuple, which readiness items this plan owns).

**Closure test.** Checkpoint top sections name `GwzV110Plan` and stop saying “no timeout implementation / Q6 paused” once those phases start.

---

### [P3-2] Unstated impact on GwzCratesIoPlan D7 / gwz-py RELEASE.md pin story

**Location.** Object Phase 8 step 7 (292–294): wait for `gwz-core` and “the binding’s crate” on “their registries”; wheels from those pins. `gwz-py/RELEASE.md` 14–22, 109–115: release/`publish.yml` pin **gwz-core by git tag** and check out core **beside** the tree; only intentional branch difference is the gwz-core source. `GwzCratesIoPlan.md` D7 (210–228): gwz-py **stays on the git-tag pin** (registry core is open item O1).

**Violated invariant.** Adding a registry binding dependency and “registries” wording without amending RELEASE.md / CratesIoPlan leaves those documents describing a release that cannot pin the new edge.

**Impact.** Release script/workflow still only rewrite gwz-core; binding path/registry pin is unspecified; provenance/parity assumptions drift.

**Remedy.** Cite and amend `gwz-py/RELEASE.md` and note CratesIoPlan D7/O1 implications when S6.2 adds a second native dependency; define exact pin form (git vs crates.io) per dependency.

**Closure test.** RELEASE.md and publish workflow checks name every native dependency pin S6.2 requires; dry-run release notes show no sibling path for those crates.

---

## 2. Invariant analysis

| Invariant | Result |
|---|---|
| Internal consistency (§2 / phases / §4 sketch / §5) | **Fail** — P2-3, P2-4; S5.3 orphaned; S5.5 vs S7 disagreement |
| Phase 8 satisfiable given Phase 1 open boundary + Phase 6 | **Fail** — P1-2 |
| Phase 5+7 discharge Transport Plan Phase 6 exit evidence | **Fail** — P2-3, P2-5 (measurements partial; observation + aggregate review dropped) |
| S3.1/S3.3 match timeout S5.1/S5.2 | **Fail** — P2-2 |
| gearu commands / `--push` / `--github-release` / no local `cargo publish` | **Pass** for product wording (lines 35–38; matches gearu docs). **Fail** for dependency-tag / pin use in the new-crate train — P1-1, P1-3 |
| Crate ownership vs Cargo.toml | **Pass** on facts cited: `git2-rs` has two packages named `git2` and `libgit2-sys` today; `gwz-git` path-deps `../git2-rs`; plan correctly forbids publishing under those rust-lang names and requires version deps later. Ownership claim does not fix P1-1’s release mechanics |
| Windows host / shell / E: vs historical D: | **Pass** — object 50–55 distinguishes E: work from historical `D:/gwz-tests` Q4/Q6 evidence; does not reattribute Batch A to E: |
| Ordinary build vs `cfg(all(unix, gwz_transport_candidate))`; Phase 4 Windows cfg before Phase 7 | **Fail** — P2-1 |
| Unstated impacts (checkpoint, crates.io plan, gwz-py RELEASE) | **Fail** — P3-1, P3-2; first-publish TP also conflicts with CratesIoPlan facts — P1-4 |

Non-findings (attacked, did not stick): deferred platform outcomes; deferred release.py retention; deferred C-tree non-crate; deferred two-clock values; gearu’s refusal to `cargo publish` (aligned); D: vs E: historical wording.

---

## 3. Risks and next action

**Risk if implemented as written:** release train stalls on first new-crate publish; Python binding branch without a Phase 8 slot cannot ship; Windows integrated gate is scheduled before construction is reachable; timeout plan can be marked “closed” while S5.1/S5.2 gates remain open; Phase 6 observation/aggregate-review exits can be skipped into a tagged 1.1.0.

**Next action:** Remediate P1-1…P1-4 and P2-1…P2-5 in the plan text (release mechanics, binding publish step, cfg ordering, timeout discharge fidelity, S5.3/S5.5 graph, Phase 6 exit traceability). Re-run Consistency on the amended hash. P3-1/P3-2 may ship in the same revision to avoid ledger drift.

End-of-review tuple: unchanged from §0 Baseline; object hash still `a52cd7a8c25c5887c99db3c839fbf2f069855b5a874e6111e039c3f3f8320f31`.

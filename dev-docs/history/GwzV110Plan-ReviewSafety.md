# GwzV110Plan — Safety-AXIS REVIEW

**Review object:** Uncommitted plan `/Users/owebeeone/limbo/gwz-dev/gwz-core/dev-docs/GwzV110Plan.md` at SHA-256 `a52cd7a8c25c5887c99db3c839fbf2f069855b5a874e6111e039c3f3f8320f31` (not in `gwz-core` HEAD). Draft-stage review of plan text only; not an implementation acceptance. Date of object bytes verified: 2026-09-22.
**Baseline:** `.` `9c0008870ecd8f209611ebd99fb1598a14021da9`; `gwz-core` `102015a5ff3abc059be699f801d1412b0fd01c8a`; `gwz-cli` `ab59011db0ee00ab0c032fc23fc06b2578bf7b68`; `gwz-py` `d07d55dacb1725d9306be9c04d157ac29a78e000`; `gwz-transport` `aa40936d0805e8cb60f8027615abe20d4f2045e4`; `gwz-git` `aa77c2ce5ad0bf6b4f4b64b2d8fd75e8547c3b4c`; `git2-rs` `ce78628308e11b4e8901d5061602619109bce21a`; `libgit2` `b172e3d187a4b6866fd9f696f40a1b8e7f56d348`. Sources read as files under those HEADs plus the named uncommitted plan hash; `gearu release --help` / `gearu plan --help`; gearu `docs/ReleaseProcess.md`. Tuple rechecked identical at review end. Out-of-scope dirty bytes were not inspected for verdict content.
**Date:** 2026-09-22
**Axis:** SAFETY — what the text permits to go wrong (degraded/mixed-version paths; irreversible steps and preconditions; disclosure scale; stuck states; “never worse than status quo” under interleavings; scope creep / blast radius). Independent, adversarial, read-only. The other axis runs in parallel; nothing here relies on it. Filed verbatim by the lane owner.

**Verdict: NO-GO** — 1×P0, 3×P1, 4×P2, 1×P3 open. I pre-commit to GO on a revision that resolves P0-1, P1-1, P1-2, P1-3, P2-1, P2-2, P2-3, and P2-4 as specified (P2-5 and P3-1 may remain only if explicitly accepted as residual with the named mitigations landed).

---

## 0. Evidence base

| Source | Role |
|---|---|
| `gwz-core/dev-docs/GwzV110Plan.md` @ `a52cd7a8…0f31` | Object under review (entire text; phases 1–8, §4 sketch, §5 out of scope) |
| `GwzRemoteTransportDesign.md` §3–4, §9–12 | Credential locality, gh-only HTTPS, timeout domains, §11 matrix, §12 “before advertising”, no secrets in messages/errors, exact-agent rule |
| `GwzRemoteTransportPlan.md` Phase 6 exit (≈346–377, 391–392) | Complete acceptance matrix; scope reduction needs accepted amendment; publish/tag not because plan exists; aggregate/activation dual review |
| `dev-docs/GwzRemoteTransportReleaseReadiness.md` items 1–6 + HTTPS fixture caveat (≈34–69) | Q6 accept, Phase 6 tune, platform gaps, reproducible sources, activation, release gate + aggregate activation/release review; fixtures ≠ real GH |
| `GwzRemoteTransportAlphaTimeoutPlan.md` §2 + Phase 5 S5.1/S5.2 (≈39–82, 239–264) | Two clocks; S5.2 requires production-graph stall regression; `--ssh-timeout 15` / `connect_ms=10000` alone do not close |
| `GwzRemoteTransportQualification.md` (retirement §, Windows limits, Batch A) | Shared-session retirement P2 blocks rollout; Windows integrated Unix-gated; zero-test Unix executable ≠ pass |
| `gearu` `docs/ReleaseProcess.md` + CLI help | `--push` / `--github-release` irreversible relative to tag; non-transactional multi-repo order; `--dependency-tag NAME=TAG` |
| `dev-docs/AgentProcessRules.md` L1-32 | Activation is a separate reviewed change |
| Deferred outcomes (not findings) | Linux ARM64 / macOS x86-64 unsupported in 1.1.0; product repos keep `scripts/release.py`; C libgit2 not a crates.io crate; accepted two-clock constants |

Start and end tuple/hash match. No builds or tree mutations.

---

## 1. Findings

### [P0-1] Phase 8 can immutably tag a sys crate that is not the qualified `libgit2` tree

**Location:** `GwzV110Plan.md` Phase 2 (lines 91–96), Phase 8 step 1 (lines 274–278); contrasted with gearu `ReleaseProcess.md` (push/tag immutable; recovery = new version only).

**Violated invariant:** The release train’s first irreversible publish must be content-identical to the admitted C tree `owebeeone/libgit2` @ `b172e3d18` that qualification and this plan name as the vendored source. A path string or prose label is not authority for what was tagged.

**Reproduction / interleaving:**
1. S2.x installs gearu and a sys crate that *claims* to vendor `b172e3d18`.
2. Before Phase 8, the vendor directory drifts (wrong submodule, partial copy, alternate commit) while manifests still say the short SHA in docs.
3. Operator runs `gearu release <ver> --push --github-release` for step 1. Gearu verifies branch/tag/dependency-tag *names*, not the vendored C tree hash.
4. crates.io receives that version; steps 2/4/5 pin it; tags cannot move (`ReleaseProcess.md` “Never move or reuse a release tag”).

**Impact:** Entire 1.1.0 Git stack composes against an unreviewed C tree while the plan and notes still say `b172e3d18`. False composition of the qualified dependency into every consumer; recovery requires a new crate version train, not a retag.

**Remedy:** Before step 1’s `--push`, require an explicit content gate: recorded tree hash / `git rev-parse` of the vendored libgit2 equals `b172e3d187a4b6866fd9f696f40a1b8e7f56d348` (full SHA), checked in the release commit and in the Phase 8 runbook. Fail closed; do not tag.

**Closure test:** A deliberate wrong-vendor fixture refuses `gearu release --push`; a matching-vendor dry-run/plan passes. Document the check command beside step 1.

---

### [P1-1] S3.3 can “close” timeout plan S5.2 without the production-graph stall regression

**Location:** `GwzV110Plan.md` S3.3 (lines 143–148); controlling `GwzRemoteTransportAlphaTimeoutPlan.md` S5.2 (lines 251–264), especially “stays open unless the S3.3 production-graph regression has passed: one idle stage expires with reason `stall` while the aggregate is ahead” and “A tree where that stall path is unwired fails this step even if default cold fetches pass.”

**Violated invariant:** Closing the open timeout qualification (S5.2) requires proof that the stall path is wired under the default clocks, not merely that a default cold fetch eventually succeeds (including via the 10 s aggregate).

**Reproduction:**
1. S3.1 records a review GO on the clock change.
2. Operator runs only live cold fetches at default `--ssh-timeout` / default `connect_ms`; fetches succeed because aggregate is 10 000 ms or because stages complete without exercising idle-stall expiry.
3. S3.3 is marked closed under V110 text (which bans `--ssh-timeout 15` and “succeeds only because `connect_ms` is 10,000” but **omits** the stall-while-aggregate-ahead production-graph requirement).
4. Phase 7 activates the transport; Phase 8 tags. Users get a build where idle/hung stages may not expire as `stall` while aggregate remains ahead — the defect S5.2 exists to catch.

**Impact:** Release ships the timeout change without the controlling plan’s fail-closed proof. Setup hangs or mis-reasons timeouts; “never worse than status quo” fails for stuck SSH/HTTPS setup under the plan’s own default clocks.

**Remedy:** Rewrite S3.3 to import S5.2’s full closure predicate verbatim: production-graph regression (idle stage → `stall`, aggregate still ahead) is mandatory; unwired stall path fails even if cold fetches pass; then default cold fetches; retain reason on failure; no aggregate raise inside the step.

**Closure test:** Checklist item with command/evidence pointers for the stall regression **and** default cold fetches; a tree with stall unwired cannot exit S3.3.

---

### [P1-2] Production activation and Phase 8 tags have no dual-review gate

**Location:** `GwzV110Plan.md` Phase 7 (245–267) and Phase 8 (269–301); §4 note “Phase 8 waits on S7.3 and S2.3” (318); authorities claim Transport Plan Phase 6 and readiness remain in force (lines 8–13).

**Violated invariant:** Aggregate/activation gates require independent dual review (`GwzRemoteTransportPlan.md` ≈391–392); readiness item 6 requires “aggregate activation/release review” then the authorized publish sequence; `AgentProcessRules.md` L1-32 requires activation as a separate reviewed change. Publishing/tagging is not performed merely because a plan exists (Transport Plan Phase 6 milestone; V110 §5).

**Reproduction:**
1. Complete S3.x–S6.x and S7.1–S7.3 wiring/docs/builds as written (reviews only at S1.2, S3.1, S3.2).
2. Operator proceeds directly to Phase 8 `gearu release … --push --github-release` and `scripts/release.py v1.1.0 --push`.
3. Irreversible tags and GitHub Releases exist with no activation/release dual-review GO on the settled activated tree.

**Impact:** Ordinary installs receive the candidate transport (credential locality, gh-only HTTPS, Windows/Unix routes) without the mandated safety gate. Blast radius is every consumer of `gwz` / `gwz-core` / `gwz-py` 1.1.0; tags cannot be moved.

**Remedy:** Insert an explicit dual-review gate (Consistency + Safety at minimum; Surface if public switches/docs change) after S7.3 and before any Phase 8 `--push` / product tag. Phase 8 depends on that GO. State that writing/exiting earlier phases does not authorize publish.

**Closure test:** Phase 8 runbook refuses to start without filed dual-review GO IDs and settled tuple; a dry checklist fails if those docs are absent.

---

### [P1-3] Plan can advertise supported SSH/HTTPS routes without Transport Phase 6 / §11 / real-GH evidence (scope shrink without amendment)

**Location:** V110 Phase 5 (measurements only), S6.3/S7.2/Phase 8 fixture smokes (240–243, 257–261, 296–301); controlling `GwzRemoteTransportPlan.md` Phase 6 exit (366–373: complete acceptance matrix; scope reduction needs accepted amendment); Design §11–§12 (797–840: matrix; TLS/proxy/auth parity before advertising; never advertise unimplemented); readiness (66–69: fixtures ≠ real GH environment).

**Violated invariant:** Routes advertised as supported in 1.1.0 must have the corresponding design/qualification evidence, or an **accepted amendment** must narrow the matrix. Synthetic fixtures must not be treated as real-account/provider/TLS/proxy proof.

**Reproduction:**
1. V110 explicitly amends only Linux ARM64 / macOS x86-64 out of scope (table lines 42–48, 57–59) — deferred outcome OK.
2. No V110 step runs or signs off the **complete** Design §11 matrix on the three supported platforms; no amendment drops context isolation, ingress bounds, URL-routing/credential refusals, proxy/TLS parity, real `gh` auth, etc.
3. S7.2 builds a route ledger for “SSH and HTTPS in both placements”; Phase 8 proves ledger commands against S4.5/S6.3 **fixtures**.
4. Release notes claim gh-only authenticated HTTPS as supported policy on ordinary installs.

**Impact:** False “supported” composition: users and agents treat gh-authenticated HTTPS / full platform SSH as qualified when readiness and Design still forbid that claim on fixture-only evidence. Failure modes (helper, TLS, proxy, redirect, real account) hit production with credentials and network effects.

**Remedy:** Either (a) add a Phase 6-complete / §11 sign-off step with attributable evidence per advertised cell before S7.2, including real-account `gh` HTTPS (and TLS/proxy as required before advertising), or (b) file and accept a bounded amendment that lists every §11 cell not required for 1.1.0 and forbids advertising those cells. S7.2 must fail if the ledger advertises a cell the amendment/evidence set does not cover.

**Closure test:** Ledger row ↔ evidence ID matrix; CI/checklist fails on any advertised row without evidence or explicit unsupported marking.

---

### [P2-1] Dependency sketch omits S3.3 from the path to activation/release

**Location:** `GwzV110Plan.md` §4 sketch (303–318): `S3.1 ── S3.3` is a dangling edge; `S3.2 ── S5.* ── S7.1` is the only timeout-adjacent path into activation. Prose “S7 waits on S3” (318) conflicts with the executable DAG.

**Violated invariant:** Control-flow authority for irreversible activation must not allow an operator or agent to skip S3.3 (timeout live qualification) while still reaching S7/Phase 8.

**Reproduction:** Execute only the sketched edges: finish S3.2 → S5 → S7 → Phase 8; never run S3.3. Sketch shows no edge from S3.3 into S7/Phase 8.

**Impact:** Same class of harm as P1-1 if the sketch is treated as the order: production transport without closed S5.2-equivalent evidence.

**Remedy:** Redraw the sketch so `S3.1 → S3.3 → S7.1` (and Phase 8) is mandatory; keep prose aligned. State that the sketch is normative for waits.

**Closure test:** Dependency table lists S3.3 as a hard prerequisite of S7.1/Phase 8; a release checklist checkbox for S3.3 evidence ID.

---

### [P2-2] Phase 8 `--dependency-tag` instructions are not valid gearu syntax

**Location:** V110 Phase 8 steps 2 and 4 (279–283); `gearu release --help` / `ReleaseProcess` / CLI: `--dependency-tag NAME=TAG`, repeatable.

**Violated invariant:** Interface shape: an option used at an irreversible boundary must be specified with the form the tool accepts, including the dependency **name**, so the override cannot silently no-op or pin the wrong configured tag.

**Reproduction:** Operator follows the plan literally: “`--dependency-tag` set to step 1’s tag” without `NAME=`. Command errors or, if the flag is omitted “to match the docs,” release uses the preconfigured dependency tag (possibly stale/wrong). Tag pushes anyway once corrected ad hoc under time pressure.

**Impact:** Consumer crate tagged against the wrong dependency release; immutable; mixed train relative to the intended step-1/2 artifacts.

**Remedy:** Write exact invocations, e.g. `gearu release <ver> --dependency-tag <sys-crate-name>=vX.Y.Z --push --github-release`, with names from S2.1 locked before Phase 8.

**Closure test:** Copy-paste commands from the plan into `gearu plan` on a fixture repo; plan verifies the intended dependency tag.

---

### [P2-3] Linux x86-64 is supported but Phase 8 never requires registry→install proof there

**Location:** V110 scope table (macOS ARM64, Linux x86-64, Windows x86-64 in release); Phase 8 post-jobs (296–301): `cargo install` / Python install only on macOS ARM64 and dabeest; “Linux x86-64 is the CI build of the same tags.” Readiness item 4 (clean independent consumer builds on supported targets).

**Violated invariant:** A platform listed as supported for 1.1.0 must prove installability from published artifacts, not only CI of the tag (which may not equal registry resolution + locked install).

**Reproduction:** Phase 8 completes Mac + Windows registry smokes; Linux CI is green from checkout. crates.io/PyPI metadata or feature flags break only on Linux install; users on the supported Linux x86-64 line get a failed or native-fallback install while the release claims three-platform support.

**Impact:** Degraded/mixed reality vs advertised support; Linux users worse than “unsupported” (they are told it works).

**Remedy:** Require the same registry install + ledger smoke on a Linux x86-64 host (or documented equivalent isolated consumer environment), not only CI compile.

**Closure test:** Phase 8 evidence includes Linux `cargo install gwz --version 1.1.0 --locked` and Python install from registries with version asserts.

---

### [P2-4] Mid-train failure has no fail-closed stop / recovery rule after irreversible tags

**Location:** V110 Phase 8 (274–294); gearu `ReleaseProcess.md` “Ordered Repository Releases” (non-transactional; never move tags; new patch if contents must change).

**Violated invariant:** After any `--push` / GitHub Release / registry publish, the plan must define stop-closed behavior so operators do not continue tagging a partial or inconsistent 1.1.0 product set, and must not claim the three-product v1.1.0 outcome.

**Reproduction:**
1. Steps 1–4 succeed; step 5 tags/publishes `gwz-core` 1.1.0 with activated transport pins.
2. Step 6 or 7 fails (CI publish, pin error, Windows wheel).
3. Plan is silent; operator either (a) leaves crates.io with core 1.1.0 and CLI/py on 1.0.17, or (b) retries with ad hoc pins, or (c) attempts to “fix forward” without a new patch version.
4. Users compose mixed 1.0.17 CLI with 1.1.0 core (or vice versa via direct crate deps). Tags already pushed cannot be rewritten.

**Impact:** Stuck / mixed-version ecosystem; irreversible partial product release; support surface claims “v1.1.0” while the train is incomplete.

**Remedy:** Add an explicit mid-train rule: on any failed step after a push/publish, **stop**; do not run later product tags; record published artifact IDs; resume only with new patch/RC versions per gearu recovery; forbid claiming v1.1.0 complete until steps 5–7 and post-job smokes all succeed. Optional: delay product tags until all three product release commits are prepared locally, still publishing deps first.

**Closure test:** Runbook table “failure after step N → actions”; checklist blocks step N+1 without N success attestation.

---

### [P2-5] Windows SSH can be ledger-advertised while exact-agent remains unproven

**Location:** V110 S4.3 (170–174: exact-agent unclaimed until fixture), S4.5 (181–186), S7.2 (257–261: ledger covers SSH on three platforms); Design §11 Platforms (819: exact-agent remains false until separately proven).

**Violated invariant:** Do not advertise an unimplemented or unproven capability as supported; exact-agent stays false until proven.

**Reproduction:** S4.5 passes clone/fetch with the default mingw agent environment; S4.3 never proves selected-identity exact-agent; S7.2 ledger lists Windows SSH as a working advertised route without retaining `exact-agent=false` / selected-identity unsupported in capabilities and migration notes.

**Impact:** Callers select identities expecting refusal/parity with Unix proven behavior; wrong key or agent fallback risk relative to Design identity/trust rules (no fallback).

**Remedy:** S7.2 must require capabilities/migration/ledger to keep exact-agent false on Windows until a named fixture GO; advertising selected-identity SSH on Windows fails the step until then.

**Closure test:** Capability snapshot on a Windows build asserts exact-agent unsupported; ledger has no selected-identity Windows SSH row unless fixture evidence ID present.

---

### [P3-1] Disclosure controls stop at Python errors; evidence/CLI/log paths are unbound

**Location:** V110 S6.3 (243: credentials not in Python-visible errors); S4.1/S5.* evidence notes; Phase 8 smokes; Design (247–248, 816–817: no secret in messages/errors; private-member suppression).

**Violated invariant:** Credential material, `gh` helper output, agent socket paths, and known_hosts contents must not scale into retained evidence, CLI diagnostics, or logs beyond what Design already forbids for messages/errors.

**Reproduction:** S4/S5/Phase 8 retain command transcripts that include `SSH_AUTH_SOCK`, known_hosts file bodies, or `gh` auth headers/tokens in verbose HTTP logs; plan never forbids it outside Python errors.

**Impact:** Local evidence repos and operator logs become disclosure surfaces larger than the endpoint-owned credential boundary.

**Remedy:** Add a single Phase 4/5/8 evidence rule: redact agent sockets, known_hosts bodies, and `gh` tokens/headers from retained logs; fail the step if secrets appear in filed evidence.

**Closure test:** Grep gate over retained evidence for token/socket patterns; intentional fixture with a fake token must be redacted before GO.

---

## 2. Invariant analysis

| Invariant | Plan posture | Attack result |
|---|---|---|
| Timeout S5.2 full closure before tag | S3.1 + weakened S3.3 | **Broken** (P1-1); sketch may skip (P2-1) |
| Q6 retirement accepted before rollout | S3.2 before S5/S7; Batch A baseline until GO | Holds in prose |
| Activation dual-reviewed; publish not “because plan exists” | Phase 7/8 operator-run, no review step | **Broken** (P1-2) |
| Never advertise without evidence / amend scope | Narrow platforms only; ledger + fixtures | **Broken** for remaining §11 / real GH (P1-3) |
| Immutable tags ↔ exact intended trees | Prose SHA; no content gate | **Broken** (P0-1) |
| Multi-repo release non-transactional → fail-closed | Wait for crates.io visibility only | Partial; no abort (P2-4) |
| Credential locality / no secrets in errors | Python S6.3; design cited | Partial (P3-1) |
| Windows ≠ skipped Unix tests as pass | S4.5 explicit | Holds |
| Exact-agent false until proven | S4.3 unclaimed; S7.2 silent | **Weak** (P2-5) |
| No second Python pool / divergent clocks | Phases 1 & 6 forbid | Holds in text |
| No iroh / carrier / rust-lang name republish / clock retune | §2 / §5 | Holds |
| `gearu` dependency override shape | Informal `--dependency-tag` | **Broken** (P2-2) |
| Supported Linux install from registries | CI only | **Broken** (P2-3) |

“Never worse than status quo” fails under P1-1 (bad/missing stall expiry after activation), P1-3 (advertised gh HTTPS without real-GH proof), and P2-4 (partial train). Status-quo 1.0.17 native paths are preferable to a falsely “supported” activated transport.

---

## 3. Risks and next action

Highest leverage fixes: **P0-1** (vendor content gate before first `--push`), **P1-1** (import full S5.2 closure into S3.3), **P1-2** (dual-review between S7.3 and Phase 8), **P1-3** (complete §11 / real-GH evidence or an accepted scope amendment tied to the ledger). Then repair the **S3.3 → S7** edge, gearu `NAME=TAG` commands, Linux registry smoke, and mid-train stop/recovery.

Do not treat this draft as authorization to activate, tag, or publish. Re-verify the exact plan hash and repo tuple before any re-review.

**End-of-review tuple:** unchanged from section Baseline; plan SHA-256 still `a52cd7a8c25c5887c99db3c839fbf2f069855b5a874e6111e039c3f3f8320f31`.

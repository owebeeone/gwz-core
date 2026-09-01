#!/usr/bin/env python3
"""One-command driver for R4b-G's seven named gates (W5's mechanised half).

R4b-G F-4 / inventory W5. `GwzM5-8Refactor.md:2243-2244` names the seven:
"aggregate fault, compatibility, byte-equivalence, unknown-field, privacy,
call-graph, and settled-tree review gates". W5 asks for a driver naming "every
§2 gate, its command, and its expected count on the settled tree", so each
command carries the marker its green result must print: a command that exits 0
while printing the wrong count still fails here -- which is why the M4 map's
marker is its whole ok line, counts included, and not the bare `ok` that let a
silently-dropped scenario through (correctness finding C-4). This DRIVES the
batteries and re-implements none of them; every command is one the runbook
already names.
The seventh gate is the two independent full-tree R4b reviews -- no script
discharges a review, so it reports REVIEW and is never counted green: a zero
exit means "the mechanical gates in this selection pass", never "R4b-G passes".

Usage: no arguments runs all seven; `--list` names them; positional selectors
take `battery` or `battery:index` (`fault:2` = that battery's 2nd command).

The `fault` battery carries all four of the disjoint, exhaustive lib partitions
evidence row 2.1 rests on -- 254 + 1 + 400 + 917 of the 1573 listed tests, the
split `GwzM5-8R4bG-Evidence.md` §3.1 records -- not just the two v1_lifecycle
commands. It ran only those two until the R4b-G correctness/evidence duals
(findings C-5 and P3-5) measured the gap: the battery is named for the
aggregate fault gate, and the `checked_artifact::` 165-key fault census row 2.1
counts toward that gate was 400 of those tests at the split §3.1 records. The
four numbers above are that recorded split and are not re-derived here; the live
per-OS counts each command must print are the ones `_fault_count` pins. Each
partition stays inside the 600 s per-command budget that forced §3.1's split in
the first place.

A full pass is ~27 min, dominated by the call-graph compiler probes and the
release-profile root fault matrix; `battery:index` exists so a host with a
per-command time budget can partition it the way §3.1 partitions the lib suite.
A partitioned run prints PARTIAL and withholds the aggregate pass line, so
reconciling across invocations stays the reader's job, explicitly. Commands run
in gwz-core whatever the caller's cwd.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
PY = sys.executable
REGISTRY = "dev-docs/GwzM5-8I2CompatibilityPredicates.json"


def check(script: str, *args: str) -> list[str]:
    return [PY, f"scripts/checks/{script}", *args]


def suite(script: str) -> list[str]:
    return [PY, "-m", "unittest", f"scripts/checks/{script}"]


def lib(*args: str) -> list[str]:
    return ["cargo", "test", "--lib", "-p", "gwz-core", *args]


def _fault_count(darwin: str, linux: str) -> str:
    """Per-OS expected counts for the two cfg-divergent fault partitions.

    checked_artifact:: and the lib remainder carry OS-gated tests
    (#[cfg(all(test, target_os = "linux"))] and #[cfg(unix)]/darwin-only
    fns), so the partition totals are host-specific. Both lib totals in THIS
    paragraph -- the v0.11.0 baseline the R2-E blocks below move from -- are
    EXECUTED, never derived from intent, and the two remainder values are
    those same executed runs' totals less the other partitions (arithmetic
    over measured numbers, marked FIRST-DISPATCH-EXPECTED below -- E6 review
    F-4, 2026-08-28); that claim is this paragraph's own
    and does not extend past it, because each dated block states the
    provenance of every value it moves and several of them carry a DERIVED
    linux count marked FIRST-DISPATCH-EXPECTED (E1 review F3 [P3], executed
    2026-08-28: the unqualified sentence that stood here read as if it
    governed the whole function). The baseline: darwin = the release script's
    green `cargo test --locked` at tag v0.11.0 (lib 1589 passed + 1
    ignored; remainder = 1589 - (256+1+400) = 932 -- the pre-R1/R2 926
    was stale, R1/R2 added remainder-partition tests); linux = release
    verify run 32954473899 at the same tag (410 measured; 933 = its lib
    total 1600 minus 256+1+410). Both remainder values are marked
    FIRST-DISPATCH-EXPECTED until their next battery execution confirms
    them. Any other host fails loudly here rather than inheriting a count
    measured elsewhere -- the release-train lesson (activation record
    S17; the Windows count-pin derivation) applied to this driver.

    R2-E Phase E2 moves the checked_artifact:: partition and only it: the
    step adds thirteen lib tests, all under that partition -- eight
    `namespace::tests_barrier_matrix` rows (the sixteen-key
    interruption/restart/convergence matrix, the single-crossing probe, the
    twelve-round repeated-boundary rows and the settled census, each on both
    target variants) and five `platform::anchor::tests` rows for the third
    `DirentBarrierClass`'s roaming arm. The remainder partition is untouched
    and keeps its existing values.

      darwin 400 -> 421: MEASURED on this step's own tree
        (`cargo test --lib -p gwz-core checked_artifact::`, 421 passed,
        2026-08-27), with the remainder re-measured unchanged at 932 in the
        same run. The E2 review's remediation added eight more rows on top of
        the step's first thirteen: four in `namespace::tests_barrier_matrix`
        driving the mid-round-trip residue and the legacy both-names tree on
        both target variants, and four in `interface_tests::schedule_records`
        splitting O6's three read-side refusals into named rows beside a
        positive control.
      linux  410 -> 431: DERIVED (410 + 21), *not* measured, and therefore
        OWED at the lane owner's three-platform landing dispatch. Marked
        LINUX-COUNT-OWED below for exactly that reason: every other value in
        this function was executed before it was written, and this one was
        not.

    Both values are against THIS branch's base (`94da3e5`). E1 has since
    landed on main and E3 is landing; both add lib tests under
    checked_artifact::, so both numbers move again at the rebase and must be
    re-measured there rather than added up on paper.

    E2 LANDING RECONCILE (2026-08-27, lane owner, the squashed final
    R2-E family landing): that re-measurement. E1 (+8), E3 (+17) and E2
    (+21, remediation included) are all on this tree. darwin 446 is
    EXECUTED on the landing host at the squashed tree ("446 passed");
    linux 456 is DERIVED (+46 over the 410 base, cfg-independent) and
    FIRST-DISPATCH-EXPECTED -- the Platform matrix dispatch at this
    landing measures it, and a measured number wins.

    R2-E Phase E5.1 (2026-08-28) moves the LIB REMAINDER partition and only
    it: the step adds exactly one lib test,
    `workspace_ops::tests::g23::compatibility_unbound_v0::
    v0_unbound_progress_shapes_are_refused_by_adapt_open` -- the one
    parametric `adapt_open` refusal test over the ten unbound progress
    shapes, which the L6 ruling requires to land with its registry rows. It
    is under neither `checked_artifact::` nor
    `workspace_ops::merge::v1_lifecycle::`, so the remainder is the only
    partition that moves and `checked_artifact::` stays at the E2 landing's
    446/456.

      darwin 932 -> 933: MEASURED on this step's own tree
        (`cargo test --lib -p gwz-core -- --skip checked_artifact:: --skip
        workspace_ops::merge::v1_lifecycle::`, 933 passed + 1 ignored,
        2026-08-28), with `checked_artifact::` re-measured unchanged at 446
        in the same run.
      linux  933 -> 934: DERIVED (+1, cfg-independent -- the new test
        carries no `cfg` gate and builds on every platform), *not* measured,
        and therefore FIRST-DISPATCH-EXPECTED at the lane owner's
        three-platform landing dispatch, in the same form as the E2 counts
        above. A measured number wins.

    R2-E Phase E5.2 (2026-08-28) moves the SAME partition and only it, by two
    more: `archive_equivalence_v0::
    archived_v0_shapes_are_byte_preserved_from_their_open_records` (tier 1 of
    the O8 archive-equivalence mechanism, the eight fixtured Table B shapes)
    and `archive_equivalence_v0::
    archive_corpus_denominators_match_the_o8_archive_dispositions` (the 8 + 2
    denominators asserted from the runtime side).

      darwin 933 -> 935: MEASURED on this step's own tree (935 passed + 1
        ignored, 2026-08-28), with `checked_artifact::` re-measured unchanged
        at 446 and `workspace_ops::merge::v1_lifecycle::` at 256 across the
        E5 pair.
      linux  934 -> 936: DERIVED (+2, cfg-independent for the same reason),
        FIRST-DISPATCH-EXPECTED at the landing dispatch.

    R2-E Phase E6.1 (2026-08-28) moves the SAME partition and only it, by two:
    O9's composed-path upgrade-failure fallback
    (`a1_activation::an_eligible_row_completes_under_v0_when_its_atomic_upgrade_fails`)
    and the guard that pins that test's fault to the filesystem rather than to
    a compatibility refusal
    (`a1_activation::the_overlong_staging_name_refuses_the_atomic_upgrade_at_the_filesystem`).
    Neither is under `checked_artifact::` or
    `workspace_ops::merge::v1_lifecycle::`, so those two stay at the E2
    landing's 446/456 and at 256.

      darwin 935 -> 937: MEASURED on this step's own tree (937 passed + 1
        ignored, 2026-08-28), with `checked_artifact::` re-measured unchanged
        at 446 and `workspace_ops::merge::v1_lifecycle::` at 256 in the same
        run.
      linux  936 -> 938: DERIVED (+2), *not* measured, and therefore
        FIRST-DISPATCH-EXPECTED at the lane owner's three-platform landing
        dispatch, in the same form as the E2 and E5 counts above. A measured
        number wins. The delta is the same on both because both tests are
        `cfg(unix)`-gated and linux satisfies that gate exactly as darwin
        does. This is the first R2-E step whose delta is NOT cfg-independent:
        the fault it installs needs a 236-byte path component, which Windows
        MAX_PATH refuses, so both tests are compiled out there and a Windows
        pin -- when this driver grows one -- moves by zero, not by two.

    R2-E Phase E6.2b (2026-08-28) moves the checked_artifact:: partition and
    only it, by one: `platform::anchor::tests::
    a_non_canonical_retired_ordinal_is_refused_not_adopted`, the executed
    anchor nit -- `survey` now admits a retired ordinal only if `retired_name`
    would have produced that exact name. The lib remainder is untouched and
    keeps E6.1's 937/938; `v1_lifecycle::` keeps 256.

      darwin 446 -> 447: MEASURED on this step's own tree (447 passed,
        2026-08-28), with the remainder re-measured unchanged at 937 + 1
        ignored and `workspace_ops::merge::v1_lifecycle::` at 256 in the same
        run.
      linux  456 -> 457: DERIVED (+1, cfg-independent -- the test carries no
        `cfg` gate and the protocol it drives is portable code under test on
        every platform), *not* measured, and therefore
        FIRST-DISPATCH-EXPECTED at the lane owner's three-platform landing
        dispatch, in the same form as the E2 and E5 counts above. A measured
        number wins.

    The conf-integrity landing (2026-08-29) moves the lib remainder and only
    it, by forty-two: the standalone conf-integrity lane (reviewed at
    GwzConfIntegrity-Review.md / -Review-2.md, squashed onto the R2-E E6 tip
    per ritual 7 citing shas b23a68e/3aa48ab/0523f0e/cf4f308/2508343) adds 41
    tests across `artifact::conf_integrity`, `workspace_bootstrap::
    {conf_gate,claude_settings}` and `tests::g02`, plus the landing
    reconcile's NF-1 regression (`g02::
    branch_and_stash_dry_runs_refuse_a_hand_edit_too`).

      darwin 937 -> 979: MEASURED on the landing tree (979 passed + 1
        ignored, 2026-08-29), with `checked_artifact::` re-measured unchanged
        at 447 and `workspace_ops::merge::v1_lifecycle::` at 256 in the same
        run.
      linux  938 -> 980: DERIVED (+42, cfg-independent -- none of the lane's
        tests carries a `cfg` gate), *not* measured, and therefore
        FIRST-DISPATCH-EXPECTED at the landing dispatch. A measured number
        wins.

    R2-E E7.2 (2026-08-29, the settled-gate acceptance) moves NO count and
    converts linux provenance only: the release verify's ubuntu-24.04 leg at
    tag v0.11.1 (= this tree, be693bd) executed the fault battery
    per-partition against the pinned counts -- checked_artifact 457, lib
    remainder 980, v1_lifecycle 256, root_fault_matrix (release profile) 1
    -- so every linux value above marked DERIVED / FIRST-DISPATCH-EXPECTED
    for the CURRENT pin set (457 and 980; 256 and the release-leg 1 were
    already measured per-partition) is now MEASURED, by direct execution at
    the exact tree under settlement. Citation: Release workflow run
    33196576270, job "Verify (ubuntu-24.04)" 98935133025, conclusion
    success, head be693bd (the checkpoint's release record notates the pair
    "33196574973 -> 33196576270"); run and job re-verified via gh at the
    acceptance, 2026-08-29. The darwin values were measured at the release's
    own local gate on the same tree (447 / 979 + 1 ignored / 256 / 1). The
    FIRST-DISPATCH-EXPECTED convention above stays in force for future
    moves; a measured number wins, and these are now the measured numbers.
    Acceptance record: dev-docs/GwzM5-8R2E-E7-Acceptance.md (gwz-dev root).

    gwz log settlement (2026-09-01) moves the lib remainder and only it by
    116 portable tests. Darwin 1095 passed + 1 ignored is MEASURED by the
    exact v0.12.0 remainder command after release workflow 33412428402
    exposed the stale marker. Linux 1096 is the same cfg-independent +116
    over the workflow-measured v0.11.1 Linux value 980; it is
    FIRST-DISPATCH-EXPECTED until the corrected release workflow executes
    it, at which point the measured result wins.

    The ahead-only pull fix (2026-09-01) moves the lib remainder and only
    it, by two: g25's ahead-only regressions
    (an_ahead_only_root_is_up_to_date_for_ff_and_merge_pulls,
    an_ahead_only_member_is_up_to_date_for_ff_and_merge_pulls) pin that a
    root or member strictly AHEAD of its remote is up to date for pull
    purposes under every non-destructive sync mode, not a
    DivergedMember/MergeRecoveryRequired misclassification.

      darwin 1095 -> 1097: MEASURED on this tree (1097 passed + 1
        ignored, 2026-09-01), with checked_artifact:: re-measured
        unchanged at 447 and v1_lifecycle:: at 256 in the same session.
      linux  1096 -> 1098: DERIVED (+2, cfg-independent -- neither test
        carries a cfg gate), FIRST-DISPATCH-EXPECTED at the next linux
        execution. A measured number wins.

    R2-F R1.2 (2026-09-01) moves the checked_artifact:: partition and only
    it, by one: the A1 activation tripwire (`interface_tests::
    catalog_activation_pin`), which pins `recover_or_create`'s production
    caller count at zero until E4.1 moves it to one.

      darwin 447 -> 448: MEASURED on this step's own tree (448 passed,
        2026-09-01), with the lib remainder re-measured unchanged at 1097
        + 1 ignored and v1_lifecycle:: at 256 in the same session.
      linux  457 -> 458: DERIVED (+1, cfg-independent -- the test carries
        no cfg gate and walks a source tree checked out on every
        platform), FIRST-DISPATCH-EXPECTED at the landing dispatch. A
        measured number wins.

    R2-F R1.1, the relocation split (2026-09-01,
    `GwzM5-8R2F-RelocationPlan.md` §3), landing sequentially after R1.2,
    moves the checked_artifact:: partition and ONLY it, by four rows,
    none carrying a cfg gate: the decisive drive-after-bootstrap row and
    the two bootstrap-over-a-resident-legacy-directory rows (workspace
    and git-directory roots) in `catalog::bootstrap::tests`, plus the
    per-persisted-field digest-movement row in
    `interface_tests::contracts`. The lib remainder is untouched by
    construction: R1.1 edits three existing `src/git/tests/**` bodies
    (g12, g15, stash) to cover the catalog's new path alongside the
    legacy one and ADDS no test there, and the markers count `N passed`.

      darwin 448 -> 452: MEASURED on the reconciled landing tree
        (`cargo test --lib -p gwz-core checked_artifact::`, 452 passed,
        2026-09-01), with the lib remainder re-measured unchanged at 1097
        passed + 1 ignored and v1_lifecycle:: at 256 in the same session.
      linux  458 -> 462: DERIVED (+4 over R1.2's derived 458, itself +1
        over the workflow-measured 457; all five deltas cfg-independent),
        *not* measured, and therefore FIRST-DISPATCH-EXPECTED at this
        package's named Windows/three-platform landing dispatch. A
        measured number wins.
      lib remainder 1097 / 1098: UNMOVED. darwin 1097 re-MEASURED on this
        tree; linux 1098 keeps its existing FIRST-DISPATCH-EXPECTED status
        from the ahead-only block above, unchanged by R1.1.

    R2-E E4.1 commit (a) -- the catalog-free hygiene riders (anchor nit 1's Q1
    bounded read, [R2-P3-3]'s wording fix, Code [P3 F3]'s stat-level family
    gate, the preservation-image path-constant second-authority pin) -- moves
    the checked_artifact:: partition and ONLY it, by two rows, neither
    carrying a cfg gate: the preservation-image path pin in
    `interface_tests::contracts` and anchor nit 1's owed bounded-read shape
    companion in `interface_tests::capability_permit`. Both read source text
    that is checked out on every platform.

      darwin 452 -> 454: MEASURED on this step's own tree (454 passed,
        2026-09-01).
      linux  462 -> 464: DERIVED (+2, cfg-independent), *not* measured, and
        therefore FIRST-DISPATCH-EXPECTED at this package's landing
        dispatch. A measured number wins.
      lib remainder 1097 / 1098 and v1_lifecycle:: 256: UNMOVED by this
        commit; darwin re-MEASURED unchanged in the same session.

    R2-E E4.1 commit (b) -- the activation package (O2: `recover_or_create`'s
    first production caller) -- moves TWO partitions, by three rows, none
    carrying a cfg gate:

      checked_artifact:: darwin 454 -> 456, MEASURED on this step's own tree
        (456 passed, 2026-09-01): precondition 6's restart arm, which drives
        the production activation door through an interrupted durable edge
        (`catalog::bootstrap::tests`), and precondition 1's remedy pin
        (`interface_tests::contracts`).
      checked_artifact:: linux 464 -> 466: DERIVED (+2, cfg-independent -- the
        fault key and the capability vocabulary are portable), *not* measured,
        and therefore FIRST-DISPATCH-EXPECTED at this package's landing
        dispatch. A measured number wins.
      v1_lifecycle:: 256 -> 257, MEASURED on this step's own tree (257 passed,
        2026-09-01): precondition 6's ordering arm, which proves the checked v1
        prologue refuses an unactivatable catalog before the operation's first
        durable mutation. This row is a single cross-platform string; the
        obstruction it plants is a plain file, so it is cfg-independent and the
        landing dispatch re-measures it on the other two platforms.
      lib remainder 1097 / 1098: UNMOVED; darwin 1097 + 1 ignored re-MEASURED
        on this tree in the same session.

    R2-E E4.1 commit (c) -- the review's [P1-1]/[P2-1] cure -- moves the LIB
    REMAINDER and only it, by two rows in `workspace_ops::tests::g23::
    a1_activation`, neither carrying a cfg gate: the cured wedge (an
    interrupted ordinary merge completes under v0 when the catalog is
    unavailable) and the v1 resume/abort recoverability row.

      lib remainder darwin 1097 -> 1099: MEASURED on this step's own tree
        (1099 passed + 1 ignored, 2026-09-01).
      lib remainder linux  1098 -> 1100: DERIVED (+2, cfg-independent -- both
        rows plant a plain file and drive production dispatch), *not*
        measured, and therefore FIRST-DISPATCH-EXPECTED. A measured number
        wins.
      checked_artifact:: 456 / 466 and v1_lifecycle:: 257: UNMOVED -- the cure
        rewrites existing rows rather than adding any. Both darwin numbers
        re-MEASURED unchanged in the same session.
    """
    if sys.platform == "darwin":
        return darwin
    if sys.platform.startswith("linux"):
        return linux
    raise SystemExit(
        f"run_r4bg_aggregate_gates: no measured fault-battery count pin for host {sys.platform!r}; "
        "measure on this host and add it explicitly"
    )


BATTERIES: dict[str, tuple[str, list[tuple[str, list[str], str]]]] = {
    "fault": ("aggregate fault/restart matrices (TransitionDesign:1469-1475)", [
        ("v1 lifecycle fault and restart matrices",
         lib("workspace_ops::merge::v1_lifecycle::", "--", "--skip", "root_fault_matrix"),
         "257 passed"),
        ("root physical/successor boundary matrix (release profile)",
         ["cargo", "test", "--release", "--lib", "-p", "gwz-core", "root_fault_matrix"], "1 passed"),
        # LINUX-COUNT-OWED (R2-E E2): the darwin value is measured on this
        # step's tree; the linux one is derived and must be re-measured at the
        # landing dispatch before it is trusted.
        #
        # WINDOWS-ARM-OWED (R2-E E2): this partition also carries the §3.6
        # obligation that `barrier.target_barrier`'s Windows arm executes
        # NATIVELY rather than skipping. It is discharged by construction --
        # `DirentBarrierClass::RoamingAnchoredTarget`'s Windows arm is
        # `anchor::round_trip_supplied`, reached through `private_barrier` with
        # no skip branch, and the five `platform::anchor::tests` roaming rows
        # ask the platform rather than reading a `cfg` -- but on darwin
        # `private_barrier` never reaches that arm, so nothing here proves it.
        # The Windows leg of the landing dispatch is the proof, and it is the
        # first Windows compile of that code. Marked here beside the linux
        # count so the dispatch cannot forget one and remember the other.
        ("checked-artifact fault census (165 keys)",
         lib("checked_artifact::"), _fault_count("456 passed", "466 passed")),
        ("lib remainder, completing the four disjoint partitions",
         lib("--", "--skip", "checked_artifact::",
             "--skip", "workspace_ops::merge::v1_lifecycle::"),
         _fault_count("1099 passed", "1100 passed")),
    ]),
    "compatibility": ("v0 compatibility gate (evidence row 2.2)", [
        # R2-E Phase E5.2 (2026-08-28): the marker gains the standalone
        # archive corpus, so the O8 archive-equivalence mechanism's ten rows
        # are counted by the gate rather than merely present in the file. The
        # rule and binding counts are unchanged -- the archive corpus is not
        # part of the migration registry, by §12.7's own finding that no
        # registry vocabulary can hold an archive shape.
        ("frozen predicate registry",
         check("check_merge_compatibility_predicates.py", REGISTRY, "--core", "."),
         "validated 7 migration rules, 7 runtime bindings, and 10 archive shapes"),
        ("registry checker suite", suite("test_merge_compatibility_predicates.py"), "OK"),
        ("merge-doc assertions", check("check_merge_docs.py"), "ok (12 sources, 155 assertions)"),
        ("merge-doc checker suite", suite("test_check_merge_docs.py"), "OK"),
    ]),
    "byte-equivalence": ("byte-equivalence gate, both halves of O8 (rows 2.3a/2.3b, §12)", [
        # R2-E E5 LANDED (2026-08-28). The two dev-docs companion edits to
        # `dev-docs/GwzM5-8R4bG-Evidence.md` each add one named test to the
        # m4-map region: E5.1's §12.3 Table A edit (the ten unbound progress
        # rows -- nine new registry case ids plus `G-VERIFYING`'s
        # DISPOSITIONED-UNLISTED, each citing the parametric test) and
        # E5.2's §12.4 Table B edit (the eight tier-1 rows citing the
        # archival byte-preservation test). Named tests go 41 -> 43 (+1 per
        # companion; the E5 review's [P1-1] measured the forward-stated 42
        # as short by exactly E5.2's +1) and registry rows go 13 -> 22 (the
        # nine new `valid_unlisted_corpus` rows). MEASURED against the real
        # map with both companion edits in place at the E5 landing,
        # 2026-08-28. The step's own worktree could not make those edits
        # (the map lives in the gwz-dev workspace's dev-docs, outside this
        # checkout, per this script's own J-7 scope disclosure), which is
        # why the marker was forward-stated between the E5.1 commit and
        # this landing.
        ("M4 scenario map",
         check("check_m4_scenario_map.py"),
         "M4 scenario map: ok (39 scenario rows, 43 named tests, "
         "22 registry rows all claimed)"),
        # R2-E Phase E5.1 (2026-08-28): 119 -> 120, the one parametric
        # `adapt_open` refusal test. E5.2: 120 -> 122, the archive-equivalence
        # tier-1 battery and its denominators test. E6.1: 122 -> 124, O9's
        # composed-path upgrade-failure fallback and its filesystem-fault
        # guard. All MEASURED on their own trees. Unlike the three before it
        # the E6.1 pair is `cfg(unix)`-gated, so this marker -- which carries
        # no per-OS split -- is a darwin/linux number and would read 122 on a
        # Windows host; the `fault` battery's `_fault_count` refuses such a
        # host first, which is why no split is added here.
        ("g23 adapted-v0, characterization and upgrade suites",
         lib("workspace_ops::tests::g23::"), "124 passed"),
    ]),
    "unknown-field": ("unknown-field gate (evidence row 2.4)", [
        ("record wire unknown/archive/decode", lib("workspace_ops::merge::record_wire::"), "75 passed"),
        ("exact unknown manifest per transition effect",
         lib("every_transition_effect_commits_its_exact_unknown_manifest"), "1 passed"),
    ]),
    "privacy": ("privacy gate (row 2.5b / W1, TransitionDesign:1478-1479)", [
        ("sealed v1 lifecycle compile probes", suite("test_v1_lifecycle_privacy_probe.py"), "OK"),
    ]),
    "call-graph": ("call-graph gate, both halves (rows 2.6a/2.6b, TransitionDesign:1480-1481)", [
        ("structural boundary and v1->v0 persistence guard",
         check("check_checked_artifact_boundaries.py"), "checked-artifact boundary: ok"),
        ("boundary checker suite and compiler probes",
         suite("test_check_checked_artifact_boundaries.py"), "OK"),
        ("release boundary suite", suite("test_release_boundary.py"), "OK"),
    ]),
    "settled-tree-review": ("two independent full-tree R4b reviews (AgentProcessRules.md:2006)", []),
}


def run_battery(selector: str) -> bool | None:
    name, _, index = selector.partition(":")
    title, all_commands = BATTERIES[name]
    commands = [all_commands[int(index) - 1]] if index else all_commands
    print(f"\n=== {selector} -- {title}", flush=True)
    if not all_commands:
        print("    REVIEW -- not mechanisable; this driver's record is its input", flush=True)
        return None
    passed = True
    for label, argv, expected in commands:
        started = time.monotonic()
        result = subprocess.run(argv, cwd=ROOT, capture_output=True, text=True)
        blob = result.stdout + result.stderr
        missing = expected not in blob
        elapsed = time.monotonic() - started
        if result.returncode == 0 and not missing:
            print(f"    ok    {label} ({elapsed:.1f}s, {expected!r})", flush=True)
            continue
        passed = False
        reason = f"expected {expected!r} absent" if missing else f"exit {result.returncode}"
        print(f"    FAIL  {label} ({elapsed:.1f}s, {reason})\n      $ {' '.join(argv)}", flush=True)
        for line in blob.strip().splitlines()[-20:]:
            print(f"      | {line}", flush=True)
    return passed


def main() -> int:
    parser = argparse.ArgumentParser(description="R4b-G aggregate gate driver")
    parser.add_argument("batteries", nargs="*", metavar="BATTERY[:INDEX]")
    parser.add_argument("--list", action="store_true", help="name the batteries and exit")
    args = parser.parse_args()
    if args.list:
        for name, (title, commands) in BATTERIES.items():
            print(f"{name:20} {len(commands)} command(s)  {title}")
        return 0
    selected = args.batteries or list(BATTERIES)
    for selector in selected:
        if selector.partition(":")[0] not in BATTERIES:
            parser.error(f"unknown battery {selector!r}; --list names them")
    results = {selector: run_battery(selector) for selector in selected}
    failed = sorted(name for name, ok in results.items() if ok is False)
    partial = sorted(name for name in results if ":" in name)
    print("\n=== R4b-G aggregate gate summary")
    for name, ok in results.items():
        state = "REVIEW" if ok is None else "ok" if ok else "FAILED"
        print(f"    {state:7} {name}{'  (PARTIAL)' if ':' in name else ''}")
    if failed:
        print(f"AGGREGATE: FAILED -- {', '.join(failed)}")
        return 1
    if partial:
        print(f"AGGREGATE: PARTIAL -- {', '.join(partial)} ran one command only;")
        print("reconcile the remaining commands across invocations before claiming a pass.")
        return 0
    print("AGGREGATE: this selection's mechanical gates pass; the settled-tree review is not.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

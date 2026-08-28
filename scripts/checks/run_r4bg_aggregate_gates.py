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
    fns), so the partition totals are host-specific. Both values are
    EXECUTED, never derived from intent: darwin = the release script's
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
         "256 passed"),
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
         lib("checked_artifact::"), _fault_count("446 passed", "456 passed")),
        ("lib remainder, completing the four disjoint partitions",
         lib("--", "--skip", "checked_artifact::",
             "--skip", "workspace_ops::merge::v1_lifecycle::"),
         _fault_count("935 passed", "936 passed")),
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
        # DOC-PENDING (R2-E Phase E5.1, 2026-08-28). This marker is the value
        # the map prints once the lane owner lands E5.1's companion edit to
        # `dev-docs/GwzM5-8R4bG-Evidence.md` §12.3 Table A -- the ten unbound
        # progress rows gaining their nine new registry case ids plus
        # `G-VERIFYING`'s DISPOSITIONED-UNLISTED, and each citing the new
        # parametric test. The step's own worktree cannot make that edit (the
        # map lives in the gwz-dev workspace's dev-docs, outside this
        # checkout, exactly as this script's own J-7 scope disclosure
        # records), so the marker is stated forward rather than left stale:
        # 39 scenario rows are unchanged, named tests go 41 -> 42 (the one new
        # test), registry rows go 13 -> 22 (the nine new
        # `valid_unlisted_corpus` rows). VERIFIED by running the checker
        # against a patched copy of the map on 2026-08-28; until the real
        # edit lands this battery command is RED and E5.1's per-commit
        # greenness rests on the compatibility and call-graph batteries, which
        # are green.
        ("M4 scenario map",
         check("check_m4_scenario_map.py"),
         "M4 scenario map: ok (39 scenario rows, 42 named tests, "
         "22 registry rows all claimed)"),
        # R2-E Phase E5.1 (2026-08-28): 119 -> 120, the one parametric
        # `adapt_open` refusal test. E5.2: 120 -> 122, the archive-equivalence
        # tier-1 battery and its denominators test. Both MEASURED on their own
        # trees.
        ("g23 adapted-v0, characterization and upgrade suites",
         lib("workspace_ops::tests::g23::"), "122 passed"),
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

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
counts toward that gate is 400 of those tests. Each partition stays inside the
600 s per-command budget that forced §3.1's split in the first place.

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


BATTERIES: dict[str, tuple[str, list[tuple[str, list[str], str]]]] = {
    "fault": ("aggregate fault/restart matrices (TransitionDesign:1469-1475)", [
        ("v1 lifecycle fault and restart matrices",
         lib("workspace_ops::merge::v1_lifecycle::", "--", "--skip", "root_fault_matrix"),
         "254 passed"),
        ("root physical/successor boundary matrix (release profile)",
         ["cargo", "test", "--release", "--lib", "-p", "gwz-core", "root_fault_matrix"], "1 passed"),
        ("checked-artifact fault census (165 keys)",
         lib("checked_artifact::"), "400 passed"),
        ("lib remainder, completing the four disjoint partitions",
         lib("--", "--skip", "checked_artifact::",
             "--skip", "workspace_ops::merge::v1_lifecycle::"), "917 passed"),
    ]),
    "compatibility": ("v0 compatibility gate (evidence row 2.2)", [
        ("frozen predicate registry",
         check("check_merge_compatibility_predicates.py", REGISTRY, "--core", "."),
         "validated 7 migration rules and 7 runtime bindings"),
        ("registry checker suite", suite("test_merge_compatibility_predicates.py"), "OK"),
        ("merge-doc assertions", check("check_merge_docs.py"), "ok (11 sources, 147 assertions)"),
        ("merge-doc checker suite", suite("test_check_merge_docs.py"), "OK"),
    ]),
    "byte-equivalence": ("byte-equivalence gate, both halves of O8 (rows 2.3a/2.3b, §12)", [
        ("M4 scenario map",
         check("check_m4_scenario_map.py"),
         "M4 scenario map: ok (39 scenario rows, 41 named tests, "
         "13 registry rows all claimed)"),
        ("g23 adapted-v0, characterization and upgrade suites",
         lib("workspace_ops::tests::g23::"), "114 passed"),
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

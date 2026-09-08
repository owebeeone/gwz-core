#!/usr/bin/env python3
"""Run selected checks, using their exit status. No test-count or output pins.

These selectors are available for focused diagnosis. Release workflows run the
ordinary Rust suite once instead of repeating it through these partitions.
"""
from __future__ import annotations
import argparse
import subprocess
import time
from pathlib import Path
import sys
ROOT = Path(__file__).resolve().parents[2]
PY = sys.executable

BATTERIES = {'fault': ('aggregate fault/restart matrices (TransitionDesign:1469-1475)',
           [('v1 lifecycle fault and restart matrices',
             ['cargo',
              'test',
              '--lib',
              '-p',
              'gwz-core',
              'workspace_ops::merge::v1_lifecycle::',
              '--',
              '--skip',
              'root_fault_matrix']),
            ('root physical/successor boundary matrix (release profile)',
             ['cargo', 'test', '--release', '--lib', '-p', 'gwz-core', 'root_fault_matrix']),
            ('checked-artifact fault census (165 keys)',
             ['cargo', 'test', '--lib', '-p', 'gwz-core', 'checked_artifact::']),
            ('lib remainder, completing the four disjoint partitions',
             ['cargo',
              'test',
              '--lib',
              '-p',
              'gwz-core',
              '--',
              '--skip',
              'checked_artifact::',
              '--skip',
              'workspace_ops::merge::v1_lifecycle::'])]),
 'compatibility': ('v0 compatibility gate (evidence row 2.2)',
                   [('frozen predicate registry',
                     [PY,
                      'scripts/checks/check_merge_compatibility_predicates.py',
                      'dev-docs/GwzM5-8I2CompatibilityPredicates.json']),
                    ('registry checker suite',
                     [PY,
                      '-m',
                      'unittest',
                      'scripts/checks/test_merge_compatibility_predicates.py']),
                    ('merge-doc assertions',
                     [PY,
                      'scripts/checks/check_merge_docs.py']),
                    ('merge-doc checker suite',
                     [PY,
                      '-m',
                      'unittest',
                      'scripts/checks/test_check_merge_docs.py']),
                    ('local-clone doc assertions',
                     [PY,
                      'scripts/checks/check_local_clone_docs.py']),
                    ('local-clone doc checker suite',
                     [PY,
                      '-m',
                      'unittest',
                      'scripts/checks/test_check_local_clone_docs.py'])]),
 'byte-equivalence': ('byte-equivalence gate, both halves of O8 (rows 2.3a/2.3b, §12)',
                      [('g23 adapted-v0, characterization and upgrade suites',
                        ['cargo',
                         'test',
                         '--lib',
                         '-p',
                         'gwz-core',
                         'workspace_ops::tests::g23::'])]),
 'unknown-field': ('unknown-field gate (evidence row 2.4)',
                   [('record wire unknown/archive/decode',
                     ['cargo',
                      'test',
                      '--lib',
                      '-p',
                      'gwz-core',
                      'workspace_ops::merge::record_wire::']),
                    ('exact unknown manifest per transition effect',
                     ['cargo',
                      'test',
                      '--lib',
                      '-p',
                      'gwz-core',
                      'every_transition_effect_commits_its_exact_unknown_manifest'])]),
 'settled-tree-review': ('two independent full-tree R4b reviews (AgentProcessRules.md:2006)', [])}

def run_battery(selector: str) -> bool | None:
    name, _, index = selector.partition(":")
    title, all_commands = BATTERIES[name]
    commands = [all_commands[int(index) - 1]] if index else all_commands
    print(f"\n=== {selector} -- {title}", flush=True)
    if not all_commands:
        print("    REVIEW -- manual review", flush=True)
        return None
    passed = True
    for label, argv in commands:
        started = time.monotonic()
        result = subprocess.run(argv, cwd=ROOT)
        passed = passed and result.returncode == 0
        print(f"    {label}: exit {result.returncode} ({time.monotonic() - started:.1f}s)", flush=True)
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

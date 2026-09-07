#!/usr/bin/env python3
"""Capture compiled Rust test identities and verify execution, alongside count pins.

Use a test binary produced by `cargo test --no-run --message-format=json`.
A baseline must come from the merge-base checkout on the same platform/package.
This tool does not infer Linux inventories from macOS or claim review approval.
"""
from __future__ import annotations
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import platform
import re
import subprocess
import shutil
import tempfile

LIFECYCLE = "workspace_ops::merge::v1_lifecycle::"
CHILD_HELPERS = {
    "operation::workspace_mutator_lock::tests::child_process_observes_lock_contention":
        "operation::workspace_mutator_lock::tests::separate_process_cannot_acquire_held_workspace_mutator_lock",
}


def cargo_test_binary(output: str, package: str) -> Path:
    matches = []
    for line in output.splitlines():
        row = json.loads(line)
        target = row.get("target", {})
        if (row.get("reason") == "compiler-artifact"
                and target.get("name") == package.replace("-", "_")
                and target.get("kind") == ["lib"]
                and row.get("profile", {}).get("test") is True
                and row.get("executable")):
            matches.append(Path(row["executable"]))
    if len(matches) != 1:
        raise ValueError("expected exactly one compiled package library test executable")
    return matches[0]


def parse_listing(output: str) -> set[str]:
    rows = [line.removesuffix(": test") for line in output.splitlines() if line.endswith(": test")]
    if not rows or len(rows) != len(set(rows)):
        raise ValueError("empty or duplicate compiled test inventory")
    return set(rows)


def partition(names: set[str]) -> dict[str, set[str]]:
    groups = {key: set() for key in ("lifecycle", "root-matrix", "artifact", "remainder")}
    for name in names:
        matches = {
            "lifecycle": LIFECYCLE in name and "root_fault_matrix" not in name,
            "root-matrix": "root_fault_matrix" in name,
            "artifact": "checked_artifact::" in name,
            "remainder": LIFECYCLE not in name and "checked_artifact::" not in name,
        }
        owners = [key for key, matches_filter in matches.items() if matches_filter]
        if len(owners) != 1:
            raise ValueError(f"test has overlapping or missing partition ownership: {name}")
        groups[owners[0]].add(name)
    return groups


def check_removals(before: set[str], after: set[str], reasons: dict[str, str]) -> None:
    removed = before - after
    unexplained = {name for name in removed if not str(reasons.get(name, "")).strip()}
    if unexplained:
        raise ValueError(f"removed/renamed tests need reviewed reasons: {sorted(unexplained)}")
    if set(reasons) - removed:
        raise ValueError("removal reasons contain stale or unknown test identities")


def check_baseline_identity(baseline: dict, identity: dict) -> None:
    if not identity.get("profile") or identity["profile"] == "unknown":
        raise ValueError("baseline comparison requires an explicit build profile")
    if any(baseline.get(key) != identity.get(key)
           for key in ("package", "platform", "architecture", "profile")):
        raise ValueError("baseline package/platform/architecture/profile mismatch")


def check_execution(expected: set[str], ignored: set[str], output: str) -> dict[str, list[str]]:
    rows = re.findall(r"^test (\S+)(?: - should panic)? \.\.\. (ok|FAILED|ignored)(?:[^\n]*)$", output, re.MULTILINE)
    nested = []
    for helper, parent in CHILD_HELPERS.items():
        if helper in ignored and parent in expected:
            if rows.count((helper, "ok")) != 1:
                raise ValueError(f"required isolated child helper did not execute once: {helper}")
            rows.remove((helper, "ok"))
            nested.append(helper)
    counts = Counter(name for name, _ in rows)
    if not expected or set(counts) != expected or any(count != 1 for count in counts.values()):
        raise ValueError("zero, missing, unexpected or duplicate test execution")
    passed = {name for name, result in rows if result == "ok"}
    skipped = {name for name, result in rows if result == "ignored"}
    if not passed or skipped != ignored or passed != expected - ignored:
        raise ValueError("failed test or unaccounted ignored test")
    return {"executed": sorted(passed), "ignored": sorted(skipped), "isolated_child_helpers": nested}


def check_harness_execution(expected: set[str], ignored: set[str], harness: str,
                            output: str) -> dict[str, list[str]]:
    # libtest owns this file; child stdout cannot splice its result records.
    # --logfile is supported by the pinned stable toolchain. Retain the raw
    # transcript separately, including proof of the isolated nested helper.
    rows = []
    for line in harness.splitlines():
        match = re.fullmatch(r"(ok|failed|ignored) (\S+)", line)
        if not match:
            raise ValueError(f"unrecognized harness record: {line!r}")
        result, name = match.groups()
        rows.append(f"test {name} ... {'FAILED' if result == 'failed' else result}")
    for helper, parent in CHILD_HELPERS.items():
        if helper in ignored and parent in expected:
            matches = re.findall(rf"^test {re.escape(helper)} \.\.\. ok$", output, re.MULTILINE)
            if len(matches) != 1:
                raise ValueError(f"required isolated child helper did not execute once: {helper}")
            rows.extend(matches)
    return check_execution(expected, ignored, "\n".join(rows))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--binary", type=Path)
    source.add_argument("--cargo-artifacts", type=Path)
    parser.add_argument("--package", required=True)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--removal-reasons", type=Path)
    parser.add_argument("--execution-log", type=Path)
    parser.add_argument("--run", action="store_true", help="execute a frozen copy of this binary and verify its log")
    parser.add_argument("--profile", default="unknown")
    parser.add_argument("--partition", choices=["all", "lifecycle", "root-matrix", "artifact", "remainder"], default="all")
    args = parser.parse_args()
    if args.run and args.execution_log:
        parser.error("--run and --execution-log are mutually exclusive")
    source_binary = args.binary or cargo_test_binary(args.cargo_artifacts.read_text(), args.package)
    binary = str(source_binary.resolve(strict=True))
    # A concurrent rebuild replaces Cargo's output path. List and execute one
    # immutable copy so evidence cannot accidentally mix two compiled trees.
    snapshot = tempfile.TemporaryDirectory(prefix="gwz-test-inventory-")
    frozen = Path(snapshot.name) / Path(binary).name
    shutil.copy2(binary, frozen)
    binary = str(frozen)
    listing = subprocess.check_output([binary, "--list"], text=True)
    names = parse_listing(listing)
    ignored_output = subprocess.check_output([binary, "--ignored", "--list"], text=True)
    ignored = {line.removesuffix(": test") for line in ignored_output.splitlines() if line.endswith(": test")}
    if not ignored <= names:
        raise ValueError("ignored inventory is not part of the compiled inventory")
    groups = partition(names)
    identity = {"package": args.package, "platform": platform.system(), "architecture": platform.machine(), "profile": args.profile}
    report = {**identity, "tests": sorted(names), "ignored": sorted(ignored),
              "binary_sha256": hashlib.sha256(frozen.read_bytes()).hexdigest(), "profile": args.profile,
              "partitions": {key: sorted(value) for key, value in groups.items()},
              "baseline_checked": False, "execution_checked": False}
    if args.baseline:
        baseline = json.loads(args.baseline.read_text())
        check_baseline_identity(baseline, identity)
        reasons = json.loads(args.removal_reasons.read_text()) if args.removal_reasons else {}
        check_removals(set(baseline["tests"]), names, reasons)
        report["baseline_checked"] = True
        report["removed"] = reasons
        report["added"] = sorted(names - set(baseline["tests"]))
    if args.run:
        filters = {
            "all": [],
            "lifecycle": [LIFECYCLE, "--skip", "root_fault_matrix"],
            "root-matrix": ["root_fault_matrix"],
            "artifact": ["checked_artifact::"],
            "remainder": ["--skip", "checked_artifact::", "--skip", LIFECYCLE],
        }
        args.execution_log = args.output.with_suffix(".execution.log")
        harness_log = args.output.with_suffix(".harness.log")
        command = [binary, *filters[args.partition], "--color", "never", "--logfile", str(harness_log.resolve())]
        report["command"] = [str(source_binary), *command[1:]]
        # Keep the exact inventory even when execution or reconciliation fails.
        args.output.write_text(json.dumps(report, indent=2) + "\n")
        with args.execution_log.open("w") as log:
            result = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT,
                                    cwd=Path(__file__).resolve().parents[2], timeout=3600)
        if result.returncode:
            raise ValueError(f"test binary failed; inspect {args.execution_log}")
    if args.execution_log:
        selected = names if args.partition == "all" else groups[args.partition]
        if args.run:
            report["execution"] = check_harness_execution(
                selected, ignored & selected, harness_log.read_text(), args.execution_log.read_text())
            report["harness_log"] = str(harness_log)
        else:
            report["execution"] = check_execution(selected, ignored & selected, args.execution_log.read_text())
        report["execution_checked"] = True
        report["executed_partition"] = args.partition
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    snapshot.cleanup()
    print(f"test inventory: {len(names)} identities; baseline={report['baseline_checked']}; execution={report['execution_checked']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

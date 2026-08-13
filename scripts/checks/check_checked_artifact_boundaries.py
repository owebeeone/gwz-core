#!/usr/bin/env python3
"""Fail-closed inventory for the checked-artifact production entry boundary."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]

ENTRY_CALLERS = {
    "acquire_merge_preservation_bundle": {
        "workspace_ops/merge/preserve/checked_bundle.rs",
    },
    "acquire_merge_preservation_git_directory": {
        "git/gitbackend/preservation_root/files.rs",
    },
    "acquire_merge_preservation_workspace": {
        "git/gitbackend/preservation_root/files.rs",
    },
    "acquire_merge_root_artifact": {
        "workspace_ops/merge/root/artifact_facts.rs",
    },
    "prepare_merge_store_parents": {
        "workspace_ops/merge/store/mod.rs",
    },
}

# These adapters are the complete merge leaf-mutation boundary.  They may
# observe through their checked artifact and may invoke its exact transition
# methods, but they must not grow a second successful filesystem writer.
CHECKED_LEAF_ADAPTERS = {
    "git/gitbackend/preservation_root/files.rs",
    "workspace_ops/merge/preserve/checked_bundle.rs",
    "workspace_ops/merge/root/artifact_facts.rs",
}

ENTRY_DEFINITION = re.compile(r"pub\(crate\) fn ([a-z][a-z0-9_]*)\s*\(")
ENTRY_CALL = re.compile(r"checked_artifact::entry::([a-z][a-z0-9_]*)")
RAW_ENTRY = re.compile(r"CheckedArtifact::(?:acquire|prepare_parent)\s*\(")
RAW_FILESYSTEM_MUTATION = re.compile(
    r"(?:"
    r"(?:crate::)?artifact::write_atomic(?:_verified)?\s*\("
    r"|(?:std::)?fs::(?:write|remove_file|rename|copy|create_dir|create_dir_all|remove_dir|remove_dir_all)\s*\("
    r"|(?:File|OpenOptions)::(?:create|new)\s*\("
    r")"
)


def production_rust_files(source: Path) -> list[Path]:
    return sorted(
        path
        for path in source.rglob("*.rs")
        if "tests" not in path.parts
        and "interface_tests" not in path.parts
        and not path.name.startswith("tests")
    )


def check(source: Path) -> list[str]:
    findings: list[str] = []
    entry = source / "checked_artifact/entry.rs"
    definitions = set(ENTRY_DEFINITION.findall(entry.read_text(encoding="utf-8")))
    expected = set(ENTRY_CALLERS)
    if definitions != expected:
        findings.append(
            "checked entry inventory changed: "
            f"expected={sorted(expected)} actual={sorted(definitions)}"
        )

    actual_callers: dict[str, set[str]] = {}
    raw_callers: set[str] = set()
    raw_leaf_mutators: set[str] = set()
    for path in production_rust_files(source):
        relative = path.relative_to(source).as_posix()
        text = path.read_text(encoding="utf-8")
        for symbol in ENTRY_CALL.findall(text):
            actual_callers.setdefault(symbol, set()).add(relative)
        if relative != "checked_artifact/entry.rs" and RAW_ENTRY.search(text):
            raw_callers.add(relative)
        if relative in CHECKED_LEAF_ADAPTERS and RAW_FILESYSTEM_MUTATION.search(text):
            raw_leaf_mutators.add(relative)

    for symbol in sorted(set(actual_callers) | expected):
        callers = actual_callers.get(symbol, set())
        allowed = ENTRY_CALLERS.get(symbol)
        if allowed is None:
            findings.append(
                f"unclassified checked entry call: {symbol}: {sorted(callers)}"
            )
        elif callers != allowed:
            findings.append(
                f"checked entry caller set changed: {symbol}: "
                f"expected={sorted(allowed)} actual={sorted(callers)}"
            )

    if raw_callers:
        findings.append(
            "raw CheckedArtifact entry escaped checked_artifact/entry.rs: "
            f"{sorted(raw_callers)}"
        )

    if raw_leaf_mutators:
        findings.append(
            "raw filesystem mutation escaped a checked merge leaf adapter: "
            f"{sorted(raw_leaf_mutators)}"
        )

    checked_mod = (source / "checked_artifact/mod.rs").read_text(encoding="utf-8")
    if "pub(crate) mod entry;" not in checked_mod:
        findings.append("checked entry module is not the exported architectural boundary")
    observation = (source / "checked_artifact/observation.rs").read_text(encoding="utf-8")
    for symbol in ("acquire", "prepare_parent"):
        if f"pub(super) fn {symbol}" not in observation:
            findings.append(f"raw CheckedArtifact::{symbol} is not module-private")
    return findings


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", type=Path, default=ROOT / "src")
    args = parser.parse_args()
    findings = check(args.source.resolve())
    if findings:
        print("checked-artifact boundary: failed", file=sys.stderr)
        for finding in findings:
            print(f"- {finding}", file=sys.stderr)
        return 1
    print(
        "checked-artifact boundary: ok "
        f"({len(ENTRY_CALLERS)} entries, "
        f"{sum(map(len, ENTRY_CALLERS.values()))} classified callers)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

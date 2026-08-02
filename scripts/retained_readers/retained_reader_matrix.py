#!/usr/bin/env python3
"""Execute retained readers against isolated lifecycle fixtures.

The matrix is offline by default. Explicitly unsupported historical tuples are
reported; every required tuple must have an artifact, runtime, and applicable
case or the matrix fails.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import sys
import tempfile
from pathlib import Path, PurePosixPath
from typing import Any, Mapping, Sequence

import retained_reader_harness as harness
import retained_reader_evidence as evidence
from retained_reader_fixture import (
    FixtureError,
    TreeSnapshot,
    _path_key,
    changed_paths,
    evaluate_expectation,
    evaluate_postconditions,
    fixture_identities,
    normalized_mutation_identity,
    snapshot_tree,
)
from retained_reader_schema import SchemaValidationError, load_schema, validate as validate_schema
from retained_reader_errors import MatrixError
from retained_reader_runtime import (
    _prepare_reader,
    _runtime_identity,
    bootstrap_python_runtime,
    extract_archive,
)


CASES_SCHEMA = "gwz.retained-reader-cases/v1"
CASES_SCHEMA_PATH = Path(__file__).with_name("cases.schema.json")
FROZEN_COMMANDS = {
    "rust-cli-v0.9.2": ("workspace-status", "legacy-branch-merge"),
    "gwz-py-v0.9.2": ("workspace-status", "legacy-branch-merge"),
    "rust-cli-v0.10.0": ("merge-status", "merge-continue", "merge-abort", "merge-preserve", "merge-gc"),
    "gwz-py-v0.10.0": (),
    "rust-cli-v0.10.2": ("merge-status", "merge-continue", "merge-abort", "merge-preserve", "merge-gc"),
    "gwz-py-v0.10.2": ("merge-status", "merge-continue", "merge-abort", "merge-preserve", "merge-gc"),
}


def validate_cases(cases: Mapping[str, Any], manifest_or_reader_ids: Mapping[str, Any] | set[str]) -> list[Mapping[str, Any]]:
    try:
        validate_schema(cases, load_schema(CASES_SCHEMA_PATH))
    except SchemaValidationError as error:
        raise MatrixError(str(error)) from error
    manifest = manifest_or_reader_ids if isinstance(manifest_or_reader_ids, Mapping) else None
    reader_ids = ({reader["id"] for reader in manifest["readers"]} if manifest else manifest_or_reader_ids)
    result = cases["cases"]
    seen: set[str] = set()
    for case in result:
        if not isinstance(case, dict) or not isinstance(case.get("id"), str) or not case["id"]:
            raise MatrixError("every case requires a non-empty id")
        if case["id"] in seen:
            raise MatrixError(f"duplicate case id {case['id']!r}")
        seen.add(case["id"])
        selected = case.get("readers")
        if not isinstance(selected, list) or not selected:
            raise MatrixError(f"case {case['id']} must select readers explicitly")
        unknown = set(selected) - reader_ids - {"*"}
        if unknown:
            raise MatrixError(f"case {case['id']} names unknown readers: {sorted(unknown)}")
        if not isinstance(case.get("args"), list) or not all(isinstance(v, str) for v in case["args"]):
            raise MatrixError(f"case {case['id']} args must be strings")
        fixture = PurePosixPath(str(case.get("fixture", "")))
        if not fixture.parts or fixture.is_absolute() or ".." in fixture.parts:
            raise MatrixError(f"case {case['id']} fixture path is unsafe")
        if not isinstance(case.get("expected"), dict) or "mutation" not in case["expected"]:
            raise MatrixError(f"case {case['id']} requires an explicit expectation and mutation policy")
        if not isinstance(case.get("postconditions", []), list):
            raise MatrixError(f"case {case['id']} postconditions must be a list")
        mutation = case["expected"]["mutation"]
        if mutation.get("mode") == "contract" and any(
            item["maximum"] < item["minimum"] for item in mutation["dynamic"]
        ):
            raise MatrixError(f"case {case['id']} dynamic mutation maximum is below minimum")
    if manifest is not None and set(reader_ids) == set(FROZEN_COMMANDS):
        for case in result:
            if not isinstance(case.get("fixture_sha256"), str):
                raise MatrixError(f"case {case['id']} must bind its canonical fixture_sha256")
        covered = {(reader, case["command"]) for case in result for reader in case["readers"] if reader != "*"}
        for reader, commands in FROZEN_COMMANDS.items():
            for command in commands:
                if (reader, command) not in covered:
                    raise MatrixError(f"required command {command!r} has no case for {reader}")
    return result


def _run_case(
    reader: Mapping[str, Any],
    entry_point: Path,
    case: Mapping[str, Any],
    fixture_root: Path,
    identities: Mapping[str, str],
    timeout_seconds: float,
) -> dict[str, Any]:
    command_name = case.get("command")
    if reader["commands"].get(command_name) != "available":
        return {"reader": reader["id"], "case": case["id"], "status": "failed", "errors": [f"command {command_name!r} is not available"]}
    fixture = fixture_root.joinpath(*PurePosixPath(case["fixture"]).parts)
    if not fixture.is_dir():
        return {"reader": reader["id"], "case": case["id"], "status": "failed", "errors": [f"fixture is missing: {fixture}"]}
    expected_fixture = case.get("fixture_sha256")
    actual_fixture = identities.get(fixture.name)
    if expected_fixture is not None and actual_fixture != expected_fixture:
        return {"reader": reader["id"], "case": case["id"], "status": "failed", "errors": [f"fixture identity differs: {actual_fixture}"]}
    with tempfile.TemporaryDirectory(prefix="gwz-retained-reader-") as temp:
        workspace = Path(temp) / "workspace"
        shutil.copytree(fixture, workspace, symlinks=True)
        home = workspace / ".harness-home"
        tmp = workspace / ".harness-tmp"
        home.mkdir()
        tmp.mkdir()
        before = snapshot_tree(workspace)
        variables = {
            "workspace": str(workspace),
            "executable": str(entry_point),
            "entry_point": str(entry_point),
        }
        command = harness.render_command(reader["invocation"], variables)
        command.extend(harness.render_command(case["args"], variables))
        completed = harness.run_command(
            command,
            timeout_seconds=timeout_seconds,
            cwd=workspace,
            env={
                "HOME": str(home),
                "TMPDIR": str(tmp),
                "TEMP": str(tmp),
                "TMP": str(tmp),
                "GIT_CONFIG_NOSYSTEM": "1",
                "GIT_CONFIG_GLOBAL": os.devnull,
                "LC_ALL": "C",
                "LANG": "C",
                "PYTHONUTF8": "1",
            },
        )
        after = snapshot_tree(workspace)
        expectation_errors = evaluate_expectation(case["expected"], completed, before, after)
        postcondition_errors, observations = evaluate_postconditions(
            case.get("postconditions"), workspace, before_root=fixture
        )
        errors = expectation_errors + postcondition_errors
        return {
            "reader": reader["id"],
            "case": case["id"],
            "status": "failed" if errors else "passed",
            **(
                {"known_incompatibility": case["known_incompatibility"]}
                if "known_incompatibility" in case
                else {}
            ),
            "errors": errors,
            "postconditions": {
                "status": "failed" if postcondition_errors else "passed",
                "count": len(case.get("postconditions", [])),
            },
            "exit_code": completed.returncode,
            "stdout": completed.stdout,
            "stderr": completed.stderr,
            "before_sha256": before.sha256,
            "after_sha256": after.sha256,
            "after_invariant_sha256": hashlib.sha256(
                json.dumps(
                    {
                        "mutations": normalized_mutation_identity(
                            case["expected"]["mutation"], changed_paths(before, after),
                            after, workspace,
                        ),
                        "observations": observations,
                    },
                    ensure_ascii=True,
                    sort_keys=True,
                    separators=(",", ":"),
                ).encode()
            ).hexdigest(),
            "changed_paths": changed_paths(before, after),
        }


def run_matrix(
    manifest: Mapping[str, Any],
    cases_document: Mapping[str, Any],
    *,
    platform: str,
    fixture_root: Path,
    cache_root: Path,
    offline: bool,
    python_executable: Path,
) -> dict[str, Any]:
    harness.validate_manifest(manifest)
    if platform not in {item["id"] for item in manifest["platforms"]}:
        raise MatrixError(f"unknown platform {platform!r}")
    cases = validate_cases(cases_document, manifest)
    identities = fixture_identities(fixture_root)
    results: list[dict[str, Any]] = []
    timeout = manifest["default_timeout_seconds"]
    with tempfile.TemporaryDirectory(prefix="gwz-retained-derived-") as derived:
        derived_root = Path(derived)
        for reader in manifest["readers"]:
            artifact = next((item for item in reader["artifacts"] if item["platform"] == platform), None)
            if artifact is None:
                results.append({"reader": reader["id"], "status": "failed", "errors": ["platform tuple is missing"]})
                continue
            if artifact["support"] == "unsupported":
                results.append({"reader": reader["id"], "status": "declared-unsupported", "reason": artifact["reason"], "substitute_evidence": artifact["substitute_evidence"]})
                continue
            selected = [case for case in cases if "*" in case["readers"] or reader["id"] in case["readers"]]
            if not selected:
                results.append({"reader": reader["id"], "status": "missing-cases", "errors": ["required tuple has no applicable cases"]})
                continue
            try:
                artifact_path = harness.acquire_artifact(
                    artifact, cache_root, offline=offline, timeout_seconds=timeout
                )
                entry_point = _prepare_reader(
                    manifest,
                    reader,
                    artifact,
                    artifact_path,
                    cache_root,
                    derived_root,
                    python_executable,
                    offline,
                )
            except (harness.HarnessError, MatrixError) as error:
                results.append({"reader": reader["id"], "status": "setup-failed", "errors": [str(error)]})
                continue
            for case in selected:
                try:
                    results.append(
                        _run_case(reader, entry_point, case, fixture_root, identities, timeout)
                    )
                except (FixtureError, MatrixError, harness.HarnessError) as error:
                    results.append(
                        {
                            "reader": reader["id"],
                            "case": case["id"],
                            "status": "failed",
                            "errors": [str(error)],
                        }
                    )
    failed = any(item["status"] not in {"passed", "declared-unsupported"} for item in results)
    return {"status": "failed" if failed else "passed", "platform": platform, "results": results}


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("cases", type=Path)
    parser.add_argument("--platform", required=True)
    parser.add_argument("--fixtures", type=Path, required=True)
    parser.add_argument("--cache", type=Path, required=True)
    parser.add_argument("--python", type=Path, default=Path(sys.executable))
    parser.add_argument("--allow-network", action="store_true")
    parser.add_argument("--evidence-out", type=Path)
    parser.add_argument("--attestation-out", type=Path)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    try:
        manifest = harness.load_manifest(args.manifest)
        cases = json.loads(args.cases.read_text(encoding="utf-8"))
        summary = run_matrix(
            manifest,
            cases,
            platform=args.platform,
            fixture_root=args.fixtures,
            cache_root=args.cache,
            offline=not args.allow_network,
            python_executable=args.python,
        )
        if summary["status"] != "passed":
            print(json.dumps(summary, ensure_ascii=True, sort_keys=True))
            return 1
        if args.evidence_out is not None:
            provenance = evidence.collect_provenance(args.fixtures, args.python, args.platform)
            normalized = evidence.build_evidence(
                manifest,
                cases,
                summary,
                manifest_sha256=hashlib.sha256(args.manifest.read_bytes()).hexdigest(),
                cases_sha256=hashlib.sha256(args.cases.read_bytes()).hexdigest(),
                provenance=provenance,
            )
            encoded = (json.dumps(normalized, ensure_ascii=True, indent=2, sort_keys=True) + "\n").encode()
            args.evidence_out.write_bytes(encoded)
            if args.attestation_out is not None:
                attestation = evidence.build_execution_attestation(encoded, args.platform)
                args.attestation_out.write_text(
                    json.dumps(attestation, ensure_ascii=True, indent=2, sort_keys=True) + "\n",
                    encoding="utf-8",
                )
        elif args.attestation_out is not None:
            raise MatrixError("--attestation-out requires --evidence-out")
    except (OSError, json.JSONDecodeError, harness.ManifestError, FixtureError, MatrixError, evidence.EvidenceError) as error:
        print(f"retained-reader matrix: {error}", file=sys.stderr)
        return 1
    print(json.dumps(summary, ensure_ascii=True, sort_keys=True))
    return 0 if summary["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())

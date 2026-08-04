#!/usr/bin/env python3
"""Execute the offline-first retained-reader lifecycle matrix."""

from __future__ import annotations

import argparse
import copy
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
from retained_reader_fixture import (FixtureError, TreeSnapshot, _path_key, changed_paths,
    evaluate_expectation, evaluate_postconditions, fixture_identities,
    normalized_mutation_identity, snapshot_tree)
from retained_reader_schema import SchemaValidationError, load_schema, validate as validate_schema
from retained_reader_errors import MatrixError
from retained_reader_runtime import (_prepare_reader, _runtime_identity,
    bootstrap_python_runtime, extract_archive)


CASES_SCHEMA = "gwz.retained-reader-cases/v1"
CASES_SCHEMA_PATH = Path(__file__).with_name("cases.schema.json")
FROZEN_COMMANDS = {
    "rust-cli-v0.9.2": ("workspace-status", "legacy-branch-merge"),
    "gwz-py-v0.9.2": ("workspace-status", "legacy-branch-merge"),
    "rust-cli-v0.10.2": ("merge-start", "merge-status", "merge-continue", "merge-abort", "merge-preserve", "merge-gc"),
    "gwz-py-v0.10.2": ("merge-start", "merge-status", "merge-continue", "merge-abort", "merge-preserve", "merge-gc"),
}
ENVELOPE_LIFECYCLES = {
    "open-start": ("merge-start", ["merge", "feature/source"], "open", "record_unreadable"),
    "open-status": ("merge-status", ["merge", "--status"], "open", "record_unreadable"),
    "open-continue": ("merge-continue", ["merge", "--continue"], "open", "record_unreadable"),
    "open-abort": ("merge-abort", ["merge", "--abort"], "open", "record_unreadable"),
    "open-preserve": ("merge-preserve", ["merge", "--abort", "--preserve"], "open", "record_unreadable"),
    "archived-status": ("merge-status", ["merge", "--status", "merge_retained"], "archived", "record_unreadable"),
    "archived-targeted-gc": ("merge-gc", ["merge", "--gc", "merge_retained"], "archived", "record_unreadable"),
    "archived-retention-gc": ("merge-gc", ["merge", "--gc"], "archived", "retention_noop"),
}
PROJECTION_ARGS = {"human": [], "json": ["--json"], "jsonl": ["--jsonl"]}
ENVELOPE_FIXTURES = {
    ("gwz.merge-operation/v1", 1): "v1", ("gwz.merge-operation/v2", 2): "v2",
    ("gwz.merge-operation/v3", 3): "v3", ("gwz.merge-operation/v4", 4): "v4",
    ("gwz.merge-operation/v0", 1): "v0-mismatch",
    ("gwz.merge-operation/future", 99): "unknown",
}
def _validate_envelope_cases(cases: Sequence[Mapping[str, Any]], manifest: Mapping[str, Any]) -> None:
    for reader in manifest["readers"]:
        unsupported = reader.get("record_envelopes", {}).get("unsupported", [])
        if not unsupported:
            continue
        expected: set[tuple[str, int, str, str, str]] = set()
        for envelope in unsupported:
            pair = (envelope["schema"], envelope["record_schema_version"])
            if pair not in ENVELOPE_FIXTURES or envelope["classification"] != "record_unreadable":
                raise MatrixError(f"reader {reader['id']} has an unfrozen envelope")
            expected.update(
                (*pair, lifecycle, projection, classification)
                for lifecycle, (_, _, _, classification) in ENVELOPE_LIFECYCLES.items()
                for projection in reader["projections"]
            )
        selected = [case for case in cases if reader["id"] in case["readers"] and "record_schema" in case]
        actual = [
            (case["record_schema"], case["record_schema_version"], case["lifecycle"], case["projection"], case["classification"])
            for case in selected
        ]
        if len(actual) != len(set(actual)) or set(actual) != expected:
            raise MatrixError(f"reader {reader['id']} envelope matrix is not exact")
        for case in selected:
            _validate_envelope_case(case)


def _validate_envelope_case(case: Mapping[str, Any]) -> None:
    if len(case["readers"]) != 1:
        raise MatrixError(f"case {case['id']} must pin one reader's exact retained output")
    command, args, location, classification = ENVELOPE_LIFECYCLES[case["lifecycle"]]
    key = ENVELOPE_FIXTURES.get((case["record_schema"], case["record_schema_version"]))
    fixture = f"future-{key}" if location == "open" else f"archived-future-{key}"
    if (case["command"], case["args"], case["fixture"], case["classification"]) != (
        command, PROJECTION_ARGS[case["projection"]] + args, fixture, classification
    ):
        raise MatrixError(f"case {case['id']} does not match its envelope lifecycle")
    expected = case["expected"]
    if expected.get("stdout", {}).get("mode") != "normalized-exact" or expected.get("stderr", {}).get("mode") != "normalized-exact" or expected.get("mutation") != {"mode": "none"}:
        raise MatrixError(f"case {case['id']} must pin exact normalized streams and no mutation")
    _validate_envelope_expectation(case, location, classification, case["readers"][0])


def _validate_envelope_expectation(
    case: Mapping[str, Any], location: str, classification: str, reader: str
) -> None:
    expected, projection = case["expected"], case["projection"]
    if expected["exit_codes"] != ([0] if classification == "retention_noop" else [1]):
        raise MatrixError(f"case {case['id']} exit code contradicts its classification")
    stdout, stderr = expected["stdout"]["value"], expected["stderr"]["value"]
    if not isinstance(stdout, str) or not isinstance(stderr, str):
        raise MatrixError(f"case {case['id']} exact streams must be text")
    if projection == "human":
        if classification == "retention_noop":
            valid = stdout == "action: merge\nstatus: Noop\nstate: idle\nNo coordinated merge is open.\n" and stderr == ""
        else:
            record = ".gwz/merge/merge_retained.yaml" if location == "open" else ".gwz/merge/done/merge_retained.yaml"
            prefix = "gwz: native bridge call failed for merge: " if reader.startswith("gwz-py-") else "gwz: "
            wanted = f"{prefix}MergeRecordUnreadable: merge record at '{{workspace}}/{record}' is unreadable: unsupported merge record schema\n"
            valid = stdout == "" and stderr == wanted
        if not valid:
            raise MatrixError(f"case {case['id']} human stream contradicts its classification")
        return
    if stderr:
        raise MatrixError(f"case {case['id']} machine stderr must be empty")
    try:
        lines = [json.loads(line) for line in stdout.splitlines()]
    except json.JSONDecodeError as error:
        raise MatrixError(f"case {case['id']} exact machine stream is invalid JSON: {error}") from error
    if len(lines) != (3 if projection == "jsonl" else 1):
        raise MatrixError(f"case {case['id']} machine stream has the wrong record count")
    if projection == "jsonl" and ([row.get("event_kind") for row in lines[:2]], [row.get("sequence") for row in lines[:2]]) != (["OperationStarted", "OperationFinished"], [0, 1]):
        raise MatrixError(f"case {case['id']} JSONL lifecycle events are not exact")
    response = lines[-1]
    if classification == "record_unreadable":
        errors = response.get("errors") if isinstance(response, dict) else None
        valid = isinstance(errors, list) and len(errors) == 1 and errors[0].get("code") == "MergeRecordUnreadable" and "unsupported merge record schema" in errors[0].get("message", "")
    else:
        merge, meta = response.get("merge"), response.get("meta")
        valid = isinstance(merge, dict) and merge.get("state") == "Idle" and isinstance(meta, dict) and meta.get("aggregate_status") == "Noop" and response.get("errors") == []
    if not valid:
        raise MatrixError(f"case {case['id']} machine stream contradicts its classification")
def validate_cases(cases: Mapping[str, Any], manifest: Mapping[str, Any]) -> list[Mapping[str, Any]]:
    if not isinstance(manifest, Mapping):
        raise MatrixError("validate_cases requires the frozen R0 manifest")
    harness.validate_manifest(manifest)
    result = _validate_cases_shape(
        cases, {reader["id"] for reader in manifest["readers"]}
    )
    for case in result:
        if not isinstance(case.get("fixture_sha256"), str):
            raise MatrixError(f"case {case['id']} must bind its canonical fixture_sha256")
    covered = {
        (reader, case["command"])
        for case in result
        for reader in case["readers"]
        if reader != "*"
    }
    for reader, commands in FROZEN_COMMANDS.items():
        for command in commands:
            if (reader, command) not in covered:
                raise MatrixError(f"required command {command!r} has no case for {reader}")
    _validate_envelope_cases(result, manifest)
    return result


def _validate_cases_shape(cases: Mapping[str, Any], reader_ids: set[str]) -> list[Mapping[str, Any]]:
    try:
        validate_schema(cases, load_schema(CASES_SCHEMA_PATH))
    except SchemaValidationError as error:
        raise MatrixError(str(error)) from error
    result: list[Mapping[str, Any]] = []
    for raw_case in cases["cases"]:
        if "envelopes" not in raw_case:
            result.append(raw_case)
            continue
        if len(raw_case["readers"]) != 1:
            raise MatrixError(f"case {raw_case['id']} must pin one reader")
        expectation_keys = {
            f"{location}-{classification}-{projection}"
            for _, _, location, classification in ENVELOPE_LIFECYCLES.values()
            for projection in PROJECTION_ARGS
        }
        if set(raw_case["expectations"]) != expectation_keys:
            raise MatrixError(f"case {raw_case['id']} expectation matrix is not exact")
        pairs: set[tuple[str, int]] = set()
        for envelope in raw_case["envelopes"]:
            pair = (envelope["schema"], envelope["record_schema_version"])
            key = ENVELOPE_FIXTURES.get(pair)
            if key is None or pair in pairs:
                raise MatrixError(f"case {raw_case['id']} has an unknown or duplicate envelope")
            pairs.add(pair)
            for lifecycle, (command, args, location, classification) in ENVELOPE_LIFECYCLES.items():
                fixture = envelope[f"{location}_fixture"]
                fixture_sha256 = envelope[f"{location}_fixture_sha256"]
                for projection, projection_args in PROJECTION_ARGS.items():
                    result.append({
                        "id": f"{raw_case['id']}-{key}-{lifecycle}-{projection}",
                        "readers": raw_case["readers"], "command": command,
                        "args": projection_args + args, "fixture": fixture,
                        "fixture_sha256": fixture_sha256, "projection": projection,
                        "lifecycle": lifecycle, "record_schema": pair[0],
                        "record_schema_version": pair[1], "classification": classification,
                        "expected": copy.deepcopy(raw_case["expectations"][f"{location}-{classification}-{projection}"]),
                    })
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
        expectation_errors = evaluate_expectation(
            case["expected"], completed, before, after, variables
        )
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
    return _run_matrix(
        manifest,
        cases_document,
        platform=platform,
        fixture_root=fixture_root,
        cache_root=cache_root,
        offline=offline,
        python_executable=python_executable,
        enforce_frozen_contract=True,
    )


def _run_synthetic_matrix(
    manifest: Mapping[str, Any],
    cases_document: Mapping[str, Any],
    *,
    platform: str,
    fixture_root: Path,
    cache_root: Path,
    offline: bool,
    python_executable: Path,
) -> dict[str, Any]:
    """Run a focused synthetic manifest without weakening the public R0 gate."""

    return _run_matrix(
        manifest,
        cases_document,
        platform=platform,
        fixture_root=fixture_root,
        cache_root=cache_root,
        offline=offline,
        python_executable=python_executable,
        enforce_frozen_contract=False,
    )


def _run_matrix(
    manifest: Mapping[str, Any],
    cases_document: Mapping[str, Any],
    *,
    platform: str,
    fixture_root: Path,
    cache_root: Path,
    offline: bool,
    python_executable: Path,
    enforce_frozen_contract: bool,
) -> dict[str, Any]:
    if enforce_frozen_contract:
        harness.validate_manifest(manifest)
        cases = validate_cases(cases_document, manifest)
    else:
        harness._validate_manifest_shape(manifest)
        cases = _validate_cases_shape(
            cases_document, {reader["id"] for reader in manifest["readers"]}
        )
    if platform not in {item["id"] for item in manifest["platforms"]}:
        raise MatrixError(f"unknown platform {platform!r}")
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
            encoded = evidence.render_checked_evidence(normalized).encode()
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

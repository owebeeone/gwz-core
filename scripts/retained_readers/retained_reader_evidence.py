"""Normalize and compare retained-reader compatibility evidence."""

from __future__ import annotations

import argparse
import copy
import json
import hashlib
import os
import platform as host_platform
import re
import subprocess
from pathlib import Path
from typing import Any, Mapping, Sequence

from retained_reader_fixture import fixture_identities, fixture_set_identity


SCHEMA = "gwz.retained-reader-evidence/v1"
DIGEST_RE = re.compile(r"^[0-9a-f]{64}$")
EVIDENCE_SOURCE_NAMES = (
    "generate_retained_reader_fixtures.py",
    "retained_reader_errors.py",
    "retained_reader_evidence.py",
    "retained_reader_fixture.py",
    "retained_reader_harness.py",
    "retained_reader_matrix.py",
    "retained_reader_process.py",
    "retained_reader_runtime.py",
    "retained_reader_schema.py",
    "retained_reader_semantics.py",
    "retained_reader_yaml.py",
    "cases.schema.json",
    "fixture-contract.json",
    "manifest.schema.json",
)


class EvidenceError(RuntimeError):
    """A matrix result cannot be promoted to successful evidence."""


def _sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_digests(source_root: Path) -> dict[str, str]:
    missing = [name for name in EVIDENCE_SOURCE_NAMES if not (source_root / name).is_file()]
    if missing:
        raise EvidenceError(f"evidence source set is incomplete: {missing}")
    return {name: _sha256(source_root / name) for name in EVIDENCE_SOURCE_NAMES}


def source_set_sha256(sources: Mapping[str, str]) -> str:
    return hashlib.sha256(
        json.dumps(sources, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()


def collect_provenance(fixture_root: Path, python_executable: Path, platform_id: str) -> dict[str, Any]:
    os_name = {"Darwin": "macos", "Linux": "linux", "Windows": "windows"}.get(host_platform.system())
    machine = {"AMD64": "x86_64", "x86_64": "x86_64", "arm64": "aarch64", "aarch64": "aarch64"}.get(host_platform.machine())
    actual_platform = f"{os_name}-{machine}"
    if actual_platform != platform_id:
        raise EvidenceError(f"declared platform {platform_id!r} differs from host {actual_platform!r}")
    git = subprocess.run(["git", "--version"], text=True, capture_output=True, check=False)
    if git.returncode:
        raise EvidenceError(f"cannot identify Git: {git.stderr.strip()}")
    formats: set[str] = set()
    for git_dir in fixture_root.glob("**/.git"):
        completed = subprocess.run(
            ["git", "-C", str(git_dir.parent), "rev-parse", "--show-object-format"],
            text=True, capture_output=True, check=False,
        )
        if completed.returncode:
            raise EvidenceError(f"cannot identify fixture object format: {completed.stderr.strip()}")
        formats.add(completed.stdout.strip())
    if formats != {"sha1"}:
        raise EvidenceError(f"fixtures must all use sha1, found {sorted(formats)}")
    code = "import json,platform,struct;print(json.dumps({'implementation':platform.python_implementation(),'version':platform.python_version(),'architecture':platform.machine(),'pointer_bits':struct.calcsize('P')*8}))"
    python = subprocess.run([str(python_executable), "-c", code], text=True, capture_output=True, check=False)
    if python.returncode:
        raise EvidenceError(f"cannot identify Python: {python.stderr.strip()}")
    python_identity = json.loads(python.stdout)
    python_identity["executable_sha256"] = _sha256(python_executable.resolve())
    here = Path(__file__).resolve().parent
    sources = source_digests(here)
    evaluator_sha = source_set_sha256(sources)
    return {
        "fixture_set_sha256": fixture_set_identity(fixture_root),
        "fixtures": fixture_identities(fixture_root),
        "generator_sha256": _sha256(here / "generate_retained_reader_fixtures.py"),
        "evaluator_sha256": evaluator_sha,
        "sources": sources,
        "source_set_sha256": evaluator_sha,
        "git": {"version": git.stdout.strip(), "object_format": "sha1"},
        "platform": {"declared": platform_id, "system": host_platform.system(), "machine": host_platform.machine()},
        "python": python_identity,
        "execution": {"identity": "separate-attestation", "required_in_ci": True},
    }


def build_execution_attestation(evidence_bytes: bytes, platform_id: str) -> dict[str, Any]:
    commit, run_id = os.environ.get("GITHUB_SHA"), os.environ.get("GITHUB_RUN_ID")
    if not commit or not run_id:
        raise EvidenceError("execution attestation requires non-null GITHUB_SHA and GITHUB_RUN_ID")
    return {
        "schema": "gwz.retained-reader-execution/v1",
        "evidence_sha256": hashlib.sha256(evidence_bytes).hexdigest(),
        "platform": platform_id,
        "github_commit": commit,
        "github_run_id": run_id,
    }


def _validate_provenance(provenance: object) -> Mapping[str, Any]:
    required = {
        "fixture_set_sha256", "fixtures", "generator_sha256", "evaluator_sha256",
        "git", "platform", "python", "execution", "sources", "source_set_sha256",
    }
    if not isinstance(provenance, Mapping) or set(provenance) != required:
        raise EvidenceError("provenance must contain the complete canonical execution identity")
    for name in ("fixture_set_sha256", "generator_sha256", "evaluator_sha256", "source_set_sha256"):
        if not isinstance(provenance[name], str) or DIGEST_RE.fullmatch(provenance[name]) is None:
            raise EvidenceError(f"provenance {name} must be a SHA-256 digest")
    fixtures = provenance["fixtures"]
    if not isinstance(fixtures, dict) or not fixtures or not all(
        isinstance(key, str) and isinstance(value, str) and DIGEST_RE.fullmatch(value)
        for key, value in fixtures.items()
    ):
        raise EvidenceError("provenance fixtures must map names to SHA-256 digests")
    fixture_set = hashlib.sha256(json.dumps(fixtures, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    if fixture_set != provenance["fixture_set_sha256"]:
        raise EvidenceError("provenance fixture-set digest does not match its fixture mapping")
    sources = provenance["sources"]
    if not isinstance(sources, dict) or set(sources) != set(EVIDENCE_SOURCE_NAMES) or not all(
        isinstance(value, str) and DIGEST_RE.fullmatch(value) for value in sources.values()
    ):
        raise EvidenceError("provenance source mapping is incomplete")
    source_set = source_set_sha256(sources)
    if source_set != provenance["source_set_sha256"] or source_set != provenance["evaluator_sha256"]:
        raise EvidenceError("provenance source-set/evaluator digest does not match its sources")
    git = provenance["git"]
    if not isinstance(git, dict) or set(git) != {"version", "object_format"} or git.get("object_format") != "sha1" or not isinstance(git.get("version"), str):
        raise EvidenceError("provenance Git identity must pin a version and sha1 object format")
    platform = provenance["platform"]
    if not isinstance(platform, dict) or set(platform) != {"declared", "system", "machine"} or not all(isinstance(value, str) and value for value in platform.values()):
        raise EvidenceError("provenance platform identity is incomplete")
    python = provenance["python"]
    python_keys = {"implementation", "version", "architecture", "pointer_bits", "executable_sha256"}
    if not isinstance(python, dict) or set(python) != python_keys or python.get("implementation") != "CPython" or not isinstance(python.get("pointer_bits"), int) or DIGEST_RE.fullmatch(str(python.get("executable_sha256"))) is None:
        raise EvidenceError("provenance Python runtime identity is incomplete")
    if provenance["execution"] != {"identity": "separate-attestation", "required_in_ci": True}:
        raise EvidenceError("provenance must require the separate CI execution attestation")
    return provenance


def expected_result_keys(
    manifest: Mapping[str, Any], cases: Mapping[str, Any], platform: str
) -> set[tuple[str, str | None]]:
    result: set[tuple[str, str | None]] = set()
    for reader in manifest["readers"]:
        artifact = next(item for item in reader["artifacts"] if item["platform"] == platform)
        if artifact["support"] == "unsupported":
            result.add((reader["id"], None))
            continue
        selected = [
            case["id"] for case in cases["cases"]
            if "*" in case["readers"] or reader["id"] in case["readers"]
        ]
        result.update((reader["id"], case_id) for case_id in selected)
    return result


def validate_result_set(
    manifest: Mapping[str, Any], cases: Mapping[str, Any], platform: str,
    results: object,
) -> None:
    if not isinstance(results, list):
        raise EvidenceError("evidence results must be an array")
    actual: list[tuple[str, str | None]] = []
    for result in results:
        if not isinstance(result, dict) or not isinstance(result.get("reader"), str):
            raise EvidenceError("every evidence result must name a reader")
        case = result.get("case")
        if case is not None and not isinstance(case, str):
            raise EvidenceError("evidence case keys must be text")
        actual.append((result["reader"], case))
    expected = expected_result_keys(manifest, cases, platform)
    if len(actual) != len(set(actual)) or set(actual) != expected:
        raise EvidenceError(
            "evidence does not contain the exact unique expected result set; "
            f"missing={sorted(expected - set(actual))}, extra={sorted(set(actual) - expected)}"
        )


def validate_source_provenance(provenance: Mapping[str, Any], source_root: Path) -> None:
    _validate_provenance(provenance)
    expected = source_digests(source_root)
    if provenance["sources"] != expected or provenance["source_set_sha256"] != source_set_sha256(expected):
        raise EvidenceError("evidence source provenance is stale")
    if provenance["generator_sha256"] != expected["generate_retained_reader_fixtures.py"]:
        raise EvidenceError("evidence generator source provenance is stale")


def portable_projection(document: Mapping[str, Any]) -> dict[str, Any]:
    projected = copy.deepcopy(dict(document))
    provenance = projected["provenance"]
    del provenance["git"]["version"]
    del provenance["python"]["version"]
    del provenance["python"]["executable_sha256"]
    return projected


def validate_evidence_document(
    document: Mapping[str, Any], manifest: Mapping[str, Any], cases: Mapping[str, Any],
    *, manifest_path: Path, cases_path: Path, fixture_root: Path, source_root: Path,
) -> None:
    if document.get("schema") != SCHEMA or document.get("status") != "passed":
        raise EvidenceError("evidence schema/status is invalid")
    platform = document.get("platform")
    if not isinstance(platform, str):
        raise EvidenceError("evidence platform is missing")
    inputs = document.get("inputs")
    expected_inputs = {
        "manifest_sha256": _sha256(manifest_path), "cases_sha256": _sha256(cases_path)
    }
    if inputs != expected_inputs:
        raise EvidenceError("evidence input digests are stale")
    validate_result_set(manifest, cases, platform, document.get("results"))
    validate_source_provenance(document.get("provenance", {}), source_root)
    provenance = document["provenance"]
    fixtures = fixture_identities(fixture_root)
    if provenance["fixtures"] != fixtures or provenance["fixture_set_sha256"] != fixture_set_identity(fixture_root):
        raise EvidenceError("evidence fixture provenance is stale")


def _outcome(stdout: object) -> str | None:
    if not isinstance(stdout, str):
        return None
    try:
        payload = json.loads(stdout)
    except json.JSONDecodeError:
        return None
    if not isinstance(payload, dict):
        return None
    errors = payload.get("errors")
    if isinstance(errors, list) and errors and isinstance(errors[0], dict) and isinstance(errors[0].get("code"), str):
        return errors[0]["code"]
    merge = payload.get("merge")
    if isinstance(merge, dict) and isinstance(merge.get("state"), str):
        return merge["state"]
    for collection in (payload.get("branch_repos"), payload.get("repos")):
        if isinstance(collection, list) and collection and isinstance(collection[0], dict):
            if isinstance(collection[0].get("result"), str):
                return collection[0]["result"]
    response = payload.get("response")
    meta = response.get("meta") if isinstance(response, dict) else payload.get("meta")
    if isinstance(meta, dict) and isinstance(meta.get("aggregate_status"), str):
        return meta["aggregate_status"]
    return None


def build_evidence(
    manifest: Mapping[str, Any],
    cases: Mapping[str, Any],
    summary: Mapping[str, Any],
    *,
    manifest_sha256: str,
    cases_sha256: str,
    provenance: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    """Drop volatile streams, IDs, changed paths, and all host/cache paths."""

    if summary.get("status") != "passed":
        raise EvidenceError("only a passing matrix run can become retained-reader evidence")
    platform = summary.get("platform")
    readers = {reader["id"]: reader for reader in manifest["readers"]}
    artifacts = []
    for reader in manifest["readers"]:
        artifact = next(item for item in reader["artifacts"] if item["platform"] == platform)
        item = {
            "reader": reader["id"],
            "release": reader["release"],
            "surface": reader["surface"],
            "support": artifact["support"],
        }
        if artifact["support"] == "required":
            item.update(name=artifact["name"], sha256=artifact["sha256"])
        else:
            item["reason"] = artifact["reason"]
        artifacts.append(item)
    runtime_artifacts = [
        {"runtime": runtime["id"], "name": item["name"], "sha256": item["sha256"]}
        for runtime in manifest["runtimes"]
        for item in runtime.get("artifacts", [])
    ]
    results = []
    for result in summary["results"]:
        if result.get("status") not in {"passed", "declared-unsupported"}:
            raise EvidenceError(f"result for {result.get('reader')!r} is not a passing result")
        reader = readers[result["reader"]]
        normalized = {
            "reader": result["reader"],
            "release": reader["release"],
            "status": result["status"],
        }
        if "case" in result:
            if result.get("postconditions", {}).get("status") != "passed":
                raise EvidenceError(f"case {result.get('case')!r} has failing postconditions")
            outcome = _outcome(result.get("stdout"))
            if outcome is None:
                raise EvidenceError(f"case {result.get('case')!r} has no parsed typed JSON outcome")
            before = result.get("before_sha256")
            invariant = result.get("after_invariant_sha256")
            if not all(isinstance(value, str) and DIGEST_RE.fullmatch(value) for value in (before, invariant)):
                raise EvidenceError(f"case {result.get('case')!r} lacks snapshot identities")
            normalized.update(
                case=result["case"],
                exit_code=result["exit_code"],
                outcome=outcome,
                postconditions=result.get("postconditions", {"status": "passed", "count": 0}),
                before_sha256=before,
                after_invariant_sha256=invariant,
            )
        else:
            normalized["reason"] = result.get("reason")
        if "known_incompatibility" in result:
            normalized["known_incompatibility"] = result["known_incompatibility"]
        results.append(normalized)
    validate_result_set(manifest, cases, str(platform), summary.get("results"))
    provenance = _validate_provenance(provenance)
    return {
        "schema": SCHEMA,
        "platform": platform,
        "inputs": {
            "manifest_sha256": manifest_sha256,
            "cases_sha256": cases_sha256,
        },
        "artifacts": artifacts,
        "runtime_artifacts": runtime_artifacts,
        "provenance": dict(provenance),
        "results": results,
        "status": "passed",
    }


def _load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise EvidenceError(f"cannot load {path}: {error}") from error
    if not isinstance(value, dict):
        raise EvidenceError(f"{path} must contain a JSON object")
    return value


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["compare"])
    parser.add_argument("--checked", required=True, type=Path)
    parser.add_argument("--actual", required=True, type=Path)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--cases", required=True, type=Path)
    parser.add_argument("--fixtures", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        checked, actual = _load_json(args.checked), _load_json(args.actual)
        manifest, cases = _load_json(args.manifest), _load_json(args.cases)
        source_root = Path(__file__).resolve().parent
        for document in (checked, actual):
            validate_evidence_document(
                document, manifest, cases, manifest_path=args.manifest,
                cases_path=args.cases, fixture_root=args.fixtures, source_root=source_root,
            )
        if portable_projection(checked) != portable_projection(actual):
            raise EvidenceError("portable semantic evidence projection differs from checked evidence")
        print(json.dumps({"status": "equal", "platform": checked["platform"]}, sort_keys=True))
        return 0
    except EvidenceError as error:
        print(f"retained-reader evidence: {error}", file=__import__("sys").stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())

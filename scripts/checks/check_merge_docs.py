#!/usr/bin/env python3
"""Check merge capability documents against the canonical milestone manifest.

The checker is intentionally standard-library-only so release gates can run it
offline. Paths in the manifest are relative to the GWZ development workspace.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping, Sequence


DEFAULT_MANIFEST = Path(__file__).with_name("merge_docs_manifest.json")
DEFAULT_WORKSPACE_ROOT = Path(__file__).resolve().parents[3]
SUPPORTED_MATCHES = frozenset({"literal", "regex"})


class ManifestError(ValueError):
    """The assertion manifest is malformed."""


@dataclass(frozen=True, order=True)
class Finding:
    source_id: str
    path: str
    assertion_id: str
    message: str


@dataclass(frozen=True)
class CheckResult:
    findings: tuple[Finding, ...]
    source_count: int
    assertion_count: int

    @property
    def ok(self) -> bool:
        return not self.findings


def load_manifest(path: Path) -> Mapping[str, Any]:
    """Load and minimally validate a JSON assertion manifest."""

    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except OSError as error:
        raise ManifestError(f"cannot read manifest {path}: {error}") from error
    except json.JSONDecodeError as error:
        raise ManifestError(f"invalid JSON in manifest {path}: {error}") from error

    if not isinstance(value, dict):
        raise ManifestError("manifest root must be an object")
    if value.get("manifest_version") != 1:
        raise ManifestError("manifest_version must be 1")
    if not isinstance(value.get("sources"), list) or not value["sources"]:
        raise ManifestError("sources must be a non-empty array")
    if not isinstance(value.get("global_forbidden", []), list):
        raise ManifestError("global_forbidden must be an array")
    return value


def _normalized(value: str) -> str:
    return re.sub(r"\s+", " ", value).strip()


def _require_string(value: Any, context: str) -> str:
    if not isinstance(value, str) or not value:
        raise ManifestError(f"{context} must be a non-empty string")
    return value


def _validate_assertion(value: Any, context: str) -> Mapping[str, Any]:
    if not isinstance(value, dict):
        raise ManifestError(f"{context} must be an object")
    _require_string(value.get("id"), f"{context}.id")
    _require_string(value.get("value"), f"{context}.value")
    match = value.get("match", "literal")
    if match not in SUPPORTED_MATCHES:
        raise ManifestError(
            f"{context}.match must be one of {sorted(SUPPORTED_MATCHES)}"
        )
    if match == "regex":
        try:
            re.compile(value["value"])
        except re.error as error:
            raise ManifestError(f"invalid regex in {context}: {error}") from error
    return value


def _matches(text: str, assertion: Mapping[str, Any]) -> bool:
    value = assertion["value"]
    if assertion.get("match", "literal") == "literal":
        return _normalized(value) in _normalized(text)
    return re.search(value, text) is not None


def _source_path(
    source: Mapping[str, Any],
    workspace_root: Path,
    source_overrides: Mapping[str, Path],
) -> Path:
    source_id = source["id"]
    if source_id in source_overrides:
        return source_overrides[source_id]
    return workspace_root / source["path"]


def check_manifest(
    manifest: Mapping[str, Any],
    workspace_root: Path,
    *,
    source_overrides: Mapping[str, Path] | None = None,
) -> CheckResult:
    """Evaluate every manifest assertion and return all deterministic findings."""

    overrides = source_overrides or {}
    sources = manifest["sources"]
    global_forbidden = manifest.get("global_forbidden", [])
    seen_source_ids: set[str] = set()
    findings: list[Finding] = []
    assertion_count = 0

    validated_global = [
        _validate_assertion(value, f"global_forbidden[{index}]")
        for index, value in enumerate(global_forbidden)
    ]

    for source_index, source_value in enumerate(sources):
        context = f"sources[{source_index}]"
        if not isinstance(source_value, dict):
            raise ManifestError(f"{context} must be an object")
        source_id = _require_string(source_value.get("id"), f"{context}.id")
        relative_path = _require_string(source_value.get("path"), f"{context}.path")
        if source_id in seen_source_ids:
            raise ManifestError(f"duplicate source id: {source_id}")
        seen_source_ids.add(source_id)

        required_values = source_value.get("required", [])
        forbidden_values = source_value.get("forbidden", [])
        if not isinstance(required_values, list) or not isinstance(
            forbidden_values, list
        ):
            raise ManifestError(f"{context} required/forbidden must be arrays")
        required = [
            _validate_assertion(value, f"{context}.required[{index}]")
            for index, value in enumerate(required_values)
        ]
        forbidden = [
            _validate_assertion(value, f"{context}.forbidden[{index}]")
            for index, value in enumerate(forbidden_values)
        ]

        path = _source_path(source_value, workspace_root, overrides)
        display_path = relative_path if source_id not in overrides else str(path)
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeError) as error:
            findings.append(
                Finding(
                    source_id,
                    display_path,
                    "source_missing",
                    f"source cannot be read as UTF-8: {error}",
                )
            )
            continue

        for assertion in required:
            assertion_count += 1
            if not _matches(text, assertion):
                findings.append(
                    Finding(
                        source_id,
                        display_path,
                        assertion["id"],
                        "required statement is absent",
                    )
                )
        for assertion in (*forbidden, *validated_global):
            assertion_count += 1
            if _matches(text, assertion):
                findings.append(
                    Finding(
                        source_id,
                        display_path,
                        assertion["id"],
                        "forbidden statement is present",
                    )
                )

    return CheckResult(
        findings=tuple(sorted(findings)),
        source_count=len(sources),
        assertion_count=assertion_count,
    )


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--workspace-root",
        type=Path,
        default=DEFAULT_WORKSPACE_ROOT,
        help="GWZ development workspace containing gwz-core and gwz-cli",
    )
    parser.add_argument(
        "--manifest",
        type=Path,
        default=DEFAULT_MANIFEST,
        help="JSON assertion manifest",
    )
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    try:
        manifest = load_manifest(args.manifest)
        result = check_manifest(manifest, args.workspace_root.resolve())
    except ManifestError as error:
        print(f"merge document consistency: invalid manifest: {error}", file=sys.stderr)
        return 2

    if result.ok:
        print(
            "merge document consistency: ok "
            f"({result.source_count} sources, {result.assertion_count} assertions)"
        )
        return 0

    print(
        "merge document consistency: failed "
        f"({len(result.findings)} finding(s))",
        file=sys.stderr,
    )
    for finding in result.findings:
        print(
            f"{finding.path}: [{finding.assertion_id}] {finding.message}",
            file=sys.stderr,
        )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())

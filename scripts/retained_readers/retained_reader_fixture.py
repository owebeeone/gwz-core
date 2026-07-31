"""Deterministic fixture snapshots and retained-reader expectation evaluation."""

from __future__ import annotations

import base64
import fnmatch
import hashlib
import json
import os
import stat
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping

from retained_reader_semantics import (
    SemanticError,
    index_observation,
    normalized_mutations,
    root_publication_observation,
    yaml_observation,
    yaml_set_observation,
)


class FixtureError(RuntimeError):
    """An isolated fixture cannot be snapshotted deterministically."""


@dataclass(frozen=True)
class TreeSnapshot:
    sha256: str
    entries: dict[str, dict[str, Any]]


def _path_key(relative: Path) -> str:
    raw = os.fsencode(relative.as_posix())
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError:
        return "b64:" + base64.b64encode(raw).decode("ascii")
    if os.fsencode(text) != raw:
        return "b64:" + base64.b64encode(raw).decode("ascii")
    return "text:" + text


def _file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def snapshot_tree(root: Path) -> TreeSnapshot:
    if not root.is_dir():
        raise FixtureError(f"snapshot root is not a directory: {root}")
    entries: dict[str, dict[str, Any]] = {}

    def visit(directory: Path, relative: Path) -> None:
        try:
            with os.scandir(directory) as scanner:
                children = sorted(scanner, key=lambda item: os.fsencode(item.name))
        except OSError as error:
            raise FixtureError(f"cannot scan fixture {directory}: {error}") from error
        for child in children:
            child_relative = relative / child.name
            key = _path_key(child_relative)
            metadata = child.stat(follow_symlinks=False)
            mode = stat.S_IMODE(metadata.st_mode)
            path = Path(child.path)
            if child.is_symlink():
                target = os.fsencode(os.readlink(path))
                entries[key] = {
                    "kind": "symlink",
                    "mode": mode,
                    "target_b64": base64.b64encode(target).decode("ascii"),
                }
            elif child.is_dir(follow_symlinks=False):
                is_object_fanout = (
                    len(child_relative.parts) >= 3
                    and child_relative.parts[-2] == "objects"
                    and child_relative.parts[-3] == ".git"
                    and __import__("re").fullmatch(r"[0-9a-f]{2}", child_relative.parts[-1])
                )
                if not is_object_fanout:
                    entries[key] = {"kind": "directory", "mode": mode}
                visit(path, child_relative)
            elif child.is_file(follow_symlinks=False):
                entries[key] = {
                    "kind": "file",
                    "mode": mode,
                    "size": metadata.st_size,
                    "sha256": _file_sha256(path),
                }
            else:
                entries[key] = {"kind": "other", "mode": mode}

    visit(root, Path())
    canonical = json.dumps(entries, ensure_ascii=True, sort_keys=True, separators=(",", ":"))
    return TreeSnapshot(hashlib.sha256(canonical.encode("utf-8")).hexdigest(), entries)


def changed_paths(before: TreeSnapshot, after: TreeSnapshot) -> list[str]:
    paths = set(before.entries) | set(after.entries)
    return sorted(path for path in paths if before.entries.get(path) != after.entries.get(path))


def _logical_snapshot(root: Path) -> str:
    snapshot = snapshot_tree(root)
    entries: dict[str, dict[str, Any]] = {}
    for key, item in snapshot.entries.items():
        logical = {name: value for name, value in item.items() if name != "mode"}
        plain = key.removeprefix("text:")
        object_match = __import__("re").fullmatch(r"(?:.+/)?\.git/objects/([0-9a-f]{2})/([0-9a-f]{38})", plain)
        if object_match and logical.get("kind") == "file":
            logical = {"kind": "git-object", "oid": "".join(object_match.groups())}
        elif plain.endswith("/.git/index") or plain == ".git/index":
            repository = root / plain.removesuffix("/.git/index") if plain != ".git/index" else root
            listed = _run_git(repository, ["ls-files", "--stage", "-z"])
            if listed.returncode:
                raise FixtureError(f"cannot identify logical Git index: {listed.stderr.strip()}")
            logical = {"kind": "git-index", "sha256": hashlib.sha256(listed.stdout.encode()).hexdigest()}
        entries[key] = logical
    canonical = json.dumps(entries, ensure_ascii=True, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(canonical.encode()).hexdigest()


def fixture_identities(root: Path) -> dict[str, str]:
    children = sorted(root.iterdir())
    unexpected = [path.name for path in children if not path.is_dir()]
    if unexpected:
        raise FixtureError(f"fixture set contains non-directory entries: {unexpected}")
    if not children:
        raise FixtureError("fixture set is empty")
    return {path.name: _logical_snapshot(path) for path in children}


def fixture_set_identity(root: Path) -> str:
    payload = json.dumps(fixture_identities(root), sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(payload.encode()).hexdigest()


def _json_contract_errors(label: str, contract: object, actual: str) -> list[str]:
    if not isinstance(contract, dict):
        return [f"{label} JSON contract must be an object"]
    try:
        payload = json.loads(actual)
    except json.JSONDecodeError as error:
        return [f"{label} is not valid json-contract JSON: {error}"]
    if not isinstance(payload, dict):
        return [f"{label} JSON contract requires an object"]
    shape = contract.get("shape")
    outcomes = contract.get("outcomes")
    if not isinstance(outcomes, list) or not outcomes or not all(isinstance(item, str) for item in outcomes):
        return [f"{label} JSON contract outcomes must be typed strings"]
    if shape == "merge":
        merge = payload.get("merge")
        if not isinstance(merge, dict):
            return [f"{label} JSON contract requires merge object"]
        if merge.get("merge_id") != contract.get("merge_id") or not isinstance(merge.get("merge_id"), str):
            return [f"{label} JSON contract merge_id differs or is not text"]
        if not isinstance(merge.get("state"), str) or merge["state"] not in outcomes:
            return [f"{label} JSON contract merge state is not allowed"]
        rows = merge.get("repos")
        if rows is None:
            rows = payload.get("repos")
        member_id = contract.get("member_id")
        member_outcomes = contract.get("member_outcomes", [])
        if member_id is not None:
            if not isinstance(rows, list):
                return [f"{label} JSON contract requires repository rows"]
            row = next((item for item in rows if isinstance(item, dict) and item.get("target_id") == member_id), None)
            if row is None or not isinstance(row.get("state"), str) or row["state"] not in member_outcomes:
                return [f"{label} JSON contract member row is missing or invalid"]
        return []
    if shape == "workspace-status":
        response = payload.get("response", payload)
        meta = response.get("meta") if isinstance(response, dict) else None
        members = response.get("members") if isinstance(response, dict) else None
        if not isinstance(meta, dict) or not isinstance(meta.get("aggregate_status"), str) or meta["aggregate_status"] not in outcomes:
            return [f"{label} JSON contract aggregate status is missing or invalid"]
        row = next((item for item in members or [] if isinstance(item, dict) and item.get("member_id") == contract.get("member_id")), None)
        if row is None or not isinstance(row.get("status"), str) or row["status"] not in contract.get("member_outcomes", []):
            return [f"{label} JSON contract member is missing or not typed"]
        workspace_git_status = response.get("workspace_git_status") or payload.get("workspace_git_status")
        if not isinstance(workspace_git_status, dict):
            return [f"{label} JSON contract workspace_git_status is missing or not typed"]
        return []
    if shape == "legacy-branch":
        rows = payload.get("branch_repos", payload.get("repos"))
        if not isinstance(rows, list):
            return [f"{label} JSON contract branch rows are missing"]
        row = next((item for item in rows if isinstance(item, dict) and item.get("member_id") == contract.get("member_id")), None)
        if row is None or not isinstance(row.get("result"), str) or row["result"] not in outcomes:
            return [f"{label} JSON contract branch result is missing or invalid"]
        return []
    if shape == "error":
        errors = payload.get("errors")
        if not isinstance(errors, list) or not errors or not isinstance(errors[0], dict):
            return [f"{label} JSON contract error envelope is missing"]
        code = errors[0].get("code")
        return [] if isinstance(code, str) and code in outcomes else [f"{label} JSON contract error code is missing or invalid"]
    return [f"{label} JSON contract has unsupported shape {shape!r}"]


def _stream_errors(label: str, specification: Mapping[str, Any], actual: str) -> list[str]:
    mode = specification.get("mode")
    expected = specification.get("value")
    try:
        if mode == "exact":
            matches = actual == expected
        elif mode == "contains":
            needles = expected if isinstance(expected, list) else [expected]
            matches = all(isinstance(item, str) and item in actual for item in needles)
        elif mode == "json":
            matches = json.loads(actual) == expected
        elif mode == "jsonl":
            matches = [json.loads(line) for line in actual.splitlines() if line] == expected
        elif mode == "json-contract":
            return _json_contract_errors(label, expected, actual)
        else:
            return [f"{label} expectation has unsupported mode {mode!r}"]
    except json.JSONDecodeError as error:
        return [f"{label} is not valid {mode}: {error}"]
    return [] if matches else [f"{label} did not match {mode} expectation"]


def evaluate_expectation(
    expected: Mapping[str, Any],
    completed: Any,
    before: TreeSnapshot,
    after: TreeSnapshot,
) -> list[str]:
    errors: list[str] = []
    exit_codes = expected.get("exit_codes")
    if not isinstance(exit_codes, list) or completed.returncode not in exit_codes:
        errors.append(f"exit code {completed.returncode} not in expected {exit_codes!r}")
    for label in ("stdout", "stderr"):
        if label in expected:
            errors.extend(_stream_errors(label, expected[label], getattr(completed, label)))
    changes = changed_paths(before, after)
    mutation = expected.get("mutation")
    if not isinstance(mutation, dict):
        errors.append("mutation expectation is required")
        return errors
    mode = mutation.get("mode")
    if mode == "none" and changes:
        errors.append(f"unexpected mutation: {', '.join(changes)}")
    elif mode == "exact":
        wanted = sorted(mutation.get("paths", []))
        if changes != wanted:
            errors.append(f"changed paths {changes!r} do not equal expected {wanted!r}")
    elif mode == "allow":
        allowed = mutation.get("allowed", [])
        unexpected = [path for path in changes if not any(fnmatch.fnmatchcase(path, pattern) for pattern in allowed)]
        missing = [pattern for pattern in mutation.get("required", []) if not any(fnmatch.fnmatchcase(path, pattern) for path in changes)]
        if unexpected:
            errors.append(f"unexpected mutation: {', '.join(unexpected)}")
        if missing:
            errors.append(f"required mutation missing: {', '.join(missing)}")
    elif mode == "contract":
        exact = set(mutation.get("exact", []))
        remaining = [path for path in changes if path not in exact]
        missing = sorted(exact - set(changes))
        if missing:
            errors.append(f"required exact mutation missing: {', '.join(missing)}")
        contracts = mutation.get("dynamic", [])
        unmatched = [path for path in remaining if sum(fnmatch.fnmatchcase(path, item["pattern"]) for item in contracts) != 1]
        if unmatched:
            errors.append(f"dynamic mutation did not match exactly one contract: {', '.join(unmatched)}")
        for item in contracts:
            count = sum(fnmatch.fnmatchcase(path, item["pattern"]) for path in remaining)
            if not item["minimum"] <= count <= item["maximum"]:
                errors.append(f"dynamic mutation {item['pattern']!r} count {count} outside {item['minimum']}..{item['maximum']}")
    elif mode not in {"none", "exact", "allow", "contract"}:
        errors.append(f"unsupported mutation mode {mode!r}")
    if expected.get("before_sha256") not in {None, before.sha256}:
        errors.append("before snapshot digest differs")
    if expected.get("after_sha256") not in {None, after.sha256}:
        errors.append("after snapshot digest differs")
    return errors


def normalized_mutation_identity(
    mutation: Mapping[str, Any], changes: list[str], after: TreeSnapshot, workspace: Path
) -> list[dict[str, Any]]:
    try:
        return normalized_mutations(mutation, changes, after, workspace)
    except (SemanticError, OSError, ValueError) as error:
        raise FixtureError(f"cannot normalize durable mutation content: {error}") from error


def _safe_relative_path(value: object, label: str) -> Path:
    path = Path(str(value))
    if not value or path.is_absolute() or ".." in path.parts:
        raise FixtureError(f"{label} path is unsafe: {value!r}")
    return path


def _run_git(repository: Path, args: list[str]) -> subprocess.CompletedProcess[str]:
    environment = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    environment.update(
        {
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_CONFIG_GLOBAL": os.devnull,
            "LC_ALL": "C",
            "LANG": "C",
        }
    )
    try:
        return subprocess.run(
            ["git", *args],
            cwd=repository,
            env=environment,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=15,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise FixtureError(f"postcondition Git invocation failed: {error}") from error


def evaluate_postconditions(
    specifications: object,
    workspace: Path,
    *,
    before_root: Path | None = None,
) -> tuple[list[str], list[dict[str, Any]]]:
    """Evaluate portable semantic checks after a retained reader exits."""

    if specifications is None:
        return [], []
    if not isinstance(specifications, list):
        return ["postconditions must be a list"], []
    errors: list[str] = []
    observations: list[dict[str, Any]] = []
    for position, specification in enumerate(specifications):
        if not isinstance(specification, dict):
            errors.append(f"postcondition {position} must be an object")
            continue
        kind = specification.get("kind")
        try:
            if kind == "path":
                path = workspace / _safe_relative_path(specification.get("path"), "postcondition")
                state = specification.get("state")
                matched = {
                    "file": path.is_file(),
                    "directory": path.is_dir(),
                    "absent": not path.exists() and not path.is_symlink(),
                }.get(str(state))
                if matched is not True:
                    errors.append(f"postcondition {position}: {path.relative_to(workspace)} is not {state}")
                observations.append({"kind": kind, "path": str(specification.get("path")), "state": state, "matched": matched is True})
                continue

            if kind == "snapshot-entries-unchanged":
                if before_root is None:
                    raise FixtureError("snapshot comparison requires before_root")
                old, new = snapshot_tree(before_root), snapshot_tree(workspace)
                matched = all(old.entries.get(f"text:{path}") == new.entries.get(f"text:{path}") for path in specification.get("paths", []))
                observations.append({"kind": kind, "paths": specification.get("paths", []), "matched": matched})
                if not matched:
                    errors.append(f"postcondition {position}: snapshot entries changed")
                continue
            if kind == "merge-record-baseline-preserved":
                if before_root is None:
                    raise FixtureError("baseline comparison requires before_root")
                source = (before_root / _safe_relative_path(specification.get("before"), "before")).read_text(encoding="utf-8")
                target = (workspace / _safe_relative_path(specification.get("after"), "after")).read_text(encoding="utf-8")
                fields = specification.get("fields", [])
                def extract(text: str, field: str) -> str | None:
                    lines = text.splitlines()
                    prefix = f"  {field}:"
                    for index, line in enumerate(lines):
                        if line.startswith(prefix):
                            value = line[len(prefix):].lstrip()
                            if value != "|":
                                return value
                            block: list[str] = []
                            for child in lines[index + 1:]:
                                if not child.startswith("    "):
                                    break
                                block.append(child[4:])
                            return "\n".join(block) + "\n"
                    return None
                matched = all(extract(source, field) is not None and extract(source, field) == extract(target, field) for field in fields)
                observations.append({"kind": kind, "fields": fields, "matched": matched})
                if not matched:
                    errors.append(f"postcondition {position}: merge-record baseline fields changed or disappeared")
                continue
            if kind == "yaml-semantic":
                matched, observation = yaml_observation(specification, workspace)
                observations.append(observation)
                if not matched:
                    errors.append(f"postcondition {position}: YAML semantic content differs")
                continue
            if kind == "yaml-set-semantic":
                matched, observation = yaml_set_observation(specification, workspace)
                observations.append(observation)
                if not matched:
                    errors.append(f"postcondition {position}: YAML semantic set differs")
                continue
            if kind == "root-publication":
                matched, observation = root_publication_observation(specification, workspace)
                observations.append(observation)
                if not matched:
                    errors.append(f"postcondition {position}: root publication semantics differ")
                continue
            if kind == "git-index-semantic":
                matched, observation = index_observation(specification, workspace)
                observations.append(observation)
                if not matched:
                    errors.append(f"postcondition {position}: Git index semantics differ")
                continue

            repository = workspace / _safe_relative_path(
                specification.get("repository"), "repository"
            )
            if not repository.is_dir():
                errors.append(f"postcondition {position}: repository is missing")
                continue
            if kind == "git-ref-equals":
                left = _run_git(
                    repository, ["rev-parse", "--verify", str(specification.get("left"))]
                )
                right = _run_git(
                    repository, ["rev-parse", "--verify", str(specification.get("right"))]
                )
                completed = left if left.returncode else right
                matched = (
                    left.returncode == 0
                    and right.returncode == 0
                    and left.stdout.strip() == right.stdout.strip()
                )
            elif kind == "git-commit-message":
                completed = _run_git(
                    repository,
                    ["log", "-1", "--format=%B", str(specification.get("ref"))],
                )
                matched = (
                    completed.returncode == 0
                    and completed.stdout.rstrip("\n") == specification.get("value")
                )
            elif kind == "git-parent-count":
                completed = _run_git(
                    repository,
                    ["rev-list", "--parents", "-n", "1", str(specification.get("ref"))],
                )
                matched = (
                    completed.returncode == 0
                    and len(completed.stdout.strip().split()) - 1 == specification.get("count")
                )
            else:
                errors.append(f"postcondition {position}: unsupported kind {kind!r}")
                continue
            if not matched:
                detail = completed.stderr.strip() or completed.stdout.strip()
                errors.append(f"postcondition {position} ({kind}) failed: {detail}")
            observations.append({
                "kind": kind,
                "left": left.stdout.strip() if kind == "git-ref-equals" and left.returncode == 0 else None,
                "right": right.stdout.strip() if kind == "git-ref-equals" and right.returncode == 0 else None,
                "matched": matched,
            })
        except (FixtureError, SemanticError, OSError, ValueError) as error:
            errors.append(f"postcondition {position}: {error}")
    return errors, observations

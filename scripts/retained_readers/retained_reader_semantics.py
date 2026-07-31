"""Portable, content-sensitive observations for retained-reader mutations."""

from __future__ import annotations

import hashlib
import json
import os
import re
import subprocess
from pathlib import Path
from typing import Any, Mapping

from retained_reader_yaml import (
    UUID_RE,
    YamlSubsetError,
    canonical_yaml_sha256,
    parse_yaml_subset,
    yaml_lookup,
)


OBJECT_RE = re.compile(r"^(?P<prefix>(?:.+/)?)\.git/objects/(?P<fan>[0-9a-f]{2})/(?P<rest>[0-9a-f]{38})$")
OID_RE = re.compile(r"\b[0-9a-f]{40}\b")
SIGNATURE_TIME_RE = re.compile(r"\s\d+ [+-]\d{4}$")


class SemanticError(RuntimeError):
    """A durable output cannot be reduced to a portable semantic identity."""


def _sha256(payload: bytes | str) -> str:
    if isinstance(payload, str):
        payload = payload.encode()
    return hashlib.sha256(payload).hexdigest()


def _git(repository: Path, args: list[str], *, binary: bool = False) -> bytes | str:
    environment = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    environment.update({"GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull, "LC_ALL": "C", "LANG": "C"})
    completed = subprocess.run(
        ["git", *args], cwd=repository, env=environment,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=not binary, check=False,
    )
    if completed.returncode:
        stderr = completed.stderr.decode(errors="replace") if binary else completed.stderr
        raise SemanticError(f"git {' '.join(args)} failed: {stderr.strip()}")
    return completed.stdout


def _canonical(value: Any) -> str:
    return _sha256(json.dumps(value, ensure_ascii=True, sort_keys=True, separators=(",", ":")))


def _normalize_name(value: str) -> str:
    return UUID_RE.sub("<uuid>", value)


def _normalize_dynamic_value(value: Any) -> Any:
    if isinstance(value, dict):
        return {
            UUID_RE.sub("<uuid>", key): _normalize_dynamic_value(item)
            for key, item in value.items()
        }
    if isinstance(value, list):
        return [_normalize_dynamic_value(item) for item in value]
    if isinstance(value, str):
        return UUID_RE.sub("<uuid>", value)
    return value


def git_object_semantic(repository: Path, oid: str, cache: dict[str, str] | None = None) -> str:
    cache = {} if cache is None else cache
    if oid in cache:
        return cache[oid]
    object_type = str(_git(repository, ["cat-file", "-t", oid])).strip()
    if object_type == "blob":
        payload = _git(repository, ["cat-file", "blob", oid], binary=True)
        assert isinstance(payload, bytes)
        try:
            text = payload.decode("utf-8")
            digest = (
                canonical_yaml_sha256(text, normalize_dynamic=True)
                if text.lstrip().startswith("schema:")
                else _sha256(payload)
            )
        except (UnicodeDecodeError, YamlSubsetError):
            digest = _sha256(payload)
        value = _canonical({"type": "blob", "content": digest})
    elif object_type == "tree":
        output = _git(repository, ["ls-tree", "-z", oid], binary=True)
        assert isinstance(output, bytes)
        rows = []
        for row in output.rstrip(b"\0").split(b"\0") if output else []:
            metadata, raw_name = row.split(b"\t", 1)
            mode, kind, child = metadata.decode().split()
            name = _normalize_name(raw_name.decode("utf-8", errors="surrogateescape"))
            rows.append({"mode": mode, "kind": kind, "name": name, "content": git_object_semantic(repository, child, cache)})
        value = _canonical({"type": "tree", "entries": sorted(rows, key=lambda item: item["name"])})
    elif object_type == "commit":
        payload = str(_git(repository, ["cat-file", "commit", oid]))
        headers, _, message = payload.partition("\n\n")
        normalized: dict[str, Any] = {"parents": [], "message": message}
        for line in headers.splitlines():
            key, value_text = line.split(" ", 1)
            if key == "tree":
                normalized["tree"] = git_object_semantic(repository, value_text, cache)
            elif key == "parent":
                normalized["parents"].append(git_object_semantic(repository, value_text, cache))
            elif key in {"author", "committer"}:
                normalized[key] = SIGNATURE_TIME_RE.sub(" <time>", value_text)
            else:
                normalized.setdefault("headers", []).append([key, value_text])
        value = _canonical({"type": "commit", **normalized})
    else:
        payload = _git(repository, ["cat-file", object_type, oid], binary=True)
        assert isinstance(payload, bytes)
        value = _canonical({"type": object_type, "content": _sha256(payload)})
    cache[oid] = value
    return value


def _repository_for(workspace: Path, plain_path: str) -> tuple[Path, str]:
    if plain_path.startswith(".git/"):
        return workspace, plain_path
    prefix, separator, git_path = plain_path.partition("/.git/")
    if not separator:
        raise SemanticError(f"path is not inside a Git repository: {plain_path}")
    return workspace / prefix, ".git/" + git_path


def _index_semantic(repository: Path) -> str:
    output = str(_git(repository, ["ls-files", "--stage", "-z"]))
    rows = []
    cache: dict[str, str] = {}
    for row in output.rstrip("\0").split("\0") if output else []:
        metadata, name = row.split("\t", 1)
        mode, oid, stage = metadata.split()
        rows.append({
            "mode": mode,
            "stage": stage,
            "path": _normalize_name(name),
            "content": git_object_semantic(repository, oid, cache),
        })
    return _canonical(rows)


def merge_record_semantic(workspace: Path, text: str) -> str:
    """Hash every record field while replacing publication UUID/OID mechanics."""

    document = parse_yaml_subset(text)
    publication = document.get("publication")
    checks: dict[str, Any] = {}
    if isinstance(publication, dict):
        candidate = publication.get("candidate")
        marker_digest = None
        if isinstance(candidate, dict) and isinstance(candidate.get("marker_yaml"), str):
            marker_text = candidate["marker_yaml"]
            marker_document = parse_yaml_subset(marker_text)
            marker = _normalize_dynamic_value(marker_document)
            marker_digest = _canonical(marker)
            marker_id = candidate.get("marker_id")
            marker_path = publication.get("candidate_marker_path")
            checks["marker_id_matches_yaml"] = marker_id == marker_document.get("gwz_commit_id")
            checks["marker_sha256_matches_yaml"] = candidate.get("marker_sha256") == _sha256(marker_text)
            checks["marker_path_matches_id"] = marker_path == f"gwz.conf/markers/{marker_id}.yaml"
            if isinstance(marker_path, str):
                relative = Path(marker_path)
                if relative.is_absolute() or ".." in relative.parts:
                    raise SemanticError("publication marker path is unsafe")
                actual_marker = workspace / relative
                checks["marker_file_matches_yaml"] = (
                    actual_marker.is_file()
                    and actual_marker.read_text(encoding="utf-8") == marker_text
                )
            candidate["marker_yaml"] = marker
            candidate["marker_sha256"] = marker_digest
        if isinstance(candidate, dict) and isinstance(candidate.get("lock_yaml"), str):
            lock_text = candidate["lock_yaml"]
            checks["lock_sha256_matches_yaml"] = publication.get("candidate_lock_sha256") == _sha256(lock_text)
            lock_path = workspace / "gwz.conf/gwz.lock.yml"
            checks["lock_file_matches_yaml"] = (
                lock_path.is_file() and lock_path.read_text(encoding="utf-8") == lock_text
            )
        hashes = publication.get("candidate_hashes")
        if marker_digest is not None and isinstance(hashes, list):
            for row in hashes:
                if not isinstance(row, dict) or not isinstance(row.get("path"), str):
                    continue
                relative = Path(row["path"])
                if relative.is_absolute() or ".." in relative.parts:
                    raise SemanticError("publication candidate hash path is unsafe")
                actual = workspace / relative
                checks[f"candidate_hash:{_normalize_name(row['path'])}"] = (
                    actual.is_file() and row.get("sha256") == _sha256(actual.read_bytes())
                )
                if "/markers/" in row["path"]:
                    row["sha256"] = marker_digest
        head = str(_git(workspace, ["rev-parse", "HEAD"])).strip()
        tree = str(_git(workspace, ["rev-parse", "HEAD^{tree}"])).strip()
        checks["composition_commit_is_head"] = publication.get("composition_commit") == head
        checks["composition_tree_is_head_tree"] = publication.get("composition_tree") == tree
        for key in ("composition_commit", "composition_tree"):
            oid = publication.get(key)
            if isinstance(oid, str) and OID_RE.fullmatch(oid):
                publication[key] = git_object_semantic(workspace, oid)
    return _canonical({"record": _normalize_dynamic_value(document), "publication_checks": checks})


def semantic_path_identity(workspace: Path, path_key: str, entry: Mapping[str, Any] | None) -> str:
    if entry is None:
        return "absent"
    if not path_key.startswith("text:"):
        return _canonical(entry)
    plain = path_key[5:]
    if entry.get("kind") == "directory":
        return "directory"
    match = OBJECT_RE.fullmatch(plain)
    if match:
        repository = workspace / match.group("prefix").removesuffix("/")
        return git_object_semantic(repository, match.group("fan") + match.group("rest"))
    path = workspace / plain
    if plain.endswith("/.git/index") or plain == ".git/index":
        repository, _ = _repository_for(workspace, plain)
        return _index_semantic(repository)
    if "/.git/refs/" in plain or plain.startswith(".git/refs/"):
        repository, git_path = _repository_for(workspace, plain)
        oid = path.read_text(encoding="ascii").strip()
        return _canonical({"ref": git_path.removeprefix(".git/"), "target": git_object_semantic(repository, oid)})
    if "/.git/logs/" in plain or plain.startswith(".git/logs/"):
        repository, _ = _repository_for(workspace, plain)
        cache: dict[str, str] = {}
        text = path.read_text(encoding="utf-8", errors="surrogateescape")
        normalized = OID_RE.sub(
            lambda match: (
                "<zero-oid>"
                if match.group() == "0" * 40
                else git_object_semantic(repository, match.group(), cache)
            ),
            text,
        )
        normalized = re.sub(r"\s\d+ [+-]\d{4}\t", " <time>\t", normalized)
        return _sha256(normalized)
    if path.suffix in {".yaml", ".yml"}:
        if "/.gwz/merge/done/" in f"/{plain}":
            return merge_record_semantic(workspace, path.read_text(encoding="utf-8"))
        return canonical_yaml_sha256(path.read_text(encoding="utf-8"), normalize_dynamic="/markers/" in f"/{plain}")
    return str(entry.get("sha256") or _canonical(entry))


def normalized_mutations(
    mutation: Mapping[str, Any], changes: list[str], after: Any, workspace: Path
) -> list[dict[str, Any]]:
    mode = mutation.get("mode")
    if mode in {"none", "exact"}:
        return [
            {"class": "exact", "path": path, "content": semantic_path_identity(workspace, path, after.entries.get(path))}
            for path in sorted(changes)
        ]
    exact = set(mutation.get("exact", []))
    result = [
        {"class": "exact", "path": path, "content": semantic_path_identity(workspace, path, after.entries.get(path))}
        for path in sorted(set(changes) & exact)
    ]
    for item in mutation.get("dynamic", []):
        matched = sorted(path for path in changes if path not in exact and __import__("fnmatch").fnmatchcase(path, item["pattern"]))
        contents = sorted(semantic_path_identity(workspace, path, after.entries.get(path)) for path in matched)
        result.append({
            "class": "dynamic", "pattern": item["pattern"], "count": len(matched),
            "content_sha256": _canonical(contents),
        })
    return result


def yaml_observation(specification: Mapping[str, Any], workspace: Path) -> tuple[bool, dict[str, Any]]:
    path = workspace / str(specification["path"])
    text = path.read_text(encoding="utf-8")
    normalize = bool(specification.get("normalize_dynamic", False))
    document = parse_yaml_subset(text)
    digest = (
        merge_record_semantic(workspace, text)
        if specification.get("semantic") == "merge-record"
        else canonical_yaml_sha256(text, normalize_dynamic=normalize)
    )
    required = specification.get("required", {})
    matched = digest == specification.get("sha256") and all(
        str(yaml_lookup(document, key)) == str(value) for key, value in required.items()
    )
    return matched, {
        "kind": "yaml-semantic", "path": str(specification["path"]),
        "sha256": digest, "required": {key: yaml_lookup(document, key) for key in sorted(required)},
        "matched": matched,
    }


def yaml_set_observation(
    specification: Mapping[str, Any], workspace: Path
) -> tuple[bool, dict[str, Any]]:
    files = sorted(workspace.glob(str(specification["pattern"])))
    digests = [
        canonical_yaml_sha256(path.read_text(encoding="utf-8"), normalize_dynamic=True)
        for path in files
        if path.is_file()
    ]
    expected = specification.get("sha256", [])
    matched = len(files) == specification.get("count") and digests == expected
    return matched, {
        "kind": "yaml-set-semantic", "pattern": str(specification["pattern"]),
        "count": len(files), "sha256": digests, "matched": matched,
    }


def root_publication_observation(
    specification: Mapping[str, Any], workspace: Path
) -> tuple[bool, dict[str, Any]]:
    repository = workspace / str(specification.get("repository", "."))
    branch = str(_git(repository, ["symbolic-ref", "HEAD"])).strip()
    head = str(_git(repository, ["rev-parse", "HEAD"])).strip()
    payload = str(_git(repository, ["cat-file", "commit", head]))
    headers, _, message = payload.partition("\n\n")
    parents = [line.split(" ", 1)[1] for line in headers.splitlines() if line.startswith("parent ")]
    semantic = git_object_semantic(repository, head)
    matched = (
        branch == specification.get("branch")
        and parents == [specification.get("parent")]
        and message == specification.get("message")
        and (specification.get("sha256") is None or semantic == specification.get("sha256"))
    )
    return matched, {
        "kind": "root-publication", "branch": branch, "parents": parents,
        "message_sha256": _sha256(message), "sha256": semantic, "matched": matched,
    }


def index_observation(
    specification: Mapping[str, Any], workspace: Path
) -> tuple[bool, dict[str, Any]]:
    repository = workspace / str(specification.get("repository", "."))
    digest = _index_semantic(repository)
    matched = digest == specification.get("sha256")
    return matched, {"kind": "git-index-semantic", "sha256": digest, "matched": matched}

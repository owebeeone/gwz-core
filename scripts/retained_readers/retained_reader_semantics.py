"""Portable, content-sensitive observations for retained-reader mutations."""

from __future__ import annotations

import base64
import hashlib
import json
import os
import re
import subprocess
from pathlib import Path
from typing import Any, Mapping

from retained_reader_process import is_regular_file, read_regular_text, regular_tree_inventory
from retained_reader_yaml import (
    UUID_RE,
    YamlSubsetError,
    canonical_yaml_sha256,
    parse_yaml_subset,
    yaml_lookup,
)


OBJECT_RE = re.compile(r"^(?P<prefix>(?:.+/)?)\.git/objects/(?P<fan>[0-9a-f]{2})/(?P<rest>[0-9a-f]{38})$")
OBJECT_STORAGE_RE = re.compile(
    r"objects/(?:[0-9a-f]{2}/[0-9a-f]{38}|info/(?:packs|commit-graph|commit-graphs/(?:commit-graph-chain|graph-[0-9a-f]{40}\.graph))|pack/(?:pack-[0-9a-f]{40}\.(?:pack|idx|rev|bitmap|mtimes)|multi-pack-index(?:-[0-9a-f]{40}\.(?:bitmap|rev))?))"
)
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


def _canonical_text(path: Path) -> str:
    return "\n".join(path.read_text(encoding="utf-8").splitlines())


def _index_rows(staged: bytes) -> list[dict[str, Any]]:
    rows = []
    for row in staged.rstrip(b"\0").split(b"\0") if staged else []:
        metadata, path = row.split(b"\t", 1)
        mode, oid, stage = metadata.decode("ascii").split()
        rows.append(
            {
                "mode": mode,
                "oid": oid,
                "stage": int(stage),
                "path_b64": base64.b64encode(path).decode("ascii"),
            }
        )
    return rows


def _git_storage_files(git_dir: Path) -> tuple[set[str], set[str]]:
    try:
        files, directories = regular_tree_inventory(git_dir)
    except (OSError, ValueError) as error:
        raise SemanticError(f"invalid Git administration storage: {error}") from error
    missing = {"HEAD", "config", "index", "info/exclude"} - files
    if missing:
        raise SemanticError(f"missing Git administration files: {sorted(missing)}")
    fixed = {"HEAD", "ORIG_HEAD", "COMMIT_EDITMSG", "config", "description", "gc.log", "index", "info/exclude", "info/refs", "logs/HEAD", "packed-refs"}
    for path in files:
        allowed = path in fixed or path.startswith("refs/") or path.startswith("logs/refs/")
        allowed |= bool(re.fullmatch(r"hooks/[^/]+\.sample", path) or OBJECT_STORAGE_RE.fullmatch(path))
        if not allowed or path.startswith("refs/replace/"):
            raise SemanticError(f"unclassified Git administration file: {path}")
    fixed_dirs = {"branches", "hooks", "info", "logs", "logs/refs", "objects", "objects/info", "objects/info/commit-graphs", "objects/pack", "refs", "refs/heads", "refs/remotes", "refs/tags"}
    for path in directories:
        allowed = path in fixed_dirs or bool(re.fullmatch(r"objects/[0-9a-f]{2}", path))
        allowed |= path.startswith("refs/") or path.startswith("logs/refs/")
        if not allowed:
            raise SemanticError(f"unclassified Git administration directory: {path}")
    return files, directories


def repository_identity(repository: Path) -> dict[str, Any]:
    """Return storage-independent, fail-closed identity for a generated Git repo."""

    git_dir = repository / ".git"
    storage_files, storage_directories = _git_storage_files(git_dir)

    _git(repository, ["fsck", "--strict", "--no-reflogs", "--unreachable", "--no-progress"])
    object_format = str(_git(repository, ["rev-parse", "--show-object-format"])).strip()
    repo_format = str(_git(repository, ["config", "--local", "--get", "core.repositoryformatversion"])).strip()
    if object_format != "sha1" or repo_format != "0":
        raise SemanticError(
            f"unsupported generated repository format: object={object_format}, repository={repo_format}"
        )

    head_value = _canonical_text(git_dir / "HEAD")
    head_target = head_value[5:] if head_value.startswith("ref: ") else None
    try:
        head_oid = str(_git(repository, ["rev-parse", "--verify", "HEAD"])).strip()
    except SemanticError:
        if head_target is None:
            raise
        head_oid = None
    ref_text = str(
        _git(
            repository,
            [
                "for-each-ref",
                "--sort=refname",
                "--format=%(refname)%00%(objectname)%00%(objecttype)%00%(symref)",
            ],
        )
    )
    refs = [line.split("\0") for line in ref_text.splitlines() if line]
    ref_names = {row[0] for row in refs}
    unexpected_refs = {path for path in storage_files if path.startswith("refs/")} - ref_names
    allowed_logs = {"logs/HEAD"} | {f"logs/{name}" for name in ref_names}
    unexpected_logs = {path for path in storage_files if path.startswith("logs/")} - allowed_logs
    ref_dirs = {"refs", "refs/heads", "refs/remotes", "refs/tags"} | {"/".join(name.split("/")[:stop]) for name in ref_names for stop in range(1, len(name.split("/")))}
    log_dirs = {"logs", "logs/refs"} | {"/".join(f"logs/{name}".split("/")[:stop]) for name in ref_names for stop in range(1, len(f"logs/{name}".split("/")))}
    unexpected_dirs = {path for path in storage_directories if path == "refs" or path.startswith(("refs/", "logs/"))} - ref_dirs - log_dirs
    unexpected = unexpected_refs | unexpected_logs | unexpected_dirs
    if unexpected:
        raise SemanticError(f"unclassified Git ref/log storage: {sorted(unexpected)}")

    staged = _git(repository, ["ls-files", "--stage", "-z"], binary=True)
    assert isinstance(staged, bytes)
    index = _index_rows(staged)
    tags = _git(repository, ["ls-files", "-v", "-z"], binary=True)
    fsmonitor = _git(repository, ["ls-files", "-f", "-z"], binary=True)
    resolve_undo = _git(repository, ["ls-files", "--resolve-undo", "-z"], binary=True)
    ita_visible = _git(repository, ["diff", "--cached", "--name-only", "-z", "--ita-visible-in-index"], binary=True)
    ita_invisible = _git(repository, ["diff", "--cached", "--name-only", "-z", "--ita-invisible-in-index"], binary=True)
    assert all(isinstance(value, bytes) for value in (tags, fsmonitor, resolve_undo, ita_visible, ita_invisible))
    if any(row[:1] == b"S" or row[:1].islower() for row in tags.rstrip(b"\0").split(b"\0") if row):
        raise SemanticError("generated fixture index has skip-worktree or assume-unchanged state")
    if any(row[:1].islower() for row in fsmonitor.rstrip(b"\0").split(b"\0") if row):
        raise SemanticError("generated fixture index has fsmonitor-valid state")
    if resolve_undo:
        raise SemanticError("generated fixture index has resolve-undo state")
    if ita_visible != ita_invisible:
        raise SemanticError("generated fixture index has intent-to-add state")

    object_text = str(
        _git(
            repository,
            [
                "cat-file",
                "--batch-all-objects",
                "--batch-check=%(objectname) %(objecttype) %(objectsize)",
            ],
        )
    )
    return {
        "object_format": object_format,
        "repository_format": repo_format,
        "head": {"symbolic": head_target, "oid": head_oid},
        "refs": refs,
        "pseudorefs": {
            name: _canonical_text(git_dir / name)
            for name in sorted(storage_files & {"ORIG_HEAD"})
        },
        "index": index,
        "objects": sorted(line.split() for line in object_text.splitlines() if line),
        "config": _canonical_text(git_dir / "config"),
        "exclude": _canonical_text(git_dir / "info/exclude"),
    }


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
                    is_regular_file(actual_marker)
                    and actual_marker.read_text(encoding="utf-8") == marker_text
                )
            candidate["marker_yaml"] = marker
            candidate["marker_sha256"] = marker_digest
        if isinstance(candidate, dict) and isinstance(candidate.get("lock_yaml"), str):
            lock_text = candidate["lock_yaml"]
            checks["lock_sha256_matches_yaml"] = publication.get("candidate_lock_sha256") == _sha256(lock_text)
            lock_path = workspace / "gwz.conf/gwz.lock.yml"
            checks["lock_file_matches_yaml"] = (
                is_regular_file(lock_path) and lock_path.read_text(encoding="utf-8") == lock_text
            )
        if isinstance(candidate, dict):
            baseline_boundary = candidate.get("baseline_boundary_text")
            boundary = candidate.get("boundary_text")
            if isinstance(baseline_boundary, str):
                checks["baseline_boundary_sha256_matches_text"] = (
                    candidate.get("baseline_boundary_sha256") == _sha256(baseline_boundary)
                )
            if isinstance(boundary, str):
                checks["boundary_sha256_matches_text"] = (
                    candidate.get("boundary_sha256") == _sha256(boundary)
                )
                boundary_path = workspace / ".git/info/exclude"
                checks["boundary_file_matches_text"] = (
                    is_regular_file(boundary_path) and boundary_path.read_bytes() == boundary.encode()
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
                    is_regular_file(actual) and row.get("sha256") == _sha256(actual.read_bytes())
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
        if entry.get("kind") != "file": raise SemanticError(f"Git object is not regular: {plain}")
        repository = workspace / match.group("prefix").removesuffix("/")
        return git_object_semantic(repository, match.group("fan") + match.group("rest"))
    path = workspace / plain
    if plain.endswith("/.git/index") or plain == ".git/index":
        if entry.get("kind") != "file": raise SemanticError(f"Git index is not regular: {plain}")
        repository, _ = _repository_for(workspace, plain)
        return _index_semantic(repository)
    if "/.git/refs/" in plain or plain.startswith(".git/refs/"):
        repository, git_path = _repository_for(workspace, plain)
        oid = read_regular_text(path, encoding="ascii").strip()
        return _canonical({"ref": git_path.removeprefix(".git/"), "target": git_object_semantic(repository, oid)})
    if "/.git/logs/" in plain or plain.startswith(".git/logs/"):
        repository, _ = _repository_for(workspace, plain)
        cache: dict[str, str] = {}
        text = read_regular_text(path, encoding="utf-8", errors="surrogateescape")
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
            return merge_record_semantic(workspace, read_regular_text(path))
        return canonical_yaml_sha256(read_regular_text(path), normalize_dynamic="/markers/" in f"/{plain}")
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
    text = read_regular_text(path)
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
        canonical_yaml_sha256(read_regular_text(path), normalize_dynamic=True)
        for path in files
        if is_regular_file(path)
    ]
    expected = specification.get("sha256", [])
    matched = all(is_regular_file(path) for path in files) and len(files) == specification.get("count") and digests == expected
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

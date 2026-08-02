#!/usr/bin/env python3
"""Offline-first retained GWZ reader inventory and invocation primitives.

Artifact acquisition is the only operation allowed to use the network, and it
does so only when explicitly requested. Reader execution consumes a verified
content-addressed cache entry and always has a bounded timeout.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any, Callable, Iterable, Mapping, Sequence

from retained_reader_schema import SchemaValidationError, load_schema, validate as validate_schema
from retained_reader_errors import HarnessError
from retained_reader_process import render_command, run_command


SCHEMA = "gwz.retained-readers/v1"
SURFACES = {"rust-cli", "gwz-py"}
LANES = {"behavioral", "artifact-smoke"}
STATUSES = {"verified", "pending-acquisition"}
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
RELEASE_RE = re.compile(r"^v\d+\.\d+\.\d+$")
SCHEMA_PATH = Path(__file__).with_name("manifest.schema.json")
FROZEN_R0_READERS = {
    "rust-cli-v0.9.2",
    "gwz-py-v0.9.2",
    "rust-cli-v0.10.2",
    "gwz-py-v0.10.2",
}
FROZEN_R0_PLATFORMS = {
    "linux-x86_64",
    "windows-x86_64",
    "linux-aarch64",
    "macos-x86_64",
    "macos-aarch64",
    "windows-aarch64",
}


class ManifestError(ValueError):
    """The retained-reader inventory violates its checked schema."""


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise ManifestError(message)


def _object(value: Any, path: str) -> dict[str, Any]:
    _require(isinstance(value, dict), f"{path} must be an object")
    return value


def _list(value: Any, path: str) -> list[Any]:
    _require(isinstance(value, list), f"{path} must be an array")
    return value


def _text(value: Any, path: str) -> str:
    _require(isinstance(value, str) and bool(value), f"{path} must be non-empty text")
    return value


def _unique_ids(items: list[Any], path: str) -> set[str]:
    result: set[str] = set()
    for index, raw in enumerate(items):
        item = _object(raw, f"{path}[{index}]")
        item_id = _text(item.get("id"), f"{path}[{index}].id")
        _require(item_id not in result, f"duplicate {path} id {item_id!r}")
        result.add(item_id)
    return result


def load_manifest(path: Path | str) -> dict[str, Any]:
    try:
        value = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ManifestError(f"cannot load retained-reader manifest {path}: {error}") from error
    return _object(value, "manifest")


def validate_manifest(manifest: Mapping[str, Any]) -> None:
    """Validate the checked R0 contract without requiring a third-party package."""

    _validate_manifest(manifest, enforce_frozen_contract=True)


def _validate_manifest_shape(manifest: Mapping[str, Any]) -> None:
    """Validate a synthetic manifest used by focused unit tests."""

    _validate_manifest(manifest, enforce_frozen_contract=False)


def _validate_manifest(
    manifest: Mapping[str, Any], *, enforce_frozen_contract: bool
) -> None:

    try:
        validate_schema(manifest, load_schema(SCHEMA_PATH))
    except SchemaValidationError as error:
        raise ManifestError(str(error)) from error

    _require(manifest.get("schema") == SCHEMA, f"schema must be {SCHEMA!r}")
    timeout = manifest.get("default_timeout_seconds")
    _require(
        isinstance(timeout, int) and not isinstance(timeout, bool) and 1 <= timeout <= 300,
        "default_timeout_seconds must be an integer from 1 through 300",
    )

    generations = _list(manifest.get("decode_generations"), "decode_generations")
    generation_ids = _unique_ids(generations, "decode_generations")
    generation_releases: dict[str, str] = {}
    for index, raw in enumerate(generations):
        item = _object(raw, f"decode_generations[{index}]")
        release = _text(item.get("release"), f"decode_generations[{index}].release")
        _require(RELEASE_RE.fullmatch(release) is not None, f"invalid release {release!r}")
        _text(item.get("description"), f"decode_generations[{index}].description")
        generation_releases[item["id"]] = release

    platforms = _list(manifest.get("platforms"), "platforms")
    platform_ids = _unique_ids(platforms, "platforms")
    for index, raw in enumerate(platforms):
        item = _object(raw, f"platforms[{index}]")
        _text(item.get("os"), f"platforms[{index}].os")
        _text(item.get("arch"), f"platforms[{index}].arch")
        _require(item.get("lane") in LANES, f"platforms[{index}].lane must name a known lane")

    runtimes = _list(manifest.get("runtimes"), "runtimes")
    runtime_ids = _unique_ids(runtimes, "runtimes")
    for index, raw in enumerate(runtimes):
        item = _object(raw, f"runtimes[{index}]")
        _require(item.get("kind") in {"native", "python"}, f"runtimes[{index}].kind is invalid")
        bootstrap = _list(item.get("bootstrap"), f"runtimes[{index}].bootstrap")
        for command_index, command in enumerate(bootstrap):
            _command(command, f"runtimes[{index}].bootstrap[{command_index}]")
        if item["kind"] == "python":
            _text(item.get("python_version"), f"runtimes[{index}].python_version")
            _text(item.get("abi"), f"runtimes[{index}].abi")
        runtime_artifacts = _list(item.get("artifacts", []), f"runtimes[{index}].artifacts")
        _unique_ids(runtime_artifacts, f"runtimes[{index}].artifacts")
        for artifact_index, raw_artifact in enumerate(runtime_artifacts):
            path = f"runtimes[{index}].artifacts[{artifact_index}]"
            artifact = _object(raw_artifact, path)
            _require(artifact.get("status") == "verified", f"{path}.status must be verified")
            name = _text(artifact.get("name"), f"{path}.name")
            url = _text(artifact.get("url"), f"{path}.url")
            digest = _text(artifact.get("sha256"), f"{path}.sha256")
            _require(SHA256_RE.fullmatch(digest) is not None, f"{path}.sha256 must be 64 lowercase hex characters")
            _require(_immutable_url(url, "", name), f"{path}.url must be immutable and content-addressed")

    readers = _list(manifest.get("readers"), "readers")
    reader_ids = _unique_ids(readers, "readers")
    seen_tuples: set[str] = set()
    for reader_index, raw_reader in enumerate(readers):
        reader_path = f"readers[{reader_index}]"
        reader = _object(raw_reader, reader_path)
        surface = reader.get("surface")
        _require(surface in SURFACES, f"{reader_path}.surface must name a distributed reader")
        release = _text(reader.get("release"), f"{reader_path}.release")
        _require(RELEASE_RE.fullmatch(release) is not None, f"{reader_path}.release is invalid")
        generation = _text(reader.get("decode_generation"), f"{reader_path}.decode_generation")
        _require(generation in generation_ids, f"{reader_path} names unknown decode generation")
        _require(
            generation_releases[generation] == release,
            f"{reader_path} release differs from its decode generation",
        )
        runtime = _text(reader.get("runtime"), f"{reader_path}.runtime")
        _require(runtime in runtime_ids, f"{reader_path} names unknown runtime")

        versions = _list(reader.get("supported_record_versions"), f"{reader_path}.supported_record_versions")
        _require(
            all(isinstance(v, int) and not isinstance(v, bool) and v >= 0 for v in versions),
            f"{reader_path}.supported_record_versions may contain only non-negative integers",
        )
        envelope = _object(reader.get("envelope_behavior"), f"{reader_path}.envelope_behavior")
        _require(bool(envelope), f"{reader_path}.envelope_behavior must not be empty")
        for key, value in envelope.items():
            _text(key, f"{reader_path}.envelope_behavior key")
            _text(value, f"{reader_path}.envelope_behavior[{key!r}]")

        commands = _object(reader.get("commands"), f"{reader_path}.commands")
        _require(bool(commands), f"{reader_path}.commands must not be empty")
        for command, availability in commands.items():
            _text(command, f"{reader_path}.commands key")
            _require(
                availability in {"available", "unavailable"},
                f"{reader_path}.commands[{command!r}] must be available or unavailable",
            )
        unavailable = {command for command, value in commands.items() if value == "unavailable"}
        absence = reader.get("command_absence_evidence", {})
        absence = _object(absence, f"{reader_path}.command_absence_evidence")
        _require(
            set(absence) == unavailable,
            f"{reader_path}.command_absence_evidence must cover exactly every unavailable command",
        )
        for command, evidence in absence.items():
            _text(evidence, f"{reader_path}.command_absence_evidence[{command!r}]")
        projections = _list(reader.get("projections"), f"{reader_path}.projections")
        _require(bool(projections), f"{reader_path}.projections must not be empty")
        for index, projection in enumerate(projections):
            _text(projection, f"{reader_path}.projections[{index}]")
        _command(reader.get("invocation"), f"{reader_path}.invocation")

        if surface == "gwz-py":
            python = _object(reader.get("python"), f"{reader_path}.python")
            _text(python.get("version"), f"{reader_path}.python.version")
            _text(python.get("abi"), f"{reader_path}.python.abi")
            _text(python.get("native_extension"), f"{reader_path}.python.native_extension")

        artifacts = _list(reader.get("artifacts"), f"{reader_path}.artifacts")
        _require(bool(artifacts), f"{reader_path}.artifacts must not be empty")
        declared_platforms = [artifact.get("platform") for artifact in artifacts]
        missing_platforms = sorted(platform_ids - set(declared_platforms))
        extra_platforms = sorted(set(declared_platforms) - platform_ids)
        _require(
            not missing_platforms and not extra_platforms and len(declared_platforms) == len(platform_ids),
            f"{reader_path} must declare exactly one artifact for every platform; "
            f"missing={missing_platforms}, extra={extra_platforms}",
        )
        for artifact_index, raw_artifact in enumerate(artifacts):
            artifact_path = f"{reader_path}.artifacts[{artifact_index}]"
            artifact = _object(raw_artifact, artifact_path)
            platform = _text(artifact.get("platform"), f"{artifact_path}.platform")
            _require(platform in platform_ids, f"{artifact_path} names unknown platform")
            tuple_id = f"{surface}:{release}:{platform}"
            _require(tuple_id not in seen_tuples, f"duplicate retained-reader tuple {tuple_id}")
            seen_tuples.add(tuple_id)
            _validate_artifact(artifact, artifact_path, release, surface)

    if enforce_frozen_contract:
        _require(reader_ids == FROZEN_R0_READERS, "frozen reader set differs from the reviewed R0 contract")
        _require(platform_ids == FROZEN_R0_PLATFORMS, "frozen platform set differs from the reviewed R0 contract")
        required = FROZEN_R0_PLATFORMS - {"windows-aarch64"}
        for reader in readers:
            expected_required = required
            actual_required = {
                artifact["platform"]
                for artifact in reader["artifacts"]
                if artifact["support"] == "required"
            }
            _require(
                actual_required == expected_required,
                f"{reader['id']} frozen support classification differs: "
                f"required={sorted(actual_required)}, expected={sorted(expected_required)}",
            )


def _command(value: Any, path: str) -> None:
    command = _list(value, path)
    _require(bool(command), f"{path} must not be empty")
    for index, argument in enumerate(command):
        _text(argument, f"{path}[{index}]")


def _validate_artifact(
    artifact: Mapping[str, Any], path: str, release: str, surface: str
) -> None:
    support = artifact.get("support")
    _require(support in {"required", "unsupported"}, f"{path}.support is invalid")
    if support == "unsupported":
        _text(artifact.get("reason"), f"{path}.reason")
        substitute = _list(artifact.get("substitute_evidence"), f"{path}.substitute_evidence")
        _require(bool(substitute), f"{path}.substitute_evidence must not be empty")
        for index, item in enumerate(substitute):
            _text(item, f"{path}.substitute_evidence[{index}]")
        return

    status = artifact.get("status")
    _require(status in STATUSES, f"{path}.status is invalid")
    _require("name" in artifact, f"{path}.name must be explicit")
    _require("url" in artifact, f"{path}.url must be explicit")
    _require("sha256" in artifact, f"{path}.sha256 must be explicit")
    _text(artifact.get("format"), f"{path}.format")
    _text(artifact.get("entry_point"), f"{path}.entry_point")
    if surface == "gwz-py":
        _require("wheel_tag" in artifact, f"{path}.wheel_tag must be explicit")
        if artifact["wheel_tag"] is not None:
            _text(artifact["wheel_tag"], f"{path}.wheel_tag")
    if status == "pending-acquisition":
        _text(artifact.get("acquisition_note"), f"{path}.acquisition_note")
        for key in ("name", "url", "sha256"):
            if artifact[key] is not None:
                _text(artifact[key], f"{path}.{key}")
        if artifact["url"] is not None:
            _require(artifact["name"] is not None, f"{path}.name is required when url is known")
            _require(
                _immutable_url(artifact["url"], release, artifact["name"]),
                f"{path}.url must be an immutable tagged or content-addressed URL",
            )
        if artifact["sha256"] is not None:
            _require(
                SHA256_RE.fullmatch(artifact["sha256"]) is not None,
                f"{path}.sha256 must be 64 lowercase hex characters",
            )
        return

    name = _text(artifact["name"], f"{path}.name")
    url = _text(artifact["url"], f"{path}.url")
    digest = _text(artifact["sha256"], f"{path}.sha256")
    _require(SHA256_RE.fullmatch(digest) is not None, f"{path}.sha256 must be 64 lowercase hex characters")
    _require(_immutable_url(url, release, name), f"{path}.url must be an immutable tagged or content-addressed URL")


def _immutable_url(url: str, release: str, name: str) -> bool:
    parsed = urllib.parse.urlparse(url)
    if parsed.scheme != "https" or parsed.query or parsed.fragment or "/latest/" in parsed.path:
        return False
    if parsed.netloc == "github.com":
        return f"/releases/download/{release}/" in parsed.path and parsed.path.endswith(f"/{name}")
    if parsed.netloc == "files.pythonhosted.org":
        return parsed.path.endswith(f"/{name}") and "/packages/" in parsed.path
    return False


def iter_tuple_ids(manifest: Mapping[str, Any]) -> Iterable[str]:
    for reader in manifest.get("readers", []):
        for artifact in reader.get("artifacts", []):
            yield f"{reader['surface']}:{reader['release']}:{artifact['platform']}"


def iter_acquirable_artifacts(manifest: Mapping[str, Any]) -> Iterable[Mapping[str, Any]]:
    seen: set[str] = set()
    for runtime in manifest.get("runtimes", []):
        for artifact in runtime.get("artifacts", []):
            digest = artifact["sha256"]
            if digest not in seen:
                seen.add(digest)
                yield artifact
    for reader in manifest.get("readers", []):
        for artifact in reader.get("artifacts", []):
            if artifact["support"] != "required" or artifact["sha256"] in seen:
                continue
            seen.add(artifact["sha256"])
            yield artifact


def gate_readiness_errors(manifest: Mapping[str, Any]) -> list[str]:
    validate_manifest(manifest)
    errors: list[str] = []
    for reader in manifest["readers"]:
        for artifact in reader["artifacts"]:
            if artifact["support"] == "required" and artifact["status"] != "verified":
                tuple_id = f"{reader['surface']}:{reader['release']}:{artifact['platform']}"
                errors.append(f"{tuple_id}: required artifact is {artifact['status']}")
            if reader["surface"] == "gwz-py" and artifact["support"] == "required" and artifact["wheel_tag"] is None:
                tuple_id = f"{reader['surface']}:{reader['release']}:{artifact['platform']}"
                errors.append(f"{tuple_id}: exact wheel_tag is pending acquisition")
    return errors


def cache_path(cache_root: Path, digest: str) -> Path:
    if SHA256_RE.fullmatch(digest) is None:
        raise HarnessError("cache key must be a 64-character lowercase SHA-256 digest")
    return cache_root / "objects" / "sha256" / digest


def _digest(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            hasher.update(block)
    return hasher.hexdigest()


def acquire_artifact(
    artifact: Mapping[str, Any],
    cache_root: Path,
    *,
    offline: bool,
    timeout_seconds: float = 30,
    opener: Callable[..., Any] = urllib.request.urlopen,
) -> Path:
    """Return a verified cache object; network use requires ``offline=False``."""

    if artifact.get("status") != "verified":
        raise HarnessError("required artifact is not checksum-verified in the manifest")
    digest = str(artifact["sha256"])
    target = cache_path(cache_root, digest)
    if target.is_file():
        if _digest(target) != digest:
            raise HarnessError(f"cached artifact digest mismatch: {target}")
        return target
    if offline:
        raise HarnessError(f"offline cache miss for sha256:{digest}")

    target.parent.mkdir(parents=True, exist_ok=True)
    temporary: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(dir=target.parent, delete=False) as sink:
            temporary = Path(sink.name)
            with opener(str(artifact["url"]), timeout=timeout_seconds) as response:
                while True:
                    block = response.read(1024 * 1024)
                    if not block:
                        break
                    sink.write(block)
            sink.flush()
            os.fsync(sink.fileno())
        actual = _digest(temporary)
        if actual != digest:
            raise HarnessError(f"downloaded artifact digest mismatch: expected {digest}, got {actual}")
        os.replace(temporary, target)
        temporary = None
        return target
    except (OSError, urllib.error.URLError) as error:
        raise HarnessError(f"artifact acquisition failed: {error}") from error
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    validate = subparsers.add_parser("validate", help="validate inventory schema")
    validate.add_argument("manifest", type=Path)
    ready = subparsers.add_parser(
        "gate-ready", help="fail unless every required tuple has immutable artifact metadata"
    )
    ready.add_argument("manifest", type=Path)
    acquire = subparsers.add_parser("acquire", help="populate/verify the content-addressed cache")
    acquire.add_argument("manifest", type=Path)
    acquire.add_argument("--cache", type=Path, required=True)
    acquire.add_argument("--allow-network", action="store_true")
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    try:
        manifest = load_manifest(args.manifest)
        validate_manifest(manifest)
        tuple_count = sum(1 for _ in iter_tuple_ids(manifest))
        if args.command == "validate":
            print(json.dumps({"status": "valid", "tuple_count": tuple_count}, sort_keys=True))
            return 0
        errors = gate_readiness_errors(manifest)
        if errors:
            raise HarnessError("retained-reader gate is not ready:\n" + "\n".join(errors))
        if args.command == "gate-ready":
            print(json.dumps({"status": "manifest-ready", "tuple_count": tuple_count}, sort_keys=True))
            return 0
        acquired = []
        for artifact in iter_acquirable_artifacts(manifest):
            path = acquire_artifact(
                artifact,
                args.cache,
                offline=not args.allow_network,
                timeout_seconds=manifest["default_timeout_seconds"],
            )
            acquired.append(str(path))
        print(json.dumps({"status": "cached", "artifacts": acquired}, sort_keys=True))
        return 0
    except (ManifestError, HarnessError) as error:
        print(f"retained-reader harness: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())

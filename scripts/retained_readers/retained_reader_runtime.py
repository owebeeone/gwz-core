"""Verified archive extraction and ephemeral retained-reader runtimes."""

from __future__ import annotations

import hashlib
import json
import os
import re
import shutil
import stat
import tarfile
import zipfile
from pathlib import Path, PurePosixPath
from typing import Any, Mapping

import retained_reader_harness as harness
from retained_reader_errors import MatrixError


DRIVE_RE = re.compile(r"^[A-Za-z]:")
WINDOWS_RESERVED = {
    "con", "prn", "aux", "nul",
    *(f"com{i}" for i in range(1, 10)),
    *(f"lpt{i}" for i in range(1, 10)),
}


def _safe_archive_path(name: str) -> PurePosixPath:
    if not name or "\\" in name or DRIVE_RE.match(name):
        raise MatrixError(f"unsafe archive path: {name!r}")
    path = PurePosixPath(name)
    if path.is_absolute() or any(part in {"", ".", ".."} for part in path.parts):
        raise MatrixError(f"unsafe archive path: {name!r}")
    return path


def _windows_archive_key(path: PurePosixPath) -> str:
    for part in path.parts:
        stem = part.split(".", 1)[0].casefold()
        if part.endswith((" ", ".")) or ":" in part or stem in WINDOWS_RESERVED:
            raise MatrixError(f"unsafe Windows archive path: {path.as_posix()!r}")
    return "/".join(part.casefold() for part in path.parts)


def extract_archive(archive: Path, archive_format: str, destination: Path) -> None:
    """Extract regular files/directories atomically without following links."""

    if destination.exists():
        raise MatrixError(f"archive destination already exists: {destination}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = Path(__import__("tempfile").mkdtemp(prefix=f".{destination.name}-", dir=destination.parent))
    seen: set[str] = set()
    try:
        if archive_format == "zip":
            with zipfile.ZipFile(archive) as source:
                for member in source.infolist():
                    relative = _safe_archive_path(member.filename.rstrip("/"))
                    key = _windows_archive_key(relative)
                    if key in seen:
                        raise MatrixError(f"archive path collision: {member.filename!r}")
                    seen.add(key)
                    mode = member.external_attr >> 16
                    if stat.S_ISLNK(mode):
                        raise MatrixError(f"unsupported archive member: {member.filename!r}")
                    target = temporary.joinpath(*relative.parts)
                    if member.is_dir():
                        target.mkdir(parents=True, exist_ok=True)
                        continue
                    target.parent.mkdir(parents=True, exist_ok=True)
                    with source.open(member) as input_file, target.open("wb") as output_file:
                        shutil.copyfileobj(input_file, output_file)
                    if os.name != "nt" and mode:
                        target.chmod(stat.S_IMODE(mode))
        elif archive_format == "tar.xz":
            with tarfile.open(archive, "r:xz") as source:
                for member in source:
                    relative = _safe_archive_path(member.name.rstrip("/"))
                    key = _windows_archive_key(relative)
                    if key in seen:
                        raise MatrixError(f"archive path collision: {member.name!r}")
                    seen.add(key)
                    if not (member.isdir() or member.isfile()):
                        raise MatrixError(f"unsupported archive member: {member.name!r}")
                    target = temporary.joinpath(*relative.parts)
                    if member.isdir():
                        target.mkdir(parents=True, exist_ok=True)
                        continue
                    extracted = source.extractfile(member)
                    if extracted is None:
                        raise MatrixError(f"cannot read archive member: {member.name!r}")
                    target.parent.mkdir(parents=True, exist_ok=True)
                    with extracted, target.open("wb") as output_file:
                        shutil.copyfileobj(extracted, output_file)
                    if os.name != "nt":
                        target.chmod(stat.S_IMODE(member.mode))
        else:
            raise MatrixError(f"unsupported archive format: {archive_format!r}")
        os.replace(temporary, destination)
        temporary = Path()
    except (OSError, tarfile.TarError, zipfile.BadZipFile) as error:
        raise MatrixError(f"archive extraction failed: {error}") from error
    finally:
        if temporary != Path() and temporary.exists():
            shutil.rmtree(temporary)


def _runtime_identity(
    runtime: Mapping[str, Any], artifact: Mapping[str, Any], interpreter: Mapping[str, Any]
) -> str:
    payload = {"runtime": runtime, "artifact_sha256": artifact["sha256"], "interpreter": interpreter}
    return hashlib.sha256(json.dumps(payload, sort_keys=True).encode()).hexdigest()


def interpreter_identity(executable: Path) -> dict[str, Any]:
    code = "import json,platform,struct;print(json.dumps({'implementation':platform.python_implementation(),'version':platform.python_version(),'architecture':platform.machine(),'pointer_bits':struct.calcsize('P')*8}))"
    completed = harness.run_command([str(executable), "-c", code], timeout_seconds=15)
    if completed.returncode:
        raise MatrixError(f"cannot identify Python interpreter: {completed.stderr.strip()}")
    try:
        result = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise MatrixError("Python interpreter returned invalid identity") from error
    result["executable_sha256"] = hashlib.sha256(executable.resolve().read_bytes()).hexdigest()
    return result


def _named_install_input(item: Mapping[str, Any], source: Path, destination_root: Path) -> Path:
    name = str(item.get("name", ""))
    if not name or Path(name).name != name:
        raise MatrixError(f"install artifact has unsafe name {name!r}")
    actual = hashlib.sha256(source.read_bytes()).hexdigest()
    if actual != item["sha256"]:
        raise MatrixError(f"install artifact checksum mismatch: {name}")
    destination = destination_root / str(item["sha256"]) / name
    if destination.exists():
        if hashlib.sha256(destination.read_bytes()).hexdigest() != item["sha256"]:
            raise MatrixError(f"named install input checksum mismatch: {destination}")
        return destination
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, destination)
    return destination


def bootstrap_python_runtime(
    runtime: Mapping[str, Any], artifact: Mapping[str, Any], artifact_path: Path,
    destination: Path, *, python_executable: Path, timeout_seconds: float,
    runtime_artifacts: Mapping[str, Path] | None = None,
) -> Path:
    artifact_path = _named_install_input(artifact, artifact_path, destination.parent / ".install-inputs")
    interpreter = interpreter_identity(python_executable)
    minimum = tuple(int(part) for part in str(runtime.get("python_version", "0")).split(".")[:2])
    actual = tuple(int(part) for part in str(interpreter["version"]).split(".")[:2])
    if interpreter["implementation"] != "CPython" or actual < minimum:
        raise MatrixError(f"interpreter {interpreter['implementation']} {interpreter['version']} does not satisfy CPython >= {runtime.get('python_version')}")
    identity = _runtime_identity(runtime, artifact, interpreter)
    if destination.exists():
        raise MatrixError(f"derived Python runtime destination already exists: {destination}")
    runtime_python = destination / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
    variables = {
        "python": str(python_executable), "runtime_dir": str(destination),
        "runtime_python": str(runtime_python),
        "runtime_bin": str(destination / ("Scripts" if os.name == "nt" else "bin")),
        "artifact": str(artifact_path),
    }
    variables.update({key: str(value) for key, value in (runtime_artifacts or {}).items()})
    try:
        for template in runtime.get("bootstrap", []):
            command = harness.render_command(template, variables)
            completed = harness.run_command(command, timeout_seconds=timeout_seconds, env={"PIP_NO_INDEX": "1", "PIP_DISABLE_PIP_VERSION_CHECK": "1"})
            if completed.returncode:
                raise MatrixError(f"Python runtime bootstrap failed ({completed.returncode}): {completed.stderr.strip()}")
        destination.mkdir(parents=True, exist_ok=True)
        (destination / ".gwz-retained-reader.json").write_text(
            json.dumps({"identity": identity, "interpreter": interpreter}, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        return _python_entry_point(destination, str(artifact["entry_point"]))
    except Exception:
        if destination.exists():
            shutil.rmtree(destination)
        raise


def _python_entry_point(runtime: Path, name: str) -> Path:
    entry = runtime / ("Scripts" if os.name == "nt" else "bin") / name
    if not entry.is_file():
        raise MatrixError(f"retained Python entry point is missing: {entry}")
    return entry


def _find_entry_point(tree: Path, name: str) -> Path:
    matches = [path for path in tree.rglob(name) if path.is_file()]
    if len(matches) != 1:
        raise MatrixError(f"archive must contain exactly one {name!r} entry point, found {len(matches)}")
    return matches[0]


def _prepare_reader(
    manifest: Mapping[str, Any], reader: Mapping[str, Any], artifact: Mapping[str, Any],
    artifact_path: Path, cache_root: Path, derived_root: Path,
    python_executable: Path, offline: bool,
) -> Path:
    if reader["surface"] == "gwz-py":
        runtime = next(item for item in manifest["runtimes"] if item["id"] == reader["runtime"])
        companions = {
            item["id"]: _named_install_input(
                item,
                harness.acquire_artifact(item, cache_root, offline=offline, timeout_seconds=manifest["default_timeout_seconds"]),
                derived_root / "install-inputs",
            )
            for item in runtime.get("artifacts", [])
        }
        return bootstrap_python_runtime(
            runtime, artifact, artifact_path, derived_root / "runtimes" / str(reader["id"]),
            python_executable=python_executable,
            timeout_seconds=manifest["default_timeout_seconds"],
            runtime_artifacts=companions,
        )
    tree = derived_root / "trees" / str(reader["id"])
    extract_archive(artifact_path, str(artifact["format"]), tree)
    return _find_entry_point(tree, str(artifact["entry_point"]))

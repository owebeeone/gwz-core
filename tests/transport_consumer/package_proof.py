#!/usr/bin/env python3
"""Build the consumer in isolation against one explicitly supplied crate.

The checked consumer manifest intentionally names the released registry
version. This runner supplies a verified local archive through a temporary
Cargo patch, so it needs neither a sibling checkout nor a registry/network.
"""

from __future__ import annotations

import argparse
import ast
import hashlib
import json
import re
import shutil
import subprocess
import tarfile
import tempfile
from pathlib import Path
from pathlib import PurePosixPath

try:
    import tomllib
except ModuleNotFoundError:  # Python 3.10: use the restricted fallback below.
    tomllib = None

ROOT = Path(__file__).resolve().parent
EXPECTED_NAME = "gwz-transport"
EXPECTED_VERSION = "0.1.0"


def digest(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(block)
    return hasher.hexdigest()


def _package_table(text: str) -> dict[str, object]:
    if tomllib is not None:
        package = tomllib.loads(text).get("package")
        return package if isinstance(package, dict) else {}
    # Cargo's name/version are plain TOML strings. This fallback deliberately
    # parses only the [package] table and uses literal_eval for values; it does
    # not execute or interpolate manifest text.
    in_package = False
    package: dict[str, object] = {}
    for raw_line in text.splitlines():
        line = raw_line.partition("#")[0].strip()
        if not line:
            continue
        if line.startswith("[") and line.endswith("]"):
            in_package = line == "[package]"
            continue
        if in_package and "=" in line:
            key, _, value = line.partition("=")
            key = key.strip()
            if key in {"name", "version"}:
                try:
                    package[key] = ast.literal_eval(value.strip())
                except (SyntaxError, ValueError):
                    raise SystemExit(f"invalid [package] value for {key}") from None
    return package


def _archive_members(bundle: tarfile.TarFile) -> tuple[list[tarfile.TarInfo], str]:
    members = bundle.getmembers()
    expected_root = f"{EXPECTED_NAME}-{EXPECTED_VERSION}"
    seen: set[str] = set()
    for member in members:
        name = member.name
        path = PurePosixPath(name)
        raw_parts = name.split("/")
        if (
            not name
            or "\x00" in name
            or name.startswith("/")
            or "\\" in name
            or ":" in name
            or any(part in {"", ".", ".."} for part in raw_parts)
            or name in seen
        ):
            raise SystemExit(f"unsafe or duplicate archive member: {name!r}")
        seen.add(name)
        if not (member.isdir() or member.isreg()):
            raise SystemExit(f"archive contains non-regular member: {name}")
        if path.parts[0] != expected_root:
            raise SystemExit(f"archive member escapes package root: {name}")
    roots = {PurePosixPath(member.name).parts[0] for member in members}
    if roots != {expected_root}:
        raise SystemExit(f"unexpected package root(s): {sorted(roots)}")
    return members, expected_root


def package_identity(archive: Path, expected_revision: str) -> tuple[str, str]:
    with tarfile.open(archive, "r:gz") as bundle:
        members, root = _archive_members(bundle)
        files = {member.name: member for member in members if member.isreg()}
        manifest_member = files.get(f"{root}/Cargo.toml")
        manifest = bundle.extractfile(manifest_member) if manifest_member else None
        if manifest is None:
            raise SystemExit("package archive has no readable Cargo.toml")
        package = _package_table(manifest.read().decode("utf-8"))
        if not package:
            raise SystemExit("package manifest has no [package] table")
        name = package.get("name")
        version = package.get("version")
        if (name, version) != (EXPECTED_NAME, EXPECTED_VERSION):
            raise SystemExit(f"unexpected package identity: {name!r} {version!r}")
        vcs_member = files.get(f"{root}/.cargo_vcs_info.json")
        vcs = bundle.extractfile(vcs_member) if vcs_member else None
        if vcs is None:
            raise SystemExit("package archive has no .cargo_vcs_info.json")
        metadata = json.loads(vcs.read().decode("utf-8"))
        git = metadata.get("git")
        if not isinstance(git, dict) or git.get("sha1") != expected_revision:
            raise SystemExit(
                "archive source revision mismatch: "
                f"expected {expected_revision}, got {git!r}"
            )
        if metadata.get("dirty") or git.get("dirty"):
            raise SystemExit("archive was produced from a dirty source tree")
    return name, version


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", required=True, type=Path)
    parser.add_argument("--archive-sha256", required=True)
    parser.add_argument("--source-revision", required=True)
    args = parser.parse_args()
    if re.fullmatch(r"[0-9a-f]{40}", args.source_revision) is None:
        raise SystemExit("source revision must be a 40-character lowercase Git revision")
    if re.fullmatch(r"[0-9a-f]{64}", args.archive_sha256) is None:
        raise SystemExit("archive SHA-256 must be 64 lowercase hexadecimal characters")
    archive = args.archive.resolve()
    if not archive.is_file():
        raise SystemExit(f"archive does not exist: {archive}")
    actual_digest = digest(archive)
    if actual_digest != args.archive_sha256:
        raise SystemExit(
            f"archive digest mismatch: expected {args.archive_sha256}, got {actual_digest}"
        )
    name, version = package_identity(archive, args.source_revision)

    with tempfile.TemporaryDirectory(prefix="gwz-transport-consumer-package-") as raw:
        isolated = Path(raw)
        package_dir = isolated / f"{name}-{version}"
        consumer_dir = isolated / "core" / "tests" / "transport_consumer"
        with tarfile.open(archive, "r:gz") as bundle:
            members, _ = _archive_members(bundle)
            for member in members:
                target = isolated / Path(*PurePosixPath(member.name).parts)
                if member.isdir():
                    target.mkdir(parents=True, exist_ok=True)
                else:
                    target.parent.mkdir(parents=True, exist_ok=True)
                    source = bundle.extractfile(member)
                    if source is None:
                        raise SystemExit(f"unable to read archive member: {member.name}")
                    with target.open("xb") as destination:
                        shutil.copyfileobj(source, destination)
        shutil.copytree(
            ROOT,
            consumer_dir,
            ignore=shutil.ignore_patterns("target", "__pycache__"),
        )
        # Preserve the core-relative source path used by the preactivation bridge
        # fixture; no source is taken from the transport owner's checkout.
        bridge = ROOT.parents[1] / "src/git/endpoint/stream_io.rs"
        bridge_copy = isolated / "core/src/git/endpoint/stream_io.rs"
        bridge_copy.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(bridge, bridge_copy)
        print(f"core_bridge_sha256={digest(bridge_copy)}")
        cargo_dir = consumer_dir / ".cargo"
        cargo_dir.mkdir()
        (cargo_dir / "config.toml").write_text(
            "[patch.crates-io]\n"
            f'gwz-transport = {{ path = "{package_dir}" }}\n'
        )
        command = [
            "cargo",
            "test",
            "--offline",
            "--locked",
            "--manifest-path",
            str(consumer_dir / "Cargo.toml"),
        ]
        print(f"source_revision={args.source_revision}")
        print(f"package={name} version={version} archive_sha256={actual_digest}")
        print("command=" + " ".join(command))
        subprocess.run(command, cwd=consumer_dir, check=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

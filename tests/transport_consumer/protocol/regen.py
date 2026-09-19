#!/usr/bin/env python3
"""Regenerate the test-only core consumer from the exported owner schema.

This is an explicit developer/CI check. Cargo never runs it, and the normal
consumer build uses only the checked-in generated module plus the pinned
registry dependency. Both the owner schema and taut source are supplied explicitly;
their content/revision pins prevent silent sibling discovery or network fetches.
"""

from __future__ import annotations

import argparse
import ast
import hashlib
import json
import os
import shutil
import subprocess
import tempfile
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # Python 3.10: use the restricted fallback below.
    tomllib = None

ROOT = Path(__file__).resolve().parent.parent
SCHEMA = ROOT / "protocol" / "consumer.taut.py"
PIN = ROOT / "protocol" / "generator.json"
OUTPUT = ROOT / "src" / "generated.rs"


def _package_identity(manifest: Path) -> tuple[str, str]:
    text = manifest.read_text()
    if tomllib is not None:
        package = tomllib.loads(text).get("package")
        if isinstance(package, dict):
            name, version = package.get("name"), package.get("version")
            if isinstance(name, str) and isinstance(version, str):
                return name, version
        raise SystemExit("owner manifest has no valid [package] identity")

    in_package = False
    values: dict[str, object] = {}
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
                    values[key] = ast.literal_eval(value.strip())
                except (SyntaxError, ValueError):
                    raise SystemExit(f"invalid [package] value for {key}") from None
    name, version = values.get("name"), values.get("version")
    if not isinstance(name, str) or not isinstance(version, str):
        raise SystemExit("owner manifest has no valid [package] identity")
    return name, version


def _load_owner(path: Path, pin: dict, schema_from_json):
    manifest = path.parent.parent / "Cargo.toml"
    name, version = _package_identity(manifest)
    if name != pin["owner-name"] or version != pin["owner-version"]:
        raise SystemExit(f"owner manifest identity mismatch: {name!r} {version!r}")
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    expected = pin["owner-schema-sha256"]
    if digest != expected:
        raise SystemExit(
            f"owner schema digest mismatch: expected {expected}, got {digest}; "
            "update the pin only with an intentional schema revision"
        )
    owner = schema_from_json(json.loads(path.read_text()))
    return owner


def _verify_taut_source(path: Path, pin: dict) -> None:
    supplied = path.resolve()
    repo = Path(
        subprocess.run(
            ["git", "-C", str(supplied), "rev-parse", "--show-toplevel"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
    ).resolve()
    expected_source = (repo / "src").resolve()
    if supplied != expected_source:
        raise SystemExit(
            "taut source must be the canonical Git checkout src directory: "
            f"expected {expected_source}, got {supplied}"
        )
    revision = subprocess.run(
        ["git", "-C", str(repo), "rev-parse", "HEAD"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    if revision != pin["taut-source-revision"]:
        raise SystemExit(
            f"taut source revision mismatch: expected {pin['taut-source-revision']}, "
            f"got {revision}"
        )
    dirty = subprocess.run(
        ["git", "-C", str(repo), "status", "--porcelain", "--untracked-files=all", "--", "src"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    if dirty:
        raise SystemExit(f"taut source checkout is dirty under src/: {dirty}")
    for relative, expected in pin["taut-extension-sha256"].items():
        file_path = repo / relative
        digest = hashlib.sha256(file_path.read_bytes()).hexdigest()
        if digest != expected:
            raise SystemExit(
                f"taut extension digest mismatch for {relative}: "
                f"expected {expected}, got {digest}"
            )


def _check_taut_module_origin(name: str, module: object, source: Path) -> None:
    raw_origin = getattr(module, "__file__", None)
    if not isinstance(raw_origin, str):
        raise SystemExit(f"imported {name} has no file origin")
    origin = Path(raw_origin).resolve()
    if source != origin and source not in origin.parents:
        raise SystemExit(
            f"imported {name} is outside canonical taut source: {origin}"
        )


def _generate(owner_path: Path, taut_source: Path) -> str:
    pin = json.loads(PIN.read_text())
    _verify_taut_source(taut_source, pin)
    import sys

    source = taut_source.resolve()
    module_names = sorted(name for name in sys.modules if name == "taut" or name.startswith("taut."))
    for name in module_names:
        module = sys.modules.get(name)
        if module is not None:
            _check_taut_module_origin(name, module, source)
            raise SystemExit(
                "taut modules are already loaded; run regeneration in a fresh interpreter"
            )
    sys.path.insert(0, str(source))
    import taut
    from taut.gen.scaffold import emit
    from taut.gen import rust_external
    from taut.ir.load import load_schema, schema_from_json

    for name, module in (
        ("taut", taut),
        ("taut.gen.scaffold", sys.modules["taut.gen.scaffold"]),
        ("taut.gen.rust_external", rust_external),
    ):
        _check_taut_module_origin(name, module, source)

    if taut.__version__ != pin["taut-proto"]:
        raise SystemExit(
            f"taut-proto pin mismatch: expected {pin['taut-proto']}, "
            f"got {taut.__version__}"
        )
    owner = _load_owner(owner_path, pin, schema_from_json)
    previous = os.environ.get("GWZ_TRANSPORT_SCHEMA")
    os.environ["GWZ_TRANSPORT_SCHEMA"] = str(owner_path)
    try:
        consumer = load_schema(SCHEMA)
    finally:
        if previous is None:
            os.environ.pop("GWZ_TRANSPORT_SCHEMA", None)
        else:
            os.environ["GWZ_TRANSPORT_SCHEMA"] = previous
    external = {
        name: f"{pin['rust-module']}::{name}"
        for name in [*owner.enums, *owner.messages]
    }
    codec = pin["codec"]
    if codec != "fail-closed":
        raise SystemExit(f"unsupported consumer codec pin: {codec!r}")
    with tempfile.TemporaryDirectory(prefix="gwz-transport-consumer-gen-") as raw:
        out = Path(raw)
        emit(
            consumer,
            out,
            langs=["rust"],
            services=[],
            rust_external_types=external,
            fail_closed=codec == "fail-closed",
        )
        generated = out / "rust" / "api.rs"
        rustfmt = shutil.which("rustfmt")
        if rustfmt is None:
            raise SystemExit("rustfmt is required for reproducible consumer generation")
        rustfmt_version = subprocess.run(
            [rustfmt, "--version"], check=True, capture_output=True, text=True
        ).stdout.strip()
        if rustfmt_version != pin["rustfmt"]:
            raise SystemExit(
                f"rustfmt pin mismatch: expected {pin['rustfmt']!r}, "
                f"got {rustfmt_version!r}"
            )
        subprocess.run([rustfmt, "--edition=2024", str(generated)], check=True)
        return generated.read_text()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--owner-schema", required=True, type=Path)
    parser.add_argument("--taut-source", required=True, type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    owner_path = args.owner_schema.resolve()
    taut_source = args.taut_source.resolve()
    if not owner_path.is_file():
        raise SystemExit(f"owner schema does not exist: {owner_path}")
    if not taut_source.is_dir():
        raise SystemExit(f"taut source directory does not exist: {taut_source}")
    generated = _generate(owner_path, taut_source)
    if args.check:
        if not OUTPUT.exists() or OUTPUT.read_text() != generated:
            raise SystemExit(f"stale generated artifact: {OUTPUT}")
    else:
        OUTPUT.write_text(generated)
    print(f"consumer regen {'verified' if args.check else 'written'}: {OUTPUT}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

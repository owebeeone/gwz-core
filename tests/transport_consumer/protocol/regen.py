#!/usr/bin/env python3
"""Regenerate the test-only core consumer from the exported owner schema.

This is an explicit developer/CI check. Cargo never runs it, and the normal
consumer build uses only the checked-in generated module plus the pinned
registry dependency. The owner schema is supplied explicitly and pinned by
content. taut is the taut-proto release installed in this interpreter's site
directories at the version the generator pin names; a `taut` package or
`taut-proto` metadata anywhere else on sys.path or PYTHONPATH is refused, so
nothing is discovered from a sibling checkout or fetched.
"""

from __future__ import annotations

import argparse
import ast
import hashlib
import importlib.metadata
import json
import os
import shutil
import site
import subprocess
import sys
import sysconfig
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


def _site_directories() -> set[Path]:
    """This interpreter's site directories, the only places an installed release lives."""
    directories = {Path(sysconfig.get_paths()[key]).resolve() for key in ("purelib", "platlib")}
    directories.update(Path(path).resolve() for path in site.getsitepackages())
    directories.add(Path(site.getusersitepackages()).resolve())
    return directories


def _released_taut(pin: dict) -> Path:
    """The package directory of the installed taut-proto release the pin names."""
    try:
        distribution = importlib.metadata.distribution("taut-proto")
    except importlib.metadata.PackageNotFoundError:
        raise SystemExit(f"taut-proto {pin['taut-proto']} is not installed") from None
    location = Path(distribution.locate_file("")).resolve()
    if location not in _site_directories():
        raise SystemExit(
            f"taut-proto metadata at {location} is not the installed taut-proto release: "
            "it lies outside this interpreter's site directories"
        )
    if distribution.version != pin["taut-proto"]:
        raise SystemExit(
            f"taut-proto pin mismatch: expected {pin['taut-proto']}, "
            f"got {distribution.version}"
        )
    return Path(distribution.locate_file("taut")).resolve()


def _check_taut_module_origin(name: str, module: object, package: Path) -> None:
    raw_origin = getattr(module, "__file__", None)
    if not isinstance(raw_origin, str):
        raise SystemExit(f"imported {name} has no file origin")
    origin = Path(raw_origin).resolve()
    if package not in origin.parents:
        raise SystemExit(
            f"imported {name} is not the installed taut-proto release: {origin}"
        )


def _check_loaded_taut(package: Path) -> None:
    for name, module in list(sys.modules.items()):
        if name == "taut" or name.startswith("taut."):
            _check_taut_module_origin(name, module, package)


def _generate(owner_path: Path) -> str:
    pin = json.loads(PIN.read_text())
    package = _released_taut(pin)
    module_names = sorted(name for name in sys.modules if name == "taut" or name.startswith("taut."))
    for name in module_names:
        module = sys.modules.get(name)
        if module is not None:
            _check_taut_module_origin(name, module, package)
            raise SystemExit(
                "taut modules are already loaded; run regeneration in a fresh interpreter"
            )
    import taut  # noqa: F401 -- checked before anything imports from it

    _check_loaded_taut(package)
    from taut.gen.scaffold import emit
    from taut.ir.load import load_schema, schema_from_json

    _check_loaded_taut(package)
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
    with tempfile.TemporaryDirectory(prefix="gwz-transport-consumer-gen-") as raw:
        out = Path(raw)
        emit(
            consumer,
            out,
            langs=["rust"],
            services=[],
            rust_external_types=external,
        )
        _check_loaded_taut(package)
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
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    owner_path = args.owner_schema.resolve()
    if not owner_path.is_file():
        raise SystemExit(f"owner schema does not exist: {owner_path}")
    generated = _generate(owner_path)
    if args.check:
        if not OUTPUT.exists() or OUTPUT.read_text() != generated:
            raise SystemExit(f"stale generated artifact: {OUTPUT}")
    else:
        OUTPUT.write_text(generated)
    print(f"consumer regen {'verified' if args.check else 'written'}: {OUTPUT}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

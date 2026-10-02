#!/usr/bin/env python3
"""Regenerate the isolated placement schema projection.

The candidate generator composes the checked-in core schema with the explicitly
supplied owner export.  It writes only the candidate artifacts:
src/protocol/candidate_generated.rs, protocol/candidate/candidate_generated.py,
and the candidate's conformance corpus under protocol/candidate/corpus/ (the
`tautc corpus` golden vectors and their Rust parity harness, which the candidate
build's `corpus_byte_parity` runs). Production protocol artifacts and manifests
are deliberately outside its output set.

It generates with the taut-proto release installed in this interpreter's site
directories at the version candidate-generator.json pins, and imports taut only
from that release: a `taut` package or `taut-proto` metadata anywhere else on
sys.path or PYTHONPATH is refused.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.metadata
import json
import os
import re
import shutil
import site
import subprocess
import sys
import sysconfig
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CANDIDATE_SCHEMA = ROOT / "protocol" / "candidate" / "candidate.taut.py"
PIN = ROOT / "protocol" / "candidate" / "candidate-generator.json"
RUST_OUT = ROOT / "src" / "protocol" / "candidate_generated.rs"
PYTHON_OUT = ROOT / "protocol" / "candidate" / "candidate_generated.py"
CORPUS_GOLDEN = ROOT / "protocol" / "candidate" / "corpus" / "golden.json"
CORPUS_VECTORS = ROOT / "protocol" / "candidate" / "corpus" / "rust" / "vectors.rs"
OLD_RUST_OUT = ROOT / "tests" / "transport_consumer" / "candidate" / "retained_old_generated.rs"
TOOLCHAIN_FILE = ROOT / "rust-toolchain.toml"


def _toolchain_channel() -> str:
    """The Rust channel gwz-core pins in its rust-toolchain.toml."""
    found = re.search(r'^channel\s*=\s*"([^"]+)"\s*$', TOOLCHAIN_FILE.read_text(), re.MULTILINE)
    if found is None:
        raise SystemExit(f"no toolchain channel in {TOOLCHAIN_FILE}")
    return found.group(1)


def _verify_pin(owner_path: Path) -> dict:
    pin = json.loads(PIN.read_text())
    digest = hashlib.sha256(owner_path.read_bytes()).hexdigest()
    if digest != pin["owner-schema-sha256"]:
        raise SystemExit(
            "owner schema digest mismatch: "
            f"expected {pin['owner-schema-sha256']}, got {digest}"
        )
    return pin


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
            f"taut-proto pin mismatch: expected {pin['taut-proto']}, got {distribution.version}"
        )
    return Path(distribution.locate_file("taut")).resolve()


def _load_taut(pin: dict):
    cached = [name for name in sys.modules if name == "taut" or name.startswith("taut.")]
    if cached:
        raise SystemExit("taut modules are already loaded; run candidate regeneration in a fresh interpreter")
    package = _released_taut(pin)
    import taut  # noqa: F401 -- checked before anything imports from it

    _check_taut_origins(package)
    from taut.corpus import kit, synth
    from taut.gen.scaffold import emit
    from taut.ir.load import load_schema, schema_from_json
    from taut.ir.validate import validate_or_raise

    _check_taut_origins(package)

    def corpus(schema) -> tuple[str, str]:
        """What `tautc corpus -l rust` writes for `schema`: golden.json, rust/vectors.rs."""
        validate_or_raise(schema)
        vectors = kit.build_corpus(schema, synth.synth_values(schema))
        return kit.golden_json(vectors), kit.rust_vectors(schema, vectors)

    return package, emit, load_schema, schema_from_json, corpus


def _check_taut_origins(package: Path) -> None:
    for name, module in list(sys.modules.items()):
        if name == "taut" or name.startswith("taut."):
            origin = getattr(module, "__file__", None)
            if not isinstance(origin, str) or package not in Path(origin).resolve().parents:
                raise SystemExit(f"imported {name} is not the installed taut-proto release: {origin}")


def _load_candidate(core_path: Path, owner_path: Path, load_schema):
    os.environ["GWZ_CORE_SCHEMA"] = str(core_path)
    os.environ["GWZ_TRANSPORT_SCHEMA"] = str(owner_path)
    return load_schema(CANDIDATE_SCHEMA)


def _generate(args: argparse.Namespace) -> dict[Path, str]:
    pin = _verify_pin(args.owner_schema)
    core_digest = hashlib.sha256(args.core_schema.read_bytes()).hexdigest()
    if core_digest != pin["current-core-schema-sha256"]:
        raise SystemExit(
            "current core schema digest mismatch: "
            f"expected {pin['current-core-schema-sha256']}, got {core_digest}"
        )
    package, emit, load_schema, schema_from_json, corpus = _load_taut(pin)
    candidate = _load_candidate(args.core_schema, args.owner_schema, load_schema)
    _check_taut_origins(package)
    golden, vectors = corpus(candidate)
    owner = schema_from_json(json.loads(args.owner_schema.read_text()))
    external = {
        name: f"gwz_transport::protocol::{name}"
        for name in [*owner.enums, *owner.messages]
    }
    rustfmt = shutil.which("rustfmt")
    if rustfmt is None:
        raise SystemExit("rustfmt is required for candidate regeneration")
    # rustup's proxy picks the toolchain from the working directory or an
    # inherited RUSTUP_TOOLCHAIN; the pin names gwz-core's own toolchain, so the
    # proof runs that rustfmt wherever it is started.
    version = subprocess.run(
        [rustfmt, "--version"],
        check=True,
        capture_output=True,
        text=True,
        env={**os.environ, "RUSTUP_TOOLCHAIN": _toolchain_channel()},
    ).stdout.strip()
    if version != pin["rustfmt"]:
        raise SystemExit(f"rustfmt pin mismatch: expected {pin['rustfmt']!r}, got {version!r}")
    with tempfile.TemporaryDirectory(prefix="gwz-placement-candidate-") as raw:
        out = Path(raw)
        emit(
            candidate,
            out,
            langs=["rust", "python"],
            services=[],
            rust_external_types=external,
        )
        candidate_rust = (out / "rust" / "api.rs").read_text()
        candidate_python = (out / "python" / "api.py").read_text()
    if not OLD_RUST_OUT.is_file():
        raise SystemExit(f"checked-in retained reader source does not exist: {OLD_RUST_OUT}")
    old_digest = hashlib.sha256(OLD_RUST_OUT.read_text().encode()).hexdigest()
    if old_digest != pin["retained-old-rust-sha256"]:
        raise SystemExit(
            "retained old Rust reader digest mismatch: "
            f"expected {pin['retained-old-rust-sha256']}, got {old_digest}"
        )
    return {
        RUST_OUT: candidate_rust,
        PYTHON_OUT: candidate_python,
        CORPUS_GOLDEN: golden,
        CORPUS_VECTORS: vectors,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--core-schema", required=True, type=Path)
    parser.add_argument("--owner-schema", required=True, type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    for path in (args.core_schema, args.owner_schema):
        if not path.is_file():
            raise SystemExit(f"schema does not exist: {path}")
    outputs = _generate(args)
    if args.check:
        stale = [str(path) for path, content in outputs.items() if not path.exists() or path.read_text() != content]
        if stale:
            raise SystemExit("stale candidate generated artifacts: " + ", ".join(stale))
    else:
        for path, content in outputs.items():
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content)
    print("placement candidate regeneration " + ("verified" if args.check else "written"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

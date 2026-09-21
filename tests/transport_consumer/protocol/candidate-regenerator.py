#!/usr/bin/env python3
"""Regenerate the isolated placement schema projection.

The candidate generator composes the checked-in core schema with the explicitly
supplied owner export.  It writes only under this consumer harness; production
protocol artifacts and manifests are deliberately outside its output set.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CANDIDATE_SCHEMA = ROOT / "protocol" / "candidate.taut.py"
PIN = ROOT / "protocol" / "candidate-generator.json"
RUST_OUT = ROOT / "candidate" / "candidate_generated.rs"
PYTHON_OUT = ROOT / "candidate" / "candidate_generated.py"
OLD_RUST_OUT = ROOT / "candidate" / "retained_old_generated.rs"


def _verify_pin(owner_path: Path, taut_source: Path) -> dict:
    pin = json.loads(PIN.read_text())
    digest = hashlib.sha256(owner_path.read_bytes()).hexdigest()
    if digest != pin["owner-schema-sha256"]:
        raise SystemExit(
            "owner schema digest mismatch: "
            f"expected {pin['owner-schema-sha256']}, got {digest}"
        )
    repo = Path(
        subprocess.run(
            ["git", "-C", str(taut_source), "rev-parse", "--show-toplevel"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
    )
    supplied = taut_source.resolve()
    expected = (repo / "src").resolve()
    if supplied != expected:
        raise SystemExit(f"taut source must be canonical src/: expected {expected}, got {supplied}")
    dirty = subprocess.run(
        ["git", "-C", str(repo), "status", "--porcelain", "--untracked-files=all", "--", "src"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    if dirty:
        raise SystemExit(f"taut source checkout is dirty under src/: {dirty}")
    for relative, expected in pin["taut-source-sha256"].items():
        path = repo / relative
        if not path.is_file():
            raise SystemExit(f"pinned taut source file is missing: {path}")
        actual = hashlib.sha256(path.read_bytes()).hexdigest()
        if actual != expected:
            raise SystemExit(
                f"taut source digest mismatch for {relative}: "
                f"expected {expected}, got {actual}"
            )
    revision = subprocess.run(
        ["git", "-C", str(repo), "rev-parse", "HEAD"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    if revision != pin["taut-source-revision"]:
        raise SystemExit(
            f"taut source revision mismatch: expected {pin['taut-source-revision']}, got {revision}"
        )
    return pin


def _load_taut(source: Path):
    source = source.resolve()
    cached = [name for name in sys.modules if name == "taut" or name.startswith("taut.")]
    if cached:
        raise SystemExit("taut modules are already loaded; run candidate regeneration in a fresh interpreter")
    sys.path.insert(0, str(source))
    from taut.gen.scaffold import emit
    from taut.ir.load import load_schema, schema_from_json

    _check_taut_origins(source)
    return emit, load_schema, schema_from_json


def _check_taut_origins(source: Path) -> None:
    for name, module in list(sys.modules.items()):
        if name == "taut" or name.startswith("taut."):
            origin = getattr(module, "__file__", None)
            if not isinstance(origin, str) or source not in Path(origin).resolve().parents:
                raise SystemExit(f"imported {name} is outside canonical taut source: {origin}")


def _load_candidate(core_path: Path, owner_path: Path, load_schema):
    os.environ["GWZ_CORE_SCHEMA"] = str(core_path)
    os.environ["GWZ_TRANSPORT_SCHEMA"] = str(owner_path)
    return load_schema(CANDIDATE_SCHEMA)


def _generate(args: argparse.Namespace) -> tuple[str, str, str]:
    pin = _verify_pin(args.owner_schema, args.taut_source)
    core_digest = hashlib.sha256(args.core_schema.read_bytes()).hexdigest()
    if core_digest != pin["retained-old-schema-sha256"]:
        raise SystemExit(
            "retained old schema digest mismatch: "
            f"expected {pin['retained-old-schema-sha256']}, got {core_digest}"
        )
    emit, load_schema, schema_from_json = _load_taut(args.taut_source)
    candidate = _load_candidate(args.core_schema, args.owner_schema, load_schema)
    _check_taut_origins(args.taut_source.resolve())
    owner = schema_from_json(json.loads(args.owner_schema.read_text()))
    external = {
        name: f"gwz_transport::protocol::{name}"
        for name in [*owner.enums, *owner.messages]
    }
    rustfmt = shutil.which("rustfmt")
    if rustfmt is None:
        raise SystemExit("rustfmt is required for candidate regeneration")
    version = subprocess.run(
        [rustfmt, "--version"], check=True, capture_output=True, text=True
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
            fail_closed=True,
        )
        candidate_rust = (out / "rust" / "api.rs").read_text()
        candidate_python = (out / "python" / "api.py").read_text()
    if not OLD_RUST_OUT.is_file():
        raise SystemExit(f"checked-in retained reader source does not exist: {OLD_RUST_OUT}")
    old_rust = OLD_RUST_OUT.read_text()
    old_digest = hashlib.sha256(old_rust.encode()).hexdigest()
    if old_digest != pin["retained-old-rust-sha256"]:
        raise SystemExit(
            "retained old Rust reader digest mismatch: "
            f"expected {pin['retained-old-rust-sha256']}, got {old_digest}"
        )
    return candidate_rust, candidate_python, old_rust


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--core-schema", required=True, type=Path)
    parser.add_argument("--owner-schema", required=True, type=Path)
    parser.add_argument("--taut-source", required=True, type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    for path in (args.core_schema, args.owner_schema):
        if not path.is_file():
            raise SystemExit(f"schema does not exist: {path}")
    candidate_rust, candidate_python, old_rust = _generate(args)
    outputs = {RUST_OUT: candidate_rust, PYTHON_OUT: candidate_python}
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

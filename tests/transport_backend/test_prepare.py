"""Preparation tests for the isolated full-core placement candidate."""

import hashlib
import json
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WORKSPACE = ROOT.parent
PREPARE = Path(__file__).with_name("prepare.py")
CANDIDATE = ROOT / "tests" / "transport_consumer" / "candidate" / "candidate_generated.rs"
PRODUCTION = ROOT / "src" / "protocol" / "generated.rs"
PROTOCOL_MOD = ROOT / "src" / "protocol" / "mod.rs"
GUIDE = ROOT / "tests" / "transport_backend" / "guide_example.rs"


def prepare(destination: Path) -> Path:
    subprocess.run(
        [sys.executable, "-B", str(PREPARE), str(destination)],
        check=True,
        capture_output=True,
        text=True,
    )
    return destination


def test_prepare_selects_candidate_overlay_without_touching_production(tmp_path):
    before = hashlib.sha256(PROTOCOL_MOD.read_bytes()).hexdigest()
    destination = prepare(tmp_path / "backend")
    metadata = json.loads((destination / "placement-candidate.json").read_text())

    assert not destination.resolve().is_relative_to(WORKSPACE.resolve())
    assert metadata["candidate_source"] == str(CANDIDATE.resolve())
    assert metadata["candidate_sha256"] == hashlib.sha256(CANDIDATE.read_bytes()).hexdigest()
    assert metadata["production_source"] == str(PRODUCTION.resolve())
    assert hashlib.sha256(PROTOCOL_MOD.read_bytes()).hexdigest() == before


def test_prepare_manifest_keeps_candidate_dependency_outside_production(tmp_path):
    destination = prepare(tmp_path / "backend")
    manifest = (destination / "Cargo.toml").read_text()

    assert f'gwz-transport = {{ path = "{WORKSPACE / "gwz-transport"}" }}' in manifest
    assert "gwz_transport_candidate" not in manifest
    assert "[patch.crates-io]" in manifest


def test_protocol_module_has_explicit_candidate_boundary():
    source = PROTOCOL_MOD.read_text()

    assert "cfg_if::cfg_if!" in source
    assert "gwz_transport_candidate" in source
    assert "candidate_generated.rs" in source
    assert 'path = "generated.rs"' in source


def test_guide_fixture_is_exact_design_example():
    document = (ROOT / "docs" / "TransportPlacement.md").read_text()
    section = document.split("## Example: configure, fetch once, remove\n", 1)[1]
    expected = section.split("```rust\n", 1)[1].split("\n```", 1)[0] + "\n"
    assert GUIDE.read_text() == expected

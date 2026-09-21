"""Provenance and retained-reader checks for placement candidate regeneration."""

import importlib.util
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
REGEN_PATH = ROOT / "protocol" / "candidate-regenerator.py"
TAUT_SOURCE = ROOT.parents[2] / "taut" / "src"
CORE = ROOT.parents[1]
CORE_SCHEMA = CORE / "protocol" / "gwz.taut.py"
OWNER_SCHEMA = ROOT.parents[2] / "gwz-transport" / "protocol" / "transport.ir.json"
SPEC = importlib.util.spec_from_file_location("candidate_regen", REGEN_PATH)
assert SPEC is not None and SPEC.loader is not None
REGEN = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REGEN)


def test_candidate_regeneration_matches_checked_artifacts():
    subprocess.run(
        [
            "python3",
            str(REGEN_PATH),
            "--core-schema",
            str(CORE_SCHEMA),
            "--owner-schema",
            str(OWNER_SCHEMA),
            "--taut-source",
            str(TAUT_SOURCE),
            "--check",
        ],
        check=True,
    )


def test_dirty_canonical_taut_source_is_rejected(monkeypatch):
    original = REGEN.subprocess.run

    def fake_run(command, **kwargs):
        if "status" in command:
            return type("Result", (), {"stdout": " M src/taut/wire/codec.py\n"})()
        return original(command, **kwargs)

    monkeypatch.setattr(REGEN.subprocess, "run", fake_run)
    with pytest.raises(SystemExit, match="checkout is dirty"):
        REGEN._verify_pin(OWNER_SCHEMA, TAUT_SOURCE)


def test_retained_reader_is_pinned_checked_in_baseline():
    retained = ROOT / "candidate" / "retained_old_generated.rs"
    metadata = REGEN.json.loads((ROOT / "protocol" / "candidate-generator.json").read_text())
    assert REGEN.hashlib.sha256(retained.read_bytes()).hexdigest() == metadata[
        "retained-old-rust-sha256"
    ]
    assert REGEN.hashlib.sha256(CORE_SCHEMA.read_bytes()).hexdigest() == metadata[
        "retained-old-schema-sha256"
    ]

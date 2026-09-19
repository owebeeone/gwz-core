"""Small pin tests for the explicit consumer regeneration gate."""

import importlib.util
import json
import subprocess
import sys
import types
from pathlib import Path

import pytest

TAUT_SOURCE = Path(__file__).resolve().parents[4] / "taut" / "src"
sys.path.insert(0, str(TAUT_SOURCE))

ROOT = Path(__file__).resolve().parent.parent
OWNER = ROOT.parents[2] / "gwz-transport" / "protocol" / "transport.ir.json"
REGEN_PATH = Path(__file__).with_name("regen.py")
SPEC = importlib.util.spec_from_file_location("consumer_regen", REGEN_PATH)
assert SPEC is not None and SPEC.loader is not None
REGEN = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REGEN)


def pin():
    return json.loads(REGEN.PIN.read_text())


def test_positive_regeneration_matches_checked_artifact():
    subprocess.run(
        [sys.executable, str(REGEN_PATH), "--owner-schema", str(OWNER),
         "--taut-source", str(TAUT_SOURCE), "--check"],
        check=True,
    )


def test_wrong_owner_pin_refuses_before_schema_load():
    from taut.ir.load import schema_from_json

    invalid = pin()
    invalid["owner-schema-sha256"] = "0" * 64
    with pytest.raises(SystemExit, match="owner schema digest mismatch"):
        REGEN._load_owner(OWNER, invalid, schema_from_json)


def test_manifest_identity_reads_explicit_package_table(tmp_path):
    manifest = tmp_path / "Cargo.toml"
    manifest.write_text(
        '[dependencies]\nname = "gwz-transport"\nversion = "0.1.0"\n'
        '[package]\nname = "wrong-owner"\nversion = "9.9.9"\n'
    )
    assert REGEN._package_identity(manifest) == ("wrong-owner", "9.9.9")


def test_wrong_taut_source_pin_refuses_before_generation():
    invalid = pin()
    invalid["taut-source-revision"] = "0" * 40
    with pytest.raises(SystemExit, match="taut source revision mismatch"):
        REGEN._verify_taut_source(TAUT_SOURCE, invalid)


def test_dirty_taut_source_refuses_generation(monkeypatch):
    original = REGEN.subprocess.run

    def fake_run(command, **kwargs):
        if command[3] == "status":
            return type("Result", (), {"stdout": " M src/taut/gen/scaffold.py\n"})()
        return original(command, **kwargs)

    monkeypatch.setattr(REGEN.subprocess, "run", fake_run)
    with pytest.raises(SystemExit, match="checkout is dirty"):
        REGEN._verify_taut_source(TAUT_SOURCE, pin())


def test_noncanonical_taut_subdirectory_refuses_generation():
    with pytest.raises(SystemExit, match="canonical.*src"):
        REGEN._verify_taut_source(TAUT_SOURCE.parent / "docs", pin())


def test_foreign_cached_same_version_modules_refuse_generation(monkeypatch, tmp_path):
    foreign = tmp_path / "cached-taut"
    foreign.mkdir()
    for module_name in ("taut", "taut.gen", "taut.gen.scaffold", "taut.gen.rust_external"):
        module = types.ModuleType(module_name)
        module.__file__ = str(foreign / (module_name.replace(".", "_") + ".py"))
        if module_name == "taut":
            module.__version__ = "0.9.1"
        monkeypatch.setitem(sys.modules, module_name, module)
    monkeypatch.setattr(REGEN, "_verify_taut_source", lambda *_args: None)
    with pytest.raises(SystemExit, match="outside canonical taut source"):
        REGEN._generate(OWNER, TAUT_SOURCE)


def test_cached_canonical_module_also_requires_fresh_interpreter(monkeypatch):
    module = types.ModuleType("taut")
    module.__file__ = str(TAUT_SOURCE / "taut" / "__init__.py")
    monkeypatch.setitem(sys.modules, "taut", module)
    monkeypatch.setattr(REGEN, "_verify_taut_source", lambda *_args: None)
    with pytest.raises(SystemExit, match="fresh interpreter"):
        REGEN._generate(OWNER, TAUT_SOURCE)

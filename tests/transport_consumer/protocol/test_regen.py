"""Small pin tests for the explicit consumer regeneration gate."""

import importlib.util
import json
import os
import subprocess
import sys
import types
from pathlib import Path

import pytest

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
        [sys.executable, str(REGEN_PATH), "--owner-schema", str(OWNER), "--check"],
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


def test_a_different_taut_proto_release_refuses_before_generation():
    invalid = pin()
    invalid["taut-proto"] = "0.0.0"
    with pytest.raises(SystemExit, match="taut-proto pin mismatch"):
        REGEN._released_taut(invalid)


def test_a_taut_that_shadows_the_release_refuses_generation(tmp_path):
    # A checkout on PYTHONPATH can carry the pinned version string; only the
    # installed release's own files count.
    shadow = tmp_path / "taut"
    shadow.mkdir()
    (shadow / "__init__.py").write_text("__version__ = '0.10.0'\n")
    result = subprocess.run(
        [sys.executable, str(REGEN_PATH), "--owner-schema", str(OWNER), "--check"],
        capture_output=True,
        text=True,
        env={**os.environ, "PYTHONPATH": str(tmp_path)},
    )
    assert result.returncode != 0
    assert "is not the installed taut-proto release" in result.stderr


def test_a_taut_carrying_its_own_release_metadata_refuses_generation(tmp_path):
    # Metadata that claims the pinned release does not make a copy on
    # PYTHONPATH the installed release; only this interpreter's site
    # directories hold that.
    version = pin()["taut-proto"]
    metadata = tmp_path / f"taut_proto-{version}.dist-info"
    metadata.mkdir()
    (metadata / "METADATA").write_text(
        f"Metadata-Version: 2.1\nName: taut-proto\nVersion: {version}\n"
    )
    shadow = tmp_path / "taut"
    shadow.mkdir()
    (shadow / "__init__.py").write_text(f"__version__ = '{version}'\n")
    result = subprocess.run(
        [sys.executable, str(REGEN_PATH), "--owner-schema", str(OWNER), "--check"],
        capture_output=True,
        text=True,
        env={**os.environ, "PYTHONPATH": str(tmp_path)},
    )
    assert result.returncode != 0
    assert "is not the installed taut-proto release" in result.stderr


def test_foreign_cached_same_version_modules_refuse_generation(monkeypatch, tmp_path):
    foreign = tmp_path / "cached-taut"
    foreign.mkdir()
    for module_name in ("taut", "taut.gen", "taut.gen.scaffold", "taut.gen.rust_external"):
        module = types.ModuleType(module_name)
        module.__file__ = str(foreign / (module_name.replace(".", "_") + ".py"))
        if module_name == "taut":
            module.__version__ = "0.10.0"
        monkeypatch.setitem(sys.modules, module_name, module)
    with pytest.raises(SystemExit, match="is not the installed taut-proto release"):
        REGEN._generate(OWNER)


def test_cached_release_module_also_requires_fresh_interpreter(monkeypatch):
    module = types.ModuleType("taut")
    module.__file__ = str(REGEN._released_taut(pin()) / "__init__.py")
    monkeypatch.setitem(sys.modules, "taut", module)
    with pytest.raises(SystemExit, match="fresh interpreter"):
        REGEN._generate(OWNER)

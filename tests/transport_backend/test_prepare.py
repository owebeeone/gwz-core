"""Preparation tests for the isolated full-core placement candidate."""

import glob
import hashlib
import json
import os
import subprocess
import sys
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WORKSPACE = ROOT.parent
PREPARE = Path(__file__).with_name("prepare.py")
CANDIDATE = ROOT / "src" / "protocol" / "candidate_generated.rs"
PRODUCTION = ROOT / "src" / "protocol" / "generated.rs"
PROTOCOL_MOD = ROOT / "src" / "protocol" / "mod.rs"
GUIDE = ROOT / "tests" / "transport_backend" / "guide_example.rs"
DEPENDENCY_TABLES = ("dependencies", "dev-dependencies", "build-dependencies")


def prepare(destination: Path) -> Path:
    subprocess.run(
        [sys.executable, "-B", str(PREPARE), str(destination)],
        check=True,
        capture_output=True,
        text=True,
    )
    return destination


def _manifest(directory: Path) -> dict:
    return tomllib.loads((directory / "Cargo.toml").read_text())


def _cargo_path(directory: Path, value: str) -> Path:
    # Cargo joins a dependency path to the directory of the manifest that names
    # it and normalizes the result lexically, without resolving symlinks first.
    return Path(os.path.normpath(directory / value))


def _dependency_paths(manifest: dict, *, dev: bool) -> list[str]:
    # Cargo reads dev-dependencies only for the workspace's own packages.
    names = DEPENDENCY_TABLES if dev else ("dependencies", "build-dependencies")
    tables = [manifest.get(name, {}) for name in names]
    for target in manifest.get("target", {}).values():
        tables.extend(target.get(name, {}) for name in names)
    return [
        spec["path"]
        for table in tables
        for spec in table.values()
        if isinstance(spec, dict) and "path" in spec
    ]


def test_prepare_selects_candidate_overlay_without_touching_production(tmp_path):
    before = hashlib.sha256(PROTOCOL_MOD.read_bytes()).hexdigest()
    destination = prepare(tmp_path / "backend")
    metadata = json.loads((destination / "placement-candidate.json").read_text())

    assert not destination.resolve().is_relative_to(WORKSPACE.resolve())
    assert metadata["candidate_source"] == str(CANDIDATE.resolve())
    assert metadata["candidate_sha256"] == hashlib.sha256(CANDIDATE.read_bytes()).hexdigest()
    assert metadata["production_source"] == str(PRODUCTION.resolve())
    # A prepared tree lives outside the workspace; tests/protocol.rs finds taut here.
    assert metadata["taut_source"] == str(WORKSPACE / "taut" / "src")
    assert hashlib.sha256(PROTOCOL_MOD.read_bytes()).hexdigest() == before


def test_prepare_manifest_keeps_candidate_dependency_outside_production(tmp_path):
    destination = prepare(tmp_path / "backend")
    manifest = (destination / "Cargo.toml").read_text()

    assert f'gwz-transport = {{ path = "{WORKSPACE / "gwz-transport"}" }}' in manifest
    assert "gwz_transport_candidate" not in manifest
    # The candidate builds against the workspace's git2-rs fork with the fork's
    # vendored libgit2, exactly as production declares it; only the path is anchored.
    git2 = tomllib.loads(manifest)["dependencies"]["git2"]
    production = tomllib.loads((ROOT / "Cargo.toml").read_text())["dependencies"]["git2"]
    assert git2 == {**production, "path": str(WORKSPACE / "git2-rs")}
    assert git2["package"] == "gwz-git2"
    assert "vendored-libgit2" in git2["features"]


def test_candidate_adds_the_windows_networking_features_the_endpoint_needs(tmp_path):
    destination = prepare(tmp_path / "backend")
    prepared = _manifest(destination)["target"]["cfg(windows)"]["dependencies"]["windows-sys"]
    production = tomllib.loads((ROOT / "Cargo.toml").read_text())["target"]["cfg(windows)"]["dependencies"]["windows-sys"]
    # The candidate adds to production's list, never replaces it: WinHttp for the HTTPS endpoint, WinSock for
    # the SSH endpoint's socket waits (GwzTransportWindowsParityPlan.md, step 1.2), and the features the SSH test
    # fixtures use (step 1.1). Production names none of them.
    assert set(production["features"]) < set(prepared["features"])
    assert set(prepared["features"]) - set(production["features"]) == {
        "Win32_Networking_WinHttp",
        "Win32_Networking_WinSock",
        "Win32_Security",
        "Win32_System_Diagnostics_ToolHelp",
        "Win32_System_IO",
        "Win32_System_Ioctl",
        "Win32_System_JobObjects",
        "Win32_System_Pipes",
        "Win32_System_Threading",
    }


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


def test_in_process_python_dependency_is_candidate_only(tmp_path):
    destination = prepare(tmp_path / "backend")
    manifest = (destination / "Cargo.toml").read_text()
    assert 'pyo3 = { version = "=0.28.3", features = ["auto-initialize"] }' in manifest
    assert 'pyo3' not in (ROOT / "Cargo.toml").read_text()


def test_tls_test_servers_are_rustls_dev_dependencies_and_product_tls_stays_native(tmp_path):
    # Step 0.5b: the fixture's servers must not load a platform identity, and the product must not gain rustls.
    manifest = _manifest(prepare(tmp_path / "backend"))
    for crate in ("rustls", "tokio-rustls"):
        assert crate in manifest["dev-dependencies"]
        assert crate not in manifest["dependencies"]
    assert manifest["dev-dependencies"]["rustls"]["default-features"] is False
    assert "ring" in manifest["dev-dependencies"]["rustls"]["features"]
    assert "native-tls" in manifest["dependencies"]
    assert "rustls" not in (ROOT / "Cargo.toml").read_text()


def test_prepared_path_dependencies_resolve_from_a_fresh_destination(tmp_path):
    destination = prepare(tmp_path / "backend")
    workspace = {destination}
    for pattern in _manifest(destination).get("workspace", {}).get("members", []):
        workspace.update(Path(member) for member in glob.glob(str(destination / pattern)))
    pending = sorted(workspace)
    seen = set()
    missing = []
    while pending:
        directory = pending.pop()
        if directory in seen:
            continue
        seen.add(directory)
        for value in _dependency_paths(_manifest(directory), dev=directory in workspace):
            target = _cargo_path(directory, value)
            if (target / "Cargo.toml").is_file():
                pending.append(target)
            else:
                missing.append(f"{directory / 'Cargo.toml'}: {value} -> {target}")
    assert missing == []


def test_prepared_patches_name_the_packages_their_paths_hold(tmp_path):
    destination = prepare(tmp_path / "backend")
    stale = []
    for source, entries in _manifest(destination).get("patch", {}).items():
        for name, spec in entries.items():
            held = _manifest(_cargo_path(destination, spec["path"]))["package"]["name"]
            if held != spec.get("package", name):
                stale.append(f"[patch.{source}] {name} -> {held}")
    assert stale == []


def test_prepared_manifest_compiles_guide_as_external_consumer(tmp_path):
    destination = prepare(tmp_path / "backend")
    targets = {target["name"]: target for target in _manifest(destination).get("test", [])}
    wrapper = destination / targets["transport_placement_guide"]["path"]

    assert wrapper.resolve() == (ROOT / "tests" / "transport_backend" / "guide_test.rs")
    assert '#[path = "guide_example.rs"]' in wrapper.read_text()
    assert "include!" not in wrapper.read_text()
    assert "guide_test.rs" not in (ROOT / "Cargo.toml").read_text()
    assert not [path for path in (ROOT / "src").rglob("*.rs") if "guide_example" in path.read_text()]

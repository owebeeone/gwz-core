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
SWITCH = "gwz_transport_candidate"
OTHER_THAN_PLATFORM_TLS = 'any(windows, target_vendor = "apple")'
# The cfg of the tables the manifest keeps behind the switch (C1 of dev-docs/GwzV110RemainingPlan.md): Cargo evaluates a
# `[target.'cfg(..)']` key against `rustc --print cfg`, which carries the `--cfg` values of RUSTFLAGS, so these crates are in
# the resolve only when the switch is on. One table per distinct condition, as the candidate manifest had.
SWITCH_ALL = f"cfg({SWITCH})"
SWITCH_PLATFORM_TLS = f"cfg(all({SWITCH}, {OTHER_THAN_PLATFORM_TLS}))"
SWITCH_OPENSSL = f"cfg(all({SWITCH}, not({OTHER_THAN_PLATFORM_TLS})))"
SWITCH_WINDOWS = f"cfg(all(windows, {SWITCH}))"
# What the candidate shape adds to the ordinary dependency set, by the table that carries it.
SWITCH_DEPENDENCIES = {
    "gwz-transport", "gwz-sspi", "base64", "socket2", "ssh2", "libssh2-sys", "tokio", "tokio-util", "hyper",
    "hyper-util", "http-body-util", "bytes",
}
SWITCH_DEV_DEPENDENCIES = {"pyo3", "rustls", "tokio-rustls", "russh"}
SWITCH_ONLY_CRATES = (
    SWITCH_DEPENDENCIES
    | SWITCH_DEV_DEPENDENCIES
    | {"native-tls", "tokio-native-tls", "openssl", "openssl-probe"}
)


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


def _production() -> dict:
    return tomllib.loads((ROOT / "Cargo.toml").read_text())


def _switch_tables(manifest: dict) -> dict:
    """The `[target.'cfg(..)']` tables whose condition names the switch, by condition."""
    return {key: table for key, table in manifest.get("target", {}).items() if SWITCH in key}


def test_prepare_manifest_keeps_candidate_dependency_outside_production(tmp_path):
    # The transport and gwz-sspi are named in the core's own manifest, in the table that only the switch turns on; the
    # prepared copy anchors their paths like any other.
    assert _production()["target"][SWITCH_ALL]["dependencies"]["gwz-transport"] == {"path": "../gwz-transport"}
    assert _production()["target"][SWITCH_ALL]["dependencies"]["gwz-sspi"] == {"path": "../gwz-sspi"}
    assert "gwz-transport" not in _production()["dependencies"]
    destination = prepare(tmp_path / "backend")
    manifest = (destination / "Cargo.toml").read_text()
    prepared = tomllib.loads(manifest)["target"][SWITCH_ALL]["dependencies"]

    assert prepared["gwz-transport"] == {"path": str(WORKSPACE / "gwz-transport")}
    assert prepared["gwz-sspi"] == {"path": str(WORKSPACE / "gwz-sspi")}
    assert "cfg(gwz_transport_candidate)" in manifest
    # The candidate builds against the workspace's git2-rs fork with the fork's
    # vendored libgit2, exactly as production declares it; only the path is anchored.
    git2 = tomllib.loads(manifest)["dependencies"]["git2"]
    production = _production()["dependencies"]["git2"]
    assert git2 == {**production, "path": str(WORKSPACE / "git2-rs")}
    assert git2["package"] == "gwz-git2"
    assert "vendored-libgit2" in git2["features"]


def test_candidate_adds_the_windows_networking_features_the_endpoint_needs(tmp_path):
    production = _production()["target"]["cfg(windows)"]["dependencies"]["windows-sys"]
    added = _production()["target"][SWITCH_WINDOWS]["dependencies"]["windows-sys"]
    # The switch's table names the same crate again with the features it adds; Cargo unifies the two entries when both
    # conditions hold, so the candidate adds to production's list and never replaces it. WinHttp for the HTTPS endpoint,
    # WinSock for the SSH endpoint's socket waits (GwzTransportWindowsParityPlan.md, step 1.2), the features the SSH test
    # fixtures use (step 1.1), Pageant's exchange (step 3.5: its mapping, descriptor and `WM_COPYDATA`; step 3.6's
    # synthetic Pageant window class, `Win32_Graphics_Gdi`). Production names only the three that the process gate of step
    # 1.3a needs in every build (`CreateSemaphoreW`, the owner check: Win32_Security, Win32_Security_Authorization,
    # Win32_System_Threading); the switch's table repeats them, because the candidate's list is the complete set the
    # endpoint needs and Cargo unifies the duplicates. The switch's removal moves the rest into production's list.
    assert set(production["features"]) & set(added["features"]) == {
        "Win32_Security",
        "Win32_Security_Authorization",
        "Win32_System_Threading",
    }
    assert set(added["features"]) == {
        "Win32_Graphics_Gdi",
        "Win32_NetworkManagement_IpHelper",
        "Win32_NetworkManagement_Ndis",
        "Win32_Networking_WinHttp",
        "Win32_Networking_WinSock",
        "Win32_Security",
        "Win32_Security_Authentication_Identity",
        "Win32_Security_Authorization",
        "Win32_Security_Credentials",
        "Win32_System_DataExchange",
        "Win32_System_Diagnostics_ToolHelp",
        "Win32_System_IO",
        "Win32_System_Ioctl",
        "Win32_System_JobObjects",
        "Win32_System_Memory",
        "Win32_System_Pipes",
        "Win32_System_SystemInformation",
        "Win32_System_Threading",
        "Win32_UI_WindowsAndMessaging",
    }
    assert added["version"] == production["version"]
    # The prepared copy carries the same two entries.
    prepared = _manifest(prepare(tmp_path / "backend"))["target"]
    assert prepared["cfg(windows)"]["dependencies"]["windows-sys"] == production
    assert prepared[SWITCH_WINDOWS]["dependencies"]["windows-sys"] == added


def test_protocol_module_has_explicit_candidate_boundary():
    source = PROTOCOL_MOD.read_text()

    assert "cfg_if::cfg_if!" in source
    # S7.1 (1.1.0) step C3a: only the session candidate selects the candidate protocol module; the transport
    # switch alone is the ordinary protocol.
    assert "gwz_session_candidate" in source
    assert "gwz_transport_candidate" not in source
    assert "candidate_generated.rs" in source
    assert 'path = "generated.rs"' in source


def test_guide_fixture_is_exact_design_example():
    document = (ROOT / "docs" / "TransportPlacement.md").read_text()
    section = document.split("## Example: configure, fetch once, remove\n", 1)[1]
    expected = section.split("```rust\n", 1)[1].split("\n```", 1)[0] + "\n"
    assert GUIDE.read_text() == expected


def test_in_process_python_dependency_is_candidate_only():
    manifest = _production()
    assert manifest["target"][SWITCH_ALL]["dev-dependencies"]["pyo3"] == {
        "version": "=0.28.3",
        "features": ["auto-initialize"],
    }
    assert "pyo3" not in manifest.get("dev-dependencies", {})
    assert "pyo3" not in manifest["dependencies"]


def test_tls_test_servers_are_rustls_dev_dependencies_and_product_tls_stays_native():
    # Step 0.5b: the fixture's servers must not load a platform identity, and the product must not gain rustls.
    manifest = _production()
    dev = manifest["target"][SWITCH_ALL]["dev-dependencies"]
    for crate in ("rustls", "tokio-rustls"):
        assert crate in dev
        assert crate not in manifest["dependencies"]
        assert crate not in manifest["target"][SWITCH_ALL]["dependencies"]
        assert crate not in manifest.get("dev-dependencies", {})
    assert dev["rustls"]["default-features"] is False
    assert "ring" in dev["rustls"]["features"]


def test_tls_stack_is_native_tls_on_apple_and_windows_and_openssl_directly_elsewhere():
    # src/git/endpoint/https_backend.rs: native-tls would parse the system bundle twice for a store it replaces, so the
    # OpenSSL build does not depend on it (only its tests do, as the oracle for what the stack puts on the wire).
    manifest = _production()
    targets = manifest["target"]
    platform = targets[SWITCH_PLATFORM_TLS]
    openssl = targets[SWITCH_OPENSSL]
    assert platform["dependencies"] == {"native-tls": "=0.2.18", "tokio-native-tls": "=0.3.1"}
    assert "dev-dependencies" not in platform
    assert openssl["dependencies"] == {"openssl": "=0.10.81", "openssl-probe": "=0.1.6"}
    assert openssl["dev-dependencies"] == {"native-tls": "=0.2.18", "tokio-native-tls": "=0.3.1"}
    assert "native-tls" not in manifest["dependencies"]
    assert "native-tls" not in targets[SWITCH_ALL]["dependencies"]
    assert "openssl" not in targets[SWITCH_ALL]["dependencies"]


def test_ssh_test_server_is_a_russh_dev_dependency_over_ring_and_the_product_has_none():
    # The controlled SSH server (src/git/endpoint/ssh_controlled_server): test-only, pure Rust over the same ring as the
    # TLS servers, so no C toolchain beyond ring's own; the product's manifest and its normal dependencies gain nothing.
    manifest = _production()
    russh = manifest["target"][SWITCH_ALL]["dev-dependencies"]["russh"]
    assert russh["version"].startswith("=")
    assert russh["default-features"] is False
    assert set(russh["features"]) == {"ring", "rsa"}
    assert "russh" not in manifest["dependencies"]
    assert "russh" not in manifest["target"][SWITCH_ALL]["dependencies"]


def test_only_tables_behind_the_switch_name_a_transport_crate():
    # C1: the manifest names the transport's crates, but no table an ordinary build evaluates does. A crate is a switch
    # crate if the candidate manifest added it; every table that names one has the switch in its own condition.
    manifest = _production()
    unconditional = [(name, manifest.get(name, {})) for name in DEPENDENCY_TABLES]
    for key, table in manifest["target"].items():
        if SWITCH not in key:
            unconditional += [(f"{key}.{name}", table.get(name, {})) for name in DEPENDENCY_TABLES]
    stray = sorted(
        f"{where}: {crate}"
        for where, table in unconditional
        for crate in table
        if crate in SWITCH_ONLY_CRATES
    )
    assert stray == []
    named = {
        crate
        for table in _switch_tables(manifest).values()
        for name in DEPENDENCY_TABLES
        for crate in table.get(name, {})
    }
    assert SWITCH_ONLY_CRATES <= named
    assert set(_switch_tables(manifest)) == {SWITCH_ALL, SWITCH_PLATFORM_TLS, SWITCH_OPENSSL, SWITCH_WINDOWS}


TARGETS = ("aarch64-apple-darwin", "x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc")


def _without_the_switch_tables(manifest: str) -> str:
    """The manifest text with every table whose header names the switch removed."""
    kept: list[str] = []
    dropped = False
    for line in manifest.splitlines(keepends=True):
        if line.startswith("["):
            dropped = SWITCH in line.split("#", 1)[0]
        if not dropped:
            kept.append(line)
    return "".join(kept)


def _resolved(manifest_dir: Path, target: str, *, switch: bool) -> str:
    environment = {key: value for key, value in os.environ.items() if key != "RUSTFLAGS"}
    if switch:
        environment["RUSTFLAGS"] = "--cfg " + SWITCH
    result = subprocess.run(
        [
            "cargo", "tree", "--manifest-path", str(manifest_dir / "Cargo.toml"),
            "-e", "normal,dev,build", "--target", target, "--prefix", "none", "-f", "{p} [{f}]",
        ],
        capture_output=True,
        text=True,
        env=environment,
    )
    assert result.returncode == 0, result.stderr
    # The root package prints its own directory, which differs between the two copies.
    return "\n".join(sorted(set(result.stdout.replace(str(manifest_dir), "<core>").splitlines())))


def test_the_ordinary_shape_resolves_the_dependencies_it_resolved_without_the_switch_tables(tmp_path):
    # The ordinary build's dependency set is unchanged by the switch tables: a copy of the manifest with them removed
    # resolves, for each platform, to exactly the set the real manifest does without the switch, and the candidate shape
    # resolves to more. (The copy's lock is cargo's pruning of the real one, which keeps every version it still uses.)
    whole = prepare(tmp_path / "whole")
    stripped = prepare(tmp_path / "stripped")
    (stripped / "Cargo.toml").write_text(_without_the_switch_tables((stripped / "Cargo.toml").read_text()))
    for target in TARGETS:
        ordinary = _resolved(whole, target, switch=False)
        assert ordinary == _resolved(stripped, target, switch=False), target
        assert ordinary != _resolved(whole, target, switch=True), target


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


def test_prepared_directory_provides_the_workflows_and_scripts_publish_workflow_reads(tmp_path):
    # Step 5.4: tests/publish_workflow.rs reads .github/workflows/*.yml and scripts/release.py. From a prepared
    # directory the path `tests/../.github` is the directory's own on Windows (`..` is collapsed before the lookup, so
    # the symlinked tests/ is never followed) and the core's on Linux; the directory provides the root both resolve from.
    destination = prepare(tmp_path / "backend")
    for name in (".github", "scripts"):
        assert (destination / name).resolve() == (ROOT / name).resolve(), name
    assert (destination / ".github" / "workflows" / "release.yml").is_file()
    assert (destination / "scripts" / "release.py").is_file()


def test_publish_workflow_reads_its_files_from_the_manifest_directory_not_through_dotdot():
    # `include_str!("../.github/...")` from a symlinked tests/ resolves differently on Windows (step 0.1's reason for
    # leaving the target to the Linux legs). The manifest directory is the prepared directory's own root.
    source = (ROOT / "tests" / "publish_workflow.rs").read_text()
    assert 'include_str!("' not in source
    assert source.count('env!("CARGO_MANIFEST_DIR")') == 1
    assert source.count("core_file!(") >= 5

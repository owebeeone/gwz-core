#!/usr/bin/env python3
"""Prepare a local full-core candidate; never changes production manifests."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil

# The `windows-sys` features the candidate's endpoint code needs beyond the production list in Cargo.toml. A step
# that needs one more Windows API adds a name here (GwzTransportWindowsParityPlan.md, section 4). They serve
# product code and test fixtures:
#   product (step 4.2's helper process owner, `https_auth/process_tree/windows*`): Win32_System_JobObjects (the
#     job), Win32_System_Threading (CreateProcessW, the attribute list, process waits), Win32_System_Pipes (the
#     overlapped pipes), Win32_Security (inheritable handle attributes), Win32_System_SystemInformation (the Windows
#     directory), and the production list's own Win32_Foundation and Win32_Storage_FileSystem; Win32_Networking_
#     WinHttp and Win32_Networking_WinSock serve the HTTPS and SSH code; step 3.5's Pageant exchange
#     (`pageant_exchange/sys.rs`) needs Win32_Security_Authorization, Win32_System_DataExchange, Win32_System_Memory
#     and Win32_UI_WindowsAndMessaging. The candidate-to-production switch must
#     carry every one of them into Cargo.toml's `windows-sys` list.
#   test only: Win32_System_Diagnostics_ToolHelp, Win32_System_IO and Win32_System_Ioctl, for the Job Object,
#     thread suspension and process lists of the SSH test servers and their native helper (`fixture_job.rs`), a
#     named pipe in the regular-file test, pipe reads and writes in the agent fixture, and the directory junction of
#     the URL tests; they are named here as well as in Cargo.toml's Windows
#     dev-dependencies, because `cargo check --lib --profile test`, which scripts/windows_lane_check.py runs, does
#     not enable dev-dependency features.
CANDIDATE_WINDOWS_FEATURES = (
    "Win32_Networking_WinHttp",
    "Win32_Networking_WinSock",
    "Win32_Security",
    "Win32_Security_Authorization",
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
)

# Cargo resolves a dependency path against the directory of the manifest that
# names it, lexically, without following symlinks. The prepared manifest is a
# copy in the destination, so each relative dependency path in it is anchored
# at the core. The libraries under crates/ are not linked into the destination:
# reached at their own location, their relative paths (../../../git2-rs)
# resolve exactly as they do in the core.
DEPENDENCY_SECTION = re.compile(r"\[(?:.+\.)?(?:dev-|build-)?dependencies(?:\..+)?\]")
DEPENDENCY_PATH = re.compile(r'(\bpath\s*=\s*)"([^"]*)"')


def replace_once(text: str, old: str, new: str) -> str:
    if text.count(old) != 1:
        raise SystemExit(f"the core manifest no longer has exactly one {old!r}")
    return text.replace(old, new)


def anchor_dependency_paths(manifest: str, core: Path) -> str:
    def anchor(match: re.Match) -> str:
        path = os.path.normpath(core / match.group(2))
        if not (Path(path) / "Cargo.toml").is_file():
            raise SystemExit(f"path dependency has no manifest: {path}")
        return match.group(1) + json.dumps(path)

    lines = []
    in_dependencies = False
    for line in manifest.splitlines(keepends=True):
        stripped = line.strip()
        if stripped.startswith("["):
            header = stripped.split("#", 1)[0].strip()
            in_dependencies = DEPENDENCY_SECTION.fullmatch(header) is not None
        elif in_dependencies and not stripped.startswith("#"):
            line = DEPENDENCY_PATH.sub(anchor, line)
        lines.append(line)
    return "".join(lines)


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("destination", type=Path)
args = parser.parse_args()
core = Path(__file__).resolve().parents[2]
root = core.parent
destination = args.destination.resolve()
if destination.exists() or destination.is_relative_to(root):
    raise SystemExit("use a new directory outside the workspace")
destination.mkdir(parents=True)
candidate_source = core / "src" / "protocol" / "candidate_generated.rs"
production_source = core / "src" / "protocol" / "generated.rs"
if not candidate_source.is_file() or not production_source.is_file():
    raise SystemExit("candidate and production generated protocol sources are required")
placement_metadata = {
    "candidate_source": str(candidate_source.resolve()),
    "candidate_sha256": hashlib.sha256(candidate_source.read_bytes()).hexdigest(),
    "production_source": str(production_source.resolve()),
    "production_sha256": hashlib.sha256(production_source.read_bytes()).hexdigest(),
    "cfg": "gwz_transport_candidate",
    # Nothing is beside the destination: tests/protocol.rs reads taut from here.
    "taut_source": str(root / "taut" / "src"),
}
for name in ["src", "build.rs", "build_support", "protocol", "tests", "README.md", "LICENSE"]:
    (destination / name).symlink_to(core / name, target_is_directory=(core / name).is_dir())
manifest = anchor_dependency_paths((core / "Cargo.toml").read_text(), core)
# The libraries are path dependencies at their own location, not members here.
manifest = replace_once(manifest, 'members = ["crates/*"]\n', '')
extra = '\n'.join([
    'gwz-transport = { path = ' + json.dumps(str(root / "gwz-transport")) + ' }',
    'gwz-sspi = { path = ' + json.dumps(str(root / "gwz-sspi")) + ' }',
    'base64 = "=0.22.1"', 'socket2 = "=0.6.4"',
    'ssh2 = "=0.9.6"', 'libssh2-sys = "=0.3.3"',
    'tokio = { version = "=1.53.1", features = ["rt", "net", "time", "sync", "macros", "process", "io-util"] }',
    'tokio-util = { version = "=0.7.19", features = ["rt"] }',
    'hyper = { version = "=1.11.1", features = ["client", "server", "http1"] }',
    'hyper-util = { version = "=0.1.20", features = ["tokio"] }',
    'http-body-util = "=0.1.5"', 'bytes = "=1.11.1"',
    'native-tls = "=0.2.18"', 'tokio-native-tls = "=0.3.1"',

])
manifest = replace_once(
    manifest,
    '"Win32_Globalization"',
    ", ".join(['"Win32_Globalization"', *(json.dumps(name) for name in CANDIDATE_WINDOWS_FEATURES)]),
)
manifest = replace_once(manifest, '[dependencies]\n', '[dependencies]\n' + extra + '\n')
# The HTTPS test fixture's TLS servers are rustls over the ring provider (pure Rust apart from ring's
# own assembly): a native-tls server loads its identity from the platform key store, which Windows
# denies under a key-based OpenSSH logon (step 0.5b). Test-only; the product's TLS stays native-tls.
dev_extra = '\n'.join([
    'pyo3 = { version = "=0.28.3", features = ["auto-initialize"] }',
    'rustls = { version = "=0.23.45", default-features = false, features = ["ring", "std", "tls12"] }',
    'tokio-rustls = { version = "=0.26.5", default-features = false, features = ["ring", "tls12"] }',
])
manifest = replace_once(manifest, '[dev-dependencies]\n', '[dev-dependencies]\n' + dev_extra + '\n')
# openssl-probe 0.1 is the crate git2 runs at its start; the transport resolves
# OpenSSL's default verify paths with it (src/git/endpoint/verify_paths.rs).
manifest += '\n[target.\'cfg(not(any(windows, target_vendor = "apple")))\'.dependencies]\nopenssl-probe = "=0.1.6"\n'
# gwz-core names the git2-rs fork (gwz-git2) directly, with its vendored
# libgit2, so the candidate needs no [patch] and no feature of its own.
# The placement guide's example (docs/TransportPlacement.md), compiled with
# gwz_core as an extern crate; ordinary builds never see this target.
manifest += '\n[[test]]\nname = "transport_placement_guide"\npath = "tests/transport_backend/guide_test.rs"\n'
(destination / "Cargo.toml").write_text(manifest)
shutil.copyfile(core / "Cargo.lock", destination / "Cargo.lock")
(destination / "placement-candidate.json").write_text(
    json.dumps(placement_metadata, indent=2) + "\n"
)
print(destination)
print("Candidate protocol source: " + placement_metadata["candidate_source"])
print("Candidate protocol sha256: " + placement_metadata["candidate_sha256"])
print("Use RUSTFLAGS='--cfg gwz_transport_candidate' and an external CARGO_TARGET_DIR.")

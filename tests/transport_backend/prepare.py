#!/usr/bin/env python3
"""Prepare a local full-core candidate; never changes production manifests."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil

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
    'base64 = "=0.22.1"', 'socket2 = "=0.6.4"',
    'ssh2 = "=0.9.6"', 'libssh2-sys = "=0.3.3"',
    'tokio = { version = "=1.53.1", features = ["rt", "net", "time", "sync", "macros", "process", "io-util"] }',
    'tokio-util = { version = "=0.7.19", features = ["rt"] }',
    'hyper = { version = "=1.11.1", features = ["client", "server", "http1"] }',
    'hyper-util = { version = "=0.1.20", features = ["tokio"] }',
    'http-body-util = "=0.1.5"', 'bytes = "=1.11.1"',
    'native-tls = "=0.2.18"', 'tokio-native-tls = "=0.3.1"',

])
manifest = replace_once(manifest, '[dependencies]\n', '[dependencies]\n' + extra + '\n')
manifest = replace_once(manifest, '[dev-dependencies]\n', '[dev-dependencies]\npyo3 = { version = "=0.28.3", features = ["auto-initialize"] }\n')
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

#!/usr/bin/env python3
"""Prepare a local full-core candidate; never changes production manifests."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("destination", type=Path)
args = parser.parse_args()
core = Path(__file__).resolve().parents[2]
root = core.parent
destination = args.destination.resolve()
if destination.exists() or destination.is_relative_to(root):
    raise SystemExit("use a new directory outside the workspace")
destination.mkdir(parents=True)
candidate_source = core / "tests" / "transport_consumer" / "candidate" / "candidate_generated.rs"
production_source = core / "src" / "protocol" / "generated.rs"
if not candidate_source.is_file() or not production_source.is_file():
    raise SystemExit("candidate and production generated protocol sources are required")
placement_metadata = {
    "candidate_source": str(candidate_source.resolve()),
    "candidate_sha256": hashlib.sha256(candidate_source.read_bytes()).hexdigest(),
    "production_source": str(production_source.resolve()),
    "production_sha256": hashlib.sha256(production_source.read_bytes()).hexdigest(),
    "cfg": "gwz_transport_candidate",
}
for name in ["src", "crates", "build.rs", "build_support", "protocol", "tests", "README.md", "LICENSE"]:
    (destination / name).symlink_to(core / name, target_is_directory=(core / name).is_dir())
manifest = (core / "Cargo.toml").read_text()
extra = '\n'.join([
    'gwz-transport = { path = ' + json.dumps(str(root / "gwz-transport")) + ' }',
    'base64 = "=0.22.1"', 'socket2 = "=0.6.4"',
    'ssh2 = "=0.9.6"', 'libssh2-sys = "=0.3.3"',
])
manifest = manifest.replace('[dependencies]\n', '[dependencies]\n' + extra + '\n', 1)
manifest = manifest.replace('[dev-dependencies]\n', '[dev-dependencies]\npyo3 = { version = "=0.28.3", features = ["auto-initialize"] }\n', 1)
manifest += '\n[patch.crates-io]\n'
for name, path in [("git2", root / "git2-rs"), ("libgit2-sys", root / "git2-rs/libgit2-sys")]:
    manifest += name + ' = { path = ' + json.dumps(str(path)) + ' }\n'
manifest = manifest.replace('features = ["https", "ssh", "unstable-sha256"]',
                            'features = ["https", "ssh", "unstable-sha256", "vendored-libgit2"]')
(destination / "Cargo.toml").write_text(manifest)
shutil.copyfile(core / "Cargo.lock", destination / "Cargo.lock")
(destination / "placement-candidate.json").write_text(
    json.dumps(placement_metadata, indent=2) + "\n"
)
print(destination)
print("Candidate protocol source: " + placement_metadata["candidate_source"])
print("Candidate protocol sha256: " + placement_metadata["candidate_sha256"])
print("Use RUSTFLAGS='--cfg gwz_transport_candidate' and an external CARGO_TARGET_DIR.")

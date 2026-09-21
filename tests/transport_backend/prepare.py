#!/usr/bin/env python3
"""Prepare a local full-core candidate; never changes production manifests."""
import argparse
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
for name in ["src", "crates", "build.rs", "build_support", "protocol", "tests", "README.md", "LICENSE"]:
    (destination / name).symlink_to(core / name, target_is_directory=(core / name).is_dir())
manifest = (core / "Cargo.toml").read_text()
extra = '\n'.join([
    'gwz-transport = { path = ' + json.dumps(str(root / "gwz-transport")) + ' }',
    'base64 = "=0.22.1"', 'socket2 = "=0.6.4"',
    'ssh2 = "=0.9.6"', 'libssh2-sys = "=0.3.3"',
])
manifest = manifest.replace('[dependencies]\n', '[dependencies]\n' + extra + '\n', 1)
manifest += '\n[patch.crates-io]\n'
for name, path in [("git2", root / "git2-rs"), ("libgit2-sys", root / "git2-rs/libgit2-sys")]:
    manifest += name + ' = { path = ' + json.dumps(str(path)) + ' }\n'
manifest = manifest.replace('features = ["https", "ssh", "unstable-sha256"]',
                            'features = ["https", "ssh", "unstable-sha256", "vendored-libgit2"]')
(destination / "Cargo.toml").write_text(manifest)
shutil.copyfile(core / "Cargo.lock", destination / "Cargo.lock")
print(destination)
print("Use RUSTFLAGS='--cfg gwz_transport_candidate' and an external CARGO_TARGET_DIR.")

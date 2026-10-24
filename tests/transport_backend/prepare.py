#!/usr/bin/env python3
"""Prepare a local full-core candidate tree; never changes production manifests.

The transport's dependencies, the `windows-sys` features it needs and the per-platform TLS tables are in gwz-core's own
Cargo.toml, behind `cfg(gwz_transport_candidate)` (dev-docs/GwzV110RemainingPlan.md C1), so a build with
RUSTFLAGS='--cfg gwz_transport_candidate' needs no generated manifest for them. What this script still makes, and why:

- a copy of Cargo.toml in a directory outside the workspace, with its relative path dependencies anchored at the core and
  `crates/*` no longer workspace members (the libraries are path dependencies at their own location);
- the `transport_placement_guide` test target, which no cfg can gate in a manifest and the core's manifest must not name.
  The guide documents the Cli placement, the 1.2.0 session host's transport entry, so its test compiles to nothing
  unless `gwz_session_candidate` is set (S7.1 step C3a; D6 of dev-docs/GwzV110S71ProtocolProposal-DRAFT.md);
- `placement-candidate.json`, the digests of the candidate and production generated protocol sources and the taut source
  directory the protocol tests read from outside the workspace. The ordinary source already carries the fields 1.1.0
  needs (step C2); the candidate source is the session candidate's, which adds the 1.2.0 placement projection;
- symlinks to the core's sources, tests, workflows and scripts, and a copy of Cargo.lock (step C4 retires the tree).

--refresh rewrites a directory this script made, in place, instead of refusing it. Cargo keys gwz-core's artifacts and
incremental state by the package's path, so a run that prepares a fresh directory each time compiles gwz-core whole; one
that refreshes the same directory, with the same external target directory, rebuilds only what changed.
"""
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
parser.add_argument(
    "--refresh",
    action="store_true",
    help="rewrite a directory this script prepared, in place, instead of refusing an existing destination",
)
args = parser.parse_args()
core = Path(__file__).resolve().parents[2]
root = core.parent
destination = args.destination.resolve()
LINKED = ["src", "build.rs", "build_support", "protocol", "tests", ".github", "scripts", "README.md", "LICENSE"]
WRITTEN = ["Cargo.toml", "Cargo.lock", "placement-candidate.json"]
if destination.is_relative_to(root):
    raise SystemExit("use a new directory outside the workspace")
if args.refresh:
    if not destination.is_dir():
        raise SystemExit(f"--refresh needs a directory this script prepared: {destination} is not a directory")
    # Only what this script writes may be replaced; anything else means the directory is not a prepared one.
    foreign = sorted(
        entry.name
        for entry in destination.iterdir()
        if not ((entry.name in LINKED and entry.is_symlink()) or (entry.name in WRITTEN and entry.is_file()))
    )
    if foreign or not (destination / "placement-candidate.json").is_file():
        raise SystemExit(
            f"--refresh refuses {destination}: it is not a directory this script prepared "
            f"(unexpected entries: {', '.join(foreign) or 'none'}; placement-candidate.json is required)"
        )
    for name in LINKED + WRITTEN:
        entry = destination / name
        if entry.is_symlink() and entry.is_dir() and os.name == "nt":
            # Windows removes a directory symlink as a directory.
            entry.rmdir()
        elif entry.is_symlink() or entry.exists():
            entry.unlink()
elif destination.exists():
    raise SystemExit("use a new directory outside the workspace")
else:
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
# `.github` and `scripts` are what tests/publish_workflow.rs reads, through CARGO_MANIFEST_DIR (step 5.4).
for name in LINKED:
    (destination / name).symlink_to(core / name, target_is_directory=(core / name).is_dir())
manifest = anchor_dependency_paths((core / "Cargo.toml").read_text(), core)
# The libraries are path dependencies at their own location, not members here.
manifest = replace_once(manifest, 'members = ["crates/*"]\n', '')
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

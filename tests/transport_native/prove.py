#!/usr/bin/env python3
"""Qualify a pinned git2 binding patch without changing a production dependency."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parent


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def checked_digest(path: Path, expected: str) -> None:
    actual = digest(path)
    if actual != expected:
        raise SystemExit(f"digest mismatch for {path.name}: expected {expected}, got {actual}")


def normalized_lock(text: str, patched: bool) -> str:
    blocks = text.split("[[package]]")
    found = 0
    for index, block in enumerate(blocks[1:], 1):
        if '\nname = "git2"\n' not in block:
            continue
        found += 1
        if '\nversion = "0.21.0"\n' not in block:
            raise SystemExit("unexpected git2 version in lock")
        source = [line for line in block.splitlines() if line.startswith(('source =', 'checksum ='))]
        if patched and source:
            raise SystemExit("patched git2 still has registry provenance")
        if not patched and len(source) != 2:
            raise SystemExit("stock git2 lock lacks registry provenance")
        blocks[index] = '\n'.join(
            line for line in block.splitlines() if not line.startswith(('source =', 'checksum ='))
        ) + '\n'
    if found != 1:
        raise SystemExit("expected exactly one git2 package")
    return '[[package]]'.join(blocks)


def verify_lock(original: str, patched: str) -> None:
    if normalized_lock(original, False) != normalized_lock(patched, True):
        raise SystemExit('dependency graph changed beyond the explicit git2 source patch')


def extract(archive: Path, destination: Path) -> Path:
    with tarfile.open(archive, 'r:gz') as source:
        for entry in source.getmembers():
            path = PurePosixPath(entry.name)
            if (path.is_absolute() or '..' in path.parts
                    or not path.parts or path.parts[0] != 'git2-0.21.0'
                    or not (entry.isfile() or entry.isdir())):
                raise SystemExit(f"unsafe archive entry: {entry.name}")
        source.extractall(destination)
    return destination / 'git2-0.21.0'


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--git2-archive', type=Path, required=True)
    parser.add_argument('--toolchain', default='1.95.0',
                        help='rustup toolchain selector (default: 1.95.0)')
    args = parser.parse_args()
    pin = json.loads((ROOT / 'binding-pin.json').read_text())
    archive = args.git2_archive.resolve(strict=True)
    patch = ROOT / 'patches' / 'git2-per-remote.patch'
    checked_digest(archive, pin['upstream_archive_sha256'])
    checked_digest(patch, pin['patch_sha256'])
    original_lock = (ROOT / 'Cargo.lock').read_text()
    checked_digest(ROOT / 'Cargo.lock', pin['lock_sha256'])
    with tempfile.TemporaryDirectory(prefix='gwz-native-binding-') as temporary:
        work = Path(temporary)
        owner = extract(archive, work)
        for name, expected in pin['original_files'].items():
            checked_digest(owner / name, expected)
        subprocess.run(['git', 'apply', '--check', str(patch)], cwd=owner, check=True)
        subprocess.run(['git', 'apply', str(patch)], cwd=owner, check=True)
        for name, expected in pin['patched_files'].items():
            checked_digest(owner / name, expected)
        fixture = work / 'proof'
        shutil.copytree(ROOT, fixture, ignore=shutil.ignore_patterns('target', '__pycache__'))
        config = 'patch.crates-io.git2.path=' + json.dumps(str(owner))
        cargo = ['cargo', '+' + args.toolchain]
        env = os.environ.copy()
        env['CARGO_TARGET_DIR'] = str(ROOT / 'target' / 'qualified')
        command = cargo + ['update', '--offline', '-p', 'git2', '--config', config]
        subprocess.run(command, cwd=fixture, env=env, check=True)
        patched_lock = (fixture / 'Cargo.lock').read_text()
        verify_lock(original_lock, patched_lock)
        command = cargo + ['test', '--offline', '--locked', '--config', config]
        print('upstream_git2=0.21.0 archive_sha256=' + pin['upstream_archive_sha256'], flush=True)
        print('patch_sha256=' + pin['patch_sha256'], flush=True)
        print('command=' + ' '.join(command), flush=True)
        subprocess.run(command, cwd=fixture, env=env, check=True)


if __name__ == '__main__':
    main()

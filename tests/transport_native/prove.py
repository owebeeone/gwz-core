#!/usr/bin/env python3
"""Qualify a pinned git2 binding patch without changing a production dependency."""
from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import stat
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parent
RELEASE = "dffaf272eb0e62ac15b74283c4e488252db9afc3"


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


def copy_member(source: Path, destination: Path, pin: dict) -> Path:
    """Admit exact release files plus the reviewed patch and native dependency edge."""
    archive = subprocess.check_output(['git', '--no-replace-objects', 'archive', '--format=tar', RELEASE], cwd=source)
    expected = {}
    with tarfile.open(fileobj=io.BytesIO(archive)) as tree:
        for entry in tree:
            if entry.isdir():
                continue
            if entry.issym():
                content = os.fsencode(entry.linkname)
            elif entry.isfile():
                content = tree.extractfile(entry).read()
            else:
                raise SystemExit(f'unsupported release entry: {entry.name}')
            expected[entry.name] = (entry, content)
    seen = set()
    for directory, dirs, files in os.walk(source, followlinks=False):
        relative = Path(directory).relative_to(source)
        for name in list(dirs):
            path = relative / name
            if str(path) in ('.git', 'target', 'libgit2-sys/libgit2'):
                dirs.remove(name)
            elif (source / path).is_symlink():
                dirs.remove(name)
                files.append(name)
        seen.update(str(relative / name) for name in files if str(relative / name) != '.git')
    if seen != set(expected):
        raise SystemExit(f'member file set drift: {sorted(seen ^ set(expected))}')
    admitted = []
    for name, (entry, original) in expected.items():
        path = source / name
        mode = path.lstat().st_mode
        if entry.issym():
            if not stat.S_ISLNK(mode) or os.fsencode(os.readlink(path)) != original:
                raise SystemExit(f'member symlink drift: {name}')
            content = original
        else:
            if not stat.S_ISREG(mode) or bool(mode & 0o111) != bool(entry.mode & 0o111):
                raise SystemExit(f'member file type/mode drift: {name}')
            content = path.read_bytes()
            if name == 'Cargo.toml':
                old = b'libgit2-sys = { path = "libgit2-sys", version = "0.18.4" }'
                if original.count(old) != 1:
                    raise SystemExit('unexpected release native dependency')
                original = original.replace(old, b'libgit2-sys = "=0.18.8"')
            required = pin['patched_files'].get(name, hashlib.sha256(original).hexdigest())
            if hashlib.sha256(content).hexdigest() != required:
                raise SystemExit(f'member source drift: {name}')
        admitted.append((name, entry, content))
    for name, entry, content in admitted:
        output = destination / name
        output.parent.mkdir(parents=True, exist_ok=True)
        if entry.issym():
            output.symlink_to(os.fsdecode(content))
        else:
            output.write_bytes(content)
            output.chmod(entry.mode)
    return destination


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    inputs = parser.add_mutually_exclusive_group(required=True)
    inputs.add_argument('--git2-archive', type=Path, help='exact upstream 0.21.0 crate archive')
    inputs.add_argument('--git2-source', type=Path, help='qualified patched member checkout')
    parser.add_argument('--toolchain', default='1.95.0',
                        help='rustup toolchain selector (default: 1.95.0)')
    args = parser.parse_args()
    pin = json.loads((ROOT / 'binding-pin.json').read_text())
    patch = ROOT / 'patches' / 'git2-per-remote.patch'
    checked_digest(patch, pin['patch_sha256'])
    original_lock = (ROOT / 'Cargo.lock').read_text()
    checked_digest(ROOT / 'Cargo.lock', pin['lock_sha256'])
    with tempfile.TemporaryDirectory(prefix='gwz-native-binding-') as temporary:
        work = Path(temporary)
        if args.git2_source:
            owner = copy_member(args.git2_source.resolve(strict=True), work / 'git2', pin)
            print('member_release=' + RELEASE, flush=True)
        else:
            archive = args.git2_archive.resolve(strict=True)
            checked_digest(archive, pin['upstream_archive_sha256'])
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
        print('upstream_git2=0.21.0 reference_archive_sha256=' + pin['upstream_archive_sha256'], flush=True)
        print('patch_sha256=' + pin['patch_sha256'], flush=True)
        print('command=' + ' '.join(command), flush=True)
        subprocess.run(command, cwd=fixture, env=env, check=True)


if __name__ == '__main__':
    main()

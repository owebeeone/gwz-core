#!/usr/bin/env python3
"""Qualify pinned Rust bindings and the local-fetch C correction in isolation."""
from __future__ import annotations

import argparse
import hashlib
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


def _temporary_directory(prefix: str):
    if os.name == 'nt':
        root = Path('D:/gwz-tests')
        root.mkdir(parents=True, exist_ok=True)
        return tempfile.TemporaryDirectory(prefix=prefix, dir=root)
    return tempfile.TemporaryDirectory(prefix=prefix)


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def checked_digest(path: Path, expected: str) -> None:
    actual = digest(path)
    if actual != expected:
        raise SystemExit(f"digest mismatch for {path.name}: expected {expected}, got {actual}")


def normalized_lock(text: str, patched: bool, native: bool = False) -> str:
    blocks = text.split("[[package]]")
    versions = {"git2": "0.21.0"}
    if native:
        versions["libgit2-sys"] = "0.18.8+1.9.7"
    found = []
    for index, block in enumerate(blocks[1:], 1):
        name = next((name for name in versions if '\nname = "' + name + '"\n' in block), None)
        if name is None:
            continue
        found.append(name)
        if '\nversion = "' + versions[name] + '"\n' not in block:
            raise SystemExit("unexpected qualified package version in lock")
        source = [line for line in block.splitlines() if line.startswith(('source =', 'checksum ='))]
        if patched and source:
            raise SystemExit("patched git2 still has registry provenance")
        if not patched and len(source) != 2:
            raise SystemExit("stock git2 lock lacks registry provenance")
        blocks[index] = '\n'.join(
            line for line in block.splitlines() if not line.startswith(('source =', 'checksum ='))
        ) + '\n'
    if sorted(found) != sorted(versions):
        raise SystemExit("expected exactly one of each qualified package")
    return '[[package]]'.join(blocks)


def verify_lock(original: str, patched: str, native: bool = False) -> None:
    if normalized_lock(original, False, native) != normalized_lock(patched, True, native):
        raise SystemExit('dependency graph changed beyond the explicit source patches')


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


def read_tree(source: Path, revision: str) -> dict:
    """Read immutable blobs and modes, never export-filtered archive contents."""
    git = ['git', '--no-replace-objects']
    listing = subprocess.check_output(git + ['ls-tree', '-rz', revision], cwd=source)
    entries = []
    for record in listing.split(b'\0'):
        if not record:
            continue
        header, raw_name = record.split(b'\t', 1)
        mode, kind, oid = header.decode('ascii').split()
        name = os.fsdecode(raw_name)
        if (mode, kind, name) == ('160000', 'commit', 'libgit2-sys/libgit2'):
            continue
        if kind != 'blob' or mode not in ('100644', '100755', '120000'):
            raise SystemExit(f'unsupported release entry: {name}')
        entries.append((name, int(mode, 8), oid))
    if not entries:
        raise SystemExit('empty source tree')
    objects = subprocess.check_output(git + ['cat-file', '--batch'], cwd=source,
                                     input=''.join(oid + '\n' for _, _, oid in entries).encode())
    expected = {}
    offset = 0
    for name, mode, oid in entries:
        end = objects.index(b'\n', offset)
        actual_oid, kind, raw_size = objects[offset:end].decode('ascii').split()
        size = int(raw_size)
        if actual_oid != oid or kind != 'blob' or size < 0:
            raise SystemExit(f'invalid Git object response: {name}')
        start = end + 1
        content = objects[start:start + size]
        if len(content) != size or objects[start + size:start + size + 1] != b'\n':
            raise SystemExit(f'truncated Git object response: {name}')
        expected[name] = (mode, content)
        offset = start + size + 1
    if offset != len(objects):
        raise SystemExit('unexpected Git object response suffix')
    return expected


def check_revision(source: Path, revision: str, ref: str = 'HEAD') -> None:
    actual = subprocess.check_output(
        ['git', '--no-replace-objects', 'rev-parse', ref], cwd=source).decode().strip()
    if actual != revision:
        raise SystemExit(f'source revision mismatch: {ref}: {actual} != {revision}')


def verify_copy(source: Path, destination: Path, expected: dict,
                excluded: tuple = ('.git', 'target')) -> Path:
    seen = set()
    for directory, dirs, files in os.walk(source, followlinks=False):
        relative = Path(directory).relative_to(source)
        for name in list(dirs):
            path = relative / name
            if path.as_posix() in excluded:
                dirs.remove(name)
            elif (source / path).is_symlink():
                dirs.remove(name)
                files.append(name)
        for name in files:
            path = relative / name
            if path.as_posix() not in excluded:
                seen.add(path.as_posix())
    if seen != set(expected):
        raise SystemExit(f'member file set drift: {sorted(seen ^ set(expected))}')
    admitted = []
    for name, (entry, original) in expected.items():
        path = source / name
        mode = path.lstat().st_mode
        if stat.S_ISLNK(entry):
            if not stat.S_ISLNK(mode) or os.fsencode(os.readlink(path)) != original:
                raise SystemExit(f'member symlink drift: {name}')
            content = original
        else:
            # Windows reports suffix-derived executable bits, not Git's POSIX
            # permission bit. Types, source bytes and symlink targets still
            # require exact admission there; modes remain Git-tree metadata.
            executable_drift = os.name != 'nt' and bool(mode & 0o111) != bool(entry & 0o111)
            if not stat.S_ISREG(mode) or executable_drift:
                raise SystemExit(f'member file type/mode drift: {name}')
            content = path.read_bytes()
            if content != original:
                raise SystemExit(f'member source drift: {name}')
        admitted.append((name, entry, content))
    for name, entry, content in admitted:
        output = destination / name
        output.parent.mkdir(parents=True, exist_ok=True)
        if stat.S_ISLNK(entry):
            output.symlink_to(os.fsdecode(content))
        else:
            output.write_bytes(content)
            output.chmod(entry & 0o777)
    return destination


def copy_member(source: Path, destination: Path, pin: dict) -> Path:
    """Admit release + binding + exact sys baseline + pinned native subtree."""
    expected = read_tree(source, RELEASE)
    native = pin.get('native')
    excluded = ('.git', 'target', 'libgit2-sys/libgit2')
    if native:
        baseline = read_tree(source, native['sys_revision'])
        expected = {name: entry for name, entry in expected.items()
                    if not name.startswith('libgit2-sys/')}
        expected.update({name: entry for name, entry in baseline.items()
                         if name.startswith('libgit2-sys/')})
        check_revision(source, native['c_revision'], 'HEAD:libgit2-sys/libgit2')
        child = source / 'libgit2-sys/libgit2'
        check_revision(child, native['c_revision'])
        expected.update({'libgit2-sys/libgit2/' + name: entry
                         for name, entry in read_tree(child, native['c_revision']).items()})
        excluded = ('.git', 'target', 'libgit2-sys/libgit2/.git')
        mode, content = expected['.gitmodules']
        expected['.gitmodules'] = (mode, content.replace(
            b'https://github.com/libgit2/libgit2', b'https://github.com/owebeeone/libgit2'))
    mode, content = expected['Cargo.toml']
    old = b'libgit2-sys = { path = "libgit2-sys", version = "0.18.4" }'
    if content.count(old) != 1:
        raise SystemExit('unexpected release native dependency')
    new = (b'libgit2-sys = { path = "libgit2-sys", version = "=0.18.8" }'
           if native else b'libgit2-sys = "=0.18.8"')
    expected['Cargo.toml'] = (mode, content.replace(old, new))
    for name, sha in pin['patched_files'].items():
        # Binding bytes are identified by the pre-existing reviewed digest.
        path = source / name
        content = path.read_bytes()
        if hashlib.sha256(content).hexdigest() != sha:
            raise SystemExit(f'member binding drift: {name}')
        expected[name] = (expected[name][0], content)
    return verify_copy(source, destination, expected, excluded)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    inputs = parser.add_mutually_exclusive_group(required=True)
    inputs.add_argument('--git2-archive', type=Path, help='exact upstream 0.21.0 crate archive')
    inputs.add_argument('--git2-source', type=Path, help='qualified member checkout with initialized pinned C submodule')
    parser.add_argument('--toolchain', default='1.95.0',
                        help='rustup toolchain selector (default: 1.95.0)')
    args = parser.parse_args()
    pin = json.loads((ROOT / 'binding-pin.json').read_text())
    patch = ROOT / 'patches' / 'git2-per-remote.patch'
    checked_digest(patch, pin['patch_sha256'])
    original_lock = (ROOT / 'Cargo.lock').read_text()
    checked_digest(ROOT / 'Cargo.lock', pin['lock_sha256'])
    with _temporary_directory(prefix='gwz-native-binding-') as temporary:
        work = Path(temporary)
        if args.git2_source:
            owner = copy_member(args.git2_source.resolve(strict=True), work / 'git2', pin)
            print('member_release=' + RELEASE, flush=True)
            print('native_source=' + json.dumps(pin.get('native')), flush=True)
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
        native = bool(args.git2_source and pin.get('native'))
        configs = ['--config', 'patch.crates-io.git2.path=' + json.dumps(str(owner))]
        if native:
            configs += ['--config', 'patch.crates-io.libgit2-sys.path=' +
                        json.dumps(str(owner / 'libgit2-sys'))]
        cargo = ['cargo', '+' + args.toolchain]
        env = os.environ.copy()
        env['CARGO_TARGET_DIR'] = str(ROOT / 'target' / 'qualified')
        env['GWZ_NATIVE_FIX'] = '1' if native else '0'
        command = cargo + ['update', '--offline', '-p', 'git2'] + configs
        subprocess.run(command, cwd=fixture, env=env, check=True)
        patched_lock = (fixture / 'Cargo.lock').read_text()
        verify_lock(original_lock, patched_lock, native)
        command = cargo + ['test', '--offline', '--locked'] + configs
        print('upstream_git2=0.21.0 reference_archive_sha256=' + pin['upstream_archive_sha256'], flush=True)
        print('patch_sha256=' + pin['patch_sha256'], flush=True)
        print('command=' + ' '.join(command), flush=True)
        subprocess.run(command, cwd=fixture, env=env, check=True)


if __name__ == '__main__':
    main()

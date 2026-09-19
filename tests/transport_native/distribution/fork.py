#!/usr/bin/env python3
"""Prepare and qualify a local gwz-git2 distribution candidate; never publish."""
from __future__ import annotations

import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('native_proof', ROOT / 'prove.py')
proof = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(proof)
NAME = 'gwz-git2'
VERSION = '0.21.0-gwz.1'


def manifest(source: str) -> str:
    match = re.search(r'(?ms)^\[package\]\n(.*?)(?=^\[|\Z)', source)
    if match is None:
        raise ValueError('missing upstream package')
    table = match.group(1)
    if not re.search(r'^name = "git2"$', table, re.M) or not re.search(r'^version = "0.21.0"$', table, re.M):
        raise ValueError('unexpected upstream package identity')
    table = table.replace('name = "git2"', f'name = "{NAME}"', 1)
    table = table.replace('version = "0.21.0"', f'version = "{VERSION}"', 1)
    table = re.sub(r'^documentation = .*\n', '', table, flags=re.M)
    table = re.sub(r'^repository = .*$', 'repository = "https://github.com/owebeeone/gwz-core"', table, flags=re.M)
    table = re.sub(r'^readme = .*$', 'readme = "README-GWZ.md"', table, flags=re.M)
    source = source[:match.start(1)] + table + source[match.end(1):]
    return source + '\n[package.metadata.gwz-fork]\nupstream = "git2"\nupstream-version = "0.21.0"\nprovenance = "GWZ-PROVENANCE.json"\n'


def fixture_manifest(source: str) -> str:
    old = 'git2 = { version = "=0.21.0",'
    if source.count(old) != 1:
        raise ValueError('expected exactly one stock git2 dependency')
    return source.replace(old, f'git2 = {{ package = "{NAME}", version = "={VERSION}",', 1)


def run(command: list[str], directory: Path, env: dict[str, str]) -> None:
    subprocess.run(command, cwd=directory, env=env, check=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--git2-archive', type=Path, required=True, help='pinned upstream git2 0.21.0 crate archive')
    parser.add_argument('--output', type=Path, required=True, help='new directory for qualified source and crate archive; must not exist')
    parser.add_argument('--fetch', action='store_true', help='allow Cargo to fetch packaging dependencies first (default: offline)')
    args = parser.parse_args()
    output = args.output.resolve()
    if output.exists():
        raise SystemExit('output already exists; choose a new directory')
    pin = json.loads((ROOT / 'binding-pin.json').read_text())
    package_pin = json.loads(Path(__file__).with_name('package-pin.json').read_text())
    package_lock = Path(__file__).with_name('Cargo.lock')
    proof.checked_digest(package_lock, package_pin['lock_sha256'])
    patch = ROOT / 'patches/git2-per-remote.patch'
    proof.checked_digest(args.git2_archive, pin['upstream_archive_sha256'])
    proof.checked_digest(patch, pin['patch_sha256'])
    proof.checked_digest(ROOT / 'Cargo.lock', pin['lock_sha256'])
    with tempfile.TemporaryDirectory(prefix='gwz-git2-distribution-') as raw:
        work = Path(raw)
        source = proof.extract(args.git2_archive.resolve(), work)
        for name, digest in pin['original_files'].items():
            proof.checked_digest(source / name, digest)
        run(['git', 'apply', '--check', str(patch)], source, os.environ.copy())
        run(['git', 'apply', str(patch)], source, os.environ.copy())
        for name, digest in pin['patched_files'].items():
            proof.checked_digest(source / name, digest)
        (source / 'Cargo.toml').write_text(manifest((source / 'Cargo.toml').read_text()))
        for name in ['Cargo.toml.orig', 'Cargo.lock', '.cargo_vcs_info.json']:
            (source / name).unlink(missing_ok=True)
        shutil.copy2(Path(__file__).with_name('README.md'), source / 'README-GWZ.md')
        (source / 'GWZ-PROVENANCE.json').write_text(json.dumps(pin, indent=2) + '\n')
        shutil.copy2(package_lock, source / 'Cargo.lock')
        fixture = work / 'proof'
        shutil.copytree(ROOT, fixture, ignore=shutil.ignore_patterns('target', '__pycache__'))
        (fixture / 'Cargo.toml').write_text(fixture_manifest((fixture / 'Cargo.toml').read_text()))
        config = 'patch.crates-io.gwz-git2.path=' + json.dumps(str(source))
        env = os.environ.copy()
        env['CARGO_TARGET_DIR'] = str(ROOT / 'target/distribution')
        cargo = ['cargo', '+1.95.0']
        run(cargo + ['update', '--offline', '-p', 'git2', '--config', config], fixture, env)
        locked = (fixture / 'Cargo.lock').read_text()
        translated = locked.replace('"gwz-git2"', '"git2"').replace('"0.21.0-gwz.1"', '"0.21.0"')
        proof.verify_lock((ROOT / 'Cargo.lock').read_text(), translated)
        run(cargo + ['test', '--offline', '--locked', '--config', config], fixture, env)
        if args.fetch:
            run(cargo + ['fetch', '--locked'], source, env)
        run(cargo + ['package', '--offline', '--locked', '--allow-dirty', '--no-verify'], source, env)
        archive = Path(env['CARGO_TARGET_DIR']) / 'package' / f'{NAME}-{VERSION}.crate'
        # Qualify the exact packaged bytes, rather than just the staging tree.
        import tarfile
        with tarfile.open(archive, 'r:gz') as bundle:
            for item in bundle.getmembers():
                path = Path(item.name)
                if path.is_absolute() or '..' in path.parts or not (item.isfile() or item.isdir()):
                    raise SystemExit('unsafe packaged entry')
            bundle.extractall(work / 'packaged')
        packaged = work / 'packaged' / f'{NAME}-{VERSION}'
        for name, digest in pin['patched_files'].items():
            proof.checked_digest(packaged / name, digest)
        packaged_config = 'patch.crates-io.gwz-git2.path=' + json.dumps(str(packaged))
        run(cargo + ['test', '--offline', '--locked', '--config', packaged_config], fixture, env)
        output.mkdir(parents=True, exist_ok=False)
        shutil.copytree(source, output / 'source')
        shutil.copy2(archive, output / archive.name)
        result = {'package': NAME, 'version': VERSION, 'archive_sha256': proof.digest(archive),
                  'upstream_archive_sha256': pin['upstream_archive_sha256'], 'patch_sha256': pin['patch_sha256']}
        (output / 'qualification.json').write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps(result, indent=2))
        print('Qualified local candidate only. No registry publication or production dependency change.')


if __name__ == '__main__':
    main()

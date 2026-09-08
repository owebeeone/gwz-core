#!/usr/bin/env python3
"""Source-policy guard for migrated filesystem consumers; complements Clippy.

This token scan is deliberately conservative, not Rust name/type resolution.
Native I/O is permitted only in filesystem/native.rs and filesystem/native/.
Pass additional migrated files/directories as positional arguments. Legacy
modules are not claimed to be protected until migrated and registered here.
"""
from pathlib import Path
import argparse
import re

ROOT = Path(__file__).resolve().parents[2]
PROTECTED = ('src/filesystem.rs', 'src/filesystem',
             'src/workspace_ops/merge/v1_lifecycle/store/rewrite.rs',
             'src/workspace_ops/merge/v1_lifecycle/checked.rs',
             'src/workspace_ops/merge/v1_lifecycle/store/archive.rs',
             'src/durable_fs.rs',
             'src/verified_write.rs',
             'src/workspace_ops/sync_workspace_boundary.rs',
             'src/checked_artifact/entry.rs',
             'src/checked_artifact/bootstrap/runtime/mod.rs',
             'src/checked_artifact/bootstrap/runtime/paths.rs',
             'src/checked_artifact/bootstrap/runtime/advisory.rs',
             'src/checked_artifact/bootstrap/runtime/catalog_lease.rs',
             'src/checked_artifact/bootstrap/runtime/catalog_lease/alias.rs',
             'src/checked_artifact/bootstrap/runtime/catalog_lease/association.rs',
             'src/checked_artifact/bootstrap/runtime/catalog_lease/target.rs',
             'src/checked_artifact/bootstrap/runtime/catalog_lease/witness.rs',
             'src/git/gitbackend/preservation_image.rs',
             'src/git/gitbackend/preservation.rs',
             'src/git/gitbackend/preservation_root/index_format.rs',
             'src/git/gitbackend/preservation_root/parent.rs',
             'src/git/gitbackend/fake_repository.rs',
             'src/git/gitbackend/fake_repository',
             'src/workspace_ops/merge/record_wire/location.rs',
             'src/workspace_ops/merge/root/v1_rollback.rs',
             'src/workspace_ops/merge/v1_lifecycle/authority/observe/finalization.rs',
             'src/workspace_ops/merge/v1_lifecycle/authority/observe/finalization/publication/live.rs',
             'src/workspace_ops/merge/v1_lifecycle/tests/reverse_rollback')
FORBIDDEN = (
    ('std', 'fs'), ('std', 'os', 'unix', 'fs'),
    ('std', 'os', 'windows', 'fs'), ('cap_std', 'fs'),
    ('std', 'process', 'Command'), ('cap_fs_ext',), ('tempfile',), ('walkdir',), ('libc',), ('windows_sys',),
)
OBSERVATIONS = {'canonicalize', 'exists', 'try_exists', 'is_file', 'is_dir',
                'is_symlink', 'symlink_metadata', 'read_dir', 'read_link'}


def tokens(source):
    """Discard comments/literals while retaining offsets, including raw strings."""
    out = []
    i = 0
    while i < len(source):
        if source.startswith('//', i):
            end = source.find('\n', i)
            i = len(source) if end < 0 else end
        elif source.startswith('/*', i):
            depth = 1
            i += 2
            while i < len(source) and depth:
                if source.startswith('/*', i): depth += 1; i += 2
                elif source.startswith('*/', i): depth -= 1; i += 2
                else: i += 1
        elif (raw := re.match(r'(?:br|cr|r)(#*)"', source[i:])):
            end = source.find('"' + raw[1], i + raw.end())
            i = len(source) if end < 0 else end + 1 + len(raw[1])
        elif source[i] == '"':
            i += 1
            while i < len(source):
                if source[i] == '\\': i += 2
                elif source[i] == '"': i += 1; break
                else: i += 1
        elif (m := re.match(r"(?:b)?'(?:\\.|[^'\\\n])'", source[i:])):
            i += m.end()
        elif (m := re.match(r'[A-Za-z_][A-Za-z_0-9]*|::|[^\s]', source[i:])):
            out.append((m[0], i))
            i += m.end()
        else:
            i += 1
    return out


def imports(items):
    """Expand Rust use trees, including grouped and renamed imports."""
    result = []
    def tree(index, prefix):
        path = list(prefix)
        while index < len(items):
            token = items[index]
            if token == '{':
                index += 1
                while index < len(items) and items[index] != '}':
                    index = tree(index, path)
                    if index < len(items) and items[index] == ',': index += 1
                return index + 1
            if token in {',', '}', ';'}:
                break
            if token == 'as':
                result.append((tuple(path), items[index + 1]))
                return index + 2
            if token != '::': path.append(token)
            index += 1
        if path:
            if path[-1] == 'self': path.pop()
            if path: result.append((tuple(path), path[-1]))
        return index
    tree(0, [])
    return result


def violations(source):
    ts = tokens(source)
    names = [t[0] for t in ts]
    aliases = {}
    found = {}
    def forbidden(path):
        for _ in range(len(aliases) + 1):
            if path and path[0] in aliases:
                replacement = aliases[path[0]] + path[1:]
                if replacement == path: break
                path = replacement
            else: break
        return any(path[:len(root)] == root for root in FORBIDDEN)
    for i, name in enumerate(names):
        if name == 'use':
            end = i + 1
            while end < len(names) and names[end] != ';': end += 1
            for path, alias in imports(names[i+1:end]):
                aliases[alias] = path
                if forbidden(path): found[ts[i][1]] = 'native filesystem import'
    for i, name in enumerate(names):
        path = [name]
        j = i + 1
        while j + 1 < len(names) and names[j] == '::':
            path.append(names[j+1]); j += 2
        if forbidden(tuple(path)):
            found[ts[i][1]] = 'native filesystem access'
        if name == '.' and i + 2 < len(names) and names[i+1] in OBSERVATIONS and names[i+2] == '(':
            found[ts[i][1]] = 'filesystem observation; use FileSystem'
        if name in {'allow', 'expect'} and i and names[i-1] == '[':
            end = i
            while end < len(names) and names[end] != ']': end += 1
            if {'disallowed_methods', 'disallowed_types'} & set(names[i:end]):
                found[ts[i][1]] = 'filesystem lint suppression outside native adapter'
    return [(source.count('\n', 0, offset)+1, message) for offset, message in sorted(found.items())]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('paths', nargs='*', type=Path)
    args = parser.parse_args()
    failures = []
    for entry in args.paths or [ROOT / p for p in PROTECTED]:
        if not entry.exists():
            failures.append(f'{entry}: protected source is missing'); continue
        files = sorted(entry.rglob('*.rs')) if entry.is_dir() else [entry]
        for path in files:
            resolved = path.resolve()
            native = ROOT / 'src/filesystem/native'
            if resolved == native.with_suffix('.rs') or resolved.is_relative_to(native): continue
            for line, message in violations(path.read_text()):
                failures.append(f'{path}:{line}: {message}; use make_filesystem()/FileSystem')
    if failures:
        print('\n'.join(failures)); return 1
    print('Filesystem source boundary passed for migrated scope.')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())

#!/usr/bin/env python3
"""Source-policy guard for migrated filesystem consumers; complements Clippy.

This token scan is deliberately conservative, not Rust name/type resolution.
Native I/O is permitted only in filesystem/native.rs and filesystem/native/.
Pass additional migrated files/directories as positional arguments. Legacy
modules are not claimed to be protected until migrated and registered here.

Step 5.5 (Git boundary plan D-G, section 6; convergence plan R9) adds the same
kind of guard for the single-repository Git work that moved to gwz-git: the
migrated area, named in `gwz_git_boundary.json`, imports no `git2` directly.
Two lists make it up. `zero` names files and directories the moves created or
replaced (`commit_tag.rs`, `reads.rs`, the retired `native_commit` tree): no
production `git2` token may appear in them. `exceptions` names the files the
moves rewrote that still hold `git2` for a row that has not moved (G3, G4, G5,
G10), each with the exact number of tokens, the rows, the reason, the exit and
the date the entry was recorded. A count that grows is a new direct use; a count
that shrinks makes the entry stale and fails until it is lowered (zero: removed),
so the list can only shrink as rows move. Nothing outside those two lists is
banned: this is a ratchet over the migrated area, not a rule about `git2`.
The scan counts the identifier `git2` outside comments, strings and test-only
items (a `use git2::..`, a `git2::` path, `extern crate git2`, an alias import);
like the rest of this file it is a token scan, not name resolution.
"""
from pathlib import Path
import argparse
import json
import re

ROOT = Path(__file__).resolve().parents[2]
PRODUCTION_TREES = ('src/checked_artifact',)
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

# These consumers receive their dependencies from a context or retained handle.
# Compatibility constructors remain outside this migrated scope.
CONTEXT_PROTECTED = (
    'src/workspace_ops/merge/v1_lifecycle',
    'src/workspace_ops/merge/preserve/checked_bundle.rs',
    'src/workspace_ops/merge/root/artifact_facts.rs',
    'src/workspace_ops/merge/root/v1_rollback.rs',
    'src/workspace_ops/merge/v1_rollback',
    'src/checked_artifact/entry.rs',
    'src/checked_artifact/entry',
    'src/checked_artifact/entry',
    'src/checked_artifact/observation.rs',
    'src/checked_artifact/bootstrap/runtime/paths.rs',
    'src/checked_artifact/bootstrap/runtime/catalog_lease.rs',
    'src/checked_artifact/bootstrap/runtime/catalog_lease',
    'src/filesystem/retained.rs',
    'src/git/gitbackend/preservation_root.rs',
    'src/git/gitbackend/preservation_root',
    'src/git/gitbackend/preservation.rs',
    'src/git/gitbackend/preservation_image.rs',
    'src/verified_write.rs',
    'src/workspace_ops/merge/v1_lifecycle/store/rewrite.rs',
    'src/workspace_ops/merge/v1_lifecycle/store/archive.rs',
    'src/checked_artifact/classification.rs',
    'src/checked_artifact/cleanup.rs',
    'src/checked_artifact/identity.rs',
    'src/checked_artifact/residue.rs',
    'src/checked_artifact/transition.rs',
    'src/checked_artifact/platform.rs',
    'src/checked_artifact/platform/anchor.rs',
    'src/checked_artifact/bootstrap/runtime/advisory.rs',
    'src/checked_artifact/bootstrap/runtime/catalog_lease/alias.rs',
    'src/checked_artifact/capability/pre_catalog/provider/index.rs',
    'src/checked_artifact/capability/pre_catalog/provider/retained.rs',
    'src/checked_artifact/capability/pre_catalog/provider/platform.rs',
    'src/checked_artifact/capability/pre_catalog/provider/mutation.rs',
    'src/checked_artifact/capability/pre_catalog/provider/directory_mutation.rs',
    'src/checked_artifact/capability/pre_catalog/provider/filesystem/bound.rs',
)


def context_violations(source):
    found = []
    ts = tokens(source)
    names = [item[0] for item in ts]
    for index, (name, offset) in enumerate(ts):
        if name in {'make_repository', 'make_filesystem'} or (
                name == 'existing' and index and names[index-1] == '::'):
            found.append((source.count('\n', 0, offset) + 1,
                          'ambient dependency construction; use the context or handle owner'))
    return found


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


def production_source(source):
    """Blank test-only Rust items without hiding platform production branches."""
    ts = tokens(source)
    names = [item[0] for item in ts]
    output = list(source)

    def requires_test(expression):
        if expression == ['test']:
            return True
        if len(expression) < 3 or expression[0] not in {'all', 'any'} or expression[1] != '(':
            return False
        groups, current, depth = [], [], 0
        for token in expression[2:-1]:
            if token == ',' and depth == 0:
                if current: groups.append(current)
                current = []
            else:
                current.append(token)
                depth += (token == '(') - (token == ')')
        if current: groups.append(current)
        values = [requires_test(group) for group in groups]
        return any(values) if expression[0] == 'all' else bool(values) and all(values)

    index = 0
    while index + 4 < len(ts):
        if names[index:index+4] != ['#', '[', 'cfg', '(']:
            index += 1
            continue
        end = index + 4
        while end < len(ts) and names[end] != ']': end += 1
        if not requires_test(names[index+4:end-1]):
            index = end + 1
            continue
        cursor = end + 1
        while cursor < len(ts) and names[cursor] not in {'{', ';'}: cursor += 1
        if cursor == len(ts): break
        if names[cursor] == '{':
            depth = 1
            cursor += 1
            while cursor < len(ts) and depth:
                depth += (names[cursor] == '{') - (names[cursor] == '}')
                cursor += 1
        else:
            cursor += 1
        for offset in range(ts[index][1], ts[cursor-1][1] + 1):
            if output[offset] != '\n': output[offset] = ' '
        index = cursor
    return ''.join(output)


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


GIT_BOUNDARY = 'scripts/checks/gwz_git_boundary.json'
GIT_BOUNDARY_FIELDS = ('path', 'git2', 'rows', 'responsibility', 'exit', 'dated')


def is_test_source(path):
    """Test directories, `*_tests.rs`, `tests*.rs` and `test_support.rs` are not production."""
    return (any(part in {'tests', 'interface_tests'} or part.endswith('_tests') for part in path.parts)
            or path.stem.startswith('tests') or path.stem.endswith('_tests')
            or path.stem == 'test_support')


def git2_uses(source):
    """Lines of the production `git2` tokens in `source`: each identifier `git2` that starts a path.

    Comments, string literals and test-only items are not code. A `git2` inside another path (`a::git2`) or after
    a `.` is a member name, not the crate; a leading `::git2::` is the crate.
    """
    ts = tokens(production_source(source))

    def member(index):
        if not index:
            return False
        if ts[index - 1][0] == '.':
            return True
        # `a::git2` names something inside `a`; a leading `::git2::` is the crate itself.
        return ts[index - 1][0] == '::' and index > 1 and re.match(r'[A-Za-z_]', ts[index - 2][0]) is not None

    return [source.count('\n', 0, offset) + 1
            for index, (name, offset) in enumerate(ts)
            if name == 'git2' and not member(index)]


def rust_files(entry):
    return sorted(entry.rglob('*.rs')) if entry.is_dir() else [entry]


def git2_boundary_failures(root, config):
    """Failures of the migrated area against `config` (the parsed `gwz_git_boundary.json`) under `root`.

    Returns (failures, scanned) where scanned counts the zero-tolerance files and the exception files.
    """
    failures, zero_files, exception_files = [], 0, 0
    exceptions = {}
    for entry in config['exceptions']:
        missing = [name for name in GIT_BOUNDARY_FIELDS if name not in entry]
        if missing:
            failures.append(f"{GIT_BOUNDARY}: exception {entry.get('path', '?')} lacks {', '.join(missing)}")
            continue
        if not re.fullmatch(r'\d{4}-\d{2}-\d{2}', entry['dated']):
            failures.append(f"{GIT_BOUNDARY}: {entry['path']}: dated must be YYYY-MM-DD, not {entry['dated']!r}")
        if not entry['responsibility'].strip() or not entry['exit'].strip() or not entry['rows']:
            failures.append(f"{GIT_BOUNDARY}: {entry['path']}: rows, responsibility and exit must say something")
        if entry['path'] in exceptions:
            failures.append(f"{GIT_BOUNDARY}: {entry['path']} is listed twice")
        exceptions[entry['path']] = entry
    zero = list(config['zero'])
    for name in zero:
        if any(name == other or other.startswith(name.rstrip('/') + '/') for other in exceptions):
            failures.append(f'{GIT_BOUNDARY}: {name} is zero-tolerance and also holds an exception')
    for name in zero:
        entry = root / name
        if not entry.exists():
            continue  # a retired path stays retired; the probe tests in gwz-core assert it is gone
        for path in rust_files(entry):
            if is_test_source(path.relative_to(root)):
                continue
            zero_files += 1
            for line in git2_uses(path.read_text(encoding='utf-8')):
                failures.append(f'{path.relative_to(root).as_posix()}:{line}: direct git2 use in the migrated area '
                                f'(gwz-git owns it; step 5.5 guard, {GIT_BOUNDARY})')
    for name, entry in sorted(exceptions.items()):
        path = root / name
        if not path.is_file():
            failures.append(f'{name}: exception for a file that is gone; delete the entry from {GIT_BOUNDARY}')
            continue
        exception_files += 1
        lines = git2_uses(path.read_text(encoding='utf-8'))
        if len(lines) > entry['git2']:
            failures.append(f"{name}: {len(lines)} direct git2 uses, {entry['git2']} recorded (rows "
                            f"{', '.join(entry['rows'])}, {entry['dated']}); lines {', '.join(map(str, lines))}. "
                            f'Call gwz-git; a new exception is a reviewed act')
        elif len(lines) < entry['git2']:
            failures.append(f"{name}: {len(lines)} direct git2 uses, {entry['git2']} recorded: lower the entry in "
                            f'{GIT_BOUNDARY} to {len(lines)}' + (' (delete it at zero)' if not lines else ''))
    return failures, (zero_files, exception_files)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('paths', nargs='*', type=Path)
    args = parser.parse_args()
    failures = []
    for entry in args.paths or [ROOT / p for p in (*PROTECTED, *PRODUCTION_TREES)]:
        if not entry.exists():
            failures.append(f'{entry}: protected source is missing'); continue
        files = sorted(entry.rglob('*.rs')) if entry.is_dir() else [entry]
        for path in files:
            resolved = path.resolve()
            native = ROOT / 'src/filesystem/native'
            if resolved == native.with_suffix('.rs') or resolved.is_relative_to(native): continue
            production = resolved == ROOT / 'src/checked_artifact.rs' or resolved.is_relative_to(ROOT / 'src/checked_artifact')
            if production and (any(part in {'tests', 'interface_tests'} or part.endswith('_tests') for part in path.parts)
                               or path.stem.startswith('tests') or path.stem.endswith('_tests')
                               or path.stem == 'test_support'):
                continue
            source = path.read_text()
            if production: source = production_source(source)
            for line, message in violations(source):
                failures.append(f'{path}:{line}: {message}; use make_filesystem()/FileSystem')
    if not args.paths:
        for relative in CONTEXT_PROTECTED:
            entry = ROOT / relative
            if not entry.exists():
                failures.append(f'{entry}: context-protected source is missing')
                continue
            files = sorted(entry.rglob('*.rs')) if entry.is_dir() else [entry]
            for path in files:
                if (any(part in {'tests', 'interface_tests'} or part.endswith('_tests') for part in path.parts)
                        or path.stem.startswith('tests') or path.stem.endswith('_tests')
                        or path.stem == 'test_support'):
                    continue
                for line, message in context_violations(production_source(path.read_text())):
                    failures.append(f'{path}:{line}: {message}')
    if not args.paths:
        config = json.loads((ROOT / GIT_BOUNDARY).read_text(encoding='utf-8'))
        git_failures, (zero_files, exception_files) = git2_boundary_failures(ROOT, config)
        failures += git_failures
    if failures:
        print('\n'.join(failures)); return 1
    print('Filesystem source boundary passed for migrated scope.')
    if not args.paths:
        recorded = sum(entry['git2'] for entry in config['exceptions'])
        print(f'gwz-git source boundary: {zero_files} zero-tolerance files, {exception_files} files with '
              f'{recorded} dated exceptional git2 uses; nothing new')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())

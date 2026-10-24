#!/usr/bin/env python3
"""Adoption check: every public gwz-git capability has a production caller in gwz-core.

Authority: the convergence plan's step 5.5 and rule R9 ("no crate, module or
capability is complete without its first production consumer"), and the Git
boundary plan D-G section 6 (prevention is part of completion). Step 5.1
(`dev-docs/GwzGitCapabilityMap.md` section 6) proposed this check; the library
guide `dev-docs/GwzGitLibraryApi.md` is the capability table.

What it reads, with no build:

- gwz-git's public surface. `src/lib.rs` names it: each `pub use` re-export and
  each `pub mod`. A capability is a `pub fn` or a `pub const` of an inherent `impl`
  of an exported type (`Repository::head`, `Services::new`, `RoutePolicy::AUTO`), a
  free `pub fn` of a public module (`native_commit::commit`), or a public trait
  (`ProcessGate`). Test items, test files and `pub(crate)` items are not surface; a
  `pub` item behind a cargo feature is (the scan does not evaluate features).
- the guide. Every capability must be documented there: the `Repository` methods
  and the free functions of public modules in a fenced code block (`fn name(`),
  the others by name; every exported name must appear; and every `pub fn` the
  guide's code blocks declare must exist (a stale guide fails too).
- gwz-core's production code: `src/**/*.rs` without test files, files reached only
  through a test-gated `mod x;`, `#[cfg(test)]` items, comments and strings. A file
  is a caller when it takes the capability's owner (the type, the module or the
  trait) from gwz-git, by a `use gwz_git::..` that brings it in or a path
  `gwz_git::..::Owner` (never `git2::Owner`: a same-named git2 type is not gwz-git),
  and calls `.name(` or `Owner::name` for a function, `Owner::NAME` for a constant,
  or has an `impl .. Trait for ..` for a trait. This is a lexical match, like the
  map's own caller counts (section 0), not name resolution; an alias of the owner is
  not followed.

A capability with no caller fails by name. `gwz_git_adoption.json` lists the
capabilities that are public and unconsumed today, each with rows, reason, exit and
date; the list is a ratchet: an entry whose capability now has a caller, or names
nothing, fails until it is deleted, so the list only shrinks.

    python3 scripts/checks/check_gwz_git_adoption.py [--gwz-git DIR] [--core DIR] [--config FILE]

gwz-git is `--gwz-git`, else `GWZ_GIT_CHECKOUT`, else the sibling `../gwz-git`
(the one `scripts/run_tests.py` checks at the pin).
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import os
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
CONFIG = HERE / 'gwz_git_adoption.json'
GUIDE = 'dev-docs/GwzGitLibraryApi.md'
CONFIG_FIELDS = ('capability', 'rows', 'reason', 'exit', 'dated')

_spec = importlib.util.spec_from_file_location('filesystem_boundary', HERE / 'check_filesystem_boundary.py')
_fsb = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_fsb)
tokens, production_source, is_test_source = _fsb.tokens, _fsb.production_source, _fsb.is_test_source

IDENT = re.compile(r'[A-Za-z_][A-Za-z_0-9]*$')
QUALIFIERS = {'const', 'unsafe', 'async', 'extern'}


class Capability:
    """One public callable (or trait) of gwz-git."""

    def __init__(self, owner, name, kind, where, receiver=False):
        self.owner, self.name, self.kind, self.where, self.receiver = owner, name, kind, where, receiver

    @property
    def key(self):
        return self.owner if self.kind == 'trait' else f'{self.owner}::{self.name}'

    def __repr__(self):
        return f'Capability({self.key}, {self.kind})'


def production_tokens(path):
    return tokens(production_source(path.read_text(encoding='utf-8')))


def balanced(names, index, opener, closer):
    """Index just past the group that opens at `index`."""
    depth = 0
    while index < len(names):
        depth += (names[index] == opener) - (names[index] == closer)
        index += 1
        if depth == 0:
            return index
    return index


def depths(names):
    """Brace depth before each token."""
    result, depth = [], 0
    for name in names:
        if name == '}':
            depth -= 1
        result.append(depth)
        if name == '{':
            depth += 1
    return result


ITEM_KINDS = {'struct', 'enum', 'trait', 'type', 'fn', 'use', 'mod'}


def public_item(names, index):
    """(kind, index of the kind token) of the `pub` item that starts at `index`; None for `pub(..)` and consts."""
    if names[index] != 'pub' or index + 1 >= len(names) or names[index + 1] == '(':
        return None
    cursor = index + 1
    while names[cursor] in QUALIFIERS and names[cursor] != 'extern':
        cursor += 1
    return (names[cursor], cursor) if names[cursor] in ITEM_KINDS else None


def use_leaves(items):
    """The names a `use` tree brings in (the new name of `a as b`; `self`, globs and path segments are skipped)."""
    leaves = set()
    for position, item in enumerate(items):
        following = items[position + 1] if position + 1 < len(items) else None
        if IDENT.match(item) and item not in {'self', 'as', 'crate', 'super'} and following not in {'::', 'as'}:
            leaves.add(item)
    return leaves


def use_end(names, index):
    while index < len(names) and names[index] != ';':
        index += 1
    return index


def surface(git_root):
    """(capabilities, exported names, public modules) of the gwz-git checkout."""
    src = git_root / 'src'
    lib = src / 'lib.rs'
    if not lib.is_file():
        raise SystemExit(f'{lib} is missing: --gwz-git must name a gwz-git checkout')
    lib_names = [name for name, _ in production_tokens(lib)]
    exported, modules = set(), set()
    for index in range(len(lib_names)):
        item = public_item(lib_names, index)
        if item and item[0] == 'mod':
            modules.add(lib_names[item[1] + 1])
        elif item and item[0] == 'use':
            exported |= use_leaves(lib_names[item[1] + 1:use_end(lib_names, item[1])])
    parsed = {path: [name for name, _ in production_tokens(path)]
              for path in sorted(src.rglob('*.rs')) if not is_test_source(path.relative_to(src))}

    public_names, free_fns = set(exported), []
    for module in sorted(modules):
        for path in (src / f'{module}.rs', src / module / 'mod.rs'):
            names = parsed.get(path)
            if names is None:
                continue
            level = depths(names)
            for index in range(len(names)):
                item = level[index] == 0 and public_item(names, index)
                if not item:
                    continue
                kind, at = item
                if kind in {'struct', 'enum', 'type', 'trait'}:
                    public_names.add(names[at + 1])
                elif kind == 'fn':
                    free_fns.append(Capability(module, names[at + 1], 'fn', path))
                elif kind == 'use':
                    for leaf in use_leaves(names[at + 1:use_end(names, at)]):
                        if leaf[:1].isupper():
                            public_names.add(leaf)
                        else:
                            free_fns.append(Capability(module, leaf, 'fn', path))
    declared_traits = {names[k + 1] for names in parsed.values() for k in range(1, len(names) - 1)
                       if names[k] == 'trait' and names[k - 1] == 'pub'}
    public_traits = public_names & declared_traits
    public_types = {name for name in public_names if name[:1].isupper()} - public_traits

    capabilities = list(free_fns)
    capabilities += [Capability(trait, trait, 'trait', lib) for trait in sorted(public_traits)]
    for path, names in parsed.items():
        level = depths(names)
        for index, name in enumerate(names):
            if name != 'impl':
                continue
            cursor = index + 1
            if names[cursor] == '<':
                cursor = balanced(names, cursor, '<', '>')
            head, trait_impl = [], False
            while cursor < len(names) and names[cursor] not in {'{', 'where'}:
                trait_impl = trait_impl or names[cursor] == 'for'
                head.append(names[cursor])
                cursor += 1
            owner = next((token for token in head if IDENT.match(token)), None)
            if trait_impl or owner not in public_types:
                continue
            while names[cursor] != '{':
                cursor += 1
            body_depth = level[cursor] + 1
            for k in range(cursor + 1, balanced(names, cursor, '{', '}')):
                if level[k] == body_depth and names[k:k + 2] == ['pub', 'const'] \
                        and names[k + 2] != 'fn' and IDENT.match(names[k + 2]):
                    capabilities.append(Capability(owner, names[k + 2], 'const', path))
                    continue
                item = level[k] == body_depth and public_item(names, k)
                if not item or item[0] != 'fn':
                    continue
                at = item[1] + 2
                if names[at] == '<':
                    at = balanced(names, at, '<', '>')
                parameters = names[at:balanced(names, at, '(', ')')]
                capabilities.append(Capability(owner, names[item[1] + 1], 'fn', path, 'self' in parameters[:6]))
    return capabilities, exported, modules


def guide_text(core_root):
    text = (core_root / GUIDE).read_text(encoding='utf-8')
    blocks = '\n'.join(re.findall(r'```[^\n]*\n(.*?)```', text, flags=re.S))
    return text, blocks


MOD_DECL = re.compile(r'\bmod\s+([A-Za-z_][A-Za-z_0-9]*)\s*;')
CFG_ATTR = re.compile(r'#\s*\[\s*cfg\s*\(((?:[^()\[\]]|\([^()]*\)|\((?:[^()]|\([^()]*\))*\))*)\)\s*\]\s*$', re.S)


def requires_test(expression):
    """Whether a cfg expression holds only when `test` does: `test`, or `all(..)` with a requiring arm, or `any(..)`
    whose every arm requires it."""
    names = [name for name, _ in tokens(expression)]
    if names == ['test']:
        return True
    if len(names) < 3 or names[0] not in {'all', 'any'} or names[1] != '(':
        return False
    arms, current, depth = [], [], 0
    for name in names[2:-1]:
        if name == ',' and depth == 0:
            arms.append(current)
            current = []
            continue
        current.append(name)
        depth += (name == '(') - (name == ')')
    arms.append(current)
    values = [requires_test(' '.join(arm)) for arm in arms if arm]
    return any(values) if names[0] == 'all' else bool(values) and all(values)


def gated_mods(text):
    """Names of the `mod name;` declarations that exist only under test: a `#[cfg(test..)]` attribute right before
    them, or before the `{` of a block they sit in (a `cfg_if!` arm). A lexical read of the raw text."""
    gated = set()
    for found in MOD_DECL.finditer(text):
        position = found.start()
        while position >= 0:
            window_start = max(0, position - 400)
            before = text[window_start:position].rstrip()
            attribute = CFG_ATTR.search(before)
            if attribute and requires_test(attribute.group(1)):
                gated.add(found.group(1))
                break
            if attribute:  # another attribute may sit between this one and the item
                position = window_start + attribute.start()
                continue
            # The innermost block that holds the declaration, then what is written before its brace.
            depth, cursor = 0, position
            while cursor > 0:
                cursor -= 1
                depth += (text[cursor] == '}') - (text[cursor] == '{')
                if depth < 0:
                    break
            if depth >= 0:
                break
            position = cursor
    return gated


def test_only_modules(src):
    """Source files reached only through a test-gated `mod name;`, and so test code (any file below one too).

    A gated module's file, and the directory of modules beside it, are test source whatever they are called.
    """
    gated = set()
    for path in sorted(src.rglob('*.rs')):
        text = path.read_text(encoding='utf-8')
        if 'test' not in text or not MOD_DECL.search(text):
            continue
        home = path.parent if path.stem in {'mod', 'lib', 'main'} else path.parent / path.stem
        for module in gated_mods(text):
            gated |= {home / f'{module}.rs', home / module / 'mod.rs'}
    return gated


def gated_roots(gated):
    """The directories of the gated modules (a module `a/b.rs` owns `a/b/`; `a/b/mod.rs` owns `a/b/`)."""
    return {module.parent if module.name == 'mod.rs' else module.with_suffix('') for module in gated}


def below(path, gated, roots):
    """Whether `path` is a gated module file or lies in the directory of one."""
    return path in gated or any(parent in roots for parent in path.parents)


def production_callers(core_root):
    """{relative path: token names} for each non-test gwz-core source file that names gwz_git."""
    result = {}
    src = core_root / 'src'
    gated = test_only_modules(src)
    roots = gated_roots(gated)
    for path in sorted(src.rglob('*.rs')):
        relative = path.relative_to(core_root)
        if is_test_source(relative) or below(path, gated, roots):
            continue
        if 'gwz_git' not in path.read_text(encoding='utf-8'):
            continue  # cheap prefilter: the token scan is quadratic in file size
        names = [name for name, _ in production_tokens(path)]
        if 'gwz_git' in names:
            result[relative.as_posix()] = names
    return result


def split_uses(names):
    """(tokens outside `use` statements, the `use` statements): an import is not a call."""
    body, uses, index = [], [], 0
    while index < len(names):
        if names[index] == 'use':
            end = use_end(names, index)
            uses.append(names[index + 1:end])
            index = end + 1
        else:
            body.append(names[index])
            index += 1
    return body, uses


def names_owner(owner, body, uses):
    """Whether the file takes `owner` from gwz-git: a `use gwz_git::..` that brings it in (a module, as a path
    segment too), or a path `gwz_git::..::owner` in the code. `git2::Owner` is never gwz-git's."""
    for statement in uses:
        if 'gwz_git' in statement and (owner in use_leaves(statement)
                                       or (owner[:1].islower() and owner in statement)):
            return True
    for index, name in enumerate(body):
        if name != 'gwz_git':
            continue
        cursor = index
        while cursor + 2 < len(body) and body[cursor + 1] == '::' and IDENT.match(body[cursor + 2]):
            if body[cursor + 2] == owner:
                return True
            cursor += 2
    return False


def first_caller(capability, callers):
    """The first production file that calls `capability`, else None."""
    for relative, names in callers.items():
        if capability.owner not in names:
            continue
        body, uses = split_uses(names)
        if not names_owner(capability.owner, body, uses):
            continue
        if capability.kind == 'trait':
            for index, name in enumerate(names):
                if name == 'impl':
                    head = names[index:use_end_brace(names, index)]
                    if capability.owner in head and 'for' in head[head.index(capability.owner):]:
                        return relative
            continue
        # A function imported by name (`use gwz_git::ops::commit;`) is called bare.
        imported = (not capability.receiver and capability.owner[:1].islower()
                    and any(capability.owner in statement and capability.name in use_leaves(statement)
                            for statement in uses))
        for index in range(len(body) - 2):
            if capability.receiver and body[index] == '.' and body[index + 1] == capability.name \
                    and body[index + 2] in {'(', '::'}:
                return relative
            if body[index] == capability.owner and body[index + 1] == '::' and body[index + 2] == capability.name:
                return relative
            if imported and body[index] == capability.name and body[index + 1] == '(' \
                    and (not index or body[index - 1] not in {'.', '::', 'fn'}):
                return relative
    return None


def use_end_brace(names, index):
    while index < len(names) and names[index] not in {'{', ';'}:
        index += 1
    return index


def check(core_root, git_root, config):
    """(failures, stats) for the gwz-core tree at `core_root`, the gwz-git tree at `git_root` and `config`."""
    failures = []
    capabilities, exported, modules = surface(git_root)
    text, blocks = guide_text(core_root)
    callers = production_callers(core_root)

    listed = {}
    for entry in config['unconsumed']:
        missing = [name for name in CONFIG_FIELDS if name not in entry]
        if missing:
            failures.append(f"{CONFIG.name}: entry {entry.get('capability', '?')} lacks {', '.join(missing)}")
            continue
        if not re.fullmatch(r'\d{4}-\d{2}-\d{2}', entry['dated']):
            failures.append(f"{CONFIG.name}: {entry['capability']}: dated must be YYYY-MM-DD, not {entry['dated']!r}")
        if not entry['reason'].strip() or not entry['exit'].strip() or not entry['rows']:
            failures.append(f"{CONFIG.name}: {entry['capability']}: rows, reason and exit must say something")
        if entry['capability'] in listed:
            failures.append(f"{CONFIG.name}: {entry['capability']} is listed twice")
        listed[entry['capability']] = entry

    for name in sorted(exported | modules):
        if not re.search(rf'\b{re.escape(name)}\b', text):
            failures.append(f'{name} is exported by gwz-git (src/lib.rs) but {GUIDE} never mentions it')

    surface_names = {capability.name for capability in capabilities if capability.kind == 'fn'}
    for capability in capabilities:
        strict = capability.kind == 'fn' and (capability.owner == 'Repository' or capability.owner in modules)
        if capability.kind in {'fn', 'const'}:
            documented = (re.search(rf'\bfn\s+{re.escape(capability.name)}\b', blocks) if strict
                          else re.search(rf'\b{re.escape(capability.name)}\b', text))
        else:
            documented = re.search(rf'\b{re.escape(capability.owner)}\b', text)
        if not documented:
            failures.append(f'{capability.key} is public in gwz-git but {GUIDE} does not document it'
                            + (' in a code block' if strict else ''))
    for declared in sorted(set(re.findall(r'\bpub\s+fn\s+([A-Za-z_][A-Za-z_0-9]*)', blocks)) - surface_names):
        failures.append(f'{GUIDE} declares pub fn {declared}, which gwz-git does not export')

    consumed, unconsumed = 0, []
    for capability in capabilities:
        caller = first_caller(capability, callers)
        entry = listed.get(capability.key)
        if caller and entry:
            failures.append(f'{CONFIG.name}: {capability.key} is consumed now ({caller}); delete its entry')
        elif caller:
            consumed += 1
        elif entry:
            unconsumed.append(capability.key)
        else:
            failures.append(f'{capability.key} has no production caller in gwz-core (R9: no capability is complete '
                            f'without its first production consumer); consume it, or record a dated exception in '
                            f'{CONFIG.name}')
    known = {capability.key for capability in capabilities}
    for key in sorted(set(listed) - known):
        failures.append(f'{CONFIG.name}: {key} names no public gwz-git capability; delete its entry')
    return failures, (len(capabilities), consumed, len(unconsumed))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--gwz-git', type=Path, help='the gwz-git checkout (default: $GWZ_GIT_CHECKOUT, else ../gwz-git)')
    parser.add_argument('--core', type=Path, default=ROOT, help='the gwz-core checkout (default: this one)')
    parser.add_argument('--config', type=Path, default=CONFIG)
    options = parser.parse_args(argv)
    git_root = options.gwz_git or Path(os.environ.get('GWZ_GIT_CHECKOUT') or options.core.parent / 'gwz-git')
    config = json.loads(options.config.read_text(encoding='utf-8'))
    failures, (total, consumed, unconsumed) = check(options.core, git_root, config)
    if failures:
        print('\n'.join(failures))
        return 1
    print(f'gwz-git adoption: {total} public capabilities, {consumed} consumed by gwz-core production code, '
          f'{unconsumed} dated exceptions; nothing new')
    return 0


if __name__ == '__main__':
    sys.exit(main())

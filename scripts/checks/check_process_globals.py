#!/usr/bin/env python3
"""Ratchet guard: no new process-global mutable state in core or extension code.

Core runs inside other people's processes (gwz-cli, the gwz-py extension, a
session host) and must stay replaceable by a wire. State kept in the process
instead of an explicit context couples every session in that process and makes
results depend on which process core happens to run in: statics with interior
mutability, thread-local slots, environment/working-directory reads at the
point of use, libgit2's process-wide options, and child processes, which
inherit the live process environment unless spawned with `env_clear`
(GwzCoreSessionDesign O9).

Every production occurrence must be listed in the allowlist with a disposition
(`debt` = scheduled for removal, `permanent` = justified process-wide state).
A new occurrence fails. A listed occurrence that disappears also fails, so the
list only shrinks. Code compiled only under `cfg(test)` is exempt:
`#[cfg(test)]` items and modules, `cfg_if!` test branches, and files reached
only through test-only `mod` declarations.

This is a lexical scan, not name resolution. It strips comments and literals,
follows `mod`/`#[path]` declarations from the crate roots named in the
allowlist, and inspects every platform branch without compiling any of them.
Files under the scan roots that no `mod` declaration reaches are scanned as
production and reported.
"""
import argparse
import json
import re
import sys
from collections import Counter
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_ALLOWLIST = Path(__file__).resolve().with_name('process_globals_allowlist.json')
DISPOSITIONS = {'debt', 'permanent'}

# Types whose statics are shared mutable (or lazily initialised) process state.
INTERIOR = re.compile(
    r'^(?:Atomic\w*|Mutex|RwLock|Cell|RefCell|UnsafeCell|SyncUnsafeCell|OnceLock|'
    r'OnceCell|LazyLock|LazyCell|Lazy|Once|Condvar|Barrier|Semaphore|PyOnceLock|'
    r'GILOnceCell|GILProtected|ThreadLocal|ArcSwap\w*)$')
PRIMITIVES = {'u8', 'u16', 'u32', 'u64', 'u128', 'usize', 'i8', 'i16', 'i32', 'i64',
              'i128', 'isize', 'f32', 'f64', 'bool', 'char', 'str'}
ENV_FUNCS = {'var', 'var_os', 'vars', 'vars_os', 'set_var', 'remove_var', 'home_dir',
             'current_dir', 'set_current_dir', 'temp_dir', 'args', 'args_os'}
HOME_CRATES = {'dirs', 'dirs_next', 'home'}
LIBC_FUNCS = {'getenv', 'setenv', 'unsetenv', 'putenv', 'chdir', 'fchdir', 'umask',
              'signal', 'sigaction'}
HOOKS = {('panic', 'set_hook'), ('panic', 'take_hook'), ('log', 'set_logger'),
         ('log', 'set_boxed_logger'), ('log', 'set_max_level'),
         ('subscriber', 'set_global_default')}
LIBGIT2_SETTERS = ('set_', 'enable_', 'strict_')

_LEX = re.compile(r"""
    (?P<ws>\s+)
  | (?P<line>//[^\n]*)
  | (?P<block>/\*)
  | (?P<raw>(?:br|cr|r)(?P<hashes>\#*)")
  | (?P<str>[bc]?"(?:\\.|[^"\\])*")
  | (?P<char>b?'(?:\\(?:x[0-9A-Fa-f]{2}|u\{[0-9A-Fa-f]{1,6}\}|.)|[^'\\\n])')
  | (?P<life>'[A-Za-z_]\w*)
  | (?P<id>[A-Za-z_]\w*)
  | (?P<num>[0-9]\w*)
  | (?P<path>::)
  | (?P<punct>.)
""", re.X | re.S)


@dataclass(frozen=True)
class Tok:
    kind: str  # id, life, str, char, num, path, punct
    text: str
    offset: int


def lex(source: str) -> list[Tok]:
    """Tokens with comments removed; string literals keep their raw body."""
    out: list[Tok] = []
    i, n = 0, len(source)
    while i < n:
        m = _LEX.match(source, i)
        if m.group('ws') is not None or m.group('line') is not None:
            i = m.end()
            continue
        if m.group('block') is not None:
            depth, j = 1, m.end()
            while j < n and depth:
                if source.startswith('/*', j):
                    depth, j = depth + 1, j + 2
                elif source.startswith('*/', j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            i = j
            continue
        if m.group('raw') is not None:
            close = '"' + m.group('hashes')
            end = source.find(close, m.end())
            end = n if end < 0 else end
            out.append(Tok('str', source[m.end():end], i))
            i = min(n, end + len(close))
            continue
        if m.group('str') is not None:
            text = m.group('str')
            out.append(Tok('str', text[text.index('"') + 1:-1], i))
            i = m.end()
            continue
        for kind in ('char', 'life', 'id', 'num', 'path', 'punct'):
            if m.group(kind) is not None:
                out.append(Tok(kind, m.group(kind), i))
                break
        i = m.end()
    return out


def parse_pred(toks: list[Tok], i: int):
    """Parse one cfg predicate starting at i; return (next index, tree)."""
    t = toks[i]
    if t.kind == 'id' and t.text in ('all', 'any', 'not') and toks[i + 1].text == '(':
        items, j = [], i + 2
        while toks[j].text != ')':
            j, item = parse_pred(toks, j)
            items.append(item)
            if toks[j].text == ',':
                j += 1
        return j + 1, (t.text, items)
    if toks[i + 1].text == '=':
        return i + 3, ('atom', f'{t.text}={toks[i + 2].text}')
    return i + 1, ('atom', t.text)


def cfg_predicate(toks: list[Tok], start: int, stop: int):
    """The predicate of `cfg(...)` spanning toks[start:stop], else None."""
    if stop - start < 3 or toks[start].text != 'cfg' or toks[start + 1].text != '(':
        return None
    try:
        return parse_pred(toks, start + 2)[1]
    except IndexError:
        return None


def implies_test(pred, test_features: frozenset[str] = frozenset()) -> bool:
    """True when every configuration satisfying `pred` is a test build."""
    kind, arg = pred
    if kind == 'atom':
        return arg == 'test' or (arg.startswith('feature=') and arg[8:] in test_features)
    if kind == 'all':
        return any(implies_test(p, test_features) for p in arg)
    if kind == 'any':
        return bool(arg) and all(implies_test(p, test_features) for p in arg)
    return kind == 'not' and len(arg) == 1 and implies_not_test(arg[0], test_features)


def implies_not_test(pred, test_features: frozenset[str] = frozenset()) -> bool:
    kind, arg = pred
    if kind == 'atom':
        return False
    if kind == 'all':
        return any(implies_not_test(p, test_features) for p in arg)
    if kind == 'any':
        return bool(arg) and all(implies_not_test(p, test_features) for p in arg)
    return kind == 'not' and len(arg) == 1 and implies_test(arg[0], test_features)


@dataclass
class ModDecl:
    name: str
    path_attr: str | None
    inline: tuple[str, ...]
    test_only: bool


@dataclass
class Occurrence:
    kind: str
    name: str
    line: int
    detail: str = ''


@dataclass
class FileInfo:
    mods: list[ModDecl] = field(default_factory=list)
    occurrences: list[Occurrence] = field(default_factory=list)


class Analysis:
    def __init__(self, source: str, test_features: frozenset[str] = frozenset()):
        self.source = source
        self.test_features = test_features
        self.toks = lex(source)
        self.n = len(self.toks)
        self.match: dict[int, int] = {}
        self.parent: list[int | None] = [None] * self.n
        stack: list[int] = []
        for i, t in enumerate(self.toks):
            self.parent[i] = stack[-1] if stack else None
            if t.kind == 'punct' and t.text in '([{':
                stack.append(i)
            elif t.kind == 'punct' and t.text in ')]}' and stack:
                j = stack.pop()
                self.match[j], self.match[i] = i, j
                self.parent[i] = self.parent[j]
        self.test_ranges: list[tuple[int, int]] = []
        self._cfg_attributes()
        self._cfg_if()

    def text(self, i: int) -> str:
        return self.toks[i].text if 0 <= i < self.n else ''

    def line(self, i: int) -> int:
        return self.source.count('\n', 0, self.toks[i].offset) + 1

    def is_test(self, i: int) -> bool:
        return any(a <= i <= b for a, b in self.test_ranges)

    def _brace_kind(self, open_index: int) -> str:
        if self.text(open_index) != '{':
            return 'group'
        i = open_index - 1
        while i >= 0:
            t = self.toks[i]
            if t.kind == 'punct' and t.text in ')]' and i in self.match:
                i = self.match[i] - 1
                continue
            if t.kind == 'punct' and t.text in ';{}':
                return 'block'
            if t.kind == 'id' and t.text == 'match':
                return 'match'
            if t.kind == 'id' and t.text in ('enum', 'struct', 'union'):
                return 'fields'
            i -= 1
        return 'block'

    def _skip_attributes(self, i: int) -> int:
        while self.text(i) == '#' and self.text(i + 1) == '[' and (i + 1) in self.match:
            i = self.match[i + 1] + 1
        return i

    def _item_end(self, start: int) -> int:
        """Last token of the item/statement/element that starts at `start`."""
        if start >= self.n:
            return self.n - 1
        enclosing = self.parent[start]
        stop_at_comma = enclosing is not None and self._brace_kind(enclosing) != 'block'
        i = start
        while i < self.n:
            t = self.toks[i]
            if t.kind == 'punct':
                if t.text in '([':
                    if i not in self.match:
                        return self.n - 1
                    i = self.match[i] + 1
                    continue
                if t.text == '{':
                    if i not in self.match:
                        return self.n - 1
                    end = self.match[i]
                    return end + 1 if self.text(end + 1) == ';' else end
                if t.text == ';':
                    return i
                if t.text in ')]}':
                    return i - 1
                if t.text == ',' and stop_at_comma:
                    return i
            i += 1
        return self.n - 1

    def _cfg_attributes(self) -> None:
        i = 0
        while i < self.n:
            if self.text(i) != '#':
                i += 1
                continue
            j = i + 1
            inner = self.text(j) == '!'
            if inner:
                j += 1
            if self.text(j) != '[' or j not in self.match:
                i += 1
                continue
            close = self.match[j]
            pred = cfg_predicate(self.toks, j + 1, close)
            if pred is not None and implies_test(pred, self.test_features):
                if inner:
                    scope = self.parent[i]
                    if scope is None:
                        self.test_ranges.append((0, self.n - 1))
                    else:
                        self.test_ranges.append((scope, self.match.get(scope, self.n - 1)))
                else:
                    self.test_ranges.append((i, self._item_end(self._skip_attributes(close + 1))))
            i = close + 1

    def _cfg_if(self) -> None:
        for i in range(self.n - 2):
            if not (self.text(i) == 'cfg_if' and self.text(i + 1) == '!'
                    and self.text(i + 2) in ('{', '(') and (i + 2) in self.match):
                continue
            j, end, prior = i + 3, self.match[i + 2], []
            while j < end and self.text(j) == 'if' and self.text(j + 1) == '#' \
                    and self.text(j + 2) == '[' and (j + 2) in self.match:
                close = self.match[j + 2]
                pred = cfg_predicate(self.toks, j + 3, close) or ('atom', '?')
                brace = close + 1
                if self.text(brace) != '{' or brace not in self.match:
                    break
                condition = ('all', [('not', [p]) for p in prior] + [pred])
                if implies_test(condition, self.test_features):
                    self.test_ranges.append((brace, self.match[brace]))
                prior.append(pred)
                j = self.match[brace] + 1
                if self.text(j) != 'else':
                    break
                j += 1
                if self.text(j) == '{' and j in self.match:
                    if implies_test(('all', [('not', [p]) for p in prior]), self.test_features):
                        self.test_ranges.append((j, self.match[j]))
                    break

    def _path_attribute(self, mod_index: int) -> str | None:
        k = mod_index
        if self.text(k - 1) == ')' and (k - 1) in self.match and self.text(self.match[k - 1] - 1) == 'pub':
            k = self.match[k - 1] - 1
        elif self.text(k - 1) == 'pub':
            k -= 1
        while self.text(k - 1) == ']' and (k - 1) in self.match and self.text(self.match[k - 1] - 1) == '#':
            open_index = self.match[k - 1]
            if self.text(open_index + 1) == 'path' and self.text(open_index + 2) == '=' \
                    and self.toks[open_index + 3].kind == 'str':
                return self.toks[open_index + 3].text
            k = open_index - 1
        return None

    def info(self) -> FileInfo:
        info = FileInfo()
        inline_mods = []
        macro_bodies = []
        for i in range(self.n - 2):
            t = self.toks[i]
            if t.kind != 'id':
                continue
            if t.text == 'mod' and self.toks[i + 1].kind == 'id' and self.text(i + 2) == '{' \
                    and (i + 2) in self.match:
                inline_mods.append((self.toks[i + 1].text, i + 2, self.match[i + 2]))
            if t.text in ('thread_local', 'lazy_static') and self.text(i + 1) == '!' \
                    and self.text(i + 2) in ('{', '(', '[') and (i + 2) in self.match:
                macro_bodies.append((i + 2, self.match[i + 2], t.text))
        for i in range(self.n - 2):
            if self.toks[i].kind == 'id' and self.toks[i].text == 'mod' \
                    and self.toks[i + 1].kind == 'id' and self.text(i + 2) == ';':
                inline = tuple(name for name, a, b in inline_mods if a < i < b)
                info.mods.append(ModDecl(self.toks[i + 1].text, self._path_attribute(i),
                                         inline, self.is_test(i)))
        for i, t in enumerate(self.toks):
            if t.kind != 'id' or self.is_test(i):
                continue
            nxt, after = self.text(i + 1), self.text(i + 2)
            if t.text == 'static':
                self._static(i, macro_bodies, info)
            elif t.text == 'env' and nxt == '::':
                if after in ENV_FUNCS:
                    info.occurrences.append(Occurrence('env', f'env::{after}', self.line(i)))
                elif after == '{' and (i + 2) in self.match:
                    for k in range(i + 3, self.match[i + 2]):
                        if self.toks[k].kind == 'id' and self.toks[k].text in ENV_FUNCS:
                            info.occurrences.append(
                                Occurrence('env', f'env::{self.toks[k].text}', self.line(k)))
            elif t.text in HOME_CRATES and nxt == '::' and after == 'home_dir':
                info.occurrences.append(Occurrence('env', f'{t.text}::home_dir', self.line(i)))
            elif t.text == 'libc' and nxt == '::' and after in LIBC_FUNCS:
                info.occurrences.append(Occurrence('env', f'libc::{after}', self.line(i)))
            elif t.text == 'opts' and nxt == '::' and self.toks[i + 2].kind == 'id' and (
                    self.text(i - 2) == 'git2' or after.startswith(LIBGIT2_SETTERS)):
                info.occurrences.append(Occurrence('libgit2', f'git2::opts::{after}', self.line(i)))
            elif t.text == 'transport' and nxt == '::' and after == 'register' \
                    and self.text(i - 2) == 'git2':
                info.occurrences.append(Occurrence('libgit2', 'git2::transport::register', self.line(i)))
            elif nxt == '::' and (t.text, after) in HOOKS:
                info.occurrences.append(Occurrence('hook', f'{t.text}::{after}', self.line(i)))
            elif t.text == 'Command' and nxt == '::' and after == 'new':
                # Keyed by the program literal when there is one, so each
                # spawn site's environment handling is listed separately.
                program = self.toks[i + 4] if self.text(i + 3) == '(' and i + 4 < self.n else None
                name = f'Command::new("{program.text}")' if program is not None and program.kind == 'str' \
                    else 'Command::new'
                info.occurrences.append(Occurrence('process', name, self.line(i)))
        return info

    def _static(self, i: int, macro_bodies, info: FileInfo) -> None:
        j = i + 1
        mutable = self.text(j) == 'mut'
        if mutable:
            j += 1
        if j >= self.n or self.toks[j].kind != 'id' or self.text(j + 1) != ':':
            return
        name = self.toks[j].text
        j += 2
        ty: list[Tok] = []
        while j < self.n and self.text(j) not in ('=', ';'):
            if self.text(j) in ('(', '[') and j in self.match:
                ty.extend(self.toks[j:self.match[j] + 1])
                j = self.match[j] + 1
                continue
            ty.append(self.toks[j])
            j += 1
        detail = ' '.join(t.text for t in ty)
        macro = next((kind for a, b, kind in macro_bodies if a < i < b), None)
        if macro is not None:
            info.occurrences.append(Occurrence(macro, name, self.line(i), detail))
            return
        idents = [t.text for t in ty if t.kind == 'id']
        if not mutable and not any(INTERIOR.match(x) for x in idents):
            if ty and ty[0].text == '&':
                return
            if idents and all(x in PRIMITIVES for x in idents):
                return
        info.occurrences.append(Occurrence('static', name, self.line(i), detail))


def analyze(source: str, test_features: frozenset[str] = frozenset()) -> FileInfo:
    return Analysis(source, test_features).info()


def module_candidates(path: Path, decl: ModDecl, is_mod_rs: bool) -> list[Path]:
    """Files a `mod` declaration may load, per the Rust reference's lookup rules."""
    base = path.parent if is_mod_rs else path.parent / path.stem
    if decl.path_attr is not None:
        if decl.inline:
            return [base.joinpath(*decl.inline, decl.path_attr)]
        return [path.parent / decl.path_attr]
    directory = base.joinpath(*decl.inline)
    return [directory / f'{decl.name}.rs', directory / decl.name / 'mod.rs']


@dataclass
class Scan:
    occurrences: dict[tuple[str, str, str], list[Occurrence]]
    unreached: list[str]
    files: int


def scan(repo: Path, roots: list[str], test_features: frozenset[str] = frozenset()) -> Scan:
    root_files = sorted({p.resolve() for pattern in roots for p in repo.glob(pattern)})
    if not root_files:
        raise SystemExit(f'no crate roots match {roots} under {repo}')
    cache: dict[Path, FileInfo] = {}
    status: dict[Path, str] = {}
    # Crate roots, mod.rs files and every `#[path]`-loaded file resolve their
    # own `mod` declarations relative to their directory (rustc treats all
    # `#[path]` files as mod.rs files).
    queue = [(p, False, True) for p in root_files]
    while queue:
        path, test, is_mod_rs = queue.pop()
        old = status.get(path)
        if old == 'production' or (old == 'test' and test):
            continue
        status[path] = 'test' if test else 'production'
        if path not in cache:
            cache[path] = analyze(path.read_text(encoding='utf-8'), test_features)
        for decl in cache[path].mods:
            for candidate in module_candidates(path, decl, is_mod_rs):
                if candidate.is_file():
                    child_mod_rs = decl.path_attr is not None or candidate.name == 'mod.rs'
                    queue.append((candidate.resolve(), test or decl.test_only, child_mod_rs))
                    break
    unreached = []
    for directory in sorted({p.parent for p in root_files}):
        for path in sorted(directory.rglob('*.rs')):
            resolved = path.resolve()
            if resolved not in status:
                status[resolved] = 'production'
                unreached.append(resolved)
                cache[resolved] = analyze(path.read_text(encoding='utf-8'), test_features)
    found: dict[tuple[str, str, str], list[Occurrence]] = {}
    for path, state in status.items():
        if state != 'production':
            continue
        try:
            relative = path.relative_to(repo.resolve()).as_posix()
        except ValueError:
            relative = path.as_posix()
        for occurrence in cache[path].occurrences:
            found.setdefault((relative, occurrence.kind, occurrence.name), []).append(occurrence)
    unreached_names = []
    for path in unreached:
        try:
            unreached_names.append(path.relative_to(repo.resolve()).as_posix())
        except ValueError:
            unreached_names.append(path.as_posix())
    return Scan(found, unreached_names, len(status))


@dataclass
class Allowlist:
    roots: list[str]
    test_features: frozenset[str]
    entries: dict[tuple[str, str, str], dict]


def load_allowlist(path: Path) -> tuple[Allowlist, list[str]]:
    data = json.loads(path.read_text(encoding='utf-8'))
    errors = []
    entries: dict[tuple[str, str, str], dict] = {}
    for entry in data.get('entries', []):
        key = (entry.get('path', ''), entry.get('kind', ''), entry.get('name', ''))
        if not all(key):
            errors.append(f'allowlist entry needs path, kind and name: {entry}')
            continue
        if key in entries:
            errors.append(f'duplicate allowlist entry: {key}')
        if entry.get('disposition') not in DISPOSITIONS:
            errors.append(f'{key}: disposition must be one of {sorted(DISPOSITIONS)}')
        if not str(entry.get('reason', '')).strip():
            errors.append(f'{key}: reason is required')
        if not isinstance(entry.get('count', 1), int) or entry.get('count', 1) < 1:
            errors.append(f'{key}: count must be a positive integer')
        entries[key] = entry
    roots = data.get('roots') or []
    if not roots:
        errors.append('allowlist must name the crate roots to scan')
    features = frozenset(data.get('test_features') or [])
    return Allowlist(roots, features, entries), errors


def check(repo: Path, allowlist_path: Path) -> tuple[list[str], Scan, dict]:
    allowlist, errors = load_allowlist(allowlist_path)
    entries = allowlist.entries
    if errors:
        return errors, Scan({}, [], 0), entries
    result = scan(repo, allowlist.roots, allowlist.test_features)
    for key, occurrences in sorted(result.occurrences.items()):
        path, kind, name = key
        where = ', '.join(f'{path}:{o.line}' for o in occurrences)
        detail = f' ({occurrences[0].detail})' if occurrences[0].detail else ''
        entry = entries.get(key)
        if entry is None:
            errors.append(f'NEW   {kind} {name}{detail} at {where}')
        elif len(occurrences) != entry.get('count', 1):
            errors.append(f'COUNT {kind} {name} at {where}: found {len(occurrences)}, '
                          f'allowlisted {entry.get("count", 1)}; update the count only if the '
                          f'change removes occurrences')
    for key in sorted(set(entries) - set(result.occurrences)):
        path, kind, name = key
        errors.append(f'STALE {kind} {name} in {path}: no longer present; delete the allowlist entry')
    return errors, result, entries


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('--repo', type=Path, default=ROOT, help='repository root (default: gwz-core)')
    parser.add_argument('--allowlist', type=Path, help='allowlist JSON (default: next to this script)')
    parser.add_argument('--list', action='store_true', help='print every production occurrence and exit')
    options = parser.parse_args(argv)
    repo = options.repo.resolve()
    allowlist = (options.allowlist or DEFAULT_ALLOWLIST).resolve()
    if options.list:
        loaded, errors = load_allowlist(allowlist)
        if errors:
            print('\n'.join(errors), file=sys.stderr)
            return 1
        result = scan(repo, loaded.roots, loaded.test_features)
        for (path, kind, name), occurrences in sorted(result.occurrences.items()):
            lines = ','.join(str(o.line) for o in occurrences)
            print(f'{path}:{lines}\t{kind}\t{name}\t{occurrences[0].detail}')
        for path in result.unreached:
            print(f'{path}\tunreached')
        return 0
    errors, result, entries = check(repo, allowlist)
    for path in result.unreached:
        print(f'note: {path} is not reached from a crate root; scanned as production')
    if errors:
        print(f'process-global state guard failed for {repo} (allowlist: {allowlist}):', file=sys.stderr)
        for error in errors:
            print(f'  {error}', file=sys.stderr)
        print('Pass state through the operation/session context instead of the process, and '
              'spawn child processes with env_clear() plus the session\'s environment snapshot. '
              'A genuinely process-wide item needs an allowlist entry with a disposition '
              'and reason (GwzCoreSessionDesign O9).', file=sys.stderr)
        return 1
    dispositions = Counter(entry['disposition'] for entry in entries.values())
    print(f'process-global state guard: {result.files} files, {len(entries)} allowlisted items '
          f'({dispositions["debt"]} debt, {dispositions["permanent"]} permanent); nothing new')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())

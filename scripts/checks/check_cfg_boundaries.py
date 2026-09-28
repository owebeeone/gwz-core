#!/usr/bin/env python3
"""Ratchet guard: conditional compilation sits inside an explicit boundary.

The root AGENTS.md rule on explicit scope: conditional sections go in
`cfg_if::cfg_if!` blocks or enclosing platform modules, never as a
`#[cfg(...)]` or a conditional `#[cfg_attr(...)]` directly on an import or
another unbraced declaration. Deleting such a declaration but not its
attribute silently moves the attribute to the next one, in an arm the
author's platform never compiles (gwz commit cfa14b8 broke v1.0.5 on Windows).

Two kinds of target are flagged, wherever they sit and in every arm:
- an unbraced item, which ends in `;` instead of a braced body, at module
  level (a file, `mod m { }`, a `cfg_if!` arm), among an `impl`'s or a
  `trait`'s associated items, in an `extern` block or inside a block: `use`,
  `extern crate`, `mod name;`, `type`, `const`, `static`, a unit or tuple
  `struct`, a function without a body, and a macro invocation item such as
  `m!(...);` or `::m::n![...];`;
- an unbraced statement in a block that is not a list of items (a function or
  closure body, a `cfg_if!` arm in statement position), at the block's top,
  after its inner attributes or after another statement: a `let`, or any
  other statement that ends in `;`, whatever braces its expression holds
  (`x = S { a };`, `return if a { b } else { c };`). A `let` is a
  declaration statement in Rust's grammar.
Braced items and statements are their own boundary and pass: `fn` with a
body, `mod m { }`, `impl`, `trait`, `enum`, `union`, `struct` with named
fields, `extern { }`, brace-delimited macros such as `cfg_if! { }`, and a
statement whose first token is `{`, `if`, `match`, `loop`, `while`, `for`,
`unsafe` or a label. An `async` block, like a closure, is an expression that
needs its `;`, so it does not brace its statement. A `{ }` const-generic
argument or default in a header (`Foo<{ N }>`) is not a body. A block's tail
expression has no `;` and is not a statement, and neither is a match arm: no
`;` ends one before its match closes.

Fields, variants, match arms and parameters are out of scope, and are not
inventoried: they are neither items nor statements, and Rust gives no
explicit boundary that could hold them. At the inventory's HEADs conditional
attributes sit on 31 fields and variants, 7 struct-expression fields, 8
match arms, and 2 parameters and 2 call arguments; 4 more sit on tail
expressions.

Attributes stack: a `cfg` separated from its target by other attributes or
comments is still directly on it. A `cfg_attr` is conditional when its effect
depends on the configuration: its predicate is not constant (`all()` always
holds, `any()` never does, and `not`, `all` and `any` of constants fold) and
it applies an attribute. One that always applies is judged by what it
applies; one that never applies has no effect. Inner attributes such as
`#![cfg(...)]` scope their whole module or block and pass.

An occurrence is keyed by repository, file, conditional attributes and
target, never by line; `count` counts identical ones in a file. The target is
the whole rendered text of a `use` or `extern crate` item, a macro item or an
expression statement, and `let` with the rendered pattern of a `let`: a
change to any of them makes a new key. It is the kind and name of a `type`,
`const`, `static`, `fn`, `struct`, `union`, `enum`, `trait` or `mod`, so a
change to such an item's type or body keeps its key. Rendering drops layout
and trailing commas.

A new or modified occurrence fails, and so does a listed one that
disappears. --shrink-from BASE compares the allowlist with a base copy of it.
It sums each repository, attributes and target's count over its files, and
fails when a sum exceeds the base's: an added occurrence or a raised count.
Debt moved between files, as a movement-only split moves it, passes. That is a
trade-off: a change may remove a listed occurrence in one file and add an
identical one elsewhere in the same repository, and it passes too, because a
movement-only split cannot be told apart from remove-and-recreate. Review
reads a change to an entry's path as a move claim to verify. It also
fails when the scanned scope narrows: a base repository gone, a repository's
path changed, or a base root no longer under the current roots. The boundary
CI job runs it against the pull request's base and the pushed-over commit, so
the list only shrinks.

The scan is lexical: comments, doc comments and literals are stripped and
every arm is inspected, none compiled. The allowlist names each repository,
its path from gwz-core and the roots to walk. The walk skips hidden
directories, so the `.github/bootstrap-crate` stubs are unscanned, and the
`target` beside a Cargo.toml, and it does not follow directory symlinks. A
repository that is not checked out fails the check, and so does an
unreadable file. --skip-repo NAME skips a sibling and prints SKIPPED GATE, for
a CI job without that checkout only; the check's own repository cannot be
skipped, and a run that scans no file fails. gwz-core's CI checks gwz-core
alone, so the coverage of gwz-cli and gwz-py is local-only (run_tests.py in a
workspace) until their own CI runs the check, a follow-up once this check is
on gwz-core's main.
"""
import argparse
from collections import Counter, namedtuple
from dataclasses import dataclass, field
import json
import os
from pathlib import Path
import posixpath
import re
import sys

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_ALLOWLIST = Path(__file__).resolve().with_name('cfg_boundaries_allowlist.json')

_LEX = re.compile(r"""
    (?P<ws>\s+)
  | (?P<line>//[^\n]*)
  | (?P<block>/\*)
  | (?P<raw>(?:br|cr|r)(?P<hashes>\#*)")
  | (?P<str>[bc]?"(?:\\.|[^"\\])*")
  | (?P<char>b?'(?:\\(?:x[0-9A-Fa-f]{2}|u\{[0-9A-Fa-f]{1,6}\}|.)|[^'\\\n])')
  | (?P<life>'[A-Za-z_]\w*)
  | (?P<id>(?:r\#)?[^\W\d]\w*)
  | (?P<num>[0-9]\w*)
  | (?P<path>::)
  | (?P<punct>.)
""", re.X | re.S)
WORDS = {'id', 'life', 'num', 'str', 'char'}
# Qualifiers that may precede an item keyword, and the words that may follow one.
QUALIFIERS = {'default', 'unsafe', 'async', 'safe', 'auto', 'const', 'extern'}
ITEM_WORDS = QUALIFIERS | {'fn', 'impl', 'trait', 'static', 'type', 'mod', 'use', 'struct', 'enum', 'union'}
BODY_OR_SEMICOLON = {'mod', 'struct', 'union', 'enum', 'trait', 'fn'}
ITEM_LISTS = {'mod', 'impl', 'trait', 'extern'}
# The first tokens of a braced statement; a label (a lifetime token) and a brace-delimited macro are braced too.
# An `async` block is not among them: like a closure, it is an expression without a block and needs its `;`.
BRACED_STATEMENTS = {'{', 'if', 'match', 'loop', 'while', 'for', 'unsafe'}
RATCHET = ('NEW', 'COUNT', 'STALE')

Tok = namedtuple('Tok', 'kind text offset')  # kind: id, life, str, char, num, path, punct
Occurrence = namedtuple('Occurrence', 'attrs item line')


def lex(source: str) -> list[Tok]:
    """Tokens with comments and doc comments removed; literals keep their full text."""
    out: list[Tok] = []
    i, n = 0, len(source)
    while i < n:
        m = _LEX.match(source, i)
        kind = m.lastgroup
        if kind in ('ws', 'line'):
            i = m.end()
        elif kind == 'block':
            depth, j = 1, m.end()
            while j < n and depth:
                if source.startswith('/*', j):
                    depth, j = depth + 1, j + 2
                elif source.startswith('*/', j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            i = j
        elif kind == 'raw':
            end = source.find('"' + m.group('hashes'), m.end())
            end = n if end < 0 else end + 1 + len(m.group('hashes'))
            out.append(Tok('str', source[i:end], i))
            i = end
        else:
            out.append(Tok(kind, m.group(), i))
            i = m.end()
    return out


def render(tokens) -> str:
    """Tokens as normalized text, so that layout and trailing commas never change a key."""
    out, prev = [], None
    for k, t in enumerate(tokens):
        if t.text == ',' and k + 1 < len(tokens) and tokens[k + 1].text in (')', ']', '}'):
            continue
        if prev is not None and ((prev.kind in WORDS and t.kind in WORDS) or prev.text == ','
                                 or '=' in (prev.text, t.text)):
            out.append(' ')
        out.append(t.text)
        prev = t
    return ''.join(out)


def split_commas(tokens) -> list[list[Tok]]:
    parts, current, depth = [], [], 0
    for t in tokens:
        depth += (t.text in ('(', '[', '{')) - (t.text in (')', ']', '}'))
        if depth == 0 and t.text == ',':
            parts.append(current)
            current = []
        else:
            current.append(t)
    return parts + [current]


def constant(pred) -> bool | None:
    """The predicate's value when every configuration gives the same one, else None."""
    if len(pred) < 3 or pred[0].text not in ('all', 'any', 'not') or pred[1].text != '(':
        return None
    values = [constant(p) for p in split_commas(pred[2:-1]) if p]
    if pred[0].text == 'not':
        return None if len(values) != 1 or values[0] is None else not values[0]
    if pred[0].text == 'all':
        return False if False in values else (True if all(v is True for v in values) else None)
    return True if True in values else (False if all(v is False for v in values) else None)


def conditional(attr) -> bool:
    """True when the attribute (the tokens inside `#[...]`) makes compilation depend on the configuration."""
    if len(attr) < 3 or attr[1].text != '(' or attr[0].text not in ('cfg', 'cfg_attr'):
        return False
    if attr[0].text == 'cfg':
        return True
    predicate, *applied = split_commas(attr[2:-1])
    applied = [a for a in applied if a]
    value = constant(predicate)
    if value is None:
        return bool(applied)
    return value and any(conditional(a) for a in applied)


class Analysis:
    def __init__(self, source: str):
        self.source = source
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

    def text(self, i: int) -> str:
        return self.toks[i].text if 0 <= i < self.n else ''

    def kind(self, i: int) -> str:
        return self.toks[i].kind if 0 <= i < self.n else ''

    def attribute(self, i: int) -> int | None:
        """The closing `]` of an outer attribute starting at i, else None."""
        if self.text(i) == '#' and self.text(i + 1) == '[' and (i + 1) in self.match:
            return self.match[i + 1]
        return None

    def generic_block(self, i: int) -> bool:
        """True for a const-generic `{ }` argument or default such as `Foo<{ N }>`: it opens after `<`, `=`
        or `,` and closes before `>` or `,`. A body or block never closes before either, where clauses included."""
        return (self.text(i) == '{' and i in self.match and self.text(i - 1) in ('<', '=', ',')
                and self.text(self.match[i] + 1) in ('>', ','))

    def keyword(self, i: int) -> int:
        """The item keyword at or after i, past visibility and qualifiers."""
        if self.text(i) == 'pub':
            i += 1
            if self.text(i) == '(' and i in self.match:
                i = self.match[i] + 1
        while self.text(i) in QUALIFIERS:
            after = i + 1 + (self.text(i) == 'extern' and self.kind(i + 1) == 'str')
            if self.text(after) not in ITEM_WORDS:
                break
            i = after
        return i

    def end(self, i: int, braces: bool = False) -> int:
        """The `;`, body `{` or enclosing close that ends the item or statement at i; `braces` steps over `{...}`."""
        while i < self.n:
            t = self.text(i)
            if (t in ('(', '[') or (t == '{' and (braces or self.generic_block(i)))) and i in self.match:
                i = self.match[i] + 1
            elif t in (';', '{', '}', ')', ']'):
                return i
            else:
                i += 1
        return i

    def in_item_list(self, i: int) -> bool:
        """True when token i sits among items: a file, module, impl, trait, extern or item-level cfg_if arm."""
        brace = self.parent[i]
        if brace is None:
            return True
        if self.text(brace) != '{':
            return False
        h = brace - 1
        while h >= 0:
            t = self.text(h)
            if (t in (')', ']') or (t == '}' and self.generic_block(self.match.get(h, -1)))) and h in self.match:
                h = self.match[h] - 1
            elif t in (';', '{', '}'):
                break
            else:
                h -= 1
        h += 1
        while self.attribute(h) is not None:
            h = self.attribute(h) + 1
        word, outer = self.text(self.keyword(h)), self.parent[brace]
        if word in ('if', 'else') and outer is not None and self.text(outer - 1) == '!' \
                and self.text(outer - 2) == 'cfg_if':
            return self.in_item_list(outer)  # an arm holds what the cfg_if! around it holds
        return word in ITEM_LISTS

    def macro_group(self, k: int) -> int | None:
        """The delimiter group of a macro invocation at k (`path!(..)`, `::path![..]`, `macro_rules! m {..}`), else None."""
        j = k + (self.text(k) == '::')
        while self.kind(j) == 'id' and self.text(j + 1) == '::':
            j += 2
        if self.kind(j) != 'id' or self.text(j + 1) != '!':
            return None
        group = j + 2 + (self.kind(j + 2) == 'id')  # macro_rules! name
        return group if group in self.match else None

    def macro_item(self, k: int) -> str | None:
        """The key of a macro invocation item at k: `path!(...);`, `path![...];` or `macro_rules! m (...);`."""
        group = self.macro_group(k)
        if group is not None and self.text(group) in ('(', '[') and self.text(self.match[group] + 1) == ';':
            return render(self.toks[k:self.match[group] + 1])
        return None

    def target(self, start: int, i: int) -> str | None:
        """The key of the unbraced item or statement at token i, whose attributes start at token `start`."""
        k = self.keyword(i)
        if k >= self.n:
            return None
        word = self.text(k)
        name = k + 1 + (word == 'static' and self.text(k + 1) == 'mut')
        if word == 'use' or (word == 'extern' and self.text(k + 1) == 'crate'):
            end = self.end(k, braces=True)
            return render(self.toks[k:end]) if self.text(end) == ';' else None
        if word in ('type', 'const', 'static'):
            return f'{word} {self.text(name)}' if self.kind(name) == 'id' else None
        if word in BODY_OR_SEMICOLON and self.kind(name) == 'id':
            return f'{word} {self.text(name)}' if self.text(self.end(k)) == ';' else None
        if word in ('impl', 'extern'):
            return None  # an impl or extern block, braced wherever it sits; never a statement
        if self.in_item_list(k):
            return self.macro_item(k)
        # A statement starts a block, or follows another statement or the block's inner attributes; a field, arm
        # or parameter never ends in `;`.
        before = start - 1
        inner = self.text(before) == ']' and before in self.match and self.text(self.match[before] - 1) == '!' \
            and self.text(self.match[before] - 2) == '#'
        if self.text(self.parent[k]) != '{' or not (inner or self.text(before) in (';', '{', '}')):
            return None
        if word == 'let':
            j = k + 1
            while j < self.n and self.text(j) not in ('=', ':', ';', ')', ']', '}'):
                j = self.match[j] + 1 if self.text(j) in ('(', '[', '{') and j in self.match else j + 1
            return f'let {render(self.toks[k + 1:j])}'
        # Only the first token makes a statement braced; braces inside its expression enclose nothing.
        group = self.macro_group(k)
        if word in BRACED_STATEMENTS or self.kind(k) == 'life' or (group is not None and self.text(group) == '{'):
            return None
        end = self.end(k, braces=True)
        return render(self.toks[k:end]) if self.text(end) == ';' and end > k else None

    def occurrences(self) -> list[Occurrence]:
        found, i = [], 0
        while i < self.n:
            if self.attribute(i) is None:
                i += 1
                continue
            start, attrs, first = i, [], None
            while (close := self.attribute(i)) is not None:
                if conditional(self.toks[i + 2:close]):
                    attrs.append(f'#[{render(self.toks[i + 2:close])}]')
                    first = i if first is None else first
                i = close + 1
            item = self.target(start, i) if attrs else None
            if item is not None:
                line = self.source.count('\n', 0, self.toks[first].offset) + 1
                found.append(Occurrence(' '.join(attrs), item, line))
        return found


def analyze(source: str) -> list[Occurrence]:
    return Analysis(source).occurrences()


def rust_files(base: Path, roots: list[str]) -> list[Path]:
    """Every .rs file under the roots, past hidden directories and Cargo build output."""
    found = set()
    for root in roots:
        top = base / root
        if top.is_file():
            found.add(top)
        for directory, subdirectories, names in os.walk(top):
            here = Path(directory)
            subdirectories[:] = [d for d in subdirectories if not d.startswith('.')
                                 and not (d == 'target' and (here / 'Cargo.toml').is_file())]
            found.update(here / name for name in names if name.endswith('.rs'))
    return sorted(found)


@dataclass
class Scan:
    occurrences: dict[tuple[str, str, str, str], list[Occurrence]] = field(default_factory=dict)
    files: int = 0
    scanned: list[str] = field(default_factory=list)
    skipped: list[str] = field(default_factory=list)


def scan(root: Path, repos: dict, skip=()) -> tuple[Scan, list[str]]:
    result, errors = Scan(), []
    for name, repo in repos.items():
        if name in skip:
            result.skipped.append(name)
            continue
        base = (root / repo['path']).resolve()
        missing = [r for r in repo['roots'] if not (base / r).exists()]
        if missing:
            errors.append(f'MISSING {name}: no {", ".join(missing)} in {base}; check {name} out there, or pass '
                          f'--skip-repo {name} (only a CI job without that checkout)')
            continue
        result.scanned.append(name)
        for path in rust_files(base, repo['roots']):
            result.files += 1
            relative = path.relative_to(base).as_posix()
            try:
                source = path.read_text(encoding='utf-8')
            except (OSError, ValueError) as error:
                errors.append(f'UNREADABLE {name}/{relative}: {error}')
                continue
            for o in analyze(source):
                result.occurrences.setdefault((name, relative, o.attrs, o.item), []).append(o)
    if not result.files:
        errors.append('no Rust file was scanned: every repository was skipped or has no .rs file under its roots')
    return result, errors


def load_allowlist(path: Path) -> tuple[dict, dict, list[str]]:
    try:
        data = json.loads(path.read_text(encoding='utf-8'))
    except (OSError, ValueError) as error:
        return {}, {}, [f'cannot read allowlist {path}: {error}']
    if not isinstance(data, dict):
        return {}, {}, [f'allowlist {path} is not a JSON object']
    repos, entries = data.get('repos') or {}, {}
    errors = [] if repos else ['allowlist must name the repositories to scan']
    errors += [f'repository {name} needs a path and a list of roots' for name, repo in repos.items()
               if not (isinstance(repo, dict) and repo.get('path') and isinstance(repo.get('roots'), list)
                       and repo['roots'])]
    for entry in data.get('entries', []):
        key = tuple(entry.get(field) for field in ('repo', 'path', 'attrs', 'item'))
        count = entry.get('count', 1)
        if not all(isinstance(part, str) and part for part in key):
            errors.append(f'allowlist entry needs repo, path, attrs and item: {entry}')
            continue
        if key[0] not in repos:
            errors.append(f'{key}: no repository {key[0]} in the allowlist')
        if key in entries:
            errors.append(f'duplicate allowlist entry: {key}')
        if not isinstance(count, int) or isinstance(count, bool) or count < 1:
            errors.append(f'{key}: count must be a positive integer')
        entries[key] = entry
    return repos, entries, errors


def check(root: Path, allowlist_path: Path, skip=()) -> tuple[list[str], Scan, dict]:
    repos, entries, errors = load_allowlist(allowlist_path)
    for name in [] if errors else skip:
        if name not in repos:
            errors.append(f'--skip-repo {name}: no such repository in the allowlist')
        elif (root / repos[name]['path']).resolve() == root.resolve():
            errors.append(f"--skip-repo {name}: refused; its path is the check's own repository, which is always "
                          'checked out and always scanned')
    if errors:
        return errors, Scan(), entries
    result, errors = scan(root, repos, skip)
    for key, occurrences in sorted(result.occurrences.items()):
        repo, path, attrs, item = key
        where = ', '.join(f'{repo}/{path}:{o.line}' for o in occurrences)
        allowed = entries[key].get('count', 1) if key in entries else 0
        if not allowed:
            errors.append(f'NEW   {attrs} {item} at {where}')
        elif len(occurrences) != allowed:
            errors.append(f'COUNT {attrs} {item} at {where}: found {len(occurrences)}, allowlisted {allowed}; '
                          'update the count only if the change removes occurrences')
    for repo, path, attrs, item in sorted(set(entries) - set(result.occurrences)):
        if repo in result.scanned:
            errors.append(f'STALE {attrs} {item} in {repo}/{path}: no longer present; remove it from the allowlist')
    return errors, result, entries


def totals(entries: dict) -> dict[tuple[str, str, str], tuple[int, list[str]]]:
    """Each (repository, attributes, target)'s count summed over its files, with those files."""
    summed: dict[tuple[str, str, str], tuple[int, list[str]]] = {}
    for (repo, path, attrs, item), entry in entries.items():
        count, paths = summed.get((repo, attrs, item), (0, []))
        summed[(repo, attrs, item)] = (count + entry.get('count', 1), paths + [path])
    return summed


def shrink(allowlist: Path, base: Path) -> tuple[list[str], str]:
    """Errors for the debt or scope the allowlist gains over `base`, a copy of it from an earlier commit, and a summary."""
    if not base.exists():
        return [], (f'conditional-compilation allowlist: no base allowlist at {base}, so nothing to compare; '
                    'only the change that creates the list has none')
    repos, now, errors = load_allowlist(allowlist)
    base_repos, before, base_errors = load_allowlist(base)
    errors += [f'base: {error}' for error in base_errors]
    loaded = not errors
    for name, was in sorted(base_repos.items()) if loaded else []:
        repo = repos.get(name)
        if repo is None:
            errors.append(f'REMOVED repository {name}: in the base allowlist, absent now')
        elif posixpath.normpath(repo['path']) != posixpath.normpath(was['path']):
            errors.append(f'MOVED repository {name}: path {was["path"]} in the base, {repo["path"]} now')
        else:
            roots = [posixpath.normpath(root) for root in repo['roots']]
            for root in map(posixpath.normpath, was['roots']):
                if not any(r in ('.', root) or root.startswith(r + '/') for r in roots):
                    errors.append(f'NARROWED repository {name}: base root {root} is not under the current roots '
                                  f'{", ".join(repo["roots"])}')
    base_totals = totals(before)
    for (repo, attrs, item), (count, paths) in sorted(totals(now).items()) if loaded else []:
        was = base_totals.get((repo, attrs, item), (0, []))[0]
        where = ', '.join(f'{repo}/{path}' for path in sorted(paths))
        if not was:
            errors.append(f'ADDED  {attrs} {item} in {where}: not in the base allowlist')
        elif count > was:
            errors.append(f'RAISED {attrs} {item} in {where}: count {count}, base {was}')
    return errors, (f'conditional-compilation allowlist: {len(now)} entries, no count summed over files above the '
                    f"base's {len(before)}, and every base repository and root still scanned")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0], allow_abbrev=False)
    parser.add_argument('--root', type=Path, default=ROOT, help='gwz-core checkout that repository paths start from')
    parser.add_argument('--allowlist', type=Path, help='allowlist JSON (default: next to this script)')
    parser.add_argument('--skip-repo', action='append', default=[], metavar='NAME',
                        help='skip one sibling repository and print SKIPPED GATE; only for a CI job without its checkout')
    parser.add_argument('--list', action='store_true', help='print every occurrence found and exit')
    parser.add_argument('--shrink-from', type=Path, metavar='BASE',
                        help='only compare the allowlist with BASE, a copy of it from an earlier commit, and fail '
                             'when a count summed over files rises or the scanned scope narrows; a missing BASE '
                             'passes')
    options = parser.parse_args(argv)
    allowlist = (options.allowlist or DEFAULT_ALLOWLIST).resolve()
    if options.shrink_from is not None:
        errors, summary = shrink(allowlist, options.shrink_from)
        if errors:
            print(f'conditional-compilation boundary guard failed (allowlist: {allowlist}, base: '
                  f'{options.shrink_from}):', file=sys.stderr)
            for error in errors:
                print(f'  {error}', file=sys.stderr)
            print('The allowlist only shrinks, in debt and never in scope: migrate a new occurrence into a '
                  'cfg_if::cfg_if! block or a platform module instead of listing it, and keep scanning every '
                  'repository and root the base scans.', file=sys.stderr)
            return 1
        print(summary)
        return 0
    errors, result, entries = check(options.root.resolve(), allowlist, options.skip_repo)
    for name in result.skipped:
        print(f'SKIPPED GATE: conditional-compilation boundary check of {name} (--skip-repo {name}): '
              f'this run has no {name} checkout, so its allowlist entries are not checked', flush=True)
    if options.list:
        for (repo, path, attrs, item), occurrences in sorted(result.occurrences.items()):
            print(f'{repo}/{path}:{",".join(str(o.line) for o in occurrences)}\t{attrs}\t{item}')
        errors = [error for error in errors if not error.startswith(RATCHET)]
    if errors:
        print(f'conditional-compilation boundary guard failed (allowlist: {allowlist}):', file=sys.stderr)
        for error in errors:
            print(f'  {error}', file=sys.stderr)
        if any(error.startswith(RATCHET) for error in errors):
            print('Put conditional code in a cfg_if::cfg_if! block, an enclosing platform module or a braced '
                  'block, never as a cfg or conditional cfg_attr directly on an import, another unbraced '
                  'declaration or a statement (root AGENTS.md, explicit scope). Remove an entry when its '
                  'occurrence goes.', file=sys.stderr)
        return 1
    if not options.list:
        counts = Counter()
        for (repo, *_), entry in entries.items():
            counts[repo] += entry.get('count', 1)
        listed = ', '.join(f'{name} {counts[name]}' for name in result.scanned)
        print(f'conditional-compilation boundary guard: {result.files} files, '
              f'{sum(counts[name] for name in result.scanned)} listed occurrences ({listed}); nothing new')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())

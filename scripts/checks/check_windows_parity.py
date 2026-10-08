#!/usr/bin/env python3
"""Ratchet guard: the Unix-only gates of the transport are inventoried and only shrink.

GwzTransportWindowsParityPlan.md, step 0.3. Every Unix-only gate and every
Unix-only OS call under the transport's scope (the inventory's `roots`) has an
entry in windows_parity_inventory.json with an owner step of the plan and a
state:
- `unported`: a Unix-only gate that still waits for its owner step;
- `paired`: the gate, or the Unix-only region that holds the OS call, has a Windows arm beside
  it (a sibling arm of the same `cfg_if!`, or a sibling item with a Windows-only `cfg`);
- `platform`: permanent, with a recorded `reason` (for example `AF_UNIX`, or an
  OpenSSL-only trust branch).

The scan is lexical and inspects every arm, none compiled (it reuses
check_cfg_boundaries.py's lexer). Two kinds of occurrence are found:
- `gate`: a `cfg_if!` arm, a `#[cfg(..)]` attribute or an inner `#![cfg(..)]`
  whose predicate is false on Windows. Predicates are evaluated three-valued
  (`unix`, `windows`, `target_os`, `target_family` and `target_vendor` are known
  for Windows; `test`, features and the rest are unknown), so `unix`,
  `all(test, unix)`, `not(windows)`, `target_os = "linux"` and
  `not(any(windows, target_vendor = "apple"))` are Unix-only, and `windows`,
  `not(unix)` and `all(windows, gwz_transport_candidate)` are not. A gate is
  keyed by its file, its predicate text and its target: the `mod name;` of a
  module list, else the first item or statement of the arm or attribute. Layout
  and line numbers never change a key.
- `os`: a use of `os::unix`, `os::fd` or `libc::..` (the extension traits such as `OsStrExt` come in through
  `os::unix`), keyed by file and path text, and `side`: `windows` when the use
  sits in a region that only a Windows build compiles.

The check fails on: NEW (an occurrence with no entry), COUNT (more occurrences
than the entry lists), STALE (an entry with no occurrence), PAIRED (a `paired`
entry whose Windows arm is gone), DONE (an `unported` entry whose owner steps
are all recorded done in `done_steps`), PROOF (a `libc::` or `os::unix` use in
a Windows arm whose entry carries no `proof`, the evidence label of the run that
shows it is right, for example step 3.4's CRT-sharing proof) and the
inventory's own errors (an unknown owner step, a `platform` entry without a
`reason`, a duplicate key). A step the plan does not name (`0.5b`, the TLS fixture identity on Windows) carries
its reason in the inventory's `step_notes`. --shrink-from BASE compares the inventory with a
copy from an earlier commit and fails when the unported count rises, when a
scope root disappears or when a step is no longer recorded done. --list prints
every occurrence as an inventory line, for adding entries.
"""
import argparse
from collections import Counter, namedtuple
import importlib.util
import json
from pathlib import Path
import posixpath
import sys

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_INVENTORY = Path(__file__).resolve().with_name('windows_parity_inventory.json')

_spec = importlib.util.spec_from_file_location('check_cfg_boundaries',
                                               Path(__file__).with_name('check_cfg_boundaries.py'))
cfg = importlib.util.module_from_spec(_spec)
sys.modules.setdefault('check_cfg_boundaries', cfg)
_spec.loader.exec_module(cfg)

STATES = ('unported', 'paired', 'platform')
NAMED_ITEMS = {'fn', 'mod', 'struct', 'enum', 'trait', 'type', 'const', 'static', 'union'}
Occurrence = namedtuple('Occurrence', 'path kind gate target side line windows_arm')
Region = namedtuple('Region', 'lo hi on_windows on_unix paired')  # token range [lo, hi); the values are True, False or None
# The Kleene environments the predicates are evaluated in; unknown atoms are None.
WINDOWS = {'atoms': {'windows': True, 'unix': False},
           'pairs': {'target_os': 'windows', 'target_family': 'windows', 'target_vendor': 'pc'}}
UNIX = {'atoms': {'windows': False, 'unix': True}, 'pairs': {'target_family': 'unix'}}


# ---- predicates -----------------------------------------------------------------------------------------------


def parse_predicate(tokens, i=0):
    """The predicate at tokens[i] as a tree, and the index after it."""
    name = tokens[i].text
    if i + 1 < len(tokens) and tokens[i + 1].text == '(':
        depth, j, children, start = 0, i + 1, [], i + 2
        while j < len(tokens):
            text = tokens[j].text
            depth += (text in '([{') - (text in ')]}')
            if depth == 0:
                break
            j += 1
        k = start
        while k < j:
            child, k = parse_predicate(tokens, k)
            children.append(child)
            if k < j and tokens[k].text == ',':
                k += 1
        return ('call', name, children), j + 1
    if i + 2 < len(tokens) and tokens[i + 1].text == '=':
        return ('pair', name, tokens[i + 2].text.strip('"')), i + 3
    return ('atom', name), i + 1


def evaluate(tree, env):
    """True, False or None (unknown) for the predicate tree in the environment."""
    kind, name = tree[0], tree[1]
    if kind == 'atom':
        return env['atoms'].get(name)
    if kind == 'pair':
        known = env['pairs'].get(name)
        return None if known is None else known == tree[2]
    values = [evaluate(child, env) for child in tree[2]]
    if name == 'not':
        return None if len(values) != 1 or values[0] is None else not values[0]
    if name == 'all':
        return False if False in values else (True if all(v is True for v in values) else None)
    if name == 'any':
        return True if True in values else (False if all(v is False for v in values) else None)
    return None


def both(tree):
    return evaluate(tree, WINDOWS), evaluate(tree, UNIX)


def negate(value):
    return None if value is None else not value


def conjoin(a, b):
    if a is False or b is False:
        return False
    return True if a is True and b is True else None


# ---- one file -------------------------------------------------------------------------------------------------


class Scanner:
    def __init__(self, source: str, path: str):
        self.a = cfg.Analysis(source)
        self.path = path
        self.source = source
        self.regions: list[Region] = []
        self.found: list[Occurrence] = []
        self.arm_attributes: set[int] = set()  # the `#[cfg(..)]` of a cfg_if! arm is not an item attribute

    def line(self, i):
        return self.source.count('\n', 0, self.a.toks[i].offset) + 1

    def predicate_at(self, open_paren):
        """The predicate tree of a `cfg(...)` whose `(` is at open_paren, else None."""
        close = self.a.match.get(open_paren)
        if close is None or open_paren + 1 >= close:
            return None, ''
        inner = self.a.toks[open_paren + 1:close]
        tree, _ = parse_predicate(inner)
        return tree, cfg.render(inner)

    def label(self, i):
        """A stable name for the item or statement starting at token i."""
        a = self.a
        k = a.keyword(i)
        word = a.text(k)
        if word == 'use':
            end = a.end(k, braces=True)
            return ('use ' + cfg.render(a.toks[k + 1:end]))[:100]
        if word in NAMED_ITEMS:
            name = k + 1 + (word == 'static' and a.text(k + 1) == 'mut')
            if a.kind(name) == 'id':
                return f'{word} {a.text(name)}'
        if word == 'impl':
            end = a.end(k)
            return ('impl ' + cfg.render(a.toks[k + 1:end]))[:80]
        group = a.macro_group(k)
        if group is not None:
            return f'macro {cfg.render(a.toks[k:group - 1])}!'[:60]
        end = min(a.end(k, braces=True), k + 8, a.n)
        return ('stmt ' + cfg.render(a.toks[k:max(end, k + 1)]))[:60]

    def item_end(self, i):
        """The index after the item or statement starting at token i."""
        a = self.a
        k = a.keyword(i)
        group = a.macro_group(k)
        if group is not None:
            after = a.match[group] + 1
            return after + (a.text(after) == ';')
        word = a.text(k)
        end = a.end(k, braces=True) if word in ('use', 'let') else a.end(k)
        if a.text(end) == '{' and end in a.match:
            return a.match[end] + 1
        return end + (a.text(end) == ';')

    def children(self, lo, hi):
        """(start, end) token ranges of the items and statements directly in [lo, hi), attributes included."""
        a, out, i = self.a, [], lo
        while i < hi:
            start = i
            while (close := a.attribute(i)) is not None:
                i = close + 1
            if i >= hi:
                break
            end = max(self.item_end(i), i + 1)
            out.append((start, i, min(end, hi)))
            i = min(end, hi)
        return out

    def scan(self):
        a = self.a
        for i in range(a.n):
            if a.text(i) == 'cfg_if' and a.text(i + 1) == '!' and a.text(i + 2) in ('{', '(', '[') and i + 2 in a.match:
                self.cfg_if(i + 2)
            close = a.attribute(i)
            if close is not None and i not in self.arm_attributes and a.text(i + 2) == 'cfg' \
                    and a.text(i + 3) == '(':
                self.attribute_gate(i, close)
            if a.text(i) == '#' and a.text(i + 1) == '!' and a.text(i + 2) == '[' and a.text(i + 3) == 'cfg' \
                    and a.text(i + 4) == '(' and i + 2 in a.match:
                self.inner_gate(i)
        self.os_uses()
        return self.found

    def gate_entries(self, gate, on_windows, on_unix, entries):
        """Records a gate that only a Unix build compiles, one occurrence per (target, windows_arm, line)."""
        if on_windows is False and on_unix is not False:
            for target, windows_arm, line in entries:
                self.found.append(Occurrence(self.path, 'gate', gate, target, '', line, windows_arm))

    def inner_gate(self, i):
        a = self.a
        tree, text = self.predicate_at(i + 4)
        if tree is None:
            return
        on_windows, on_unix = both(tree)
        parent = a.parent[i]
        lo, hi = (0, a.n) if parent is None else (parent + 1, a.match.get(parent, a.n))
        self.regions.append(Region(lo, hi, on_windows, on_unix, False))
        self.gate_entries(text, on_windows, on_unix, [('file' if parent is None else 'mod body', False, self.line(i))])

    def attribute_gate(self, i, close):
        a = self.a
        tree, text = self.predicate_at(i + 3)
        if tree is None:
            return
        on_windows, on_unix = both(tree)
        after = close + 1
        while (more := a.attribute(after)) is not None:
            after = more + 1
        if after >= a.n:
            return
        end = max(self.item_end(after), after + 1)
        label = self.label(after)
        paired = self.windows_sibling(i, after, label)
        self.regions.append(Region(i, end, on_windows, on_unix, paired))
        self.gate_entries(text, on_windows, on_unix, [(label, paired, self.line(i))])

    def windows_sibling(self, attribute, item, label):
        """True when a sibling declaration of the same name carries a Windows-only cfg."""
        a = self.a
        parent = a.parent[attribute]
        lo, hi = (0, a.n) if parent is None else (parent + 1, a.match.get(parent, a.n))
        for start, first, end in self.children(lo, hi):
            if first == item or first == start:
                continue
            for j in range(start, first):
                if a.attribute(j) is not None and a.text(j + 2) == 'cfg' and a.text(j + 3) == '(':
                    tree, _ = self.predicate_at(j + 3)
                    if tree is None:
                        continue
                    on_windows, on_unix = both(tree)
                    sibling = self.label(first)
                    same = sibling == label or (sibling.startswith('use ') and label.startswith('use '))
                    if same and on_windows is not False and on_unix is False:
                        return True
        return False

    def cfg_if(self, group):
        a = self.a
        close_group = a.match[group]
        p, arms = group + 1, []  # an arm: (predicate text, tree or None, body open)
        while p < close_group:
            if a.text(p) == 'if' and a.text(p + 1) == '#' and a.text(p + 2) == '[' and a.text(p + 3) == 'cfg' \
                    and a.text(p + 4) == '(' and (p + 2) in a.match:
                tree, text = self.predicate_at(p + 4)
                body = a.match[p + 2] + 1
                if tree is None or a.text(body) != '{' or body not in a.match:
                    return
                self.arm_attributes.add(p + 1)
                arms.append((text if not arms else f'else if {text}', tree, body))
                p = a.match[body] + 1
                if a.text(p) == 'else':
                    p += 1
                    if a.text(p) == '{' and p in a.match:
                        arms.append(('else', None, p))
                        break
                continue
            return
        raws = [both(tree) if tree is not None else (True, True) for _, tree, _ in arms]
        evaluated = []
        for index, raw in enumerate(raws):
            on_windows, on_unix = raw
            for prior in raws[:index]:  # an arm runs only when every earlier arm's condition failed
                on_windows, on_unix = conjoin(on_windows, negate(prior[0])), conjoin(on_unix, negate(prior[1]))
            evaluated.append((on_windows, on_unix))
        for index, (text, tree, body) in enumerate(arms):
            on_windows, on_unix = evaluated[index]
            lo, hi = body + 1, a.match[body]
            windows_arm = any(v[0] is not False for j, v in enumerate(evaluated) if j != index)
            self.regions.append(Region(lo, hi, on_windows, on_unix, windows_arm))
            if on_windows is False and on_unix is not False:
                entries = self.arm_entries(lo, hi, windows_arm)
                self.gate_entries(text, on_windows, on_unix, entries)

    def arm_entries(self, lo, hi, windows_arm):
        """A `mod name;` child is its own target; the other children share one, named for the first."""
        a, mods, first = self.a, [], None
        for start, item, end in self.children(lo, hi):
            label = self.label(item)
            line = self.line(start)
            if label.startswith('macro cfg_if'):
                continue
            if label.startswith('mod ') and a.text(end - 1) == ';':
                mods.append((label, windows_arm, line))
            elif first is None:
                first = (f'arm: {label}', windows_arm, line)
        return mods + ([first] if first is not None else [])

    def os_path(self, i):
        """The end index (exclusive) of the `a::b::c` path or `a::{..}` group starting at token i."""
        a, p = self.a, i
        while a.kind(p) == 'id' and a.text(p + 1) == '::':
            p += 2
            if a.text(p) == '{' and p in a.match:
                return a.match[p] + 1
        return p + (a.kind(p) == 'id')

    def os_uses(self):
        a, i = self.a, 0
        while i < a.n:
            word = a.text(i)
            start = None
            if word == 'libc' and a.kind(i) == 'id' and a.text(i - 1) != '::' and a.text(i + 1) in ('::', ';', ',', '}'):
                start = i
            elif word == 'os' and a.text(i + 1) == '::' and a.text(i + 2) in ('unix', 'fd'):
                start = i
            if start is None:
                i += 1
                continue
            end = self.os_path(start)
            text = cfg.render(a.toks[start:end])
            side = 'windows' if self.in_windows_arm(start) else ''
            self.found.append(Occurrence(self.path, 'os', text, '', side, self.line(start), self.beside_windows_arm(start)))
            i = max(end, i + 1)

    def beside_windows_arm(self, t):
        """True when the Unix-only region that holds token t has a Windows arm beside it."""
        return any(r.lo <= t < r.hi and r.on_windows is False and r.paired for r in self.regions)

    def in_windows_arm(self, t):
        inside = [r for r in self.regions if r.lo <= t < r.hi]
        return any(r.on_unix is False and r.on_windows is not False for r in inside) \
            and not any(r.on_windows is False for r in inside)


def analyze(source: str, path: str = 'src/x.rs') -> list[Occurrence]:
    """Every gate and OS use in the source; the os pass runs after the gates so their regions are known."""
    return Scanner(source, path).scan()


# ---- the tree and the inventory -------------------------------------------------------------------------------


def scope_files(root: Path, roots: list[str]) -> tuple[list[Path], list[str]]:
    """Every .rs file under the scope roots, and the roots that matched nothing. A root ending in `*` is a prefix."""
    files, missing = set(), []
    for pattern in roots:
        tops = sorted((root / pattern).parent.glob(Path(pattern).name)) if pattern.endswith('*') \
            else ([root / pattern] if (root / pattern).exists() else [])
        if not tops:
            missing.append(pattern)
        files.update(cfg.rust_files(root, [top.relative_to(root).as_posix() for top in tops]))
    return sorted(f for f in files if f.suffix == '.rs'), missing


def key_of(entry) -> tuple:
    return (entry['path'], entry['kind'], entry['gate'], entry['target'], entry.get('side', ''))


def load_inventory(path: Path):
    try:
        data = json.loads(path.read_text(encoding='utf-8'))
    except (OSError, ValueError) as error:
        return None, [f'cannot read inventory {path}: {error}']
    errors = []
    if not isinstance(data, dict) or not isinstance(data.get('roots'), list) or not data['roots']:
        return None, [f'inventory {path} needs a JSON object with a list of scope roots']
    steps, done = data.get('steps') or [], data.get('done_steps') or []
    errors += [f'done step {step} is not in steps' for step in done if step not in steps]
    seen = set()
    for entry in data.get('entries', []):
        where = f"{entry.get('path')} {entry.get('kind')} {entry.get('gate')} {entry.get('target')}"
        if not all(isinstance(entry.get(f), str) for f in ('path', 'kind', 'gate', 'target', 'state')):
            errors.append(f'entry needs path, kind, gate, target and state: {entry}')
            continue
        if entry['kind'] not in ('gate', 'os'):
            errors.append(f'{where}: kind must be gate or os')
        if entry['state'] not in STATES:
            errors.append(f'{where}: state must be one of {", ".join(STATES)}')
        owners = entry.get('owner')
        if not (isinstance(owners, list) and owners and all(o in steps for o in owners)):
            errors.append(f'{where}: owner must be a list of steps from the inventory\'s steps, got {owners}')
        if entry['state'] == 'platform' and not entry.get('reason'):
            errors.append(f'{where}: a platform entry needs a reason')
        if entry.get('proof') is not None and not (isinstance(entry['proof'], str) and entry['proof']):
            errors.append(f'{where}: proof must name an evidence label')
        count = entry.get('count', 1)
        if not isinstance(count, int) or isinstance(count, bool) or count < 1:
            errors.append(f'{where}: count must be a positive integer')
        if key_of(entry) in seen:
            errors.append(f'duplicate inventory entry: {where}')
        seen.add(key_of(entry))
    return data, errors


def check(root: Path, inventory_path: Path):
    data, errors = load_inventory(inventory_path)
    if data is None or errors:
        return errors, 0, {}
    files, missing = scope_files(root, data['roots'])
    errors += [f'MISSING scope root {pattern}: nothing there; a scope never narrows' for pattern in missing]
    entries = {key_of(e): e for e in data.get('entries', [])}
    found: dict[tuple, list[Occurrence]] = {}
    for file in files:
        relative = file.relative_to(root).as_posix()
        try:
            source = file.read_text(encoding='utf-8')
        except (OSError, ValueError) as error:
            errors.append(f'UNREADABLE {relative}: {error}')
            continue
        for o in analyze(source, relative):
            found.setdefault((o.path, o.kind, o.gate, o.target, o.side), []).append(o)
    done = set(data.get('done_steps') or [])
    for key, occurrences in sorted(found.items()):
        path, kind, gate, target, side = key
        where = ', '.join(f'{path}:{o.line}' for o in occurrences)
        entry = entries.get(key)
        if entry is None:
            errors.append(f'NEW   {kind} {gate} {target} at {where}: add an entry with an owner step of the plan, '
                          'or port it; --list prints the lines')
            continue
        if len(occurrences) > entry.get('count', 1):
            errors.append(f'COUNT {kind} {gate} {target} at {where}: found {len(occurrences)}, '
                          f"listed {entry.get('count', 1)}")
        elif len(occurrences) < entry.get('count', 1):
            errors.append(f'STALE {kind} {gate} {target} in {path}: found {len(occurrences)}, '
                          f"listed {entry.get('count', 1)}; lower the count")
        if entry['state'] == 'paired' and not all(o.windows_arm for o in occurrences):
            errors.append(f'PAIRED {gate} {target} at {where}: listed paired, but no Windows arm sits beside it')
        if entry['state'] == 'unported' and set(entry['owner']) <= done:
            errors.append(f"DONE  {gate} {target} at {where}: its owner step {', '.join(entry['owner'])} is recorded "
                          'done, but the gate is still Unix-only; port it, or record why it stays (platform)')
        if side == 'windows' and not entry.get('proof'):
            errors.append(f'PROOF {gate} at {where}: a Unix-only call in a Windows arm needs a proof field naming '
                          'the evidence label that shows it is right')
    scanned = {f.relative_to(root).as_posix() for f in files}
    for key in sorted(set(entries) - set(found)):
        errors.append(f'STALE {key[1]} {key[2]} {key[3]} in {key[0]}: no longer present; remove it from the inventory'
                      + ('' if key[0] in scanned else ' (the file is gone or out of scope)'))
    return errors, len(files), entries


def unported(data) -> int:
    return sum(e.get('count', 1) for e in data.get('entries', []) if e.get('state') == 'unported')


def shrink(inventory: Path, base: Path):
    """Errors for what the inventory gains over BASE, a copy from an earlier commit, and a summary line."""
    if not base.exists():
        return [], f'Windows-parity inventory: no base inventory at {base}, so nothing to compare'
    now, errors = load_inventory(inventory)
    was, base_errors = load_inventory(base)
    errors += [f'base: {error}' for error in base_errors]
    if now is None or was is None or errors:
        return errors, ''
    for root in map(posixpath.normpath, was['roots']):
        if not any(root == posixpath.normpath(r) for r in now['roots']):
            errors.append(f'NARROWED scope root {root} is in the base inventory, absent now')
    for step in was.get('done_steps') or []:
        if step not in (now.get('done_steps') or []):
            errors.append(f'REOPENED step {step} is recorded done in the base, not now')
    if unported(now) > unported(was):
        errors.append(f'RAISED the unported count is {unported(now)}, the base has {unported(was)}: port the gate, '
                      'pair it with a Windows arm, or record why it stays (platform)')
    return errors, (f'Windows-parity inventory: {unported(now)} unported against the base\'s {unported(was)}; '
                    'no scope root dropped')


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0], allow_abbrev=False)
    parser.add_argument('--root', type=Path, default=ROOT, help='gwz-core checkout the scope roots start from')
    parser.add_argument('--inventory', type=Path, help='inventory JSON (default: next to this script)')
    parser.add_argument('--list', action='store_true', help='print every occurrence as an inventory line and exit')
    parser.add_argument('--shrink-from', type=Path, metavar='BASE',
                        help='only compare the inventory with BASE, a copy from an earlier commit; a missing BASE passes')
    options = parser.parse_args(argv)
    inventory = (options.inventory or DEFAULT_INVENTORY).resolve()
    if options.shrink_from is not None:
        errors, summary = shrink(inventory, options.shrink_from)
        if errors:
            print(f'Windows-parity guard failed (inventory: {inventory}, base: {options.shrink_from}):', file=sys.stderr)
            for error in errors:
                print(f'  {error}', file=sys.stderr)
            return 1
        print(summary)
        return 0
    if options.list:
        data, errors = load_inventory(inventory)
        if data is None:
            print('\n'.join(errors), file=sys.stderr)
            return 1
        files, _ = scope_files(options.root.resolve(), data['roots'])
        counted: Counter = Counter()
        lines: dict = {}
        for file in files:
            for o in analyze(file.read_text(encoding='utf-8'), file.relative_to(options.root.resolve()).as_posix()):
                key = (o.path, o.kind, o.gate, o.target, o.side)
                counted[key] += 1
                lines.setdefault(key, []).append((o.line, o.windows_arm))
        for key, count in sorted(counted.items()):
            path, kind, gate, target, side = key
            print(json.dumps({'path': path, 'kind': kind, 'gate': gate, 'target': target, 'side': side, 'count': count,
                              'lines': [n for n, _ in lines[key]], 'windows_arm': all(w for _, w in lines[key])}))
        return 0
    errors, files, entries = check(options.root.resolve(), inventory)
    if errors:
        print(f'Windows-parity guard failed (inventory: {inventory}):', file=sys.stderr)
        for error in errors:
            print(f'  {error}', file=sys.stderr)
        print('Every Unix-only gate or OS call in the transport has an inventory entry with an owner step of '
              'GwzTransportWindowsParityPlan.md; the inventory only shrinks (step 0.3).', file=sys.stderr)
        return 1
    states = Counter()
    for entry in entries.values():
        states[entry['state']] += entry.get('count', 1)
    print(f'Windows-parity guard: {files} files, {sum(states.values())} listed occurrences '
          f'({", ".join(f"{s} {states[s]}" for s in STATES)}); nothing new')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())

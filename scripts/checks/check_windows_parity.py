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

The inventory is the directory windows_parity/ next to this script: meta.json
(`roots`, `scans`, `steps`, `done_steps`, `step_notes`) and one `<step>.json` per
owning step, `{"done": true, "entries": [...]}`, so two lanes porting different steps
edit different files. An entry lives in the file of its first owner step; a step is
done when meta.json lists it in `done_steps` or its file says `"done": true`. The
old single windows_parity_inventory.json is still read, as a --shrink-from base.

The scan is lexical and inspects every arm, none compiled (it reuses
check_cfg_boundaries.py's lexer). Three kinds of occurrence are found:
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
- `os`: a use of `os::unix`, `os::fd`, `os::linux`, `os::macos` or `libc::..` (the extension traits such as
  `OsStrExt` come in through `os::unix`), keyed by file and path text, and `side`: `windows` when the use
  sits in a region that only a Windows build compiles.
- `split`: a runtime platform split that one build carries both sides of: `cfg!(..)` or `cfg_attr(.., ..)` whose
  predicate names `unix`, `windows`, `target_os`, `target_family` or `target_vendor`, keyed by its predicate text
  (and, for `cfg_attr`, the item it sits on). It always has a Windows arm beside it, so `paired` is the natural state;
  `unported` records a split whose Windows side is a stub or a skip, and counts in the ratchet. A predicate that names
  `gwz_windows_https_qualification` is left out: those sites are counted by check_candidate_switches.py and step 5.1.

The check fails on: NEW (an occurrence with no entry), COUNT (more occurrences
than the entry lists), STALE (an entry with no occurrence), PAIRED (a `paired`
entry whose Windows arm is gone), DONE (an `unported` entry whose owner steps
are all recorded done in `done_steps`), PROOF (a `libc::` or `os::unix` use in
a Windows arm whose entry carries no `proof`, the evidence label of the run that
shows it is right, for example step 3.4's CRT-sharing proof) and the
inventory's own errors (an unknown owner step, a `platform` entry without a
`reason`, a duplicate key). A step the plan does not name (`0.5b`, the TLS fixture identity on Windows) carries
its reason in the inventory's `step_notes`. --shrink-from BASE compares the inventory with a
copy from an earlier commit (a directory, or the old single file) and fails when the unported count rises, when a
scope root disappears or when a step is no longer recorded done. Two rules keep the comparison honest:
- Only the kinds both sides scan are compared (`scans` in meta.json; a single file scanned `gate` and `os`). A kind
  this checker gained after the base was made, such as `split`, therefore adds rows without raising the count against
  that base; once the base scans it, the kind is compared like any other. The one-time reclassification passes
  once, and never again.
- A row that becomes `platform` (new, or relabelled from `unported` or `paired`, or with a higher count) is listed
  in the output and fails with PLATFORM unless it carries a `reason` and `"approved_platform": "<where the decision
  is recorded>"` (an operator decision, a review finding, an open decision of the plan). `true` is refused: a
  relabel must name a record the reviewer can open. --list prints
every occurrence as an inventory line, for adding entries.

Maintaining the inventory (one row per line, so lane edits stay line-local):
- A new Unix-only gate, OS call or split: the check prints NEW with its file and line; --list prints the row as
  JSON. Put it, with an `owner` list of plan steps and a short `appendix` note naming where it comes from, in
  the file of its first owner (`1.4.json`); a step the plan does not name goes in meta.json's `steps`, with a
  `step_notes` reason.
- Porting a gate (it is ungated, or gains a Windows arm): delete its row, or set `state` to `paired`; lower
  `count` when only some of a row's occurrences go.
- A finished step: set `"done": true` in its file (keep the file, with `"entries": []` if it has no rows left), or
  add the step to meta.json's `done_steps`. A row still `unported` whose owners are all done fails with DONE.
- Never edit another step's rows to make room for yours; a row moves to another file only when its first owner
  changes.
"""
import argparse
from collections import Counter, namedtuple
import importlib.util
import json
from pathlib import Path
import posixpath
import sys

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_INVENTORY = Path(__file__).resolve().with_name('windows_parity')
META = 'meta.json'

_spec = importlib.util.spec_from_file_location('check_cfg_boundaries',
                                               Path(__file__).with_name('check_cfg_boundaries.py'))
cfg = importlib.util.module_from_spec(_spec)
sys.modules.setdefault('check_cfg_boundaries', cfg)
_spec.loader.exec_module(cfg)

STATES = ('unported', 'paired', 'platform')
# The kinds of occurrence this checker scans. An inventory directory's meta.json lists them as `scans`, and a
# single-file inventory (the format before the split) scanned only the first two; --shrink-from compares only the
# kinds both sides scan, so a kind added later never reads as a rise against a base that could not see it.
SCANS = ('gate', 'os', 'split')
LEGACY_SCANS = ('gate', 'os')
# A runtime split `cfg!(..)` or `cfg_attr(..)` is seen when its predicate names the platform. The Windows
# qualification switch is counted elsewhere (check_candidate_switches.py, plan step 5.1) and is left out here.
PLATFORM_ATOMS = {'unix', 'windows', 'target_os', 'target_family', 'target_vendor'}
OS_MODULES = ('unix', 'fd', 'linux', 'macos')
QUALIFICATION_SWITCH = 'gwz_windows_https_qualification'
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


def names_platform(tree) -> bool:
    """True when the predicate tree mentions the platform (`unix`, `windows`, `target_os` and kin)."""
    if tree[0] == 'call':
        return any(names_platform(child) for child in tree[2])
    return tree[1] in PLATFORM_ATOMS


def mentions(tree, name: str) -> bool:
    if tree[0] == 'call':
        return any(mentions(child, name) for child in tree[2])
    return tree[1] == name


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

    def first_predicate_at(self, open_paren):
        """The first argument of a `cfg_attr(pred, ..)` whose `(` is at open_paren: its tree and rendered text."""
        close = self.a.match.get(open_paren)
        if close is None or open_paren + 1 >= close:
            return None, ''
        inner = self.a.toks[open_paren + 1:close]
        tree, end = parse_predicate(inner)
        return tree, cfg.render(inner[:end])

    def split(self, gate, target, line):
        """A runtime split carries both platforms' behaviour in one build, so a Windows arm is always beside it."""
        self.found.append(Occurrence(self.path, 'split', gate, target, '', line, True))

    def cfg_macro(self, i):
        """`cfg!(pred)` with a platform predicate (not the qualification switch) is a runtime split."""
        tree, text = self.predicate_at(i + 2)
        if tree is not None and names_platform(tree) and not mentions(tree, QUALIFICATION_SWITCH):
            self.split(f'cfg!({text})', '', self.line(i))

    def cfg_attr_split(self, i, open_paren, target):
        tree, text = self.first_predicate_at(open_paren)
        if tree is not None and names_platform(tree) and not mentions(tree, QUALIFICATION_SWITCH):
            self.split(f'cfg_attr({text})', target, self.line(i))

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
            if a.text(i) == 'cfg' and a.text(i + 1) == '!' and a.text(i + 2) == '(' and i + 2 in a.match:
                self.cfg_macro(i)
            close = a.attribute(i)
            if close is not None and i not in self.arm_attributes and a.text(i + 2) == 'cfg' \
                    and a.text(i + 3) == '(':
                self.attribute_gate(i, close)
            if close is not None and a.text(i + 2) == 'cfg_attr' and a.text(i + 3) == '(' and close + 1 < a.n:
                after = close + 1
                while (more := a.attribute(after)) is not None:
                    after = more + 1
                self.cfg_attr_split(i, i + 3, self.label(after) if after < a.n else 'file')
            if a.text(i) == '#' and a.text(i + 1) == '!' and a.text(i + 2) == '[' and a.text(i + 3) == 'cfg_attr' \
                    and a.text(i + 4) == '(' and i + 2 in a.match:
                self.cfg_attr_split(i, i + 4, 'file' if a.parent[i] is None else 'mod body')
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
            elif word == 'os' and a.text(i + 1) == '::' and a.text(i + 2) in OS_MODULES:
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


def describe(entry) -> str:
    return f"{entry.get('path')} {entry.get('kind')} {entry.get('gate')} {entry.get('target')}"


def step_order(steps: list, stem: str) -> tuple:
    return (steps.index(stem), '') if stem in steps else (len(steps), stem)


def read_file(path: Path):
    try:
        data = json.loads(path.read_text(encoding='utf-8'))
    except (OSError, ValueError) as error:
        return None, [f'cannot read inventory {path}: {error}']
    if not isinstance(data, dict):
        return None, [f'inventory {path} needs a JSON object']
    return data, []


def read_directory(path: Path):
    """An inventory directory as one inventory: meta.json (roots, steps, done_steps, step_notes, scans) and one
    `<step>.json` per owning step, `{"done": true, "entries": [...]}`. An entry lives in the file of its first owner."""
    meta, errors = read_file(path / META)
    if meta is None:
        return None, errors
    steps, done = list(meta.get('steps') or []), list(meta.get('done_steps') or [])
    entries = []
    for file in sorted((f for f in path.glob('*.json') if f.name != META), key=lambda f: step_order(steps, f.stem)):
        body, problems = read_file(file)
        errors += problems
        if body is None:
            continue
        if file.stem not in steps:
            errors.append(f'{file.name}: {file.stem} is not a step in {META}\'s steps')
        for key in sorted(set(body) - {'done', 'entries'}):
            errors.append(f'{file.name}: unknown key {key}; a step file has `done` and `entries`')
        if body.get('done') not in (None, True, False):
            errors.append(f'{file.name}: done must be true or false')
        if body.get('done') is True and file.stem not in done:
            done.append(file.stem)
        listed = body.get('entries', [])
        if not isinstance(listed, list):
            errors.append(f'{file.name}: entries must be a list')
            continue
        for entry in listed:
            owners = entry.get('owner') if isinstance(entry, dict) else None
            if isinstance(owners, list) and owners and owners[0] != file.stem:
                errors.append(f'FILED {describe(entry)}: lives in {file.name}, but its first owner is {owners[0]}; '
                              f'move it to {owners[0]}.json')
        entries += listed
    data = dict(meta, done_steps=done, entries=entries)
    data.setdefault('scans', list(SCANS))
    return data, errors


def load_inventory(path: Path, current: bool = True):
    """The inventory at PATH, a directory (see read_directory) or the single file it replaced, and its errors.
    `current` is False for a base copy from an earlier commit, whose scans need not be the checker's."""
    if path.is_dir():
        data, errors = read_directory(path)
        if data is not None and current and sorted(data['scans']) != sorted(SCANS):
            errors.append(f'{META}: scans must list exactly the kinds this checker scans, {", ".join(SCANS)}; '
                          f'got {data["scans"]}')
    else:
        data, errors = read_file(path)
        if data is not None:
            data.setdefault('scans', list(LEGACY_SCANS))
    if data is None:
        return None, errors
    if not isinstance(data.get('roots'), list) or not data['roots']:
        return None, [f'inventory {path} needs a list of scope roots']
    steps, done = data.get('steps') or [], data.get('done_steps') or []
    errors += [f'done step {step} is not in steps' for step in done if step not in steps]
    seen = set()
    for entry in data.get('entries', []):
        where = describe(entry)
        if not all(isinstance(entry.get(f), str) for f in ('path', 'kind', 'gate', 'target', 'state')):
            errors.append(f'entry needs path, kind, gate, target and state: {entry}')
            continue
        if entry['kind'] not in SCANS:
            errors.append(f'{where}: kind must be one of {", ".join(SCANS)}')
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


def unported(data, kinds=SCANS) -> int:
    return sum(e.get('count', 1) for e in data.get('entries', []) if e.get('state') == 'unported' and e['kind'] in kinds)


def approval(entry):
    """The named decision behind a platform row, or None. `true` names nothing, so it does not count."""
    named = entry.get('approved_platform')
    return named.strip() if isinstance(named, str) and named.strip() else None


def new_platform(now, was, kinds):
    """(entry, count_before, state_before) for every row whose platform count rises against the base."""
    before = {key_of(e): e for e in was.get('entries', [])}
    rows = []
    for entry in now.get('entries', []):
        if entry['state'] != 'platform' or entry['kind'] not in kinds:
            continue
        old = before.get(key_of(entry))
        old_count = old.get('count', 1) if old is not None and old['state'] == 'platform' else 0
        if entry.get('count', 1) > old_count:
            rows.append((entry, old_count, old['state'] if old is not None else 'absent'))
    return rows


def shrink(inventory: Path, base: Path):
    """Errors for what the inventory gains over BASE, a copy from an earlier commit, and a summary.

    Only the kinds both inventories scan are compared (a kind the base cannot see would otherwise read as a
    rise), and every row that is newly `platform` is listed and needs a named approval (`approved_platform`)."""
    if not base.exists():
        return [], f'Windows-parity inventory: no base inventory at {base}, so nothing to compare'
    now, errors = load_inventory(inventory)
    was, base_errors = load_inventory(base, current=False)
    errors += [f'base: {error}' for error in base_errors]
    if now is None or was is None or errors:
        return errors, ''
    kinds = [kind for kind in now['scans'] if kind in was['scans']]
    skipped = [kind for kind in now['scans'] if kind not in was['scans']]
    for root in map(posixpath.normpath, was['roots']):
        if not any(root == posixpath.normpath(r) for r in now['roots']):
            errors.append(f'NARROWED scope root {root} is in the base inventory, absent now')
    for step in was.get('done_steps') or []:
        if step not in (now.get('done_steps') or []):
            errors.append(f'REOPENED step {step} is recorded done in the base, not now')
    if unported(now, kinds) > unported(was, kinds):
        errors.append(f'RAISED the unported count is {unported(now, kinds)}, the base has {unported(was, kinds)}: '
                      'port the gate, pair it with a Windows arm, or record why it stays (platform)')
    lines = []
    for entry, old_count, old_state in new_platform(now, was, kinds):
        named = approval(entry)
        row = (f"{describe(entry)} (count {old_count} -> {entry.get('count', 1)}, was {old_state}): "
               f"{entry.get('reason')}")
        if named is None:
            errors.append(f'PLATFORM {row}: a row that becomes platform needs "approved_platform": "<where the '
                          'decision is recorded>" besides its reason; true alone names nothing')
        else:
            lines.append(f'  newly platform: {row} [approved: {named}]')
    summary = (f"Windows-parity inventory: {unported(now, kinds)} unported against the base's "
               f'{unported(was, kinds)}; no scope root dropped')
    if skipped:
        summary += f'; not compared, the base does not scan them yet: {", ".join(skipped)}'
    if lines:
        summary += f'\n{len(lines)} row(s) newly platform, for the reviewer:\n' + '\n'.join(lines)
    return errors, summary


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0], allow_abbrev=False)
    parser.add_argument('--root', type=Path, default=ROOT, help='gwz-core checkout the scope roots start from')
    parser.add_argument('--inventory', type=Path, help='inventory directory, or the old single JSON file '
                        '(default: windows_parity/ next to this script)')
    parser.add_argument('--list', action='store_true', help='print every occurrence as an inventory line and exit')
    parser.add_argument('--shrink-from', type=Path, metavar='BASE',
                        help='only compare the inventory with BASE, a copy from an earlier commit (a directory, or the old '
                             'single file); a missing BASE passes')
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

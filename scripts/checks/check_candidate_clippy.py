#!/usr/bin/env python3
"""Shrink-only clippy gate for the transport candidate shape (operator decision D-U, 2026-10-31).

The ordinary build is clippy-clean under `-D warnings` and gated (checked-artifact-boundary.yml). The
`gwz_transport_candidate` build is not: it carried 65 (lib) and 123 (lib test) clippy errors that nothing checked,
because transport-candidate.yml ran only `cargo check` and cargo stops at the first failing unit. Fixing them is plan
step 6.1. Until then this gate holds the debt where it is: it may shrink, it may not grow.

The gate runs clippy over the prepared candidate tree (tests/transport_backend/prepare.py) with
`RUSTFLAGS='--cfg gwz_transport_candidate'`, `--all-targets --all-features --no-deps --locked --keep-going`, and
`--message-format=json`. It does not pass `-D warnings`: a denied lint in the library would stop every unit that
depends on it, so the integration test targets would never be checked. Without it every unit is linted and every
diagnostic is counted at its own level.

Each error or warning reduces to a key of
    (lint, file, enclosing item, unit)
and never a line number, so an edit elsewhere in a file does not move it. The enclosing item is read from the source
by indentation (`mod tests > impl Foo > fn bar`; the empty string at file level), and the unit is the cargo target
(`gwz_core:lib`, `protocol:test`). cargo's JSON does not say whether a message came from the library or its test
build, so both are `gwz_core:lib`, and a diagnostic in library code that the test build repeats counts twice: fixing
it lowers its count by two. The counts are compared with
scripts/checks/candidate_clippy_allowlist.json:

- a key that is not in the list, or whose count rose, fails;
- a key whose count fell, or that disappeared, is reported and passes, so the list can shrink (`--update` rewrites the
  list with the lower counts and refuses to write if anything grew).

Every entry names its owner (`6.1`, the step that removes it) and a reason class. Entries are grouped by shape, one
per OS: `macos`, `linux` (and `windows` when it is generated). Dead code differs by platform (a helper used on one OS
only, a `cfg(target_os)` branch), so a list made on one OS fails on another. The default shape is the running
platform's (`sys.platform`: darwin is `macos`, linux is `linux`, win32 is `windows`); `--shape` overrides it, and a
`--target` run names the shape for the target. `--baseline` prints a fresh list for a shape.

`--json FILE` (or `-`) reads the JSON lines of an earlier `cargo clippy --message-format=json` run instead of running
it, which is how the unit tests and a saved log exercise the comparison.
"""
import argparse
import json
import os
import re
import subprocess
import sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
DEFAULT_ALLOWLIST = HERE / 'candidate_clippy_allowlist.json'
DEFAULT_CONF_DIR = HERE / 'filesystem_lints'
DEFAULT_MANIFEST = ROOT / 'Cargo.toml'
CANDIDATE_CFG = '--cfg gwz_transport_candidate'
OWNER = '6.1'
# sys.platform -> allowlist shape. Dead code differs by platform, so each OS has its own shape.
SHAPE_BY_PLATFORM = (('darwin', 'macos'), ('linux', 'linux'), ('win32', 'windows'))
LEVELS = ('error', 'warning')

# The reason classes: what kind of debt an entry is. Chosen from the lint at baseline time only; the gate never
# reads a class.
CLASS_BY_LINT = (
    (re.compile(r'^(?:dead_code|unused_\w+|unreachable_\w+)$'), 'dead-code'),
    (re.compile(r'^clippy::(?:await_holding_\w+)$'), 'concurrency'),
    (re.compile(r'^clippy::(?:type_complexity|too_many_arguments|large_enum_variant|cognitive_complexity|'
                r'too_many_lines|result_large_err|large_types_passed_by_value)$'), 'design'),
)
DEFAULT_CLASS = 'style'

# An item header: optional visibility and modifiers, then the keyword. `const NAME` is a header too (the modifier
# loop backtracks when no keyword follows).
ITEM = re.compile(
    r'^(?P<indent>[ \t]*)(?:pub(?:\([^)]*\))?[ \t]+)?(?:default[ \t]+)?(?:(?:const|async|unsafe)[ \t]+)*'
    r'(?:extern[ \t]+"[^"]*"[ \t]+)?'
    r'(?P<kw>fn|struct|enum|trait|union|type|mod|impl|static|const|macro_rules!)\b(?P<rest>.*)$')
# A line that continues the header above it at the header's own indentation (`where`, the opening brace on its own
# line, the closing parenthesis of a long signature): it bounds nothing.
CONTINUATION = re.compile(r'^[ \t]*(?:\{|where\b|\)|->)')
NAME = re.compile(r'[ \t]*(?:mut[ \t]+)?(?P<name>r#[A-Za-z_]\w*|[A-Za-z_]\w*)')


def platform_shape():
    """The shape for the running platform (by sys.platform); the platform name itself when it has none listed."""
    for prefix, shape in SHAPE_BY_PLATFORM:
        if sys.platform.startswith(prefix):
            return shape
    return sys.platform


def indent_of(line):
    return len(line) - len(line.lstrip(' \t'))


def header_name(lines, number, match):
    """`fn bar`, `impl<T> Foo<T>`, `mod tests` for the header found on lines[number]."""
    keyword = match.group('kw')
    if keyword == 'impl':
        text = match.group('rest')
        joined = 0
        while '{' not in text and ' where' not in text and joined < 3 and number + 1 + joined < len(lines):
            joined += 1
            text += ' ' + lines[number + joined].strip()
        text = ' '.join(re.split(r'\{|[ \t]where\b', text, maxsplit=1)[0].split())
        return ('impl' + text.rstrip() if text.startswith('<') else 'impl ' + text.strip())[:100]
    named = NAME.match(match.group('rest'))
    return f"{keyword.rstrip('!')} {named.group('name')}" if named else keyword


def enclosing_item(lines, line_number):
    """The chain of items around a 1-based line: 'mod tests > impl Foo > fn bar', or '' at file level.

    Reads by indentation: a header encloses the line when it is the nearest one above (or on) the line whose
    indentation is below every line between it and the line, so a sibling item that ended earlier never does.
    """
    if not 1 <= line_number <= len(lines):
        return ''
    chain = []
    floor = None
    for number in range(line_number - 1, -1, -1):
        line = lines[number]
        if not line.strip():
            continue
        if CONTINUATION.match(line) and number != line_number - 1:
            continue
        indent = indent_of(line)
        match = ITEM.match(line)
        if match is not None and (floor is None or indent < floor):
            chain.append(header_name(lines, number, match))
        floor = indent if floor is None else min(floor, indent)
        if floor == 0:
            break
    return ' > '.join(reversed(chain))


class SourceCache:
    """The source lines of the files diagnostics point at, read once."""

    def __init__(self, root):
        self.root = Path(root)
        self.files = {}

    def lines(self, name):
        if name not in self.files:
            try:
                text = (self.root / name).read_text(encoding='utf-8', errors='replace')
                self.files[name] = text.splitlines()
            except OSError:
                self.files[name] = []
        return self.files[name]

    def item(self, name, line_number):
        return enclosing_item(self.lines(name), line_number)


def user_span(message):
    """The primary span in the repository's own source: out of a macro's expansion if need be."""
    primary = [s for s in message.get('spans') or [] if s.get('is_primary')]
    if not primary:
        return None
    span = primary[0]
    while span is not None and is_foreign(span['file_name']) and span.get('expansion'):
        span = span['expansion'].get('span')
    return span


def is_foreign(name):
    """A path outside the checkout: absolute (the toolchain's sources, the registry) or '<...>'."""
    return (name.startswith(('<', '/', '\\')) or Path(name).is_absolute()
            or re.match(r'^[A-Za-z]:[\\/]', name) is not None)


def normalise_path(name):
    name = name.replace('\\', '/')
    while name.startswith('./'):
        name = name[2:]
    return name


def unit_name(record):
    target = record.get('target') or {}
    kinds = target.get('kind') or ['?']
    kind = 'lib' if kinds[0] in ('lib', 'rlib', 'cdylib', 'dylib', 'staticlib', 'proc-macro') else kinds[0]
    return f"{target.get('name', '?')}:{kind}"


def lint_name(message):
    code = message.get('code') or {}
    if code.get('code'):
        return code['code']
    text = message.get('message', '')
    return f"{message.get('level')}: {text[:60]}"


def diagnostics(lines, sources):
    """Yield (key, level) for every error and warning in cargo's JSON lines. key = (lint, file, item, unit)."""
    for raw in lines:
        raw = raw.strip()
        if not raw.startswith('{'):
            continue
        try:
            record = json.loads(raw)
        except ValueError:
            continue
        if record.get('reason') != 'compiler-message':
            continue
        message = record.get('message') or {}
        level = message.get('level')
        if level not in LEVELS:
            continue
        span = user_span(message)
        if span is None:
            # "aborting due to N previous errors", "N warnings emitted": summaries with no place.
            if not (message.get('code') or {}).get('code'):
                continue
            file_name, line_number = '', 0
        else:
            file_name, line_number = normalise_path(span['file_name']), span.get('line_start', 0)
        item = sources.item(file_name, line_number) if file_name else ''
        yield (lint_name(message), file_name, item, unit_name(record)), level


def count_keys(lines, sources):
    counts = Counter()
    for key, _level in diagnostics(lines, sources):
        counts[key] += 1
    return counts


def reason_class(lint):
    for pattern, name in CLASS_BY_LINT:
        if pattern.match(lint):
            return name
    return DEFAULT_CLASS


def compare(actual, allowed):
    """Compare a run's counts with the list's. Returns (grew, new, shrank, gone).

    grew and new fail the gate; shrank and gone are reported so the list can be reduced.
    grew: [(key, allowed count, now)]; new: [(key, now)]; shrank: [(key, allowed, now)]; gone: [(key, allowed)].
    """
    grew, new, shrank, gone = [], [], [], []
    for key in sorted(actual):
        if key not in allowed:
            new.append((key, actual[key]))
        elif actual[key] > allowed[key]:
            grew.append((key, allowed[key], actual[key]))
        elif actual[key] < allowed[key]:
            shrank.append((key, allowed[key], actual[key]))
    for key in sorted(allowed):
        if key not in actual:
            gone.append((key, allowed[key]))
    return grew, new, shrank, gone


def describe(key):
    lint, file_name, item, unit = key
    place = f"{file_name} [{item}]" if item else file_name
    return f"{lint}  {place}  ({unit})"


def entry_key(entry):
    return (entry['lint'], entry['file'], entry['item'], entry['unit'])


def load_allowlist(path):
    """(data, errors). A malformed list is an error, never a silent pass."""
    try:
        data = json.loads(Path(path).read_text(encoding='utf-8'))
    except (OSError, ValueError) as error:
        return None, [f'{path}: cannot be read: {error}']
    errors = []
    shapes = data.get('shapes')
    if not isinstance(shapes, dict):
        return None, [f"{path}: needs a 'shapes' object (one shape per OS: macos, linux)"]
    for shape, body in shapes.items():
        seen = set()
        for number, entry in enumerate(body.get('entries', [])):
            where = f'{path}: shape {shape!r} entry {number}'
            for field in ('lint', 'file', 'item', 'unit'):
                if not isinstance(entry.get(field), str):
                    errors.append(f'{where}: {field!r} must be a string')
            count = entry.get('count')
            if not isinstance(count, int) or isinstance(count, bool) or count < 1:
                errors.append(f"{where}: 'count' must be a positive integer")
            if not entry.get('owner'):
                errors.append(f"{where}: 'owner' is required (the step that removes it)")
            if not entry.get('class'):
                errors.append(f"{where}: 'class' is required (the reason class)")
            if not errors and entry_key(entry) in seen:
                errors.append(f'{where}: duplicate key {describe(entry_key(entry))}')
            if not errors:
                seen.add(entry_key(entry))
    return (None, errors) if errors else (data, [])


def allowed_counts(data, shape):
    body = data['shapes'].get(shape)
    if body is None:
        return None
    return {entry_key(e): e['count'] for e in body.get('entries', [])}


def entries_for(counts, previous=None):
    """The list entries for a run's counts. Owner and class carry over from `previous` entries when there are some."""
    kept = {entry_key(e): e for e in (previous or [])}
    entries = []
    for key in sorted(counts):
        old = kept.get(key, {})
        lint, file_name, item, unit = key
        entries.append({
            'lint': lint, 'file': file_name, 'item': item, 'unit': unit, 'count': counts[key],
            'owner': old.get('owner', OWNER), 'class': old.get('class', reason_class(lint)),
        })
    return entries


def summarise(counts):
    by_lint = Counter()
    by_unit = Counter()
    for (lint, _file, _item, unit), count in counts.items():
        by_lint[lint] += count
        by_unit[unit] += count
    return by_lint, by_unit


def report(grew, new, shrank, gone, stream):
    for key, was, now in grew:
        print(f'  GREW    {describe(key)}: {was} -> {now}', file=stream)
    for key, now in new:
        print(f'  NEW     {describe(key)}: {now}', file=stream)
    for key, was, now in shrank:
        print(f'  shrank  {describe(key)}: {was} -> {now}', file=stream)
    for key, was in gone:
        print(f'  gone    {describe(key)}: was {was}', file=stream)


def clippy_command(options):
    command = ['cargo', *([f'+{options.toolchain}'] if options.toolchain else []), 'clippy',
               '--no-deps', '--all-targets', '--all-features', '--locked', '--keep-going',
               '--message-format=json', '--manifest-path', str(options.manifest_path)]
    if options.target_dir:
        command += ['--target-dir', str(options.target_dir)]
    if options.target:
        command += ['--target', options.target]
    if options.lib:
        # The library unit alone (the cross-target shape builds no test targets).
        command.remove('--all-targets')
        command.append('--lib')
    return command


def clippy_environment(options, base=None):
    env = dict(os.environ if base is None else base)
    flags = env.get('RUSTFLAGS', '')
    if CANDIDATE_CFG not in flags:
        env['RUSTFLAGS'] = (flags + ' ' + CANDIDATE_CFG).strip()
    env['CLIPPY_CONF_DIR'] = str(Path(options.conf_dir).resolve())
    return env


def run_clippy(options):
    """(stdout lines, return code, stderr text)."""
    process = subprocess.run(clippy_command(options), env=clippy_environment(options), cwd=options.cwd,
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding='utf-8',
                             errors='replace')
    return process.stdout.splitlines(), process.returncode, process.stderr


def parse_arguments(argv):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('--allowlist', type=Path, default=DEFAULT_ALLOWLIST)
    parser.add_argument('--shape', default=None,
                        help="the allowlist shape to compare with (default: the running platform's: macos on "
                             "darwin, linux on linux, windows on win32)")
    parser.add_argument('--json', metavar='FILE', help="read cargo's JSON lines from FILE ('-' for stdin); run nothing")
    parser.add_argument('--manifest-path', type=Path, default=DEFAULT_MANIFEST,
                        help='the prepared candidate tree\'s Cargo.toml (tests/transport_backend/prepare.py)')
    parser.add_argument('--target-dir', type=Path, help='the cargo target directory (reuse the build\'s cache)')
    parser.add_argument('--target', help='a cross target triple; name its shape with --shape')
    parser.add_argument('--lib', action='store_true', help='lint the library unit only (the Windows shape)')
    parser.add_argument('--toolchain', default='1.95.0', help="rustup toolchain ('' to use the environment's)")
    parser.add_argument('--conf-dir', type=Path, default=DEFAULT_CONF_DIR, help='CLIPPY_CONF_DIR')
    parser.add_argument('--source-root', type=Path,
                        help="where the diagnostics' relative paths resolve (default: the manifest's directory)")
    parser.add_argument('--cwd', type=Path, help='the directory cargo runs in (default: the manifest directory)')
    parser.add_argument('--save-json', type=Path, help="keep cargo's JSON lines in FILE")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument('--baseline', action='store_true', help="print the shape's fresh entries as JSON and exit")
    mode.add_argument('--update', action='store_true',
                      help="rewrite the shape's entries with the lower counts; refuses if anything grew")
    options = parser.parse_args(argv)
    manifest_dir = options.manifest_path.resolve().parent
    options.shape = options.shape or platform_shape()
    options.source_root = options.source_root or manifest_dir
    options.cwd = options.cwd or manifest_dir
    return options


def main(argv=None):
    options = parse_arguments(argv)
    if options.json:
        stream = sys.stdin if options.json == '-' else open(options.json, encoding='utf-8')
        with stream:
            lines, code, stderr = stream.read().splitlines(), 0, ''
    else:
        lines, code, stderr = run_clippy(options)
    if options.save_json:
        options.save_json.write_text('\n'.join(lines) + '\n', encoding='utf-8')
    sources = SourceCache(options.source_root)
    diags = list(diagnostics(lines, sources))
    counts = Counter(key for key, _ in diags)
    if code != 0 and not any(level == 'error' for _, level in diags):
        print(f'candidate clippy did not complete (cargo exited {code}, no diagnostic explains it):', file=sys.stderr)
        quiet = [line for line in stderr.splitlines() if not line.startswith('warning: ')]
        print('\n'.join(quiet[-40:]), file=sys.stderr)
        return 2
    if not diags and not lines:
        print('candidate clippy produced no output; nothing was checked', file=sys.stderr)
        return 2
    data, errors = load_allowlist(options.allowlist)
    if options.baseline:
        previous = (data['shapes'].get(options.shape, {}).get('entries') if data else None)
        print(json.dumps(entries_for(counts, previous), indent=1))
        return 0
    if errors:
        print('\n'.join(errors), file=sys.stderr)
        return 1
    allowed = allowed_counts(data, options.shape)
    if allowed is None:
        have = ', '.join(sorted(data['shapes'])) or 'none'
        print(f"{options.allowlist}: no shape {options.shape!r} (it has: {have}); run with --baseline to generate "
              "its entries, or name another with --shape", file=sys.stderr)
        return 1
    grew, new, shrank, gone = compare(counts, allowed)
    by_lint, by_unit = summarise(counts)
    total = sum(counts.values())
    if grew or new:
        print(f'candidate clippy gate failed (shape {options.shape}): {len(grew)} key(s) grew, {len(new)} new; '
              f'{total} diagnostics against {sum(allowed.values())} allowed', file=sys.stderr)
        report(grew, new, shrank, gone, sys.stderr)
        print('Fix the diagnostic. The list only shrinks; a new entry is not a way out (plan step 6.1 removes the '
              'existing ones).', file=sys.stderr)
        return 1
    if options.update:
        data['shapes'][options.shape]['entries'] = entries_for(counts, data['shapes'][options.shape]['entries'])
        options.allowlist.write_text(json.dumps(data, indent=1) + '\n', encoding='utf-8')
        print(f'rewrote {options.allowlist}: shape {options.shape} is now {total} diagnostics in {len(counts)} keys')
        return 0
    report([], [], shrank, gone, sys.stdout)
    shrunk = sum(was - now for _, was, now in shrank) + sum(was for _, was in gone)
    print(f'candidate clippy gate (shape {options.shape}): {total} diagnostics in {len(counts)} keys, '
          f'{sum(allowed.values())} allowed; nothing new' + (f'; {shrunk} fewer than allowed, run with --update '
                                                              'to shrink the list' if shrunk else ''))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())

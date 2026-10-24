#!/usr/bin/env python3
"""No production constructor mints a spawn gate (step 1.3a of the transport convergence plan; contract 7.3 of the
Connection API; operator decision D-E).

A spawn gate keeps a creation window (Windows handle inheritance) from overlapping another gwz spawn. It is one gate per
process, made by the composition and handed to every host, so that two hosts cannot each hold "the" gate and exclude
nothing between them. This check fails on any production code that makes a gate or a gate's holder anywhere else:

- `SpawnGate::new`, `SpawnGate::default` and `SpawnGate::with_exclusion`, `HelperSlots::new` and `NativeHost::new`
  (which routes to the composition for the entries that exist and so is no site of gwz-core's own), wherever they appear;
- `SpawnGate::process`, outside `src/transport_host/composition.rs`, the one site that makes the process's gate;
- `SpawnGate::ungated`, outside `src/git/gitbackend/backend.rs`, where a backend with no host takes the gate that
  excludes nothing.

A listed site that no longer contains its call also fails, so the check cannot pass by finding nothing. Code compiled only
under `cfg(test)` is exempt, by check_process_globals.py's rules, whose scan this reuses: `#[cfg(test)]` items and
modules, `cfg_if!` test branches and files reached only through test-only `mod` declarations.

This is a lexical scan, not name resolution: a call spelled through an alias is not seen, and the compiler is the
second line (the constructors outside the composition exist only in test builds).
"""
import argparse
import importlib.util
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
ROOTS = ['src/lib.rs', 'crates/*/src/lib.rs']
TEST_FEATURES = frozenset({'contract-tests'})
# Calls that are never production code, and the only sites of the two that are.
FORBIDDEN = {
    ('SpawnGate', 'new'), ('SpawnGate', 'default'), ('SpawnGate', 'with_exclusion'),
    ('HelperSlots', 'new'), ('NativeHost', 'new'),
}
ALLOWED = {
    ('SpawnGate', 'process'): 'src/transport_host/composition.rs',
    ('SpawnGate', 'ungated'): 'src/git/gitbackend/backend.rs',
}

_spec = importlib.util.spec_from_file_location('check_process_globals', Path(__file__).with_name('check_process_globals.py'))
cpg = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(cpg)


def calls(source: str, features: frozenset[str]) -> list[tuple[int, str, str]]:
    """The production `Type::function(` calls of the gate types in `source`: (line, type, function)."""
    analysis = cpg.Analysis(source, features)
    found = []
    types = {t for t, _ in FORBIDDEN} | {t for t, _ in ALLOWED}
    watched = FORBIDDEN | set(ALLOWED)
    for i in range(analysis.n - 4):
        if analysis.toks[i].kind != 'id' or analysis.text(i) not in types:
            continue
        if analysis.text(i + 1) != '::' or analysis.toks[i + 2].kind != 'id' or analysis.text(i + 3) != '(':
            continue
        if (analysis.text(i), analysis.text(i + 2)) not in watched or analysis.is_test(i):
            continue
        found.append((analysis.line(i), analysis.text(i), analysis.text(i + 2)))
    return found


def check(repo: Path) -> list[str]:
    original = cpg.analyze

    def analyze(source, features=frozenset()):
        info = original(source, features)
        for line, kind, function in calls(source, features):
            info.occurrences.append(cpg.Occurrence('gate', f'{kind}::{function}', line))
        return info

    cpg.analyze = analyze
    try:
        scan = cpg.scan(repo, ROOTS, TEST_FEATURES)
    finally:
        cpg.analyze = original
    errors = []
    seen = set()
    for (path, kind, name), occurrences in sorted(scan.occurrences.items()):
        if kind != 'gate':
            continue
        function = tuple(name.split('::'))
        lines = ','.join(str(o.line) for o in occurrences)
        if function in FORBIDDEN:
            errors.append(f'GATE  {path}:{lines}: {name} makes a spawn gate in production code; take the '
                          f'composition\'s (transport_host::process_safety)')
        elif ALLOWED.get(function) == path:
            seen.add(function)
        else:
            errors.append(f'GATE  {path}:{lines}: {name} is for {ALLOWED[function]} alone')
    for function, path in sorted(ALLOWED.items()):
        if function not in seen:
            errors.append(f'MISSING {path}: the one allowed site of {function[0]}::{function[1]} has no such call')
    return errors


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('--repo', type=Path, default=ROOT, help='repository root (default: gwz-core)')
    options = parser.parse_args(argv)
    errors = check(options.repo.resolve())
    if errors:
        print(f'spawn gate minting guard failed for {options.repo.resolve()}:', file=sys.stderr)
        for error in errors:
            print(f'  {error}', file=sys.stderr)
        return 1
    print('spawn gate minting guard: the composition is the one site that makes the process gate')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())

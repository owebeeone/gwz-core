#!/usr/bin/env python3
"""Lane gate rule: a lane that changes a Windows-compile trigger path names a receipt.

GwzTransportWindowsParityPlan.md, step 0.4 says a lane cannot merge without a Windows compile, and the
Phase 0 skim (P3-3) found that nothing enforced it. check_lane_commits.sh runs this over the lane's commits.

The trigger paths are windows_lane_check.py's: every scope root of the Windows-parity inventory, `Cargo.toml`,
`Cargo.lock`, `tests/transport_backend/prepare.py` and `.github/*.commit`.

The receipt convention is one line anywhere in a commit message:

    Windows-receipt: <label>               the label that windows_lane_check.py was run with, for example
                                           winp1-20261010-a (lowercase, hyphens, an 8-digit date)
    Windows-receipt: ci-only <reason>      no gwz-core source changes how it compiles on Windows (a sibling pin
                                           that the candidate-windows job covers, a manifest comment); the gate
                                           prints the reason so the reviewer sees the waiver

A label covers its own commit and every earlier commit of the range; a ci-only waiver covers its own commit
only. A trigger-path change after the last label fails, so the last triggering commit, or a later one, carries a
label (or the triggering commit itself carries the waiver). The label is checked for form
only: receipts live outside the repository (the receipt directory of the run), and the reviewer reads the
receipt the label names.

    check_windows_receipts.py BASE HEAD [--floor SHA]
"""
import argparse
from collections import namedtuple
import importlib.util
from pathlib import Path
import re
import subprocess
import sys

_spec = importlib.util.spec_from_file_location('windows_lane_check', Path(__file__).resolve().parents[1] / 'windows_lane_check.py')
lane = importlib.util.module_from_spec(_spec)
sys.modules.setdefault('windows_lane_check', lane)
_spec.loader.exec_module(lane)

Commit = namedtuple('Commit', 'sha message paths')
Receipt = namedtuple('Receipt', 'label ci_only error')  # a label, or a ci-only reason, or an error
LINE = re.compile(r'^Windows-receipt:[ \t]*(.*?)[ \t]*$', re.MULTILINE)
FORM = 'Windows-receipt: <label> (windows_lane_check.py\'s label) or Windows-receipt: ci-only <reason>'


def parse(message: str):
    """The commit message's receipt (the first valid one, else the first malformed one), or None."""
    first_error = None
    for value in LINE.findall(message):
        word, _, reason = value.partition(' ')
        if word == 'ci-only':
            if reason.strip():
                return Receipt(None, reason.strip(), None)
            error = 'ci-only needs a reason'
        elif len(value) <= 64 and lane.LABEL.fullmatch(value) and lane.DATE.search(value):
            return Receipt(value, None, None)
        else:
            error = f'{value!r} is neither a dated label nor ci-only <reason>'
        first_error = first_error or Receipt(None, None, error)
    return first_error


def gate(commits):
    """(errors, notes) for the commits, oldest first. A label covers its own commit and the earlier ones; a
    ci-only waiver covers its own commit only."""
    errors, notes, pending = [], [], []
    patterns = lane.trigger_patterns()
    for commit in commits:
        receipt = parse(commit.message)
        if receipt is not None and receipt.error:
            errors.append(f'{commit.sha[:12]}: malformed Windows-receipt line, {receipt.error}; the form is {FORM}')
        elif receipt is not None and receipt.ci_only:
            # A waiver speaks for its own commit only: a documentation commit after a code change must not
            # stand in for the code change's compile.
            notes.append(f'{commit.sha[:12]}: Windows compile waived, ci-only: {receipt.ci_only}')
            continue
        elif receipt is not None:
            pending = []
            continue
        hits = lane.triggered(commit.paths, patterns)
        if hits:
            pending.append((commit, hits))
    for commit, hits in pending:
        errors.append(f'{commit.sha[:12]} changes a Windows-compile trigger path ({", ".join(hits[:4])}'
                      f'{", ..." if len(hits) > 4 else ""}) and no commit at or after it names a receipt; run '
                      f'scripts/windows_lane_check.py, then add "{FORM}" to a commit message '
                      '(AGENTS.md, "Windows compile gate")')
    return errors, notes


def git(repo: Path, *args: str) -> str:
    return subprocess.run(['git', '-C', str(repo), *args], check=True, capture_output=True, text=True).stdout


def commits_between(repo: Path, base: str, head: str, floor: str | None):
    """The commits of the lane, oldest first, with their messages and the paths each changed. A merge commit
    lists only what its resolution changed, so merging the base in adds no trigger change to the lane."""
    exclude = [base] + ([floor] if floor else [])
    shas = git(repo, 'rev-list', '--reverse', head, '--not', *exclude).split()
    return [Commit(sha, git(repo, 'log', '-1', '--format=%B', sha),
                   [p for p in git(repo, 'diff-tree', '--no-commit-id', '--name-only', '-r', '--root', '-z', sha).split('\0') if p])
            for sha in shas]


def main(argv=None, cwd: Path | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0], allow_abbrev=False)
    parser.add_argument('base')
    parser.add_argument('head')
    parser.add_argument('--floor', help='commits that are ancestors of this one are not checked')
    options = parser.parse_args(argv)
    errors, notes = gate(commits_between(cwd or Path.cwd(), options.base, options.head, options.floor))
    for note in notes:
        print(f'lane gate: {note}')
    if errors:
        print('Windows-receipt check failed:', file=sys.stderr)
        for error in errors:
            print(f'  {error}', file=sys.stderr)
        return 1
    print('lane gate: Windows receipts ok')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())

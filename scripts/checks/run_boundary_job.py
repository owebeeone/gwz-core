#!/usr/bin/env python3
"""Run the checked-artifact boundary job's own check steps, locally.

CI's "Structural boundary" job (`.github/workflows/checked-artifact-boundary.yml`) runs the source checks AND the
unit tests of those checks, a list of unittest modules the workflow names one by one. `scripts/run_tests.py` runs the
checks and none of their tests, so a change that moves what a check counts (an allowlist entry added without the test
that pins its length) passed every local gate and failed only there, from 2026-10-11 on. This runs the workflow's own
command lines, read from the workflow, so the two cannot drift apart:

    python3 scripts/checks/run_boundary_job.py                 # the check steps, clippy included
    python3 scripts/checks/run_boundary_job.py --skip-clippy   # without the slow part

Run it in a checkout with the sibling gwz-transport at its pin (the first step of the job reads it) or pass
--skip-clippy and accept that the transport allowlist is judged by `run_tests.py`.
"""
import argparse
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / '.github/workflows/checked-artifact-boundary.yml'
STEPS = ('Check complete entry and call graph', 'Selection ownership')


def step_commands(text: str, step: str) -> list[str]:
    """The command lines of the `run: |` block of the step named `step`, without comments and blank lines."""
    lines = text.splitlines()
    start = next((i for i, line in enumerate(lines) if re.fullmatch(rf'\s*- name: {re.escape(step)}\s*', line)), None)
    if start is None:
        raise SystemExit(f'run_boundary_job: the workflow has no step named {step!r}')
    run = next((i for i in range(start + 1, len(lines)) if re.fullmatch(r'\s*run:\s*\|?\s*', lines[i]) or lines[i].lstrip().startswith('run: ')), None)
    if run is None:
        raise SystemExit(f'run_boundary_job: the step {step!r} has no run block')
    first = lines[run].lstrip()
    if first.startswith('run: ') and not first.rstrip().endswith('|'):
        return [first[len('run: '):].strip()]
    indent = len(lines[run + 1]) - len(lines[run + 1].lstrip())
    commands = []
    for line in lines[run + 1:]:
        if line.strip() and len(line) - len(line.lstrip()) < indent:
            break
        text_line = line.strip()
        if text_line and not text_line.startswith('#'):
            commands.append(text_line)
    return commands


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--skip-clippy', action='store_true', help='leave out the cargo clippy line (SKIPPED GATE is printed)')
    options = parser.parse_args()
    text = WORKFLOW.read_text(encoding='utf-8')
    status = 0
    for step in STEPS:
        for command in step_commands(text, step):
            if options.skip_clippy and 'cargo clippy' in command:
                print(f'SKIPPED GATE: {command}', flush=True)
                continue
            print(f'+ {command}', flush=True)
            # The workflow says `python`; this interpreter is the one that runs it.
            shell = f'python() {{ "{sys.executable}" "$@"; }}; {command}'
            result = subprocess.run(['bash', '-c', shell], cwd=ROOT, check=False)
            if result.returncode != 0:
                print(f'run_boundary_job: FAILED ({result.returncode}): {command}', file=sys.stderr, flush=True)
                status = 1
    return status


if __name__ == '__main__':
    raise SystemExit(main())

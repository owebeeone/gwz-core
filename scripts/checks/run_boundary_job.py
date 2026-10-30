#!/usr/bin/env python3
"""Run the checked-artifact boundary job's own check steps, locally.

CI's "Structural boundary" job (`.github/workflows/checked-artifact-boundary.yml`) runs the source checks AND the
unit tests of those checks, a list of unittest modules the workflow names one by one. `scripts/run_tests.py` runs the
checks and none of their tests, so a change that moves what a check counts (an allowlist entry added without the test
that pins its length) passed every local gate and failed only there, from 2026-10-11 on. This runs the workflow's own
command lines, read from the workflow, so the two cannot drift apart:

    python3 scripts/checks/run_boundary_job.py                 # the check steps, clippy included
    python3 scripts/checks/run_boundary_job.py --skip-clippy   # without the slow part

The library step of the same workflow (the "Local clone library boundaries" job: every crate under crates/ gets
`cargo test --lib`, `cargo test --doc` and `cargo clippy -D warnings`, `--locked`) runs too, as the one bash script the
workflow gives it, under `bash -e` as Actions runs a step with no `shell:` (the first failing cargo call, an empty package
list or a failing `--list-present` ends it), because its loop means nothing line by line. It is the slow part (every crate, cold or warm): the
doctests and clippy of the crates the core test groups never reach failed only in CI until this step ran here.
`--skip-library` leaves it out (SKIPPED GATE is printed); `--skip-clippy` leaves out only its clippy line.

    python3 scripts/checks/run_boundary_job.py --skip-library  # the check steps alone

Run it in a checkout with the sibling gwz-transport at its pin (the first step of the job reads it) or pass
--skip-clippy and accept that the transport allowlist is judged by `run_tests.py`. Set CARGO_TARGET_DIR (and
RUSTUP_TOOLCHAIN=1.95.0, the job's toolchain) as for any local cargo run.
"""
import argparse
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / '.github/workflows/checked-artifact-boundary.yml'
STEPS = ('Check complete entry and call graph', 'Selection ownership')
# The per-crate loop of the other job in the same workflow; see step_script.
LIBRARY_STEP = "Each library's Tier A command, locked against gwz-core's workspace lock"


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


def step_script(text: str, step: str) -> str:
    """The `run: |` block of `step` as one bash script (comments and blank lines dropped), for a step whose lines depend
    on each other, such as a `for` loop."""
    return '\n'.join(step_commands(text, step)) + '\n'


def without_clippy(script: str) -> tuple[str, list[str]]:
    """`script` without its clippy lines, and those lines. The loop around them stays whole."""
    kept, skipped = [], []
    for line in script.splitlines():
        (skipped if 'cargo clippy' in line else kept).append(line)
    return '\n'.join(kept) + '\n', skipped


def run_bash(shell: str, errexit: bool = False) -> int:
    """Runs `shell` in bash from the checkout root. `errexit` is Actions' default for a step with no `shell:` (`bash -e {0}`):
    the first failing command ends the script, so an empty package list (`test -n`) and a failing `cargo` call fail the step.
    Without it the script's status is its last command's, and a red crate before the last one passes."""
    # The workflow says `python`; this interpreter is the one that runs it.
    wrapped = f'python() {{ "{sys.executable}" "$@"; }}; {shell}'
    return subprocess.run(['bash', *(['-e'] if errexit else []), '-c', wrapped], cwd=ROOT, check=False).returncode


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--skip-clippy', action='store_true', help='leave out the cargo clippy lines (SKIPPED GATE is printed)')
    parser.add_argument('--skip-library', action='store_true',
                        help='leave out the per-crate library step: tests, doctests and clippy of every crate under crates/ '
                             '(SKIPPED GATE is printed)')
    options = parser.parse_args()
    text = WORKFLOW.read_text(encoding='utf-8')
    status = 0
    for step in STEPS:
        for command in step_commands(text, step):
            if options.skip_clippy and 'cargo clippy' in command:
                print(f'SKIPPED GATE: {command}', flush=True)
                continue
            print(f'+ {command}', flush=True)
            code = run_bash(command)
            if code != 0:
                print(f'run_boundary_job: FAILED ({code}): {command}', file=sys.stderr, flush=True)
                status = 1
    if options.skip_library:
        print(f'SKIPPED GATE: the library step ({LIBRARY_STEP}), --skip-library', flush=True)
        return status
    script = step_script(text, LIBRARY_STEP)
    if options.skip_clippy:
        script, skipped = without_clippy(script)
        for line in skipped:
            print(f'SKIPPED GATE: {line}', flush=True)
    print(f'+ [{LIBRARY_STEP}]\n{script}', end='', flush=True)
    code = run_bash(script, errexit=True)  # `bash -e`, as Actions runs a step with no `shell:`
    if code != 0:
        print(f'run_boundary_job: FAILED ({code}): the library step ({LIBRARY_STEP})', file=sys.stderr, flush=True)
        status = 1
    return status


if __name__ == '__main__':
    raise SystemExit(main())

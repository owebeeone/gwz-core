#!/usr/bin/env python3
"""Windows compile gate for a lane (GwzTransportWindowsParityPlan.md, step 0.4).

Archives the lane's gwz-core (a ref, or with --worktree the working tree), puts it in a NEW labelled
directory on the Windows host, checks out the siblings the candidate build needs at gwz-core's pins, runs
`cargo check` of the three shapes the candidate-windows CI job checks (ordinary; `--cfg gwz_transport_candidate`;
and that plus `--cfg gwz_windows_https_qualification`) and archives a receipt. Nothing is cross-compiled: the
Windows build is only ever made on Windows (V110 S4.1).

    python3 scripts/windows_lane_check.py --label winp0-20261009-a --receipt-dir DIR
    python3 scripts/windows_lane_check.py --label NAME --receipt-dir DIR --if-triggered BASE   # skip when no trigger path changed
    python3 scripts/windows_lane_check.py --label NAME --receipt-dir DIR --cache-from OLDER-LABEL   # warm: reuse its target dir

The label names the directory E:/gwz-tests/<label> on the host and the receipt directory DIR/<label>. It must
be new: a label whose host directory or receipt directory exists is refused, so no receipt is overwritten and no
earlier run's directory is reused. The host is the shared dabeest machine: the SSH options below are the host
rules (no forwarding, no prompts); the script installs nothing, changes no host setting and uses the 1.95.0
toolchain through RUSTUP_TOOLCHAIN. `--tests` also checks the library in test mode (cfg(test)), for a lane that edits test code; the integration targets are the CI job's.
"""
import argparse
from dataclasses import dataclass, field
import datetime
import fnmatch
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import sys
import time

CORE = Path(__file__).resolve().parents[1]
_spec = importlib.util.spec_from_file_location('check_windows_parity', CORE / 'scripts' / 'checks' / 'check_windows_parity.py')
parity = importlib.util.module_from_spec(_spec)
sys.modules.setdefault('check_windows_parity', parity)
_spec.loader.exec_module(parity)
HOST = 'gianni@dabeest'
HOST_ROOT = '/e/gwz-tests'  # E:/gwz-tests in MinGW bash
SSH = ['ssh', '-o', 'ClearAllForwardings=yes', '-o', 'ForwardAgent=no', '-o', 'ForwardX11=no',
       '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=10', HOST]
TOOLCHAIN = '1.95.0'
QUALIFICATION = '--cfg gwz_transport_candidate --cfg gwz_windows_https_qualification'
SHAPES = (('ordinary', ''), ('transport', '--cfg gwz_transport_candidate'), ('qualification', QUALIFICATION))
# A lane that changes one of these needs the compile gate before it merges (AGENTS.md, "Windows compile gate"): every
# scope root of the Windows-parity inventory (so the two lists cannot drift apart) and the build inputs below.
TRIGGER_EXTRA = ('Cargo.toml', 'Cargo.lock', 'tests/transport_backend/prepare.py', '.github/*.commit')
SIBLING_PINS = (('gwz-transport', 'https://github.com/owebeeone/gwz-transport.git'),
                ('gwz-sspi', 'https://github.com/owebeeone/gwz-sspi.git'))
LABEL = re.compile(r'^[a-z][a-z0-9]*(-[a-z0-9]+)*$')
DATE = re.compile(r'(^|-)20\d{6}(-|$)')
# The host's Application Control policy sometimes refuses to run a build script that cargo has just written
# (os error 4551); the shape is run once more and the receipt records both attempts.
BLOCKED = 'os error 4551'
WARNINGS = re.compile(r'`gwz-core` \(lib[^)]*\) generated (\d+) warnings?')


class Refused(SystemExit):
    """A usage refusal; the message is printed and the exit status is 2."""

    def __init__(self, message):
        super().__init__(f'windows_lane_check: {message}')
        self.code = 2


def check_label(label: str) -> str:
    if len(label) > 64 or not LABEL.fullmatch(label) or not DATE.search(label):
        raise Refused(f'label {label!r} must be lowercase letters, digits and hyphens, at most 64 characters, '
                      'and carry the date as 8 digits, for example winp0-20261009-a')
    return label


def trigger_patterns(inventory: Path | None = None) -> list[str]:
    """The inventory's scope roots, then TRIGGER_EXTRA. A root is a file, a directory or a `*` pattern."""
    data, errors = parity.load_inventory(inventory or parity.DEFAULT_INVENTORY)
    if data is None:
        raise Refused('the Windows-parity inventory cannot be read, so the trigger paths are unknown: '
                      + '; '.join(errors))
    return list(data['roots']) + list(TRIGGER_EXTRA)


def matches(path: str, pattern: str) -> bool:
    if '*' in pattern:
        return fnmatch.fnmatchcase(path, pattern)
    return path == pattern or path.startswith(pattern.rstrip('/') + '/')


def triggered(paths, patterns=None) -> list[str]:
    """The paths among `paths` that start the gate."""
    patterns = trigger_patterns() if patterns is None else patterns
    return [p for p in paths if any(matches(p, t) for t in patterns)]


@dataclass
class Host:
    """The Windows host. `run` is subprocess.run, replaced in tests; `clock` is time.monotonic."""
    run: object = subprocess.run
    clock: object = time.monotonic

    def ssh(self, script: str, check=True, capture=True):
        """Runs a bash script on the host through stdin, so nothing needs shell quoting."""
        return self.run(SSH + ['bash', '-s'], input=script, text=True, capture_output=capture, check=check)

    def timed(self, script: str):
        start = self.clock()
        result = self.ssh(script, check=False)
        return result, round(self.clock() - start, 1)


@dataclass
class Receipt:
    label: str
    source: str
    commit: str
    cache_from: str | None
    with_tests: bool
    started: str
    stage_seconds: float = 0.0
    shapes: list = field(default_factory=list)
    toolchain: str = ''

    def passed(self) -> bool:
        return bool(self.shapes) and all(s['exit'] == 0 for s in self.shapes)

    def as_json(self) -> str:
        total = round(self.stage_seconds + sum(a['seconds'] for s in self.shapes for a in s['attempts']), 1)
        return json.dumps({'label': self.label, 'host_dir': f'{HOST_ROOT}/{self.label}', 'source': self.source,
                           'commit': self.commit, 'cache_from': self.cache_from, 'with_tests': self.with_tests,
                           'toolchain': self.toolchain, 'started': self.started, 'stage_seconds': self.stage_seconds,
                           'shapes': self.shapes, 'total_seconds': total, 'passed': self.passed()}, indent=2) + '\n'


def stage_script(label: str, cache_from: str | None) -> str:
    """Host script: the siblings at gwz-core's pins, then the prepared candidate. The core is already in place."""
    d = f'{HOST_ROOT}/{label}'
    lines = ['set -euo pipefail', 'export GIT_TERMINAL_PROMPT=0', f'cd {d}/ws/gwz-core',
             'bash .github/checkout-git2-rs.sh']
    for name, url in SIBLING_PINS:
        lines += [f'pin=$(grep -m1 -E "^[0-9a-f]{{40}}$" .github/{name}.commit | tr -d "\\r")',
                  f'git init -q ../{name}', f'git -C ../{name} remote add origin {url}',
                  f'git -C ../{name} fetch -q --depth 1 origin "$pin"', f'git -C ../{name} checkout -q --detach FETCH_HEAD']
    if cache_from:
        lines += [f'test -d {HOST_ROOT}/{cache_from}/target', f'cp -a {HOST_ROOT}/{cache_from}/target {d}/target']
    # `cargo metadata` completes the candidate's lock file (the core's lists only the ordinary build), as the CI job does.
    lines += [f'python tests/transport_backend/prepare.py {d}/candidate', f'export RUSTUP_TOOLCHAIN={TOOLCHAIN}',
              f'cargo metadata --format-version 1 --manifest-path {d}/candidate/Cargo.toml > /dev/null', 'rustc --version']
    return '\n'.join(lines) + '\n'


def shape_script(label: str, flags: str, with_tests: bool, name: str) -> str:
    d = f'{HOST_ROOT}/{label}'
    targets = '--lib --profile test' if with_tests else '--lib'  # the test profile also checks in test mode (cfg(test))
    return '\n'.join([
        'set -uo pipefail', f'mkdir -p {d}/logs', f'cd {d}/ws/gwz-core',
        f'export RUSTUP_TOOLCHAIN={TOOLCHAIN} CARGO_INCREMENTAL=0 RUSTFLAGS="{flags}"',
        f'cargo check --locked {targets} --manifest-path {d}/candidate/Cargo.toml --target-dir {d}/target '
        f'> {d}/logs/{name}.log 2>&1',
        'status=$?', f'tail -n 40 {d}/logs/{name}.log', 'exit $status']) + '\n'


UNTRACKED_EXCLUDED = ':(exclude)dev-docs'  # the worktree archive leaves untracked documents at home


def archive_command(core: Path, ref: str | None) -> tuple[list[str], str]:
    """The local command whose stdout is a tar of the lane's gwz-core, and the source it names."""
    if ref is None:
        # COPYFILE_DISABLE: macOS tar would add ._* AppleDouble files, which the host would unpack as sources.
        # Tracked files as they are on disk (a deleted one is left out), plus untracked, unignored files outside dev-docs.
        listing = ('{ git ls-files -z --cached; git ls-files -z --others --exclude-standard -- . '
                   f"'{UNTRACKED_EXCLUDED}'; }} | sort -zu | while IFS= read -r -d '' f; do "
                   '[ -e "$f" ] && printf \'%s\\0\' "$f"; done | COPYFILE_DISABLE=1 tar -cf - --null -T -')
        return (['bash', '-c', listing], 'worktree (tracked files, and untracked ones outside dev-docs)')
    return ['git', 'archive', '--format=tar', ref], f'ref {ref}'


def git_output(core: Path, *args: str, run=subprocess.run) -> str:
    return run(['git', '-C', str(core), *args], text=True, capture_output=True, check=True).stdout.strip()


def execute(options, host: Host, core: Path = CORE, now=datetime.datetime.now) -> int:
    label = check_label(options.label)
    receipt_dir = options.receipt_dir / label
    if receipt_dir.exists():
        raise Refused(f'receipt directory {receipt_dir} exists; a receipt is never overwritten, use a new label')
    if options.cache_from:
        check_label(options.cache_from)
    ref = None if options.worktree else options.ref
    commit = git_output(core, 'rev-parse', ref or 'HEAD', run=host.run)
    claim = host.ssh(f'mkdir {HOST_ROOT}/{label} 2>&1 && mkdir -p {HOST_ROOT}/{label}/ws/gwz-core', check=False)
    if claim.returncode != 0:
        raise Refused(f'{HOST_ROOT}/{label} on the host cannot be created (it may exist; labels are never reused): '
                      f'{(claim.stdout or "").strip()}')
    archive, source = archive_command(core, ref)
    receipt = Receipt(label, source, commit, options.cache_from, options.tests, now().isoformat(timespec='seconds'))
    start = host.clock()
    producer = host.run(archive, cwd=core, capture_output=True, check=True)
    unpack = host.run(SSH + ['tar', '-x', '-C', f'{HOST_ROOT}/{label}/ws/gwz-core'], input=producer.stdout,
                      capture_output=True, check=False)
    if unpack.returncode != 0:
        raise SystemExit(f'windows_lane_check: unpacking the archive on the host failed: {unpack.stderr!r}')
    staged = host.ssh(stage_script(label, options.cache_from), check=False)
    receipt.stage_seconds = round(host.clock() - start, 1)
    if staged.returncode != 0:
        print(staged.stdout, staged.stderr, sep='\n', file=sys.stderr)
        raise SystemExit('windows_lane_check: staging the siblings or preparing the candidate failed; the host '
                         f'directory {HOST_ROOT}/{label} is kept for inspection')
    receipt.toolchain = (staged.stdout or '').strip().splitlines()[-1] if (staged.stdout or '').strip() else ''
    for name, flags in SHAPES:
        result, seconds = host.timed(shape_script(label, flags, options.tests, name))
        attempts = [{'exit': result.returncode, 'seconds': seconds}]
        if result.returncode != 0 and BLOCKED in (result.stdout or ''):
            print(f'{name}: a build script was blocked by the host (os error 4551); running it once more', flush=True)
            result, seconds = host.timed(shape_script(label, flags, options.tests, name))
            attempts.append({'exit': result.returncode, 'seconds': seconds})
        text = result.stdout or ''
        found = WARNINGS.findall(text)
        receipt.shapes.append({'shape': name, 'rustflags': flags, 'exit': result.returncode, 'seconds': seconds,
                               'warnings': int(found[-1]) if found else 0, 'attempts': attempts,
                               'log': f'{HOST_ROOT}/{label}/logs/{name}.log'})
        print(f'{name}: exit {result.returncode} in {seconds}s, {receipt.shapes[-1]["warnings"]} warnings', flush=True)
        if result.returncode != 0:
            print(text, file=sys.stderr)
    receipt_dir.mkdir(parents=True)
    (receipt_dir / 'receipt.json').write_text(receipt.as_json(), encoding='utf-8')
    host.ssh(f"cat > {HOST_ROOT}/{label}/receipt.json <<'RECEIPT'\n{receipt.as_json()}RECEIPT\n", check=False)
    print(f'receipt: {receipt_dir / "receipt.json"} ({"PASS" if receipt.passed() else "FAIL"})')
    return 0 if receipt.passed() else 1


def parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0], allow_abbrev=False)
    p.add_argument('--label', required=True, help='new host directory name under E:/gwz-tests, with the date')
    p.add_argument('--receipt-dir', required=True, type=Path, help='directory that receives <label>/receipt.json')
    source = p.add_mutually_exclusive_group()
    source.add_argument('--ref', default='HEAD', help='commit of gwz-core to check (default HEAD)')
    source.add_argument('--worktree', action='store_true', help='check the working tree instead of a ref')
    p.add_argument('--tests', action='store_true', help='also check the library in test mode (cfg(test))')
    p.add_argument('--cache-from', metavar='LABEL', help='copy an earlier run\'s target directory (a warm run)')
    p.add_argument('--if-triggered', metavar='BASE', help='do nothing unless BASE..ref changed a trigger path')
    return p


def main(argv=None, host: Host | None = None, core: Path = CORE) -> int:
    options = parser().parse_args(argv)
    check_label(options.label)
    host = host or Host()
    if options.if_triggered:
        ref = 'HEAD' if options.worktree else options.ref
        changed = git_output(core, 'diff', '--name-only', f'{options.if_triggered}..{ref}', run=host.run).splitlines()
        hits = triggered(changed)
        if not hits:
            print(f'windows_lane_check: no trigger path changed since {options.if_triggered}; not required')
            return 0
        print('windows_lane_check: required, changed: ' + ', '.join(hits))
    return execute(options, host, core)


if __name__ == '__main__':
    raise SystemExit(main())

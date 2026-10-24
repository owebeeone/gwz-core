#!/usr/bin/env python3
"""Windows compile gate for a lane (GwzTransportWindowsParityPlan.md, step 0.4).

Archives the lane's gwz-core (a ref, or with --worktree the working tree), puts it in a NEW labelled
directory on the Windows host, checks out the siblings the candidate build needs at gwz-core's pins, runs
`cargo check` of the two shapes the candidate-windows CI job checks (ordinary, and `--cfg gwz_transport_candidate`; step
5.1 retired the third, qualification, shape) and archives a receipt. Nothing is cross-compiled: the
Windows build is only ever made on Windows (V110 S4.1).

    python3 scripts/windows_lane_check.py --label winp0-20261009-a --receipt-dir DIR
    python3 scripts/windows_lane_check.py --label NAME --receipt-dir DIR --if-triggered BASE   # skip when no trigger path changed
    python3 scripts/windows_lane_check.py --label NAME --receipt-dir DIR --cache-from OLDER-LABEL   # warm: reuse its target dir

The label names the directory E:/gwz-tests/<label> on the host and the receipt directory DIR/<label>. It must
be new: a label whose host directory or receipt directory exists is refused, so no receipt is overwritten and no
earlier run's directory is reused. The host is the shared dabeest machine: the SSH options below are the host
rules (no forwarding, no prompts); the script installs nothing, changes no host setting and uses the 1.95.0
toolchain through RUSTUP_TOOLCHAIN. `--tests` also checks the library in test mode (cfg(test)), for a lane that edits test code; the integration targets are the CI job's.

`--hosted-like` also RUNS the candidate build's library suites the way the hosted `candidate-windows` job does, which
`cargo check` cannot: the job's own `run_tests.py ... --lib` command with the portable Win32-OpenSSH `sshd.exe` (the
job's pinned release, SHA-256 checked, unpacked inside the run's directory), `GWZ_TEST_SH`, and a `TEMP` that is an
8.3 short path (a hosted runner's user directory is spelled `RUNNER~1`; the script refuses to run when the host would not
spell it short). `--git-root DIR` puts another Git for Windows (for example a portable 2.56) first on `PATH` and names
its `sh.exe`. The run is long (about twenty minutes warm) and belongs under the host booking. Its result is a
`hosted-like` entry in the receipt: exit status, the suite's passed and failed counts and the names of the failed tests.
After the suite the mode runs the job's integration targets (`INTEGRATION_TARGETS`, step 5.4) into the same log; a
filtered run leaves them out. `--hosted-like-filter NAME` narrows the run to the tests whose path contains NAME, for iterating on one failure; the
receipt says it was filtered. It reaches `run_tests.py` as `--test-filter`, which narrows each group to its own tests. `--no-check-shapes` skips the `cargo check` shapes (a run that is only the suite).
"""
import argparse
from dataclasses import dataclass, field
import datetime
import fnmatch
import hashlib
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
CANDIDATE = '--cfg gwz_transport_candidate'
# The portable OpenSSH server of the hosted job (.github/workflows/transport-candidate.yml, "Install the OpenSSH
# server the SSH fixtures run"); a test pins both values to the workflow's.
OPENSSH_RELEASE = 'v9.8.3.0p2-Preview'
OPENSSH_SHA256 = '0ca131f3a78f404dc819a6336606caec0db1663a692ccc3af1e90232706ada54'
OPENSSH_URL = f'https://github.com/PowerShell/Win32-OpenSSH/releases/download/{OPENSSH_RELEASE}/OpenSSH-Win64.zip'
DEFAULT_GIT_ROOT = 'C:\\Program Files\\Git'  # Git for Windows as the hosted image and the host have it
# The hosted job's suite command (the same run_tests.py line, with gwz-py also skipped: the host has no gwz-py checkout).
# Tests the hosted-like run cannot pass on a host that lacks what the hosted job installs: the embedded-Python rows run
# gwz-py's codec (a gwz-py checkout beside gwz-core) and import taut-proto from the interpreter's site-packages. The mode
# installs neither (that would change the host), so they fail there; they are named in the receipt as gaps and are the
# only failures a receipt passes. Anything else that fails fails the receipt.
HOST_GAPS = ('transport_host::message_embedding_tests::python_messages_',)
# The same gap in the integration targets (step 5.4): tests/protocol.rs runs `python -m taut.cli` and the log-additive
# check, which import taut from the interpreter's site-packages (the hosted job installs taut-proto; the mode does not).
# Whole names, since an integration test has no module path to take a prefix of.
HOST_GAP_NAMES = ('generated_checked_artifact_protocol_is_current', 'generated_protocol_is_current',
                  'log_addition_check_refuses_a_schema_that_declares_a_taut_option',
                  'log_addition_preserves_the_complete_pre_log_wire_projection')
SUITE_TARGETS = ('target-test', 'target-validator')
# The hosted job's "Integration targets" step (step 5.4): every tests/*.rs target and the placement guide, which the
# suite's `--lib` leaves out; a test pins the list to the workflow's.
INTEGRATION_TARGETS = ('protocol', 'transport_placement_guide', 'native_libgit2', 'rename', 'diff_render_spike',
                       'publish_workflow')
HOSTED_SUITE = 'python scripts/run_tests.py --skip-cfg-sibling gwz-cli --skip-cfg-sibling gwz-py --no-fail-fast --lib'
SHAPES = (('ordinary', ''), ('transport', CANDIDATE))
# A lane that changes one of these needs the compile gate before it merges (AGENTS.md, "Windows compile gate"): every
# scope root of the Windows-parity inventory (so the two lists cannot drift apart) and the build inputs below.
TRIGGER_EXTRA = ('Cargo.toml', 'Cargo.lock', 'tests/transport_backend/prepare.py', '.github/*.commit')
SIBLING_PINS = (('gwz-transport', 'https://github.com/owebeeone/gwz-transport.git'),
                ('gwz-sspi', 'https://github.com/owebeeone/gwz-sspi.git'),
                ('gwz-git', 'https://github.com/owebeeone/gwz-git.git'))
LABEL = re.compile(r'^[a-z][a-z0-9]*(-[a-z0-9]+)*$')
DATE = re.compile(r'(^|-)20\d{6}(-|$)')
# The host's Application Control policy sometimes refuses to run a build script that cargo has just written
# (os error 4551); the shape is run once more and the receipt records both attempts.
BLOCKED = 'os error 4551'
WARNINGS = re.compile(r'`gwz-core` \(lib[^)]*\) generated (\d+) warnings?')


class Refused(SystemExit):
    """A usage refusal; the message is printed (by `run`) and the exit status is 2.

    The status is an integer, and Python prints nothing for a `SystemExit` with an integer code, so the message is
    kept in `message` and `run` prints it."""

    def __init__(self, message):
        self.message = f'windows_lane_check: {message}'
        super().__init__(self.message)
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
        return bool(self.shapes) and all(s.get('passed', s['exit'] == 0) for s in self.shapes)

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
        # The hosted-like run's own build directories, when the earlier run made them.
        for name in SUITE_TARGETS:
            lines += [f'if [ -d {HOST_ROOT}/{cache_from}/{name} ]; then '
                      f'cp -a {HOST_ROOT}/{cache_from}/{name} {d}/{name}; fi']
    # `cargo metadata` prunes the prepared copy's lock file to the packages the copy uses (it is not a workspace), as the CI job does.
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


GIT_ROOT = re.compile(r'^[A-Za-z]:\\[A-Za-z0-9 ._()\\-]+$')
FILTER = re.compile(r'^[A-Za-z0-9_:]+$')


def check_git_root(root: str) -> str:
    """A Windows directory spelled with nothing a shell would interpret; it goes into a script as a quoted word."""
    if not GIT_ROOT.fullmatch(root):
        raise Refused(f'--git-root {root!r} must be a Windows path such as C:\\Program Files\\Git')
    return root


def check_filter(name: str) -> str:
    if not FILTER.fullmatch(name):
        raise Refused(f'--hosted-like-filter {name!r} must be a fragment of a test path (letters, digits, _ and ::)')
    return name


def temp_directory_name(label: str) -> str:
    """The suite's TEMP directory under the user's Temp: `hl` and seven hex digits of the label. Nine characters, so
    that Windows spells it with an 8.3 name (`HLxxxx~1`), and no more, because the suite builds paths near MAX_PATH
    from it and a runner's TEMP is short in both spellings."""
    return 'hl' + hashlib.sha256(label.encode()).hexdigest()[:7]


def integration_lines(d: str, test_filter: str | None) -> list[str]:
    """The hosted job's "Integration targets" step, after the suite, into the suite's log; the script's status is the
    suite's, or the step's when the suite passed. A filtered run is about one failure and leaves the step out.

    The step exits nonzero when a row fails, a named host gap among them, so its status alone cannot tell that from a
    step that never ran: it also prints how many `test result:` lines the step added to the log, one per target that
    ran to a result (`HOSTED_LIKE_INTEGRATION_RESULTS`)."""
    if test_filter:
        return []
    log = f'{d}/logs/hosted-like.log'
    tests = ' '.join(f'--test {name}' for name in INTEGRATION_TARGETS)
    return [f'before=$(wc -l < {log})',
            f'cargo test --locked --no-fail-fast --manifest-path {d}/candidate/Cargo.toml '
            f'--target-dir {d}/target-test {tests} >> {log} 2>&1',
            'integration_status=$?',
            f"results=$(tail -n +$((before + 1)) {log} | grep -a -c '^test result:')",
            'echo "HOSTED_LIKE_INTEGRATION_EXIT $integration_status"',
            'echo "HOSTED_LIKE_INTEGRATION_RESULTS $results"',
            'if [ "$status" -eq 0 ]; then status=$integration_status; fi']


def hosted_like_script(label: str, git_root: str, test_filter: str | None) -> str:
    """Host script: the candidate build's library suites run under the hosted job's conditions (see the module doc).

    The suite command is the job's, the environment is the job's (the portable sshd, `GWZ_TEST_SH`, the real Git
    backend, the candidate switch) except where the host differs: the TEMP is made short here, because the
    hosted runner's user directory is. Nothing outside the run's directory and the user's TEMP is written.
    """
    d = f'{HOST_ROOT}/{label}'
    ssh = f'{d}/openssh'
    filter_word = f' --test-filter {test_filter}' if test_filter else ''
    return '\n'.join([
        'set -uo pipefail', f'mkdir -p {d}/logs {ssh}', f"GIT_ROOT='{git_root}'",
        # The short TEMP: a directory whose long name has an 8.3 spelling, as `RUNNER~1` is on the runner. A host
        # that does not generate short names would run the suite with a long one and prove nothing.
        f'long="$(cygpath -u "$LOCALAPPDATA")/Temp/{temp_directory_name(label)}"', 'mkdir -p "$long"',
        'short="$(cygpath -d "$long")"',
        'case "$short" in *~*) ;; *) echo "HOSTED_LIKE_REFUSED: no 8.3 short name for $long"; exit 4;; esac',
        'export TEMP="$short" TMP="$short"', 'export TEMP TMP', 'echo "HOSTED_LIKE_TEMP $TEMP"',
        # The portable sshd at the job's release, downloaded and checked here (the job does the same).
        f'if [ ! -f {ssh}/OpenSSH-Win64/sshd.exe ]; then',
        f'  curl --fail --silent --show-error --location --retry 5 --retry-all-errors --output {ssh}/OpenSSH-Win64.zip '
        f'"{OPENSSH_URL}" || exit 5',
        f'  echo "{OPENSSH_SHA256}  {ssh}/OpenSSH-Win64.zip" | sha256sum --check - || exit 6',
        f'  powershell -NoProfile -Command "Expand-Archive -LiteralPath \'$(cygpath -w {ssh}/OpenSSH-Win64.zip)\' '
        f'-DestinationPath \'$(cygpath -w {ssh})\'" || exit 7',
        'fi',
        f'test -f {ssh}/OpenSSH-Win64/sshd.exe && test -f {ssh}/OpenSSH-Win64/ssh-keygen.exe || exit 8',
        f'export GWZ_TEST_SSHD="$(cygpath -w {ssh}/OpenSSH-Win64/sshd.exe)"',
        'echo "HOSTED_LIKE_SSHD $("$GWZ_TEST_SSHD" -V 2>&1 | head -n 1)"',
        # The Git the suite runs: the host's, or the one --git-root names, first on PATH.
        # (cygpath -u answers "/" for a path with a space on this host's Git bash, so the conversion is spelled out.)
        "GIT_UNIX=\"/$(echo \"$GIT_ROOT\" | sed -E 's#\\\\#/#g; s#^([A-Za-z]):#\\L\\1#')\"",
        'export PATH="$GIT_UNIX/cmd:$PATH"', 'export GWZ_TEST_SH="$GIT_ROOT\\usr\\bin\\sh.exe"',
        'echo "HOSTED_LIKE_GIT $(command -v git): $(git --version)"',
        f'export RUSTUP_TOOLCHAIN={TOOLCHAIN} CARGO_INCREMENTAL=0 RUSTFLAGS="{CANDIDATE}" GWZ_TEST_GIT=real',
        f'export CARGO_TARGET_DIR={d}/target-validator', f'cd {d}/ws/gwz-core',
        f'{HOSTED_SUITE}{filter_word} --manifest-path={d}/candidate/Cargo.toml --target-dir={d}/target-test '
        f'> {d}/logs/hosted-like.log 2>&1',
        'status=$?',
        *integration_lines(d, test_filter),
        # One line per fact, for the receipt: the last "test result:" of each test binary run adds up.
        f"passed=$(grep -a -E '^test result:' {d}/logs/hosted-like.log | sed -n -E 's/^test result: [A-Za-z]+\\. ([0-9]+) passed.*/\\1/p' "
        "| awk '{s+=$1} END{print s+0}')",
        f"failed=$(grep -a -E '^test result:' {d}/logs/hosted-like.log | sed -n -E 's/^test result: [A-Za-z]+\\. [0-9]+ passed; ([0-9]+) failed.*/\\1/p' "
        "| awk '{s+=$1} END{print s+0}')",
        'echo "HOSTED_LIKE_RESULT passed=$passed failed=$failed"',
        f"grep -a -E '^test .* \\.\\.\\. FAILED' {d}/logs/hosted-like.log | sed -E 's/^test (.*) \\.\\.\\. FAILED.*/HOSTED_LIKE_FAILED \\1/' | sort -u",
        'exit $status']) + '\n'


def parse_hosted_like(text: str, test_filter: str | None) -> dict:
    """The facts the host script printed: TEMP, Git, sshd and the suite's counts and failed test names."""
    facts = {'tests': {'passed': 0, 'failed': 0, 'failed_names': [], 'filter': test_filter}}
    for line in text.splitlines():
        word, _, rest = line.strip().partition(' ')
        if word == 'HOSTED_LIKE_TEMP':
            facts['temp'] = rest
        elif word == 'HOSTED_LIKE_GIT':
            facts['git'] = rest
        elif word == 'HOSTED_LIKE_SSHD':
            facts['sshd'] = rest
        elif word == 'HOSTED_LIKE_INTEGRATION_EXIT':
            facts['integration_exit'] = int(rest)
        elif word == 'HOSTED_LIKE_INTEGRATION_RESULTS':
            facts['integration_results'] = int(rest)
        elif word == 'HOSTED_LIKE_FAILED':
            facts['tests']['failed_names'].append(rest)
        elif word == 'HOSTED_LIKE_RESULT':
            counts = dict(part.split('=', 1) for part in rest.split())
            facts['tests'].update(passed=int(counts.get('passed', 0)), failed=int(counts.get('failed', 0)))
    return facts


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


def run_entry(host: Host, receipt: Receipt, name: str, flags: str, script: str, log: str, test_filter=None) -> None:
    """Runs one host script and records its entry. A build script the host blocked (os error 4551) is run once more,
    both attempts are recorded; a genuine failure, a failed test among them, is never run again."""
    result, seconds = host.timed(script)
    attempts = [{'exit': result.returncode, 'seconds': seconds}]
    if result.returncode != 0 and BLOCKED in (result.stdout or ''):
        print(f'{name}: a build script was blocked by the host (os error 4551); running it once more', flush=True)
        result, seconds = host.timed(script)
        attempts.append({'exit': result.returncode, 'seconds': seconds})
    text = result.stdout or ''
    found = WARNINGS.findall(text)
    entry = {'shape': name, 'rustflags': flags, 'exit': result.returncode, 'seconds': seconds,
             'warnings': int(found[-1]) if found else 0, 'attempts': attempts, 'log': log}
    if name == 'hosted-like':
        entry.update(parse_hosted_like(text, test_filter))
        names = entry['tests']['failed_names']
        entry['gaps'] = [n for n in names if n.startswith(HOST_GAPS) or n in HOST_GAP_NAMES]
        entry['failures'] = [n for n in names if n not in entry['gaps']]
        # A gap-only failure passes. An integration step that died (a build error, a binary that could not load) names
        # no failed test, so it passes only when it exited 0 or every target reported a result.
        integration_ran = (entry.get('integration_exit', 0) == 0
                           or entry.get('integration_results') == len(INTEGRATION_TARGETS))
        entry['passed'] = result.returncode == 0 or bool(
            entry['gaps'] and not entry['failures'] and entry['tests']['failed'] == len(entry['gaps']) and integration_ran)
    receipt.shapes.append(entry)
    detail = ''
    if name == 'hosted-like':
        detail = f', {entry["tests"]["passed"]} passed, {entry["tests"]["failed"]} failed'
        detail += f' ({"PASS" if entry["passed"] else "FAIL"}; {len(entry["gaps"])} named host gaps)'
        for failed in entry['failures']:
            detail += f'\n  FAILED {failed}'
        for gap in entry['gaps']:
            detail += f'\n  GAP {gap} (needs gwz-py and taut-proto, which the host does not have)'
    else:
        detail = f', {entry["warnings"]} warnings'
    print(f'{name}: exit {result.returncode} in {seconds}s{detail}', flush=True)
    if not entry.get('passed', result.returncode == 0):
        print(text, file=sys.stderr)


def check_hosted_options(options) -> None:
    """The hosted-like options mean nothing without the mode; a run that would silently ignore one is refused."""
    check_git_root(options.git_root)
    if options.hosted_like_filter is not None:
        check_filter(options.hosted_like_filter)
    stray = [flag for flag, given in (('--git-root', options.git_root != DEFAULT_GIT_ROOT),
                                      ('--hosted-like-filter', options.hosted_like_filter is not None),
                                      ('--no-check-shapes', options.no_check_shapes)) if given]
    if stray and not options.hosted_like:
        raise Refused(f'{", ".join(stray)} only apply with --hosted-like')


def execute(options, host: Host, core: Path = CORE, now=datetime.datetime.now) -> int:
    label = check_label(options.label)
    check_hosted_options(options)
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
    if options.no_check_shapes:
        shapes = ()
    else:
        shapes = SHAPES
    for name, flags in shapes:
        run_entry(host, receipt, name, flags, shape_script(label, flags, options.tests, name),
                  f'{HOST_ROOT}/{label}/logs/{name}.log')
    if options.hosted_like:
        run_entry(host, receipt, 'hosted-like', CANDIDATE,
                  hosted_like_script(label, options.git_root, options.hosted_like_filter),
                  f'{HOST_ROOT}/{label}/logs/hosted-like.log', test_filter=options.hosted_like_filter)
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
    p.add_argument('--hosted-like', action='store_true',
                   help='also run the candidate library suites as the hosted job does (short TEMP, portable sshd)')
    p.add_argument('--git-root', default=DEFAULT_GIT_ROOT, metavar='DIR',
                   help='Windows directory of the Git for Windows the hosted-like run uses (default: the host\'s)')
    p.add_argument('--hosted-like-filter', metavar='NAME', help='run only the tests whose path contains NAME')
    p.add_argument('--no-check-shapes', action='store_true', help='with --hosted-like: skip the cargo check shapes')
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


def run(argv=None) -> int:
    """`main`, printing a refusal's message on stderr before returning its status."""
    try:
        return main(argv)
    except Refused as refused:
        print(refused.message, file=sys.stderr)
        return refused.code


if __name__ == '__main__':
    raise SystemExit(run())

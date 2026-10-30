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

The siblings are staged from what the lane has: gwz-transport is the lane's own tree, beside gwz-core (or the one
`--transport-tree DIR` names), archived as the core is (a `--ref` receipt: tracked files; `--worktree`: untracked, unignored
ones too, recorded), with its `.git` so the suite's pin check reads its HEAD on the host, and never fetched from GitHub, so a
lane need not push gwz-transport before it can receipt gwz-core. The Mac's `.git/config`, hooks, logs and info are NOT shipped
(a repo-local `core.autocrlf=false` would beat the host's global `true` and leave the checkout LF, and a config can hold
credentials); the host writes a minimal config with `core.autocrlf = true`, as the hosted runner image has it (the host's own
Git has no such setting), and checks the files out again. The receipt records the effective value and its origin
(`transport.host_autocrlf`). A `.git`
that is a gitfile or a symlink, or has `objects/info/alternates`, is refused by name: its objects are not in the tree.
`--preflight-only` judges the tree, the pin and the label and exits without touching the host (win-receipt.zsh runs it before
it books the host's one slot). gwz-sspi and gwz-git are fetched at gwz-core's pins. The receipt's `transport` entry records the tree,
its HEAD, its tracked changes and untracked files, the pin (`.github/gwz-transport.commit` of the commit being receipted)
and whether HEAD equals it. CI builds the pin, so a tree whose HEAD is not the pin, or that has tracked changes, would
certify code CI does not build: that is refused before the host is touched, unless `--allow-transport-pin-mismatch` says the
lane moves the pin (the line then reads PIN MISMATCH, the receipt records it, the hosted-like suite gets the same flag, and the
final line says `(PASS, PIN MISMATCH allowed)`). On a clean tree the hosted-like run must find the owner export CRLF
(`HOSTED_LIKE_OWNER_EOL crlf`), as a hosted Windows checkout has it; a receipt that finds LF fails.

`--hosted-like` also RUNS the candidate build's library suites the way the hosted `candidate-windows` job does, which
`cargo check` cannot: the job's own `run_tests.py ... --lib` command with the portable Win32-OpenSSH `sshd.exe` (the
job's pinned release, SHA-256 checked, unpacked inside the run's directory), `GWZ_TEST_SH`, and a `TEMP` that is an
8.3 short path (a hosted runner's user directory is spelled `RUNNER~1`; the script refuses to run when the host would not
spell it short). `--git-root DIR` puts another Git for Windows (for example a portable 2.56) first on `PATH` and names
its `sh.exe`. The run is long (about twenty minutes warm) and belongs under the host booking. Its result is a
`hosted-like` entry in the receipt: exit status, the suite's passed and failed counts and the names of the failed tests.
After the suite the mode runs the job's integration targets (`INTEGRATION_TARGETS`, step 5.4) into the same log; a
filtered run leaves them out. `--taut-proto RELEASE` installs that taut-proto in a venv under the run's directory (nothing system-wide) and names it to the
suite as `TAUT_PYTHON`, so tests/protocol.rs runs rather than failing as a host gap; the receipt records the owner export's line
endings in the checkout (a Windows host checks gwz-transport out with CRLF) and the probe of taut-proto. A failed test is a named
gap only when its own failure text says gwz-py or taut-proto is missing (the embedded-Python rows), or, for the protocol target's
rows, when the probe found no taut-proto. `--hosted-like-filter NAME` narrows the run to the tests whose path contains NAME, for iterating on one failure; the
receipt says it was filtered. It reaches `run_tests.py` as `--test-filter`, which narrows each group to its own tests. `--no-check-shapes` skips the `cargo check` shapes (a run that is only the suite).

A watchdog bounds every phase (`stage`, `ordinary`, `transport`, `hosted-like`; the builds are part of their phase). Each has a
wall-clock deadline, and each phase with a log has a no-progress deadline: the phase is aborted when its log has not grown for
that long. The defaults come from the 247 receipts recorded so far (ordinary at most 117 s, transport 109 s, hosted-like 900 s
cold and about 730 s warm, with 1482 s the slowest ever, a failing run; staging 2905 s once) and are several times those, and
far below the 7944 s that one hung hosted-like run took:

    phase        --deadline   --no-progress
    stage        3600 s       (no log: wall clock only)
    ordinary      900 s        600 s
    transport     900 s        600 s
    hosted-like  2700 s       1200 s

    --deadline PHASE=SECONDS   --no-progress PHASE=SECONDS   --watchdog-poll SECONDS   (0 switches a limit off)

On an abort, before anything is killed, the runner captures on the host, next to the phase's logs: the tests libtest reported as
"running for over 60 seconds" that never finished, the last lines of the phase log, the Windows process tree under the phase's
own root process (PID, parent, image, command line, CPU time, start time, working set; the root is the `bash` that runs the
phase's script, whose Windows PID the script records in `logs/PHASE.pid`) and the host's CPU load. It then kills that tree with
`taskkill /T /F /PID <root>` (never by image name, and only while the root is still the process that wrote the pid file),
checks that every recorded PID has ended (a survivor is killed by its PID) and writes the receipt: `result` is `TIMEOUT`,
`passed` is false, and `abort` names the phase, the elapsed seconds, the reason (`deadline` or `no-progress`), the stalled
tests and the evidence files (`watchdog-PHASE.json` and `watchdog-PHASE-log-tail.txt` beside `receipt.json`, and
`logs/watchdog-PHASE.json` on the host). A watchdog abort is never a PASS.

Exit status: 0 every phase passed, 1 a phase failed, 2 refused (usage), 124 the watchdog aborted a phase (TIMEOUT).
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
import threading
import time

CORE = Path(__file__).resolve().parents[1]
_spec = importlib.util.spec_from_file_location('check_windows_parity', CORE / 'scripts' / 'checks' / 'check_windows_parity.py')
parity = importlib.util.module_from_spec(_spec)
sys.modules.setdefault('check_windows_parity', parity)
_spec.loader.exec_module(parity)
HOST = 'gianni@dabeest'
HOST_ROOT = '/e/gwz-tests'  # E:/gwz-tests in MinGW bash
HOST_ROOT_WINDOWS = 'E:\\gwz-tests'  # the same directory as Windows spells it
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
# What a missing gwz-py checkout or missing taut-proto looks like in a failed test's output. A row named above is a gap only
# when its own failure text says so (plan step 2.2 review, S-P2-1): the same row failing for any other reason, a changed
# line ending among them, is a failure. A row with no recorded text is not refuted.
MISSING_TOOLING = re.compile(r"ModuleNotFoundError|No module named|codec input is missing|cannot locate gwz-dev workspace|"
                             r"gwz-py checkout|taut-proto")
# The same gap in the integration targets (step 5.4): tests/protocol.rs runs `python -m taut.cli` and the log-additive
# check, which import taut from the interpreter's site-packages (the hosted job installs taut-proto; the mode does not).
# Whole names, since an integration test has no module path to take a prefix of. Their own panic says only "taut generator
# failed" (the child's stderr is not the test's), so they are gaps only when the host's own probe found no taut-proto
# (`HOSTED_LIKE_TAUT missing`); with `--taut-proto` installed they must pass.
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
# Fetched at gwz-core's pins. gwz-transport is not here: the lane's own tree is staged (see `stage_transport`).
SIBLING_PINS = (('gwz-sspi', 'https://github.com/owebeeone/gwz-sspi.git'),
                ('gwz-git', 'https://github.com/owebeeone/gwz-git.git'))
PIN_FLAG = '--allow-transport-pin-mismatch'
TRANSPORT_PIN_FILE = '.github/gwz-transport.commit'
FULL_SHA = re.compile(r'[0-9a-f]{40}')
TAUT_RELEASE = re.compile(r'[0-9]+\.[0-9]+\.[0-9]+')
LABEL = re.compile(r'^[a-z][a-z0-9]*(-[a-z0-9]+)*$')
DATE = re.compile(r'(^|-)20\d{6}(-|$)')
# The host's Application Control policy sometimes refuses to run a build script that cargo has just written
# (os error 4551); the shape is run once more and the receipt records both attempts.
BLOCKED = 'os error 4551'
# The watchdog (see the module doc for where the numbers come from). Seconds; 0 switches a limit off.
EXIT_TIMEOUT = 124
PHASES = ('stage', 'ordinary', 'transport', 'hosted-like')
DEFAULT_DEADLINES = {'stage': 3600, 'ordinary': 900, 'transport': 900, 'hosted-like': 2700}
DEFAULT_QUIET = {'ordinary': 600, 'transport': 600, 'hosted-like': 1200}  # the stage has no log
DEFAULT_POLL = 60
POLL_TIMEOUT, EVIDENCE_TIMEOUT, KILL_TIMEOUT = 45, 180, 90  # per ssh call of the watchdog, seconds
KILL_ROUNDS = 3  # survivors of the tree kill are killed by PID this many times before they are reported
STAMP = re.compile(r'^[0-9T:.+Z-]+$')  # an ISO time as PowerShell prints it ('o' format)
SLOW_TEST = re.compile(r'^test (.+?) has been running for over \d+ seconds\s*$')
DONE_TEST = re.compile(r'^test (.+?) \.\.\. (?:ok|FAILED|ignored)\b')
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


def wait_for(done, seconds):
    """Waits up to `seconds` for the event; whether it is set."""
    return done.wait(seconds)


@dataclass
class Host:
    """The Windows host. `run` is subprocess.run, replaced in tests; `clock` is time.monotonic; `wait` is how the watchdog
    waits for a phase to end (`wait_for`), replaced in tests so that a deadline passes without anyone sleeping."""
    run: object = subprocess.run
    clock: object = time.monotonic
    wait: object = wait_for

    def ssh(self, script: str, check=True, capture=True):
        """Runs a bash script on the host through stdin, so nothing needs shell quoting."""
        return self.run(SSH + ['bash', '-s'], input=script, text=True, capture_output=capture, check=check)

    def remote(self, script: str, timeout: float):
        """A watchdog call: a bash script on the host with a time limit; None when the call failed or timed out."""
        try:
            return self.run(SSH + ['bash', '-s'], input=script, text=True, errors='replace', capture_output=True,
                            check=False, timeout=timeout)
        except (subprocess.TimeoutExpired, OSError):
            return None


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
    abort: dict | None = None  # the watchdog's summary, when it aborted a phase
    transport: dict | None = None  # the gwz-transport tree staged (see `transport_facts`)

    def passed(self) -> bool:
        return bool(self.shapes) and all(s.get('passed', s['exit'] == 0) for s in self.shapes)

    def as_json(self) -> str:
        total = round(self.stage_seconds + sum(a['seconds'] for s in self.shapes for a in s['attempts']), 1)
        data = {'label': self.label, 'host_dir': f'{HOST_ROOT}/{self.label}', 'source': self.source,
                'commit': self.commit, 'cache_from': self.cache_from, 'with_tests': self.with_tests,
                'toolchain': self.toolchain, 'started': self.started, 'stage_seconds': self.stage_seconds,
                'shapes': self.shapes, 'total_seconds': total, 'passed': self.passed()}
        if self.transport is not None:
            data['transport'] = self.transport
        if self.abort:
            data.update(result='TIMEOUT', abort=self.abort)
        return json.dumps(data, indent=2) + '\n'


def pid_lines(d: str, name: str) -> list[str]:
    """Host script lines: the Windows PID and the MSYS PID of the script's own bash in the phase's pid file, which is where the
    watchdog finds the root of the process tree it may have to end. (MSYS keeps its own numbers, and a program that MSYS
    `exec`s leaves the Windows parent chain, so the MSYS descendants are found by the MSYS PID.)"""
    return [f'mkdir -p {d}/logs', f'echo "$(cat /proc/$$/winpid) $$" > {d}/logs/{name}.pid']


# `core.autocrlf = true` mirrors the hosted Windows runner image, whose SYSTEM config resolves it (the comments in
# .github/workflows/windows-matrix.yml and transport-candidate.yml; windows-matrix.yml also sets it globally). The receipt states it
# rather than inherit whatever the host's Git says. (The 19 LF owner exports recorded 2026-10-25 to 2026-10-30 were the probe's
# grep without -U, which strips CR on Git for Windows; receipt gate-fixes-20261030-b found 2402 CRLF lines that it called lf.)
TRANSPORT_HOST_CONFIG = "printf '[core]\\n\\trepositoryformatversion = 0\\n\\tbare = false\\n\\tautocrlf = true\\n'"
AUTOCRLF_WORD = 'TRANSPORT_AUTOCRLF'


def transport_host_lines(d: str, host_checkout: bool) -> list[str]:
    """Host script lines: the unpacked gwz-transport tree becomes a repository again. Its `.git` came without the Mac's
    config, so the minimal repo-local config is written: `core.autocrlf = true` as the hosted runner image has it (a repo-local
    value beats the host's global and system ones, so the checkout is CRLF whatever the host's own Git is set to), and no eol
    or credential key. With `host_checkout` the files are written again from the index by the host's own Git, as that job's
    checkout writes them; a tree with tracked changes keeps the files it was archived with, since that would discard the
    changes. The effective `core.autocrlf` and the file it comes from are printed for the receipt."""
    tree = f'{d}/ws/gwz-transport'
    lines = [f'{TRANSPORT_HOST_CONFIG} > {tree}/.git/config']
    if host_checkout:
        lines.append(f'git -C {tree} checkout-index -a -f')
    lines.append(f'echo "{AUTOCRLF_WORD} $(git -C {tree} config --show-origin --get core.autocrlf)"')
    return lines


def parse_autocrlf(text: str) -> dict:
    """The stage's `TRANSPORT_AUTOCRLF <origin>\\t<value>` line (`git config --show-origin`) as the receipt records it; absent
    ones read as empty strings, never as a guessed value."""
    for line in text.splitlines():
        if line.startswith(AUTOCRLF_WORD + ' '):
            origin, _, value = line[len(AUTOCRLF_WORD) + 1:].rstrip('\r').partition('\t')
            return {'value': value, 'origin': origin}
    return {'value': '', 'origin': ''}


def stage_script(label: str, cache_from: str | None, host_checkout: bool = False) -> str:
    """Host script: the siblings at gwz-core's pins, then the prepared candidate. The core and gwz-transport are already in
    place; the transport tree is made a repository again (see `transport_host_lines`)."""
    d = f'{HOST_ROOT}/{label}'
    lines = ['set -euo pipefail', *pid_lines(d, 'stage'), 'export GIT_TERMINAL_PROMPT=0', f'cd {d}/ws/gwz-core',
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
    lines += transport_host_lines(d, host_checkout)
    # `cargo metadata` prunes the prepared copy's lock file to the packages the copy uses (it is not a workspace), as the CI job does.
    lines += [f'python tests/transport_backend/prepare.py {d}/candidate', f'export RUSTUP_TOOLCHAIN={TOOLCHAIN}',
              f'cargo metadata --format-version 1 --manifest-path {d}/candidate/Cargo.toml > /dev/null', 'rustc --version']
    return '\n'.join(lines) + '\n'


def shape_script(label: str, flags: str, with_tests: bool, name: str) -> str:
    d = f'{HOST_ROOT}/{label}'
    targets = '--lib --profile test' if with_tests else '--lib'  # the test profile also checks in test mode (cfg(test))
    return '\n'.join([
        'set -uo pipefail', *pid_lines(d, name), f'cd {d}/ws/gwz-core',
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


def taut_lines(d: str, taut_proto: str | None) -> list[str]:
    """Host script lines: taut-proto in a venv inside the run's directory when asked (nothing system-wide), the probe, and
    the line ending of the owner export the checkout produced (the plan step 2.2 review's CRLF case)."""
    lines = []
    if taut_proto:
        venv = f'{d}/taut-venv'
        lines += [f'python -m venv {venv} || exit 9',
                  f'{venv}/Scripts/python.exe -m pip install --quiet --disable-pip-version-check "taut-proto=={taut_proto}" || exit 10',
                  f'export TAUT_PYTHON="$(cygpath -w {venv}/Scripts/python.exe)"']
    lines += ['taut=$("${TAUT_PYTHON:-python}" -c "import importlib.metadata as m; print(m.version(\'taut-proto\'))" 2>/dev/null || echo missing)',
              'echo "HOSTED_LIKE_TAUT $taut"',
              f"if grep -qU \"$(printf '\\r')\" {d}/ws/gwz-transport/protocol/transport.ir.json; then eol=crlf; else eol=lf; fi",
              'echo "HOSTED_LIKE_OWNER_EOL $eol"']
    return lines


def failure_text_lines(d: str) -> list[str]:
    """Host script lines: one `HOSTED_LIKE_FAILURE_TEXT name<TAB>text` per failed test, from the libtest report's
    `---- name stdout ----` sections, flattened to one line and cut at 600 characters."""
    log = f'{d}/logs/hosted-like.log'
    awk = ('/^---- .* stdout ----/ {if (n != "") print "HOSTED_LIKE_FAILURE_TEXT " n "\t" substr(t, 1, 600); '
           'n = $0; sub(/^---- /, "", n); sub(/ stdout ----.*$/, "", n); t = ""; next} '
           '/^(failures:|test result:)/ {if (n != "") print "HOSTED_LIKE_FAILURE_TEXT " n "\t" substr(t, 1, 600); n = ""; next} '
           'n != "" {gsub(/\r/, ""); t = t " " $0} END {if (n != "") print "HOSTED_LIKE_FAILURE_TEXT " n "\t" substr(t, 1, 600)}')
    return [f"awk '{awk}' {log}"]


def hosted_like_script(label: str, git_root: str, test_filter: str | None, taut_proto: str | None = None,
                       allow_pin_mismatch: bool = False) -> str:
    """Host script: the candidate build's library suites run under the hosted job's conditions (see the module doc).

    The suite command is the job's, the environment is the job's (the portable sshd, `GWZ_TEST_SH`, the real Git
    backend, the candidate switch) except where the host differs: the TEMP is made short here, because the
    hosted runner's user directory is. Nothing outside the run's directory and the user's TEMP is written.
    """
    d = f'{HOST_ROOT}/{label}'
    ssh = f'{d}/openssh'
    filter_word = f' --test-filter {test_filter}' if test_filter else ''
    # The suite checks the gwz-transport tree's HEAD against gwz-core's pin; a lane that moves the pin (and was let past
    # the receipt's own check by the flag) runs it with the same flag.
    pin_word = f' {PIN_FLAG}' if allow_pin_mismatch else ''
    return '\n'.join([
        'set -uo pipefail', *pid_lines(d, 'hosted-like'), f'mkdir -p {ssh}', f"GIT_ROOT='{git_root}'",
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
        f'export CARGO_TARGET_DIR={d}/target-validator', *taut_lines(d, taut_proto), f'cd {d}/ws/gwz-core',
        f'{HOSTED_SUITE}{pin_word}{filter_word} --manifest-path={d}/candidate/Cargo.toml --target-dir={d}/target-test '
        f'> {d}/logs/hosted-like.log 2>&1',
        'status=$?',
        # The library suite's own count, before the integration step adds its lines to the log: a library binary that never
        # ran leaves only the integration targets' passes in the total, and the total alone cannot tell.
        f"library=$(grep -a -E '^test result:' {d}/logs/hosted-like.log | sed -n -E 's/^test result: [A-Za-z]+\\. ([0-9]+) passed.*/\\1/p' "
        "| awk '{s+=$1} END{print s+0}')",
        'echo "HOSTED_LIKE_LIBRARY passed=$library"',
        *integration_lines(d, test_filter),
        # One line per fact, for the receipt: the last "test result:" of each test binary run adds up.
        f"passed=$(grep -a -E '^test result:' {d}/logs/hosted-like.log | sed -n -E 's/^test result: [A-Za-z]+\\. ([0-9]+) passed.*/\\1/p' "
        "| awk '{s+=$1} END{print s+0}')",
        f"failed=$(grep -a -E '^test result:' {d}/logs/hosted-like.log | sed -n -E 's/^test result: [A-Za-z]+\\. [0-9]+ passed; ([0-9]+) failed.*/\\1/p' "
        "| awk '{s+=$1} END{print s+0}')",
        'echo "HOSTED_LIKE_RESULT passed=$passed failed=$failed"',
        f"grep -a -E '^test .* \\.\\.\\. FAILED' {d}/logs/hosted-like.log | sed -E 's/^test (.*) \\.\\.\\. FAILED.*/HOSTED_LIKE_FAILED \\1/' | sort -u",
        *failure_text_lines(d),
        'exit $status']) + '\n'


def is_host_gap(name: str, facts: dict) -> bool:
    """Whether a failed test is a named gap: its row needs gwz-py or taut-proto, and the evidence says the host lacks it.

    An embedded-Python row is a gap when its own failure text names the missing tooling (or no text was recorded). A
    protocol-target row is a gap only when the host's probe found no taut-proto: with it installed the row must pass."""
    if name.startswith(HOST_GAPS):
        recorded = facts.get('failure_texts', {}).get(name)
        return recorded is None or bool(MISSING_TOOLING.search(recorded))
    return name in HOST_GAP_NAMES and facts.get('taut_proto', 'missing') == 'missing'


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
        elif word == 'HOSTED_LIKE_LIBRARY':
            facts['library_passed'] = int(rest.partition('=')[2])
        elif word == 'HOSTED_LIKE_FAILED':
            facts['tests']['failed_names'].append(rest)
        elif word == 'HOSTED_LIKE_TAUT':
            facts['taut_proto'] = rest
        elif word == 'HOSTED_LIKE_OWNER_EOL':
            facts['owner_export_line_endings'] = rest
        elif word == 'HOSTED_LIKE_FAILURE_TEXT':
            name, _, text = rest.partition('\t')
            facts.setdefault('failure_texts', {})[name] = text
        elif word == 'HOSTED_LIKE_RESULT':
            counts = dict(part.split('=', 1) for part in rest.split())
            facts['tests'].update(passed=int(counts.get('passed', 0)), failed=int(counts.get('failed', 0)))
    return facts


# ---- The watchdog -------------------------------------------------------------------------------------------------

@dataclass
class Limits:
    """The watchdog's limits in seconds: a wall-clock deadline per phase, a no-progress deadline per phase with a log, the poll."""
    deadline: dict
    quiet: dict
    poll: float = DEFAULT_POLL


def parse_limit(text: str, flag: str, allowed) -> tuple[str, float]:
    """`PHASE=SECONDS` from --deadline or --no-progress; 0 switches that limit off."""
    phase, _, seconds = text.partition('=')
    try:
        value = float(seconds)
    except ValueError:
        value = -1.0
    if phase not in allowed or value < 0:
        raise Refused(f'{flag} {text!r} must be PHASE=SECONDS with PHASE one of {", ".join(allowed)} and SECONDS >= 0')
    return phase, int(value) if value == int(value) else value


def watchdog_limits(options) -> Limits:
    """The defaults with the command line's overrides applied; a refusal for a limit that means nothing."""
    deadline, quiet = dict(DEFAULT_DEADLINES), dict(DEFAULT_QUIET)
    for text in options.deadline or ():
        phase, seconds = parse_limit(text, '--deadline', PHASES)
        deadline[phase] = seconds
    for text in options.no_progress or ():
        phase, seconds = parse_limit(text, '--no-progress', tuple(DEFAULT_QUIET))
        quiet[phase] = seconds
    if options.watchdog_poll <= 0:
        raise Refused(f'--watchdog-poll {options.watchdog_poll} must be more than 0 seconds')
    return Limits(deadline, quiet, options.watchdog_poll)


def stalled_tests(text: str) -> list[str]:
    """The tests libtest reported "has been running for over N seconds" and never reported finished, in report order."""
    running = {}
    for raw in text.splitlines():
        line = raw.rstrip('\r')
        slow = SLOW_TEST.match(line)
        if slow:
            running.setdefault(slow.group(1))
            continue
        done = DONE_TEST.match(line)
        if done:
            running.pop(done.group(1), None)
    return list(running)


def powershell(script: str) -> str:
    """Host script lines that run a PowerShell script from stdin. Every statement is on one line: `-Command -` reads the
    script as typed input."""
    return f"powershell -NoProfile -NonInteractive -Command - <<'WATCHDOG_PS'\n{script}\nWATCHDOG_PS\n"


def poll_script(label: str, name: str) -> str:
    """Host script: the size of the phase's log (a phase without one has nothing to watch)."""
    if name not in DEFAULT_QUIET:
        return '# watchdog-poll\ntrue\n'
    return ('# watchdog-poll\n'
            f'f={HOST_ROOT}/{label}/logs/{name}.log\n'
            'if [ -f "$f" ]; then echo "WATCHDOG_SIZE $(stat -c %s "$f")"; else echo "WATCHDOG_SIZE none"; fi\n')


EVIDENCE_PS = r"""$pidfile = '{pidfile}'
$root = 0
try { $root = [int]((Get-Content -Raw -LiteralPath $pidfile).Trim().Split(' ')[0]) } catch { }
$all = @(Get-CimInstance Win32_Process)
$mtime = $null
try { $mtime = (Get-Item -LiteralPath $pidfile).LastWriteTimeUtc } catch { }
$self = $all | Where-Object { [int]$_.ProcessId -eq $root } | Select-Object -First 1
$valid = $false
$why = ''
if (-not $self) { $why = 'no such process: the root has exited or the pid file is missing' } elseif (-not $mtime) { $why = 'the pid file has no time' } else { $gap = ($mtime - $self.CreationDate.ToUniversalTime()).TotalSeconds; if ($gap -ge -1 -and $gap -le 120) { $valid = $true } else { $why = "the process is not the one that wrote the pid file (it was created $gap s before the file was written)" } }
$started = $null
if ($self) { $started = $self.CreationDate.ToUniversalTime().ToString('o') }
$written = $null
if ($mtime) { $written = $mtime.ToString('o') }
'WATCHDOG_ROOT ' + ([pscustomobject]@{ pid = $root; started = $started; pidfile_utc = $written; valid = $valid; why = $why } | ConvertTo-Json -Compress)
$tree = New-Object System.Collections.Generic.List[object]
$queue = New-Object System.Collections.Generic.Queue[object]
$seen = @{}
if ($valid) { $queue.Enqueue($self); $seen[$root] = 1 }
$extra = @()
if ($env:WATCHDOG_EXTRA) { $extra = @($env:WATCHDOG_EXTRA.Split(',') | Where-Object { $_ } | ForEach-Object { [int]$_ }) }
if ($valid) { foreach ($e in $extra) { $q = $all | Where-Object { [int]$_.ProcessId -eq $e -and $_.CreationDate -ge $self.CreationDate } | Select-Object -First 1; if ($q -and -not $seen.ContainsKey($e)) { $seen[$e] = 1; $queue.Enqueue($q) } } }
while ($queue.Count -gt 0) { $p = $queue.Dequeue(); $tree.Add($p); foreach ($c in ($all | Where-Object { [int]$_.ParentProcessId -eq [int]$p.ProcessId -and $_.CreationDate -ge $p.CreationDate })) { if (-not $seen.ContainsKey([int]$c.ProcessId)) { $seen[[int]$c.ProcessId] = 1; $queue.Enqueue($c) } } }
foreach ($p in $tree) { 'WATCHDOG_PROC ' + ([pscustomobject]@{ pid = [int]$p.ProcessId; ppid = [int]$p.ParentProcessId; image = $p.Name; cmd = $p.CommandLine; cpu_seconds = [math]::Round(([double]$p.KernelModeTime + [double]$p.UserModeTime) / 1e7, 2); started = $p.CreationDate.ToUniversalTime().ToString('o'); working_set = [int64]$p.WorkingSetSize } | ConvertTo-Json -Compress) }
$cpu = (Get-CimInstance Win32_Processor | Measure-Object -Property LoadPercentage -Average).Average
$os = Get-CimInstance Win32_OperatingSystem
$logical = (Get-CimInstance Win32_ComputerSystem).NumberOfLogicalProcessors
'WATCHDOG_LOAD ' + ([pscustomobject]@{ cpu_percent = $cpu; logical_processors = $logical; memory_free_mb = [int]($os.FreePhysicalMemory / 1024); memory_total_mb = [int]($os.TotalVisibleMemorySize / 1024) } | ConvertTo-Json -Compress)"""


# The MSYS descendants of the phase's bash, from `ps -W`: MSYS keeps its own parent chain, which a Windows parent id loses
# when MSYS `exec`s a program (the new process's Windows parent is a stub that has already exited). Prints their Windows PIDs,
# comma separated.
MSYS_DESCENDANTS = (r'NR > 1 { win[$1] = $4; parent[$1] = $2; n++; id[n] = $1 } '
                    r'END { seen[root] = 1; again = 1; while (again) { again = 0; for (i = 1; i <= n; i++) { q = id[i]; '
                    r'if (!(q in seen) && (parent[q] in seen)) { seen[q] = 1; again = 1; printf "%s,", win[q] } } } }')
LIBTEST_LINES = r"^test .*( has been running for over [0-9]+ seconds| \.\.\. (ok|FAILED|ignored))"


def evidence_script(label: str, name: str) -> str:
    """Host script, run before anything is killed: the root process named by the phase's pid file (valid only while it is the
    process that wrote the file) and the processes under it, the host's load, the libtest lines that tell which tests are
    stalled, and the last lines of the phase's log."""
    d = f'{HOST_ROOT}/{label}'
    pidfile = f'{HOST_ROOT_WINDOWS}\\{label}\\logs\\{name}.pid'
    return '\n'.join([
        '# watchdog-evidence', 'set +e', f'log={d}/logs/{name}.log', f'read -r winpid msyspid < {d}/logs/{name}.pid',
        "extra=''", f'if [ -n "$msyspid" ]; then extra=$(ps -W | awk -v root="$msyspid" \'{MSYS_DESCENDANTS}\'); fi',
        'export WATCHDOG_EXTRA="$extra"',
        powershell(EVIDENCE_PS.replace('{pidfile}', pidfile)).rstrip('\n'),
        'echo WATCHDOG_TESTS_BEGIN', f'[ -f "$log" ] && grep -a -E \'{LIBTEST_LINES}\' "$log" | tr -d \'\\r\'',
        'echo WATCHDOG_TESTS_END', 'echo WATCHDOG_TAIL_BEGIN',
        '[ -f "$log" ] && tail -n 60 "$log" | tr -d \'\\r\' | cut -c1-500', 'echo WATCHDOG_TAIL_END',
        '[ -f "$log" ] && echo "WATCHDOG_LOGSIZE $(stat -c %s "$log")"', 'true']) + '\n'


def check_stamp(stamp: str) -> str:
    if not isinstance(stamp, str) or not STAMP.fullmatch(stamp):
        raise ValueError(f'not a recorded process start time: {stamp!r}')
    return stamp


def kill_script(root: int, started: str) -> str:
    """Host script: ends the process tree under `root` (`taskkill /T /F /PID`), and only while `root` is still the process the
    evidence recorded (same start time), so a PID the host has reused for someone else's process is left alone."""
    ps = (f"$root = {int(root)}\n$expected = '{check_stamp(started)}'\n"
          '$p = Get-CimInstance Win32_Process -Filter "ProcessId = $root"\n'
          "if ($p -and $p.CreationDate.ToUniversalTime().ToString('o') -eq $expected) { $out = & taskkill.exe /T /F /PID $root 2>&1; "
          "'WATCHDOG_KILL taskkill rc=' + $LASTEXITCODE; $out | ForEach-Object { 'WATCHDOG_KILL_OUT ' + $_ } } "
          "else { 'WATCHDOG_KILL refused: the root is gone or is not the recorded process' }\n"
          'Start-Sleep -Seconds 3')
    return '# watchdog-kill\n' + powershell(ps)


def verify_script(recorded: list[dict], kill: bool) -> str:
    """Host script: which of the recorded processes (same PID and start time) are still alive; with `kill`, each is first ended
    by its PID alone. The list is plain `pid;start` words: ConvertFrom-Json would turn the start times into dates."""
    want = ', '.join(f"'{int(r['pid'])};{check_stamp(r['started'])}'" for r in recorded)
    alive = ('$id, $at = $w.Split(";"); $p = Get-CimInstance Win32_Process -Filter "ProcessId = $id"; '
             "if ($p -and $p.CreationDate.ToUniversalTime().ToString('o') -eq $at)")
    lines = [f'$want = @({want})']
    if kill:
        lines += [f'foreach ($w in $want) {{ {alive} {{ & taskkill.exe /F /PID $id | Out-Null }} }}', 'Start-Sleep -Seconds 2']
    lines += [f"foreach ($w in $want) {{ {alive} {{ 'WATCHDOG_ALIVE ' + ([pscustomobject]@{{ pid = [int]$id; "
              f"ppid = [int]$p.ParentProcessId; image = $p.Name; started = $at }} | ConvertTo-Json -Compress) }} }}",
              "'WATCHDOG_ALIVE_DONE'"]
    return '# watchdog-verify\n' + powershell('\n'.join(lines))


def tagged(text: str, tag: str) -> list:
    """The JSON values of the `TAG {json}` lines in a watchdog script's output."""
    values = []
    for line in (text or '').splitlines():
        word, _, rest = line.rstrip('\r').partition(' ')
        if word == tag:
            try:
                values.append(json.loads(rest))
            except ValueError:
                pass
    return values


def between(text: str, begin: str, end: str) -> list[str]:
    lines, inside = [], False
    for raw in (text or '').splitlines():
        line = raw.rstrip('\r')
        if line == begin:
            inside = True
        elif line == end:
            inside = False
        elif inside:
            lines.append(line)
    return lines


def parse_evidence(text: str) -> dict:
    """What the evidence script printed, as a dict (missing parts are None or empty)."""
    roots, loads = tagged(text, 'WATCHDOG_ROOT'), tagged(text, 'WATCHDOG_LOAD')
    size = [line.split()[1] for line in (text or '').splitlines() if line.startswith('WATCHDOG_LOGSIZE ')]
    return {'root': roots[0] if roots else None, 'process_tree': tagged(text, 'WATCHDOG_PROC'),
            'host_load': loads[0] if loads else None, 'test_lines': between(text, 'WATCHDOG_TESTS_BEGIN', 'WATCHDOG_TESTS_END'),
            'tail': between(text, 'WATCHDOG_TAIL_BEGIN', 'WATCHDOG_TAIL_END'),
            'log_size': int(size[0]) if size and size[0].isdigit() else None}


class PhaseAborted(Exception):
    """The watchdog ended a phase; `info` is what it captured and what it did."""

    def __init__(self, info: dict):
        super().__init__(f"{info['phase']}: {info['reason']}")
        self.info = info


class Watchdog:
    """Runs a phase's host script in a worker thread and watches it: a wall-clock deadline for the phase and, when it has a log,
    a no-progress deadline (the log has not grown). On a breach it captures the evidence, ends the phase's own process tree on
    the host, checks that it ended, and raises PhaseAborted. The worker is a daemon, so a local ssh that never returns cannot hold
    the process."""

    def __init__(self, host: Host, limits: Limits, label: str):
        self.host, self.limits, self.label = host, limits, label

    def run(self, name: str, script: str, phase_start: float | None = None):
        """The script's result and its seconds. `phase_start` is when the phase began, for a phase run more than once."""
        host = self.host
        attempt_start = host.clock()
        phase_start = attempt_start if phase_start is None else phase_start
        box, done = {}, threading.Event()

        def work():
            try:
                box['result'] = host.ssh(script, check=False)
            except BaseException as error:  # handed to the caller's thread
                box['error'] = error
            finally:
                done.set()

        worker = threading.Thread(target=work, name='watchdog-phase', daemon=True)
        worker.start()
        deadline, quiet = self.limits.deadline.get(name), self.limits.quiet.get(name)
        last_size, progress_at = None, attempt_start
        while not done.is_set():
            now = host.clock()
            waits = [self.limits.poll]
            if deadline:
                waits.append(phase_start + deadline - now)
            if quiet:
                waits.append(progress_at + quiet - now)
            if host.wait(done, max(0.0, min(waits))):
                break
            now = host.clock()
            size = self.poll(name) if quiet else None
            if size is not None and size != last_size:
                last_size, progress_at = size, now
            reason, limit = None, None
            if deadline and now - phase_start >= deadline - 1e-6:
                reason, limit = 'deadline', deadline
            elif quiet and now - progress_at >= quiet - 1e-6:
                reason, limit = 'no-progress', quiet
            if reason:
                raise PhaseAborted(self.abort(name, reason, limit, phase_start, done, box))
        if 'error' in box:
            raise box['error']
        return box['result'], round(host.clock() - attempt_start, 1)

    def poll(self, name: str):
        """The phase's log size now, or None when it cannot be read (no growth is credited then)."""
        result = self.host.remote(poll_script(self.label, name), POLL_TIMEOUT)
        for line in ((result.stdout or '') if result is not None and result.returncode == 0 else '').splitlines():
            if line.startswith('WATCHDOG_SIZE '):
                value = line.split()[1]
                return int(value) if value.isdigit() else None
        return None

    def abort(self, name: str, reason: str, limit, phase_start: float, done, box: dict) -> dict:
        host = self.host
        elapsed = round(host.clock() - phase_start, 1)
        errors = []
        reply = host.remote(evidence_script(self.label, name), EVIDENCE_TIMEOUT)
        if reply is None:
            errors.append('the evidence script did not answer')
        found = parse_evidence(reply.stdout if reply is not None else '')
        kill = self.kill(found, errors)
        for _ in range(0 if 'refused' in kill else 12):  # the killed script ends its ssh; one that stays is left to its daemon thread
            if host.wait(done, 5):
                break
        partial = box['result'].stdout if 'result' in box and box['result'] is not None else ''
        tests = stalled_tests('\n'.join(found['test_lines']))
        return {'phase': name, 'label': self.label, 'reason': reason, 'limit_seconds': limit, 'elapsed_seconds': elapsed,
                'stalled_tests': tests, 'running_over_60s_reports': sum(1 for l in found['test_lines'] if SLOW_TEST.match(l)),
                'log': {'path': f'{HOST_ROOT}/{self.label}/logs/{name}.log', 'size': found['log_size'], 'tail': found['tail']},
                'host_load': found['host_load'], 'root': found['root'], 'process_tree': found['process_tree'],
                'kill': kill, 'phase_script_ended': done.is_set(), 'phase_output_so_far': (partial or '')[-2000:],
                'errors': errors}

    def kill(self, found: dict, errors: list) -> dict:
        """Ends the root's tree and proves it: every recorded PID (with its start time) is looked up again afterwards, and a
        survivor is killed by its PID, up to KILL_ROUNDS times. `survivors` is None when nothing could be proved."""
        recorded = found['process_tree']
        root = found['root'] or {}
        kill = {'recorded_pids': [r['pid'] for r in recorded], 'survivors': None}
        if not root.get('valid') or not recorded:
            kill['refused'] = root.get('why') or 'no root process was recorded'
            return kill
        reply = self.host.remote(kill_script(root['pid'], root['started']), KILL_TIMEOUT)
        kill['taskkill_output'] = [] if reply is None else (reply.stdout or '').strip().splitlines()[-20:]
        if reply is None:
            errors.append('the kill script did not answer')
        alive = self.alive(recorded, False, errors)
        for _ in range(KILL_ROUNDS):
            if not alive:
                break
            alive = self.alive(alive, True, errors)
        kill['survivors'] = alive
        return kill

    def alive(self, recorded: list, kill: bool, errors: list):
        """The recorded processes still alive, or None when the check could not be made."""
        reply = self.host.remote(verify_script(recorded, kill), KILL_TIMEOUT)
        if reply is None or 'WATCHDOG_ALIVE_DONE' not in (reply.stdout or ''):
            errors.append('the check that the processes ended did not answer')
            return None
        return tagged(reply.stdout, 'WATCHDOG_ALIVE')


UNTRACKED_EXCLUDED = ':(exclude)dev-docs'  # the worktree archive leaves untracked documents at home


def worktree_listing(exclude: str | None, extra: str = '') -> str:
    """A bash pipeline whose stdout is a tar of the working tree it runs in: tracked files as they are on disk (a deleted
    one is left out), untracked, unignored ones outside `exclude`, and `extra` (printf statements naming more paths)."""
    others = f"git ls-files -z --others --exclude-standard -- . '{exclude}'" if exclude else \
        'git ls-files -z --others --exclude-standard'
    # COPYFILE_DISABLE: macOS tar would add ._* AppleDouble files, which the host would unpack as sources.
    return (f'{{ git ls-files -z --cached; {others}; {extra}}} | sort -zu | while IFS= read -r -d \'\' f; do '
            '[ -e "$f" ] && printf \'%s\\0\' "$f"; done | COPYFILE_DISABLE=1 tar -cf - --null -T -')


def archive_command(core: Path, ref: str | None) -> tuple[list[str], str]:
    """The local command whose stdout is a tar of the lane's gwz-core, and the source it names."""
    if ref is None:
        return (['bash', '-c', worktree_listing(UNTRACKED_EXCLUDED)], 'worktree (tracked files, and untracked ones outside dev-docs)')
    return ['git', 'archive', '--format=tar', ref], f'ref {ref}'


# What of the Mac's `.git` stays at home: the config (repo-local keys beat the host's global ones, and it can hold credentials
# and remote URLs), the hooks, the logs and info/attributes (which can switch text handling off).
TRANSPORT_GIT_EXCLUDED = ('.git/config', '.git/hooks', '.git/logs', '.git/info')


def transport_archive_command(untracked: bool = False) -> list[str]:
    """The command (run in the tree) whose stdout is a tar of the lane's gwz-transport: its tracked files as they are on disk
    (and, with `untracked`, the untracked unignored ones: a `--worktree` receipt), and its `.git` minus
    TRANSPORT_GIT_EXCLUDED, which the hosted suite's pin check and the host's checkout read. Every path is listed, so tar
    does not recurse (`--no-recursion`)."""
    prune = ' -o '.join(f'-path {path}' for path in TRANSPORT_GIT_EXCLUDED)
    parts = ['git ls-files -z --cached']
    if untracked:
        parts.append('git ls-files -z --others --exclude-standard')
    parts.append(f'find .git \\( {prune} \\) -prune -o -print0')
    # COPYFILE_DISABLE: macOS tar would add ._* AppleDouble files, which the host would unpack as sources.
    return ['bash', '-c', f'{{ {"; ".join(parts)}; }} | sort -zu | while IFS= read -r -d \'\' f; do '
            '[ -e "$f" ] && printf \'%s\\0\' "$f"; done | COPYFILE_DISABLE=1 tar -cf - --null --no-recursion -T -']


def git_output(core: Path, *args: str, run=subprocess.run) -> str:
    return run(['git', '-C', str(core), *args], text=True, capture_output=True, check=True).stdout.strip()


def pin_in(text: str, where: str) -> str:
    """The first 40-hex line of a pin file's text, as the workflow and run_tests.py read it."""
    for line in text.splitlines():
        if FULL_SHA.fullmatch(line.strip()):
            return line.strip()
    raise Refused(f'{where} names no gwz-transport commit (a 40-hex line)')


def core_transport_pin(core: Path, ref: str | None, run=subprocess.run) -> str:
    """The gwz-transport commit CI builds against for the gwz-core being receipted: the pin file of the working tree, or
    of the ref (the archive the host unpacks is that tree, so its own pin is the one its suite checks)."""
    if ref is None:
        path = core / TRANSPORT_PIN_FILE
        try:
            return pin_in(path.read_text(encoding='utf-8'), str(path))
        except OSError as error:
            raise Refused(f'cannot read {path}: {error}') from None
    try:
        return pin_in(git_output(core, 'show', f'{ref}:{TRANSPORT_PIN_FILE}', run=run), f'{ref}:{TRANSPORT_PIN_FILE}')
    except subprocess.CalledProcessError as error:
        raise Refused(f'cannot read {TRANSPORT_PIN_FILE} at {ref}: {(error.stderr or "").strip()}') from None


def check_transport_git_dir(tree: Path) -> None:
    """Refuses a tree whose `.git` the archive could not carry whole: a gitfile (a linked worktree or a submodule) or a
    symlink points outside the tree, and `objects/info/alternates` names objects outside it, so the host would fail with an
    opaque error in checkout-index or rev-parse."""
    hint = 'the receipt stages the lane\'s own gwz-transport clone: the checkout beside gwz-core, or --transport-tree DIR'
    git_dir = tree / '.git'
    if git_dir.is_symlink():
        raise Refused(f'the gwz-transport tree {tree}: .git is a symlink, so its objects are not in the tree; {hint}')
    if git_dir.is_file():
        raise Refused(f'the gwz-transport tree {tree}: .git is a gitfile (a linked worktree or a submodule), so its objects '
                      f'are outside the tree; use a full clone. {hint}')
    if not git_dir.is_dir():
        raise Refused(f'cannot read the gwz-transport tree {tree} (no .git directory); {hint}')
    if (git_dir / 'objects' / 'info' / 'alternates').exists():
        raise Refused(f'the gwz-transport tree {tree}: .git/objects/info/alternates names objects outside the tree, which '
                      f'the archive would not carry; repack it (git repack -a -d) and remove the alternates file. {hint}')


def transport_facts(tree: Path, pin: str, allow_mismatch: bool, run=subprocess.run, stage_untracked: bool = False) -> dict:
    """What the receipt records of the gwz-transport tree it stages: the tree, its HEAD, its tracked changes (any change
    but an untracked file) and untracked files (staged only when `stage_untracked`: a --worktree receipt), the pin and
    whether HEAD is the pin. A tree that is not at the pin, or has tracked changes, is not what CI builds: it is refused
    unless `allow_mismatch` (the lane moves the pin)."""
    check_transport_git_dir(tree)
    try:
        head = git_output(tree, 'rev-parse', 'HEAD', run=run)
        # not git_output: it strips, and the leading space of ` M path` is part of the status
        status = run(['git', '-C', str(tree), 'status', '--porcelain'], text=True, capture_output=True,
                     check=True).stdout
    except (subprocess.CalledProcessError, OSError) as error:
        detail = (getattr(error, 'stderr', None) or str(error)).strip()
        raise Refused(f'cannot read the gwz-transport tree {tree} ({detail}); the receipt stages the lane\'s own '
                      'gwz-transport: it is the checkout beside gwz-core, or --transport-tree DIR') from None
    lines = [line for line in status.splitlines() if line.strip()]
    tracked = [line[3:] for line in lines if not line.startswith('??')]
    untracked = [line[3:] for line in lines if line.startswith('??')]
    mismatch = head != pin or bool(tracked)
    facts = {'source': 'lane tree', 'path': str(tree), 'head': head, 'pin': pin, 'matches_pin': head == pin,
             'pin_mismatch': mismatch, 'pin_mismatch_allowed': allow_mismatch, 'tracked_changes': tracked,
             'untracked': untracked, 'untracked_staged': stage_untracked, 'host_checkout': not tracked}
    if mismatch:
        what = f'HEAD {head} is not the pin {pin}' if head != pin else f'HEAD is the pin {pin}'
        if tracked:
            what += f', and tracked files are changed ({", ".join(tracked[:5])}{", ..." if len(tracked) > 5 else ""})'
        if not allow_mismatch:
            raise Refused(f'the gwz-transport tree {tree}: {what}. CI builds the pin ({TRANSPORT_PIN_FILE}), so this '
                          f'receipt would certify code CI does not build. Check the pin out, or, for a lane that moves '
                          f'the pin in this change, pass {PIN_FLAG}')
        print(f'PIN MISMATCH ({PIN_FLAG}): gwz-transport {what}; move {TRANSPORT_PIN_FILE} to a pushed commit '
              'that contains these changes before gwz-core is pushed', flush=True)
    if tracked:
        print('gwz-transport: tracked changes keep the files as archived (LF); a clean tree is written again by the '
              "host's Git as a Windows checkout does (CRLF)", flush=True)
    if untracked:
        names = f'{", ".join(untracked[:5])}{", ..." if len(untracked) > 5 else ""}'
        if stage_untracked:
            print(f'gwz-transport: {len(untracked)} untracked file(s) are staged with the tree (--worktree): {names}',
                  flush=True)
        else:
            print(f'gwz-transport: {len(untracked)} untracked file(s) are NOT staged (a --ref receipt stages tracked '
                  f'files only; --worktree stages them): {names}', flush=True)
    return facts


def run_entry(watchdog: Watchdog, receipt: Receipt, name: str, flags: str, script: str, log: str, test_filter=None,
              require_crlf: bool = False) -> None:
    """Runs one host script under the watchdog and records its entry. A build script the host blocked (os error 4551) is run
    once more, both attempts are recorded and both count against the phase's deadline; a genuine failure, a failed test among
    them, is never run again. A phase the watchdog ends raises PhaseAborted."""
    phase_start = watchdog.host.clock()
    result, seconds = watchdog.run(name, script, phase_start)
    attempts = [{'exit': result.returncode, 'seconds': seconds}]
    if result.returncode != 0 and BLOCKED in (result.stdout or ''):
        print(f'{name}: a build script was blocked by the host (os error 4551); running it once more', flush=True)
        result, seconds = watchdog.run(name, script, phase_start)
        attempts.append({'exit': result.returncode, 'seconds': seconds})
    text = result.stdout or ''
    found = WARNINGS.findall(text)
    entry = {'shape': name, 'rustflags': flags, 'exit': result.returncode, 'seconds': seconds,
             'warnings': int(found[-1]) if found else 0, 'attempts': attempts, 'log': log}
    if name == 'hosted-like':
        entry.update(parse_hosted_like(text, test_filter))
        names = entry['tests']['failed_names']
        entry['gaps'] = [n for n in names if is_host_gap(n, entry)]
        entry['failures'] = [n for n in names if n not in entry['gaps']]
        # A gap-only failure passes. An integration step that died (a build error, a binary that could not load) names
        # no failed test, so it passes only when it exited 0 or every target reported a result.
        integration_ran = (entry.get('integration_exit', 0) == 0
                           or entry.get('integration_results') == len(INTEGRATION_TARGETS))
        # A suite that ran no library test passes nothing, whatever the integration targets did (a missing count is zero).
        entry['passed'] = entry.get('library_passed', 0) > 0 and (result.returncode == 0 or bool(
            entry['gaps'] and not entry['failures'] and entry['tests']['failed'] == len(entry['gaps']) and integration_ran))
        # A clean transport tree was written again by the host's Git as a Windows checkout is: its owner export must be CRLF.
        # LF means the receipt lost the coverage it exists for (the step 2.2 review's S-P2-1 case), whatever else passed.
        if require_crlf and entry.get('owner_export_line_endings') != 'crlf':
            entry['passed'] = False
            entry['line_ending_failure'] = (f'HOSTED_LIKE_OWNER_EOL is {entry.get("owner_export_line_endings", "absent")!r}, '
                                            'not crlf: the host checkout of gwz-transport is not a Windows checkout')
    receipt.shapes.append(entry)
    detail = ''
    if name == 'hosted-like':
        detail = f', {entry["tests"]["passed"]} passed, {entry["tests"]["failed"]} failed'
        detail += f' ({"PASS" if entry["passed"] else "FAIL"}; {len(entry["gaps"])} named host gaps; {entry.get("library_passed", 0)} library tests passed)'
        for failed in entry['failures']:
            detail += f'\n  FAILED {failed}'
        for gap in entry['gaps']:
            detail += f'\n  GAP {gap} (needs gwz-py and taut-proto, which the host does not have)'
        if 'line_ending_failure' in entry:
            detail += f'\n  FAILED line endings: {entry["line_ending_failure"]}'
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
    if options.taut_proto is not None and not TAUT_RELEASE.fullmatch(options.taut_proto):
        raise Refused(f'--taut-proto {options.taut_proto!r} must be a release such as 0.10.0')
    stray = [flag for flag, given in (('--git-root', options.git_root != DEFAULT_GIT_ROOT),
                                      ('--hosted-like-filter', options.hosted_like_filter is not None),
                                      ('--no-check-shapes', options.no_check_shapes),
                                      ('--taut-proto', options.taut_proto is not None)) if given]
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
    # The lane's own gwz-transport, judged against the pin of the gwz-core being receipted, before the host is touched.
    tree = options.transport_tree or core.parent / 'gwz-transport'
    transport = transport_facts(tree, core_transport_pin(core, ref, host.run), options.allow_transport_pin_mismatch,
                                host.run, stage_untracked=options.worktree)
    if options.preflight_only:
        print(f'windows_lane_check: preflight ok for {label}: the gwz-transport tree {tree} may be receipted'
              f'{" (PIN MISMATCH allowed)" if transport["pin_mismatch"] else ""}; the host was not touched', flush=True)
        return 0
    claim = host.ssh(f'mkdir {HOST_ROOT}/{label} 2>&1 && mkdir -p {HOST_ROOT}/{label}/ws/gwz-core '
                     f'{HOST_ROOT}/{label}/ws/gwz-transport', check=False)
    if claim.returncode != 0:
        raise Refused(f'{HOST_ROOT}/{label} on the host cannot be created (it may exist; labels are never reused): '
                      f'{(claim.stdout or "").strip()}')
    archive, source = archive_command(core, ref)
    receipt = Receipt(label, source, commit, options.cache_from, options.tests, now().isoformat(timespec='seconds'),
                      transport=transport)
    start = host.clock()
    producer = host.run(archive, cwd=core, capture_output=True, check=True)
    unpack = host.run(SSH + ['tar', '-x', '-C', f'{HOST_ROOT}/{label}/ws/gwz-core'], input=producer.stdout,
                      capture_output=True, check=False)
    if unpack.returncode != 0:
        raise SystemExit(f'windows_lane_check: unpacking the archive on the host failed: {unpack.stderr!r}')
    transport_producer = host.run(transport_archive_command(options.worktree), cwd=tree, capture_output=True, check=True)
    transport_unpack = host.run(SSH + ['tar', '-x', '-C', f'{HOST_ROOT}/{label}/ws/gwz-transport'],
                                input=transport_producer.stdout, capture_output=True, check=False)
    if transport_unpack.returncode != 0:
        raise SystemExit('windows_lane_check: unpacking the gwz-transport tree on the host failed: '
                         f'{transport_unpack.stderr!r}')
    watchdog = Watchdog(host, options.watchdog, label)
    aborted = None
    try:
        staged, _ = watchdog.run('stage', stage_script(label, options.cache_from, transport['host_checkout']))
        receipt.stage_seconds = round(host.clock() - start, 1)
        if staged.returncode != 0:
            print(staged.stdout, staged.stderr, sep='\n', file=sys.stderr)
            raise SystemExit('windows_lane_check: staging the siblings or preparing the candidate failed; the host '
                             f'directory {HOST_ROOT}/{label} is kept for inspection')
        receipt.toolchain = (staged.stdout or '').strip().splitlines()[-1] if (staged.stdout or '').strip() else ''
        receipt.transport['host_autocrlf'] = parse_autocrlf(staged.stdout or '')
        if options.no_check_shapes:
            shapes = ()
        else:
            shapes = SHAPES
        for name, flags in shapes:
            run_entry(watchdog, receipt, name, flags, shape_script(label, flags, options.tests, name),
                      f'{HOST_ROOT}/{label}/logs/{name}.log')
        if options.hosted_like:
            run_entry(watchdog, receipt, 'hosted-like', CANDIDATE,
                      hosted_like_script(label, options.git_root, options.hosted_like_filter, options.taut_proto,
                                         allow_pin_mismatch=not transport['matches_pin']),
                      f'{HOST_ROOT}/{label}/logs/hosted-like.log', test_filter=options.hosted_like_filter,
                      require_crlf=transport['host_checkout'])
    except PhaseAborted as stopped:
        aborted = stopped.info
        record_abort(receipt, aborted, flags_of(aborted['phase']), start, host.clock())
    note = ', PIN MISMATCH allowed' if transport['pin_mismatch'] else ''
    receipt_dir.mkdir(parents=True)
    if aborted:
        save_evidence(host, receipt, receipt_dir, aborted)
    (receipt_dir / 'receipt.json').write_text(receipt.as_json(), encoding='utf-8')
    host.ssh(f"cat > {HOST_ROOT}/{label}/receipt.json <<'RECEIPT'\n{receipt.as_json()}RECEIPT\n", check=False)
    if receipt.abort:
        print(f'receipt: {receipt_dir / "receipt.json"} (TIMEOUT{note})')
        return EXIT_TIMEOUT
    print(f'receipt: {receipt_dir / "receipt.json"} ({"PASS" if receipt.passed() else "FAIL"}{note})')
    return 0 if receipt.passed() else 1


def flags_of(phase: str) -> str:
    """The RUSTFLAGS a phase runs with, for its receipt entry."""
    return dict(SHAPES).get(phase, CANDIDATE if phase == 'hosted-like' else '')


def record_abort(receipt: Receipt, info: dict, flags: str, start: float, now: float) -> None:
    """The receipt's account of an aborted phase: an entry that failed (a stage abort has none, it is not a shape), and the
    summary the receipt carries at the top."""
    name = info['phase']
    receipt.abort = {'phase': name, 'elapsed_seconds': info['elapsed_seconds'], 'reason': info['reason'],
                     'limit_seconds': info['limit_seconds'], 'stalled_tests': info['stalled_tests'],
                     'remote_processes_ended': (None if info['kill']['survivors'] is None else not info['kill']['survivors']),
                     'evidence': {}}
    if name == 'stage':
        receipt.stage_seconds = round(now - start, 1)
        return
    receipt.shapes.append({'shape': name, 'rustflags': flags, 'exit': None, 'seconds': info['elapsed_seconds'], 'warnings': 0,
                           'attempts': [{'exit': None, 'seconds': info['elapsed_seconds']}], 'log': info['log']['path'],
                           'passed': False, 'aborted': True})


def save_evidence(host: Host, receipt: Receipt, receipt_dir: Path, info: dict) -> None:
    """Writes the evidence beside the receipt and on the host next to the phase's logs, and names the files in the receipt."""
    name = info['phase']
    local, tail = receipt_dir / f'watchdog-{name}.json', receipt_dir / f'watchdog-{name}-log-tail.txt'
    remote = f'{HOST_ROOT}/{receipt.label}/logs/watchdog-{name}.json'
    receipt.abort['evidence'] = {'local': str(local), 'log_tail': str(tail), 'host': remote}
    text = json.dumps(info, indent=2) + '\n'
    local.write_text(text, encoding='utf-8')
    tail.write_text('\n'.join(info['log']['tail']) + '\n', encoding='utf-8')
    host.ssh(f"cat > {remote} <<'WATCHDOG_EVIDENCE'\n{text}WATCHDOG_EVIDENCE\n", check=False)
    print(f'{name}: TIMEOUT ({info["reason"]}) after {info["elapsed_seconds"]}s; {len(info["stalled_tests"])} stalled tests; '
          f'remote processes ended: {receipt.abort["remote_processes_ended"]}; evidence {local}')


class Parser(argparse.ArgumentParser):
    """The parser, with the watchdog's limits worked out from the options as `options.watchdog`."""

    def parse_args(self, args=None, namespace=None):
        options = super().parse_args(args, namespace)
        options.watchdog = watchdog_limits(options)
        return options


USAGE_EXIT_CODES = """exit status: 0 every phase passed; 1 a phase failed; 2 refused (usage); 124 the watchdog aborted a phase
(receipt result TIMEOUT, evidence beside the receipt): a deadline or a stalled log is never a PASS. Defaults (seconds):
deadline stage 3600, ordinary 900, transport 900, hosted-like 2700; no-progress ordinary 600, transport 600, hosted-like 1200."""


def parser() -> argparse.ArgumentParser:
    p = Parser(description=__doc__.splitlines()[0], allow_abbrev=False, epilog=USAGE_EXIT_CODES,
               formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('--label', required=True, help='new host directory name under E:/gwz-tests, with the date')
    p.add_argument('--receipt-dir', required=True, type=Path, help='directory that receives <label>/receipt.json')
    source = p.add_mutually_exclusive_group()
    source.add_argument('--ref', default='HEAD', help='commit of gwz-core to check (default HEAD)')
    source.add_argument('--worktree', action='store_true', help='check the working tree instead of a ref')
    p.add_argument('--tests', action='store_true', help='also check the library in test mode (cfg(test))')
    p.add_argument('--transport-tree', type=Path, metavar='DIR',
                   help='the gwz-transport tree to stage (default: the checkout beside gwz-core); it is never fetched from GitHub')
    p.add_argument(PIN_FLAG, action='store_true', dest='allow_transport_pin_mismatch',
                   help='stage a gwz-transport tree whose HEAD is not gwz-core\'s pin, or that has tracked changes: for a lane '
                        'that moves the pin in this change; the receipt and the output say PIN MISMATCH')
    p.add_argument('--preflight-only', action='store_true',
                   help='judge the label, the options and the gwz-transport tree (pin, tracked changes, .git) and exit 0 or '
                        '2 without touching the host: win-receipt.zsh runs it before it books the host\'s one slot')
    p.add_argument('--cache-from', metavar='LABEL', help='copy an earlier run\'s target directory (a warm run)')
    p.add_argument('--if-triggered', metavar='BASE', help='do nothing unless BASE..ref changed a trigger path')
    p.add_argument('--hosted-like', action='store_true',
                   help='also run the candidate library suites as the hosted job does (short TEMP, portable sshd)')
    p.add_argument('--git-root', default=DEFAULT_GIT_ROOT, metavar='DIR',
                   help='Windows directory of the Git for Windows the hosted-like run uses (default: the host\'s)')
    p.add_argument('--hosted-like-filter', metavar='NAME', help='run only the tests whose path contains NAME')
    p.add_argument('--no-check-shapes', action='store_true', help='with --hosted-like: skip the cargo check shapes')
    p.add_argument('--taut-proto', metavar='RELEASE',
                   help='with --hosted-like: install that taut-proto release in a venv inside the run\'s directory '
                        '(nothing system-wide), so the protocol rows run instead of being named gaps')
    p.add_argument('--deadline', action='append', metavar='PHASE=SECONDS',
                   help='wall-clock deadline of a phase (stage, ordinary, transport, hosted-like); repeatable; 0 = none')
    p.add_argument('--no-progress', action='append', metavar='PHASE=SECONDS',
                   help='abort a phase (ordinary, transport, hosted-like) whose log has not grown for this long; repeatable; 0 = none')
    p.add_argument('--watchdog-poll', type=float, default=DEFAULT_POLL, metavar='SECONDS',
                   help=f'how often the watchdog reads the log size (default {DEFAULT_POLL})')
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

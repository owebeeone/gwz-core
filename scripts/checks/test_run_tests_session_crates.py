"""run_tests.py runs every session crate, and a failure in one group never hides the groups after it.

The hosted checked-artifact-boundary job tests, doctests and clippies each crate under crates/. The session crates are
the ones the core groups do not reach, so run_tests.py runs them itself. Until 2026-10-30 it listed only
gwz-session-host, and `result = result or run_session_crates()` skipped the crates entirely whenever a core group had
failed, so a failing run showed the core failure and hid whatever the session crates had to say.
"""
import contextlib
import importlib.util
import io
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('runner', Path(__file__).resolve().parents[1] / 'run_tests.py')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)

CORE = Path(__file__).resolve().parents[2]
PINNED = 'a' * 40
SESSION_CRATES = ('gwz-session-channel', 'gwz-session-contract', 'gwz-session-host')


def crate_names() -> list[str]:
    """The package name of every crate under crates/, read from its manifest."""
    names = []
    for manifest in sorted((CORE / 'crates').glob('*/Cargo.toml')):
        match = re.search(r'^name\s*=\s*"([^"]+)"', manifest.read_text(encoding='utf-8'), re.MULTILINE)
        names.append(match.group(1))
    return names


class Cargo:
    """A fake subprocess.run: a Cargo command with `-p CRATE` exits with CRATE's code, any other Cargo command with `core`."""

    def __init__(self, crates=None, core=0):
        self.crates, self.core = crates or {}, core
        self.calls = []

    def __call__(self, command, **kwargs):
        self.calls.append([str(part) for part in command])
        if command[0] != 'cargo':
            return subprocess.CompletedProcess(command, 0, stdout='', stderr='')
        if '-p' in command:
            code = self.crates.get(command[command.index('-p') + 1], 0)
        else:
            code = self.core
        return subprocess.CompletedProcess(command, code, stdout='', stderr='')

    def session_runs(self):
        return [call for call in self.calls if call[0] == 'cargo' and '-p' in call]


class SessionCrates(unittest.TestCase):
    def test_the_runner_names_every_session_crate_the_repository_has(self):
        have = sorted(name for name in crate_names() if name.startswith('gwz-session-'))
        self.assertEqual(sorted(runner.SESSION_CRATES), have)
        self.assertEqual(sorted(runner.SESSION_CRATES), list(SESSION_CRATES))

    def test_each_crate_gets_its_tests_and_its_clippy(self):
        cargo = Cargo()
        with patch.object(runner.subprocess, 'run', cargo), contextlib.redirect_stdout(io.StringIO()):
            code = runner.run_session_crates()
        self.assertEqual(code, 0)
        expected = []
        for crate in runner.SESSION_CRATES:
            expected += [
                ['cargo', 'test', '--locked', '-p', crate],
                ['cargo', 'clippy', '--locked', '-p', crate, '--all-targets', '--', '-D', 'warnings'],
            ]
        self.assertEqual(cargo.session_runs(), expected)

    def test_a_failing_crate_does_not_hide_the_crates_after_it(self):
        cargo = Cargo(crates={'gwz-session-channel': 101})
        with patch.object(runner.subprocess, 'run', cargo), contextlib.redirect_stdout(io.StringIO()):
            code = runner.run_session_crates()
        self.assertEqual(code, 101)
        ran = {call[call.index('-p') + 1] for call in cargo.session_runs()}
        self.assertEqual(ran, set(runner.SESSION_CRATES))


class Aggregate(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.workspace = Path(directory.name)
        self.core = self.workspace / 'gwz-core'
        (self.core / '.github').mkdir(parents=True)
        (self.core / '.github' / 'gwz-transport.commit').write_text(PINNED + '\n')
        (self.core / '.github' / 'gwz-git.commit').write_text(PINNED + '\n')
        (self.workspace / 'gwz-transport').mkdir()
        (self.workspace / 'gwz-git').mkdir()

    def run_main(self, cargo, argv=('--no-fail-fast',)):
        """main() with `cargo` as subprocess.run; returns the SystemExit code."""
        with patch.dict(os.environ):
            os.environ.pop(runner.TRANSPORT_CHECKOUT, None)
            os.environ.pop(runner.GIT_CHECKOUT, None)
            with patch.object(runner, 'ROOT', self.core), \
                    patch.object(runner, 'transport_head', return_value=PINNED), \
                    patch.object(runner.subprocess, 'run', cargo), \
                    contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                try:
                    runner.main(['--skip-cfg-siblings', *argv])
                except SystemExit as exit_:
                    return exit_.code
        self.fail('main() returned without exiting')

    def sessions_run(self, cargo):
        return {call[call.index('-p') + 1] for call in cargo.session_runs()}

    def test_every_session_crate_runs_when_everything_passes(self):
        cargo = Cargo()
        self.assertEqual(self.run_main(cargo), 0)
        self.assertEqual(self.sessions_run(cargo), set(SESSION_CRATES))

    def test_a_core_group_failure_does_not_skip_the_session_crates(self):
        cargo = Cargo(core=101)
        self.assertEqual(self.run_main(cargo), 101)
        self.assertEqual(self.sessions_run(cargo), set(SESSION_CRATES))

    def test_a_session_crate_failure_fails_the_run_after_the_core_groups_passed(self):
        cargo = Cargo(crates={'gwz-session-contract': 7})
        self.assertEqual(self.run_main(cargo), 7)
        self.assertEqual(self.sessions_run(cargo), set(SESSION_CRATES))

    def test_a_filtered_run_leaves_the_session_crates_out(self):
        cargo = Cargo()
        self.run_main(cargo, argv=('--no-fail-fast', '--test-filter', 'nothing_matches_this'))
        self.assertEqual(cargo.session_runs(), [])


if __name__ == '__main__':
    unittest.main()

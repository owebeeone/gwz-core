"""The runner starts its source checks side by side and judges them in the order it names them.

Each check is a read-only scan of the sources; run one after another they took 20 s of a warm run on an idle
machine and 122 s of one on a loaded machine (2026-10-25 test-time survey), before Cargo started. Running them at
once changes no outcome: the first check, in the runner's order, that fails is the failure the run reports, with
the output of every check before it printed first, and no Cargo command runs. A step between checks that fails (a
gwz-transport or gwz-git checkout off its pin, or missing) ends the run only when every check started before it
passed, as it did when the checks ran one at a time.
"""
import contextlib
import importlib.util
import io
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('runner', Path(__file__).resolve().parents[1] / 'run_tests.py')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)

PINNED = 'a' * 40
OTHER = 'b' * 40
# How long a check waits for the next one to start before the test calls the checks serial.
OVERLAP_WAIT = 2


def check_name(command):
    """The checker script a command runs, or None for a Cargo or Git command."""
    for part in command:
        if str(part).endswith('.py') and '/scripts/checks/' in str(part).replace('\\', '/'):
            return Path(part).name
    return None


class Checks:
    """A fake subprocess.run: Cargo and Git succeed; each check prints its name and exits with its code.

    It honours `check=True` and an uncaptured stdout as subprocess.run does, so the serial runner passes every
    test here but the first.
    """

    def __init__(self, codes=None, overlap=False):
        self.codes = codes or {}
        self.overlap = overlap
        self.lock = threading.Lock()
        self.running = 0
        self.overlapped = threading.Event()
        self.calls = []

    def __call__(self, command, **kwargs):
        name = check_name(command)
        with self.lock:
            self.calls.append([str(part) for part in command])
            if name is not None:
                self.running += 1
                if self.running > 1:
                    self.overlapped.set()
        try:
            if name is not None and self.overlap:
                # The first check returns only once a second check has started beside it.
                self.overlapped.wait(OVERLAP_WAIT)
            code = self.codes.get(name, 0)
            output = f'{name} output\n' if name is not None else ''
            if kwargs.get('stdout') is None and not kwargs.get('capture_output'):
                # Not captured: the child writes to the runner's own stdout, as a real check does.
                print(output, end='')
                output = None
            if kwargs.get('check') and code:
                raise subprocess.CalledProcessError(code, command)
            return subprocess.CompletedProcess(command, code, stdout=output, stderr='' if output is not None else None)
        finally:
            if name is not None:
                with self.lock:
                    self.running -= 1

    def cargo(self):
        return [command for command in self.calls if command[0] == 'cargo']


class ParallelChecks(unittest.TestCase):
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

    def run_main(self, checks, argv=(), head=PINNED):
        """main() with `checks` as subprocess.run; returns the SystemExit code and stdout."""
        stdout = io.StringIO()
        with patch.dict(os.environ):
            os.environ.pop(runner.TRANSPORT_CHECKOUT, None)
            os.environ.pop(runner.GIT_CHECKOUT, None)
            with patch.object(runner, 'ROOT', self.core), \
                    patch.object(runner, 'transport_head', return_value=head), \
                    patch.object(runner.subprocess, 'run', checks), \
                    contextlib.redirect_stdout(stdout):
                try:
                    runner.main(['--skip-cfg-siblings', *argv])
                except SystemExit as exit_:
                    return exit_.code, stdout.getvalue()
                except subprocess.CalledProcessError as error:
                    return error, stdout.getvalue()
        self.fail('main() returned without exiting')

    def test_a_check_starts_before_the_one_before_it_finishes(self):
        checks = Checks(overlap=True)
        code, _ = self.run_main(checks)
        self.assertEqual(code, 0)
        self.assertTrue(checks.overlapped.is_set(), 'every check ran alone: the checks are serial')

    def test_every_check_runs_and_its_output_is_printed_in_the_runners_order(self):
        checks = Checks()
        code, stdout = self.run_main(checks)
        self.assertEqual(code, 0)
        printed = [line.split()[0] for line in stdout.splitlines() if line.endswith(' output')]
        self.assertEqual(printed, [
            'check_filesystem_boundary.py',
            'check_process_globals.py',
            'check_spawn_gate_minting.py',
            'check_member_step_boundary.py',
            'check_cfg_boundaries.py',
            'check_candidate_switches.py',
            'check_carrier_peer_rule.py',
            'check_driver_only_boundary.py',
            'check_windows_parity.py',
            'check_process_globals.py',
            'check_process_globals.py',
            # After the side-by-side checks, one at a time.
            'check_gwz_git_adoption.py',
            'check_crate_versions.py',
        ])
        self.assertTrue(checks.cargo())

    def test_the_first_failed_check_in_order_is_the_failure_and_no_cargo_runs(self):
        checks = Checks(codes={'check_cfg_boundaries.py': 3, 'check_windows_parity.py': 4})
        error, stdout = self.run_main(checks)
        self.assertIsInstance(error, subprocess.CalledProcessError)
        self.assertEqual(error.returncode, 3)
        self.assertTrue(str(error.cmd[1]).endswith('check_cfg_boundaries.py'), error.cmd)
        printed = [line.split()[0] for line in stdout.splitlines() if line.endswith(' output')]
        self.assertEqual(printed, [
            'check_filesystem_boundary.py', 'check_process_globals.py', 'check_spawn_gate_minting.py',
            'check_member_step_boundary.py', 'check_cfg_boundaries.py',
        ])
        self.assertEqual(checks.cargo(), [])

    def test_a_pin_refusal_ends_the_run_when_every_earlier_check_passed(self):
        checks = Checks()
        code, _ = self.run_main(checks, head=OTHER)
        self.assertIn('--allow-transport-pin-mismatch', str(code))
        self.assertEqual(checks.cargo(), [])

    def test_an_earlier_failed_check_wins_over_a_later_pin_refusal(self):
        checks = Checks(codes={'check_process_globals.py': 5})
        error, _ = self.run_main(checks, head=OTHER)
        self.assertIsInstance(error, subprocess.CalledProcessError)
        self.assertEqual(error.returncode, 5)
        self.assertEqual(checks.cargo(), [])


if __name__ == '__main__':
    unittest.main()

"""The runner refuses a gwz-transport that is not the one CI builds against.

P2-1 of dev-docs/GwzTransportIdleLossDesign-ReviewState.md: a gwz-core commit
used a gwz-transport field that its CI pin (.github/gwz-transport.commit) did
not have, and every local gate passed against a sibling checkout at a newer
commit. The runner now fails when the sibling's HEAD is not the pin, unless
the lane that develops both repositories at once says so out loud with
--allow-transport-pin-mismatch, which prints both commits.
"""
import contextlib
import importlib.util
import io
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('runner', Path(__file__).resolve().parents[1] / 'run_tests.py')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)

PINNED = 'a' * 40
OTHER = 'b' * 40
OVERRIDE = '--allow-transport-pin-mismatch'


def commit_in(path, message='one'):
    """Makes `path` a repository with one commit, and returns its HEAD."""
    env = {**os.environ, 'GIT_AUTHOR_NAME': 't', 'GIT_AUTHOR_EMAIL': 't@t', 'GIT_COMMITTER_NAME': 't', 'GIT_COMMITTER_EMAIL': 't@t'}
    for command in (['init', '-q'], ['commit', '-q', '--allow-empty', '-m', message]):
        subprocess.run(['git', '-C', str(path), *command], check=True, env=env)
    return subprocess.run(['git', '-C', str(path), 'rev-parse', 'HEAD'], check=True, capture_output=True, text=True).stdout.strip()


class TransportPin(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.workspace = Path(directory.name)
        self.core = self.workspace / 'gwz-core'
        (self.core / '.github').mkdir(parents=True)
        self.sibling = self.workspace / 'gwz-transport'
        self.sibling.mkdir()

    def pin(self, text):
        (self.core / '.github' / 'gwz-transport.commit').write_text(text)

    def check(self, allow=False, skip=False):
        stdout = io.StringIO()
        with patch.object(runner, 'ROOT', self.core), contextlib.redirect_stdout(stdout):
            try:
                runner.check_transport_pin(skip, allow)
            except SystemExit as exit_:
                return str(exit_.code), stdout.getvalue()
        return None, stdout.getvalue()

    def test_head_equal_to_the_pin_passes(self):
        head = commit_in(self.sibling)
        self.pin(f'# comment\n{head}\n')
        failure, _ = self.check()
        self.assertIsNone(failure)

    def test_a_crlf_pin_file_reads_the_same(self):
        head = commit_in(self.sibling)
        self.pin(f'# comment\r\n{head}\r\n')
        failure, _ = self.check()
        self.assertIsNone(failure)

    def test_head_that_differs_from_the_pin_fails_and_names_both(self):
        head = commit_in(self.sibling)
        self.pin(PINNED + '\n')
        failure, _ = self.check()
        self.assertIsNotNone(failure)
        self.assertIn(head, failure)
        self.assertIn(PINNED, failure)
        self.assertIn(OVERRIDE, failure)

    def test_the_override_passes_and_prints_both_commits(self):
        head = commit_in(self.sibling)
        self.pin(PINNED + '\n')
        failure, stdout = self.check(allow=True)
        self.assertIsNone(failure)
        self.assertIn('PIN OVERRIDE', stdout)
        self.assertIn(head, stdout)
        self.assertIn(PINNED, stdout)

    def test_the_override_is_silent_when_the_pin_matches(self):
        head = commit_in(self.sibling)
        self.pin(head + '\n')
        failure, stdout = self.check(allow=True)
        self.assertIsNone(failure)
        self.assertNotIn('PIN OVERRIDE', stdout)

    def test_a_pin_file_without_a_commit_fails(self):
        commit_in(self.sibling)
        self.pin('# nothing\n')
        failure, _ = self.check()
        self.assertIsNotNone(failure)

    def test_the_skip_flag_skips_and_says_so(self):
        failure, stdout = self.check(skip=True)
        self.assertIsNone(failure)
        self.assertIn('SKIPPED GATE', stdout)

    def test_the_named_checkout_is_checked_too(self):
        head = commit_in(self.sibling)
        self.pin(head + '\n')
        named = self.workspace / 'elsewhere'
        named.mkdir()
        commit_in(named, 'other')
        with patch.dict(os.environ, {runner.TRANSPORT_CHECKOUT: str(named)}):
            failure, _ = self.check()
        self.assertIsNotNone(failure)
        self.assertIn(str(named), failure)

    def main_status(self, argv, head):
        """main()'s exit status with every subprocess green and gwz-transport at `head`."""
        stdout = io.StringIO()
        with patch.object(runner, 'ROOT', self.core), \
                patch.object(runner, 'transport_head', return_value=head), \
                patch.object(runner.subprocess, 'run') as run, \
                contextlib.redirect_stdout(stdout), \
                self.assertRaises(SystemExit) as raised:
            run.return_value.returncode = 0
            runner.main(argv)
        cargo = [call for call in run.call_args_list if call.args[0][0] == 'cargo']
        return raised.exception.code, stdout.getvalue(), cargo

    def test_main_refuses_before_any_cargo_run(self):
        self.pin(PINNED + '\n')
        code, _, cargo = self.main_status([], OTHER)
        self.assertIn(OVERRIDE, str(code))
        self.assertEqual(cargo, [])

    def test_main_runs_cargo_under_the_override_and_with_a_matching_head(self):
        self.pin(PINNED + '\n')
        for argv, head in (([OVERRIDE], OTHER), ([], PINNED)):
            code, _, cargo = self.main_status(argv, head)
            self.assertEqual(code, 0)
            self.assertTrue(cargo)

    def test_main_does_not_pass_the_override_to_cargo(self):
        self.pin(PINNED + '\n')
        _, _, cargo = self.main_status([OVERRIDE], OTHER)
        self.assertFalse(any(OVERRIDE in call.args[0] for call in cargo))


if __name__ == '__main__':
    unittest.main()

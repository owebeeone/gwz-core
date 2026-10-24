"""The runner checks gwz-git's process globals, at the pin CI builds against, or fails closed.

Step 5.2 of the transport convergence plan moved native commit and tag, with
their Git CLI spawns, from gwz-core to gwz-git, and gwz-core builds against it
by path. gwz-core, the consumer, runs the process-global check over the gwz-git
checkout (GWZ_GIT_CHECKOUT, or the one beside gwz-core) with the allowlist
gwz-git carries (scripts/process_globals_allowlist.json), and refuses a checkout
that is not at .github/gwz-git.commit unless --allow-git-pin-mismatch says a
lane develops both at once. There is no skip flag: every job that builds
gwz-core has gwz-git beside it.
"""
import contextlib
import importlib.util
import io
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('runner', Path(__file__).resolve().parents[1] / 'run_tests.py')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)

NAMED = 'GWZ_GIT_CHECKOUT'
OVERRIDE = '--allow-git-pin-mismatch'
PINNED = 'a' * 40
OTHER = 'b' * 40
ALLOWLIST = Path('scripts') / 'process_globals_allowlist.json'


class GitGlobals(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.workspace = Path(directory.name)
        self.core = self.workspace / 'gwz-core'
        (self.core / '.github').mkdir(parents=True)
        (self.core / '.github' / 'gwz-git.commit').write_text(f'# pin\n{PINNED}\n')
        self.sibling = self.workspace / 'gwz-git'

    def run_check(self, allow=False, environ=None, head=PINNED):
        """check_git_process_globals() with every subprocess green; returns (SystemExit message, stdout, commands)."""
        stdout = io.StringIO()
        message = None
        with patch.dict(os.environ):
            os.environ.pop(NAMED, None)
            os.environ.update(environ or {})
            with patch.object(runner, 'ROOT', self.core), \
                    patch.object(runner, 'transport_head', return_value=head), \
                    patch.object(runner.subprocess, 'run') as run, \
                    contextlib.redirect_stdout(stdout):
                run.return_value.returncode = 0
                try:
                    runner.check_git_process_globals(allow)
                except SystemExit as exit_:
                    message = str(exit_.code)
        commands = [[str(part) for part in call.args[0]] for call in run.call_args_list]
        return message, stdout.getvalue(), commands

    def test_no_checkout_fails_and_names_both_ways(self):
        message, _, commands = self.run_check()
        self.assertIn(NAMED, message)
        self.assertIn(str(self.sibling), message)
        self.assertEqual(commands, [])

    def test_the_sibling_is_checked_with_the_allowlist_it_carries(self):
        self.sibling.mkdir()
        message, _, commands = self.run_check()
        self.assertIsNone(message)
        self.assertEqual(len(commands), 1)
        command = commands[0]
        self.assertEqual(command[command.index('--repo') + 1], str(self.sibling))
        self.assertEqual(command[command.index('--allowlist') + 1], str(self.sibling / ALLOWLIST))

    def test_the_named_checkout_wins_over_the_sibling(self):
        self.sibling.mkdir()
        named = self.workspace / 'elsewhere'
        named.mkdir()
        message, _, commands = self.run_check(environ={NAMED: str(named)})
        self.assertIsNone(message)
        self.assertEqual(commands[0][commands[0].index('--repo') + 1], str(named))

    def test_a_named_checkout_that_is_missing_fails_without_falling_back(self):
        self.sibling.mkdir()
        missing = self.workspace / 'missing'
        message, _, commands = self.run_check(environ={NAMED: str(missing)})
        self.assertIn(str(missing), message)
        self.assertEqual(commands, [])

    def test_a_checkout_off_the_pin_is_refused_before_the_check_runs(self):
        self.sibling.mkdir()
        message, _, commands = self.run_check(head=OTHER)
        self.assertIn(OVERRIDE, message)
        self.assertIn('gwz-git.commit', message)
        self.assertEqual(commands, [])

    def test_the_override_runs_the_check_and_prints_both_commits(self):
        self.sibling.mkdir()
        message, stdout, commands = self.run_check(allow=True, head=OTHER)
        self.assertIsNone(message)
        self.assertIn('PIN OVERRIDE', stdout)
        self.assertIn(OTHER, stdout)
        self.assertIn(PINNED, stdout)
        self.assertEqual(len(commands), 1)

    def test_a_missing_pin_file_fails(self):
        self.sibling.mkdir()
        (self.core / '.github' / 'gwz-git.commit').unlink()
        message, _, commands = self.run_check()
        self.assertIn('gwz-git.commit', message)
        self.assertEqual(commands, [])

    def test_main_runs_the_check_and_does_not_pass_the_override_to_cargo(self):
        self.sibling.mkdir()
        (self.core / '.github' / 'gwz-transport.commit').write_text(PINNED + '\n')
        transport = self.workspace / 'gwz-transport'
        transport.mkdir()
        stdout = io.StringIO()
        with patch.object(runner, 'ROOT', self.core), \
                patch.object(runner, 'transport_head', return_value=OTHER), \
                patch.object(runner.subprocess, 'run') as run, \
                contextlib.redirect_stdout(stdout), \
                self.assertRaises(SystemExit) as raised:
            run.return_value.returncode = 0
            runner.main([OVERRIDE, '--allow-transport-pin-mismatch'])
        self.assertEqual(raised.exception.code, 0)
        commands = [[str(part) for part in call.args[0]] for call in run.call_args_list]
        self.assertTrue(any(str(self.sibling) in command for command in commands), commands)
        self.assertFalse(any(OVERRIDE in command for command in commands if command[0] == 'cargo'))

    def test_the_adoption_check_runs_over_the_same_checkout_and_main_runs_it_after_the_globals(self):
        # Step 5.5: every public gwz-git capability has a gwz-core production caller.
        self.sibling.mkdir()
        with patch.object(runner, 'ROOT', self.core), patch.object(runner.subprocess, 'run') as run:
            run.return_value.returncode = 0
            runner.check_git_adoption()
        (command,) = [[str(part) for part in call.args[0]] for call in run.call_args_list]
        self.assertTrue(command[1].endswith('check_gwz_git_adoption.py'), command)
        self.assertEqual(command[command.index('--gwz-git') + 1], str(self.sibling))
        source = Path(runner.__file__).read_text(encoding='utf-8')
        main = source[source.index('def main('):]
        self.assertLess(main.index('check_git_process_globals('), main.index('check_git_adoption()'))
        self.assertTrue(run.call_args.kwargs['check'])


if __name__ == '__main__':
    unittest.main()

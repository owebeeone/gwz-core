"""The runner checks gwz-transport's process globals or fails closed.

B11 of the core session contract's Verdict-3 (GwzCoreSessionDesign §5.7): the
checkout comes from GWZ_TRANSPORT_CHECKOUT, or else from beside gwz-core.
With neither, the run fails and names both ways. Only a CI job that has no
gwz-transport checkout passes --skip-transport-globals, which prints
SKIPPED GATE with the reason instead of passing silently.
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

NAMED = 'GWZ_TRANSPORT_CHECKOUT'
TRANSPORT_ALLOWLIST = 'process_globals_allowlist_gwz_transport.json'


def exit_status(code):
    """The process exit status Python gives a SystemExit code."""
    if code is None:
        return 0
    if isinstance(code, int):
        return code
    return 1


class TransportGlobals(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.workspace = Path(directory.name)
        self.core = self.workspace / 'gwz-core'
        (self.core / '.github').mkdir(parents=True)
        (self.core / '.github' / 'gwz-transport.commit').write_text('a' * 40 + '\n')
        self.sibling = self.workspace / 'gwz-transport'

    def run_main(self, argv, environ):
        """main() in a workspace without the real sibling, every subprocess green.

        Returns the exit status, the SystemExit message, stdout and the commands run.
        """
        stdout = io.StringIO()
        with patch.dict(os.environ):
            os.environ.pop(NAMED, None)
            os.environ.update(environ)
            with patch.object(runner, 'ROOT', self.core), \
                    patch.object(runner, 'transport_head', return_value='a' * 40), \
                    patch.object(runner.subprocess, 'run') as run, \
                    contextlib.redirect_stdout(stdout):
                run.return_value.returncode = 0
                with self.assertRaises(SystemExit) as raised:
                    runner.main(argv)
        commands = [[str(part) for part in call.args[0]] for call in run.call_args_list]
        code = raised.exception.code
        return exit_status(code), code if isinstance(code, str) else '', stdout.getvalue(), commands

    def transport_repos(self, commands):
        """The --repo of every checker run that uses the gwz-transport allowlist."""
        return [command[command.index('--repo') + 1] for command in commands
                if '--allowlist' in command and command[command.index('--allowlist') + 1].endswith(TRANSPORT_ALLOWLIST)]

    def test_no_checkout_fails_and_names_both_ways(self):
        status, message, _, commands = self.run_main([], {})
        self.assertNotEqual(status, 0)
        self.assertIn(NAMED, message)
        self.assertIn(str(self.sibling), message)
        self.assertEqual(self.transport_repos(commands), [])
        self.assertFalse(any(command[0] == 'cargo' for command in commands), commands)

    def test_skip_flag_exits_zero_and_prints_skipped_gate(self):
        status, _, stdout, commands = self.run_main(['--skip-transport-globals'], {})
        self.assertEqual(status, 0)
        self.assertIn('SKIPPED GATE', stdout)
        self.assertIn('--skip-transport-globals', stdout)
        self.assertEqual(self.transport_repos(commands), [])
        self.assertFalse(any('--skip-transport-globals' in command for command in commands), commands)

    def test_named_checkout_is_honoured_before_the_sibling(self):
        named = self.workspace / 'elsewhere'
        named.mkdir()
        self.sibling.mkdir()
        status, _, _, commands = self.run_main([], {NAMED: str(named)})
        self.assertEqual(status, 0)
        self.assertEqual(self.transport_repos(commands), [str(named)])

    def test_sibling_is_checked_when_nothing_is_named(self):
        self.sibling.mkdir()
        status, _, _, commands = self.run_main([], {})
        self.assertEqual(status, 0)
        self.assertEqual(self.transport_repos(commands), [str(self.sibling)])

    def test_named_checkout_that_is_missing_fails_without_falling_back(self):
        missing = self.workspace / 'missing'
        self.sibling.mkdir()
        status, message, _, commands = self.run_main([], {NAMED: str(missing)})
        self.assertNotEqual(status, 0)
        self.assertIn(str(missing), message)
        self.assertEqual(self.transport_repos(commands), [])

    def test_an_abbreviated_flag_does_not_skip(self):
        # argparse would otherwise take any unique prefix, such as --skip, as
        # the skip flag; only the full flag may turn the gate off.
        status, message, stdout, _ = self.run_main(['--skip'], {})
        self.assertNotEqual(status, 0)
        self.assertNotIn('SKIPPED GATE', stdout)
        self.assertIn(NAMED, message)


if __name__ == '__main__':
    unittest.main()

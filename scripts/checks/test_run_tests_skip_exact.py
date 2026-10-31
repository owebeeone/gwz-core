"""--skip-exact NAME leaves one named test out of the libtest runs, for a CI step that runs it elsewhere with the privilege it needs.

The sandbox rule's mount-namespace and pid-reuse rows (plan 2.3a) need root; the unprivileged suites step skips exactly
those two by full path, a separate `sudo -E` step runs them, and the rows themselves still fail loudly without root.
A name is checked against the test binaries' `--list` before any test runs: it must equal exactly one listed test and be
part of no other, so the option cannot leave a module out (remediation round 2: Code C2-3, State N-5).
"""
import importlib.util
import io
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('runner', Path(__file__).resolve().parents[1] / 'run_tests.py')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)

MOUNT = 'transport_host::carrier::tests::sandbox_rows::a_process_in_another_mount_namespace_is_refused_as_client_and_listener'
REUSE = 'transport_host::carrier::tests::hold_rows::a_reused_process_id_does_not_stand_for_the_process_that_connected_or_listened'
OTHER = 'transport_host::carrier::tests::sandbox_rows::a_plain_client_is_accepted_and_a_plain_listener_is_reached'
LISTING = f'{MOUNT}: test\n{REUSE}: test\n{OTHER}: test\nunrelated::tests::one: test\nunrelated::tests::one_more: test\n'


class Cargo:
    def __init__(self, listing: str = ''):
        self.listing, self.calls = listing, []

    def __call__(self, command, **kwargs):
        self.calls.append(command)
        stdout = self.listing if '--list' in command and '--ignored' not in command else ''
        return subprocess.CompletedProcess(command, 0, stdout=stdout)

    def runs(self):
        return [command for command in self.calls if '--list' not in command and command[:2] == ['cargo', 'test']]


class SkipExact(unittest.TestCase):
    def test_each_name_becomes_a_libtest_skip_after_the_groups_own_arguments(self):
        cargo = Cargo()
        with patch.object(runner.subprocess, 'run', cargo):
            code = runner.run('real', ['--lib'], ['--skip', 'matrix::'], skip_exact=(MOUNT, REUSE))
        self.assertEqual(code, 0)
        [command] = cargo.runs()
        self.assertEqual(command[command.index('--'):], ['--', '--skip', 'matrix::', '--skip', MOUNT, '--skip', REUSE])

    def test_without_the_option_the_command_is_what_it_was(self):
        cargo = Cargo()
        with patch.object(runner.subprocess, 'run', cargo):
            runner.run('real', ['--lib'], ['--skip', 'matrix::'])
        [command] = cargo.runs()
        self.assertEqual(command[command.index('--'):], ['--', '--skip', 'matrix::'])

    def test_a_filtered_run_leaves_the_named_test_out_of_its_exact_names(self):
        other = 'transport_host::carrier::tests::sandbox_rows::a_plain_client_is_accepted_and_a_plain_listener_is_reached'
        cargo = Cargo(listing=f'{MOUNT}: test\n{other}: test\n')
        with patch.object(runner.subprocess, 'run', cargo):
            runner.run('real', ['--lib'], [], test_filter='sandbox_rows', skip_exact=(MOUNT,))
        [command] = cargo.runs()
        self.assertEqual(command[command.index('--'):], ['--', '--exact', other])

    def test_only_a_full_test_path_is_accepted(self):
        for bad in ('sandbox_rows', 'a_process_in_another_mount_namespace', 'two words::here', ''):
            with self.subTest(bad=bad), self.assertRaises(Exception):
                runner.skip_exact_name(bad)
        self.assertEqual(runner.skip_exact_name(MOUNT), MOUNT)

    def test_a_name_that_is_not_exactly_one_listed_test_is_refused_before_any_test_runs(self):
        refused = (
            '::', 'a::', 'transport_host::', 'transport_host::carrier',
            'transport_host::carrier::tests::sandbox_rows',  # a part of listed tests, not one of them
            'transport_host::carrier::tests::sandbox_rows::no_such_test',  # matches no listed test
            'unrelated::tests::one',  # one listed test, and a part of another
        )
        for name in refused:
            with self.subTest(name=name):
                cargo = Cargo(listing=LISTING)
                with patch.object(runner.subprocess, 'run', cargo), patch('sys.stdout', new_callable=io.StringIO) as out:
                    with self.assertRaises(SystemExit) as raised:
                        runner.check_skip_exact([name], ['--lib'])
                self.assertNotEqual(raised.exception.code, 0)
                self.assertIn('--skip-exact', out.getvalue())
                self.assertEqual(cargo.runs(), [], 'nothing ran')

    def test_a_name_listed_twice_is_refused(self):
        cargo = Cargo(listing=f'{MOUNT}: test\n{MOUNT}: test\n')
        with patch.object(runner.subprocess, 'run', cargo), patch('sys.stdout', new_callable=io.StringIO):
            with self.assertRaises(SystemExit):
                runner.check_skip_exact([MOUNT], ['--lib'])

    def test_a_full_single_test_name_is_accepted_and_the_listing_uses_the_runs_cargo_arguments(self):
        cargo = Cargo(listing=LISTING)
        with patch.object(runner.subprocess, 'run', cargo):
            runner.check_skip_exact([MOUNT, REUSE, 'unrelated::tests::one_more'], ['--no-fail-fast', '--lib'])
        listing = [command for command in cargo.calls if '--list' in command]
        self.assertEqual(len(listing), 1)
        self.assertEqual(listing[0][:4], ['cargo', 'test', '--locked', '--no-fail-fast'])
        self.assertEqual(cargo.runs(), [])

    def test_without_names_nothing_is_listed(self):
        cargo = Cargo(listing=LISTING)
        with patch.object(runner.subprocess, 'run', cargo):
            runner.check_skip_exact([], ['--lib'])
        self.assertEqual(cargo.calls, [])

    def test_a_listing_that_fails_refuses_the_run(self):
        def failing(command, **kwargs):
            return subprocess.CompletedProcess(command, 101, stdout='')
        with patch.object(runner.subprocess, 'run', failing), patch('sys.stdout', new_callable=io.StringIO):
            with self.assertRaises(SystemExit) as raised:
                runner.check_skip_exact([MOUNT], ['--lib'])
        self.assertEqual(raised.exception.code, 101)

    def test_main_refuses_a_name_that_skips_a_module_before_running_any_test(self):
        cargo = Cargo(listing=LISTING)
        with patch.object(runner.subprocess, 'run', cargo), \
                patch.object(runner, 'check_cfg_boundaries'), \
                patch.object(runner, 'check_transport_process_globals'), \
                patch.object(runner, 'check_transport_pin'), \
                patch.object(runner, 'check_git_process_globals'), \
                patch.object(runner, 'check_git_adoption'), \
                patch.object(runner, 'run_session_crates', return_value=0), \
                patch('sys.stdout', new_callable=io.StringIO) as out:
            with self.assertRaises(SystemExit) as raised:
                runner.main(['--no-fail-fast', '--skip-exact', 'transport_host::carrier'])
        self.assertNotEqual(raised.exception.code, 0)
        self.assertEqual(cargo.runs(), [], 'no test ran')
        self.assertNotIn('SKIPPED TEST', out.getvalue())

    def test_main_prints_each_skipped_name_with_its_reason_and_passes_it_to_libtest(self):
        cargo = Cargo(listing=LISTING)
        with patch.object(runner.subprocess, 'run', cargo), \
                patch.object(runner, 'check_cfg_boundaries'), \
                patch.object(runner, 'check_transport_process_globals'), \
                patch.object(runner, 'check_transport_pin'), \
                patch.object(runner, 'check_git_process_globals'), \
                patch.object(runner, 'check_git_adoption'), \
                patch.object(runner, 'run_session_crates', return_value=0), \
                patch('sys.stdout', new_callable=io.StringIO) as out:
            with self.assertRaises(SystemExit):
                runner.main(['--no-fail-fast', '--skip-exact', MOUNT, '--skip-exact', REUSE])
        text = out.getvalue()
        for name in (MOUNT, REUSE):
            self.assertIn(f'SKIPPED TEST {name}: run in the privileged step', text)
        real = [c for c in cargo.runs() if '--skip' in c and MOUNT in c]
        self.assertTrue(real, 'the real-Git run carries the skips')
        for command in cargo.runs():
            after = command[command.index('--') + 1:] if '--' in command else []
            self.assertEqual(after.count(MOUNT), 1 if '--skip' in after else 0, command)


if __name__ == '__main__':
    unittest.main()

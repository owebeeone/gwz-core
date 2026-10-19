"""--test-filter narrows each group to its own tests; it never adds another group's tests to it.

A bare test name among the Cargo arguments is a second libtest filter, and libtest ORs filters: the fake-filesystem
groups then also ran every matching test written for the real filesystem, which fails there with
"open ambient artifact root: entity not found" on every host (2026-10-14-fixc, receipts -d and -f).
"""
import importlib.util
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('runner', Path(__file__).resolve().parents[1] / 'run_tests.py')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)

LISTING = '\n'.join([
    'filesystem::filesystem_contract_tests::cloned_directory_retains_the_same_namespace: test',
    'checked_artifact::tests::recovery_protocol::source_equals_goal_is_proof_only_and_preserves_identity: test',
    'checked_artifact::tests::recovery_protocol::every_non_cleanup_fault_restarts_to_one_exact_goal: test',
    'a_benchmark_that_matches_recovery_protocol: benchmark',
    '',
])


class Cargo:
    """Answers a `--list` with the listing it is given and every other cargo command with `code`."""

    def __init__(self, listing: str = LISTING, code: int = 0, list_code: int = 0, codes: list[int] | None = None):
        self.listing, self.code, self.list_code = listing, code, list_code
        self.codes = list(codes) if codes is not None else None
        self.calls = []

    def __call__(self, command, **kwargs):
        self.calls.append((command, kwargs))
        listed = '--list' in command
        stdout = self.listing if listed else ''
        if listed:
            code = self.list_code
        elif self.codes is not None and command[:2] == ['cargo', 'test']:
            code = self.codes.pop(0)
        else:
            code = self.code
        return subprocess.CompletedProcess(command, code, stdout=stdout)

    def runs(self):
        return [command for command, _ in self.calls if '--list' not in command]

    def lists(self):
        return [command for command, _ in self.calls if '--list' in command]


class TestFilter(unittest.TestCase):
    def test_the_filter_selects_exact_names_from_the_group_listing(self):
        cargo = Cargo()
        with patch.object(runner.subprocess, 'run', cargo):
            code = runner.run('fake', ['--lib'], ['filesystem::'], filesystem='fake', test_filter='recovery_protocol')
        self.assertEqual(code, 0)
        [listing] = cargo.lists()
        self.assertEqual(listing[listing.index('--'):], ['--', 'filesystem::', '--list', '--format', 'terse'])
        [command] = cargo.runs()
        self.assertEqual(command[command.index('--'):], [
            '--', '--exact',
            'checked_artifact::tests::recovery_protocol::source_equals_goal_is_proof_only_and_preserves_identity',
            'checked_artifact::tests::recovery_protocol::every_non_cleanup_fault_restarts_to_one_exact_goal',
        ])
        self.assertNotIn('recovery_protocol', command[:command.index('--')])
        self.assertEqual(cargo.calls[0][1]['env']['GWZ_TEST_FS'], 'fake')
        self.assertEqual(cargo.calls[-1][1]['env']['GWZ_TEST_FS'], 'fake')

    def test_a_group_with_no_matching_test_is_not_run(self):
        cargo = Cargo()
        with patch.object(runner.subprocess, 'run', cargo):
            code = runner.run('real', ['--lib'], ['real_git::'], test_filter='no_such_test')
        self.assertEqual(code, 0)
        self.assertEqual(cargo.runs(), [])

    def test_a_listing_that_fails_is_the_group_result(self):
        cargo = Cargo(list_code=101)
        with patch.object(runner.subprocess, 'run', cargo):
            code = runner.run('real', ['--lib'], [], test_filter='recovery_protocol')
        self.assertEqual(code, 101)
        self.assertEqual(cargo.runs(), [])

    def test_a_long_selection_runs_in_batches_that_fit_a_windows_command_line(self):
        names = [f'module_{i:04}::recovery_protocol::{"x" * 80}' for i in range(600)]
        cargo = Cargo(listing=''.join(f'{name}: test\n' for name in names), code=0)
        with patch.object(runner.subprocess, 'run', cargo):
            code = runner.run('real', ['--lib'], [], test_filter='recovery_protocol')
        self.assertEqual(code, 0)
        runs = cargo.runs()
        self.assertGreater(len(runs), 1)
        ran = [name for command in runs for name in command[command.index('--exact') + 1:]]
        self.assertEqual(ran, names)
        for command in runs:
            self.assertLessEqual(len(subprocess.list2cmdline(command)), runner.EXACT_COMMAND_BUDGET + 1000)

    def test_a_failed_batch_fails_the_group(self):
        names = [f'module_{i:04}::recovery_protocol::{"x" * 80}' for i in range(600)]
        cargo = Cargo(listing=''.join(f'{name}: test\n' for name in names), code=101)
        with patch.object(runner.subprocess, 'run', cargo):
            self.assertEqual(runner.run('real', ['--lib'], [], test_filter='recovery_protocol'), 101)

    def test_any_failed_batch_fails_the_group_whichever_batch_it_is(self):
        names = [f'module_{i:04}::recovery_protocol::{"x" * 80}' for i in range(600)]
        listing = ''.join(f'{name}: test\n' for name in names)
        for codes in ([101, 0, 0], [0, 101, 0], [0, 0, 101]):
            cargo = Cargo(listing=listing, codes=codes)
            with patch.object(runner.subprocess, 'run', cargo):
                self.assertEqual(runner.run('real', ['--lib'], [], test_filter='recovery_protocol'), 101, codes)
            self.assertEqual(len(cargo.runs()), 3, 'the selection is three batches')

    def test_a_filter_that_matches_no_test_in_any_group_fails_the_run(self):
        cargo = Cargo()
        with patch.object(runner.subprocess, 'run', cargo), \
                patch.object(runner, 'check_cfg_boundaries'), \
                patch.object(runner, 'check_transport_process_globals'), \
                patch.object(runner, 'check_transport_pin'):
            with self.assertRaises(SystemExit) as exit:
                runner.main(['--no-fail-fast', '--lib', '--test-filter', 'no_such_test'])
        self.assertNotEqual(exit.exception.code, 0)
        self.assertEqual(len(cargo.lists()), 4)
        self.assertEqual([command for command in cargo.runs() if command[:2] == ['cargo', 'test']], [])

    def test_main_takes_the_filter_and_never_hands_cargo_a_bare_test_name(self):
        cargo = Cargo()
        with patch.object(runner.subprocess, 'run', cargo), \
                patch.object(runner, 'check_cfg_boundaries'), \
                patch.object(runner, 'check_transport_process_globals'), \
                patch.object(runner, 'check_transport_pin'):
            with self.assertRaises(SystemExit) as exit:
                runner.main(['--no-fail-fast', '--lib', '--test-filter', 'recovery_protocol'])
        self.assertEqual(exit.exception.code, 0)
        cargo_commands = [command for command, _ in cargo.calls if command[:2] == ['cargo', 'test']]
        self.assertEqual(len(cargo.lists()), 4)
        for command in cargo_commands:
            self.assertNotIn('recovery_protocol', command[:command.index('--')])
            self.assertNotIn('--test-filter', command)


if __name__ == '__main__':
    unittest.main()

"""Tests for the lane gate's Windows-receipt rule (P3-3 of the Windows parity Phase 0 skim)."""
import contextlib
import importlib.util
import io
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('windows_receipts', Path(__file__).with_name('check_windows_receipts.py'))
receipts = importlib.util.module_from_spec(spec)
spec.loader.exec_module(receipts)

TRIGGER = 'src/git/endpoint/ssh_network.rs'
OTHER = 'README.md'


def commit(sha, message='A change', *paths):
    return receipts.Commit(sha, message, list(paths))


class Parse(unittest.TestCase):
    def test_a_dated_label_is_a_receipt(self):
        self.assertEqual(receipts.parse('Fix\n\nBody.\n\nWindows-receipt: winp1-20261010-a\n'),
                         receipts.Receipt('winp1-20261010-a', None, None))

    def test_ci_only_needs_a_reason(self):
        self.assertEqual(receipts.parse('Fix\n\nWindows-receipt: ci-only the pin moves a sibling the lane does not build\n'),
                         receipts.Receipt(None, 'the pin moves a sibling the lane does not build', None))
        self.assertIn('reason', receipts.parse('Fix\n\nWindows-receipt: ci-only\n').error)

    def test_a_message_without_the_line_has_no_receipt(self):
        self.assertIsNone(receipts.parse('Fix\n\nThe Windows-receipt: is mentioned mid-line.\n'))

    def test_a_malformed_value_is_an_error_not_a_receipt(self):
        for value in ('winp1', 'Winp1-20261010', 'none', 'skip because', '<label>', 'a b-20261010'):
            parsed = receipts.parse(f'Fix\n\nWindows-receipt: {value}\n')
            self.assertIsNotNone(parsed.error, value)
            self.assertIsNone(parsed.label, value)

    def test_the_line_may_sit_anywhere_in_the_message_and_carry_trailing_spaces(self):
        self.assertEqual(receipts.parse('Windows-receipt: a-20261010  \n\nBody\n').label, 'a-20261010')


class Gate(unittest.TestCase):
    def run_gate(self, *commits):
        return receipts.gate(list(commits))

    def test_no_trigger_path_needs_no_receipt(self):
        errors, notes = self.run_gate(commit('a1', 'Docs', OTHER), commit('a2', 'More docs', OTHER, 'src/git/mod.rs'))
        self.assertEqual((errors, notes), ([], []))

    def test_a_trigger_path_with_no_receipt_fails_and_names_the_commit_and_path(self):
        errors, _ = self.run_gate(commit('a1', 'Port', TRIGGER, OTHER))
        self.assertEqual(len(errors), 1)
        self.assertIn('a1', errors[0])
        self.assertIn(TRIGGER, errors[0])
        self.assertIn('Windows-receipt:', errors[0])

    def test_a_receipt_on_the_commit_or_a_later_one_covers_it(self):
        errors, _ = self.run_gate(commit('a1', 'Port', TRIGGER), commit('a2', 'Port more', TRIGGER),
                                  commit('a3', 'Receipt\n\nWindows-receipt: winp1-20261010-a', OTHER))
        self.assertEqual(errors, [])
        errors, _ = self.run_gate(commit('a1', 'Port\n\nWindows-receipt: winp1-20261010-a', TRIGGER))
        self.assertEqual(errors, [])

    def test_a_receipt_does_not_cover_a_later_trigger_change(self):
        errors, _ = self.run_gate(commit('a1', 'Port\n\nWindows-receipt: winp1-20261010-a', TRIGGER),
                                  commit('a2', 'Port more', 'Cargo.lock'))
        self.assertEqual(len(errors), 1)
        self.assertIn('a2', errors[0])

    def test_every_uncovered_commit_is_named(self):
        errors, _ = self.run_gate(commit('a1', 'Port', TRIGGER), commit('a2', 'Port more', 'Cargo.lock'))
        self.assertEqual(len(errors), 2)

    def test_ci_only_waives_the_gate_and_is_noted_for_the_reviewer(self):
        errors, notes = self.run_gate(commit('a1', 'Pin\n\nWindows-receipt: ci-only sibling pin, no gwz-core source', '.github/gwz-sspi.commit'))
        self.assertEqual(errors, [])
        self.assertEqual(len(notes), 1)
        self.assertIn('ci-only', notes[0])
        self.assertIn('sibling pin', notes[0])

    def test_a_later_ci_only_waiver_does_not_cover_an_earlier_trigger_change(self):
        # The adaptive lane's round-2 review commit (documentation only, ci-only) passed the gate for the
        # code commits before it; a waiver speaks for its own commit only.
        errors, notes = self.run_gate(commit('a1', 'Fix', TRIGGER),
                                      commit('a2', 'Review\n\nWindows-receipt: ci-only documentation only', OTHER))
        self.assertEqual(len(errors), 1)
        self.assertIn('a1', errors[0])
        self.assertEqual(len(notes), 1)

    def test_a_label_after_a_ci_only_waiver_still_covers_the_earlier_change(self):
        errors, _ = self.run_gate(commit('a1', 'Fix', TRIGGER),
                                  commit('a2', 'Review\n\nWindows-receipt: ci-only documentation only', OTHER),
                                  commit('a3', 'Merge\n\nWindows-receipt: adaptive-20261011-b', OTHER))
        self.assertEqual(errors, [])

    def test_a_malformed_receipt_fails_even_without_a_trigger_path(self):
        errors, _ = self.run_gate(commit('a1', 'Docs\n\nWindows-receipt: later', OTHER))
        self.assertEqual(len(errors), 1)
        self.assertIn('a1', errors[0])

    def test_a_malformed_receipt_does_not_cover_a_trigger_change(self):
        errors, _ = self.run_gate(commit('a1', 'Port', TRIGGER), commit('a2', 'Receipt\n\nWindows-receipt: nope', OTHER))
        self.assertEqual(len(errors), 2)


class Wiring(unittest.TestCase):
    def test_the_lane_gate_runs_the_receipt_check_over_the_range_with_its_floor(self):
        gate_script = (Path(__file__).with_name('check_lane_commits.sh')).read_text(encoding='utf-8')
        self.assertIn('check_windows_receipts.py', gate_script)
        self.assertIn('--floor "${floor}"', gate_script)

    def test_the_ci_job_runs_these_tests(self):
        workflow = (Path(__file__).resolve().parents[2] / '.github' / 'workflows' / 'checked-artifact-boundary.yml')
        self.assertIn('test_check_windows_receipts.py', workflow.read_text(encoding='utf-8'))


def git(repo, *args, env=None):
    return subprocess.run(['git', '-C', str(repo), *args], check=True, capture_output=True, text=True,
                          env={**os.environ, 'GIT_AUTHOR_NAME': 't', 'GIT_AUTHOR_EMAIL': 't@t', 'GIT_COMMITTER_NAME': 't',
                               'GIT_COMMITTER_EMAIL': 't@t', **(env or {})}).stdout


class FromGit(unittest.TestCase):
    """The glue against a real scratch repository, outside the workspace."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = Path(self.tmp.name)
        git(self.repo, 'init', '-q', '-b', 'main')
        self.make('README.md', 'base', 'Base')
        self.base = git(self.repo, 'rev-parse', 'HEAD').strip()

    def tearDown(self):
        self.tmp.cleanup()

    def make(self, path, text, message):
        target = self.repo / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)
        git(self.repo, 'add', '-A')
        git(self.repo, 'commit', '-q', '-m', message)

    def head(self):
        return git(self.repo, 'rev-parse', 'HEAD').strip()

    def main(self, *argv):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = receipts.main([*argv], cwd=self.repo)
        return code, out.getvalue(), err.getvalue()

    def test_reads_messages_and_changed_paths_oldest_first(self):
        self.make(TRIGGER, 'x', 'Port\n\nBody line')
        self.make(OTHER, 'y', 'Docs')
        commits = receipts.commits_between(self.repo, self.base, self.head(), None)
        self.assertEqual([(c.message.splitlines()[0], c.paths) for c in commits], [('Port', [TRIGGER]), ('Docs', [OTHER])])

    def test_main_fails_without_a_receipt_and_passes_with_one(self):
        self.make(TRIGGER, 'x', 'Port')
        code, _, err = self.main(self.base, self.head())
        self.assertEqual(code, 1)
        self.assertIn(TRIGGER, err)
        self.make(OTHER, 'y', 'Receipt\n\nWindows-receipt: winp1-20261010-a')
        self.assertEqual(self.main(self.base, self.head())[0], 0)

    def test_a_merge_of_the_base_brings_no_trigger_change_into_the_lane(self):
        git(self.repo, 'checkout', '-q', '-b', 'lane')
        self.make(OTHER, 'lane', 'Lane docs')
        git(self.repo, 'checkout', '-q', 'main')
        self.make(TRIGGER, 'main', 'Main port')
        main_head = self.head()
        git(self.repo, 'checkout', '-q', 'lane')
        git(self.repo, 'merge', '-q', '--no-ff', '-m', 'Merge main', 'main')
        code, out, err = self.main(main_head, self.head())
        self.assertEqual((code, err), (0, ''))

    def test_the_floor_excludes_older_commits(self):
        self.make(TRIGGER, 'x', 'Old port')
        floor = self.head()
        self.make(OTHER, 'y', 'Docs')
        self.assertEqual(self.main(self.base, self.head(), '--floor', floor)[0], 0)


if __name__ == '__main__':
    unittest.main()

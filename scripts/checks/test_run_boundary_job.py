"""The local mirror of the boundary job runs the workflow's own commands (run_boundary_job.py)."""
import contextlib
import importlib.util
import io
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('mirror', Path(__file__).resolve().with_name('run_boundary_job.py'))
mirror = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mirror)

WORKFLOW = """\
    steps:
      - name: First
        run: |
          # a comment
          python a.py

          python -m unittest x.py y.py
      - name: Single
        run: python b.py
      - name: Last
        run: |
          python c.py
"""


class StepCommands(unittest.TestCase):
    def test_a_block_gives_its_commands_without_comments_or_blank_lines(self):
        self.assertEqual(mirror.step_commands(WORKFLOW, 'First'), ['python a.py', 'python -m unittest x.py y.py'])

    def test_a_one_line_run_and_the_last_step_are_read(self):
        self.assertEqual(mirror.step_commands(WORKFLOW, 'Single'), ['python b.py'])
        self.assertEqual(mirror.step_commands(WORKFLOW, 'Last'), ['python c.py'])

    def test_a_missing_step_is_refused(self):
        with self.assertRaises(SystemExit):
            mirror.step_commands(WORKFLOW, 'Absent')

    def test_the_real_workflow_names_the_unit_tests_of_the_checks(self):
        text = mirror.WORKFLOW.read_text(encoding='utf-8')
        commands = [c for step in mirror.STEPS for c in mirror.step_commands(text, step)]
        joined = '\n'.join(commands)
        # The module whose length pin went stale unseen, 2026-10-11.
        self.assertIn('scripts/checks/test_check_process_globals.py', joined)
        self.assertIn('scripts/checks/check_checked_artifact_boundaries.py', joined)
        self.assertTrue(any('cargo clippy' in c for c in commands))


LOOP = """\
    steps:
      - name: Library
        run: |
          names="$(python list.py)"
          test -n "$names"
          for name in $names; do
            cargo test -p "$name" --lib --locked
            # a comment inside the loop
            cargo clippy -p "$name" --all-targets --locked -- -D warnings
          done
      - name: After
        run: python after.py
"""


class LibraryStep(unittest.TestCase):
    """The workflow's per-crate loop (tests, doctests, clippy) is one script: its lines mean nothing apart."""

    def test_a_block_is_one_script_with_its_loop_whole(self):
        script = mirror.step_script(LOOP, 'Library')
        self.assertEqual(script.splitlines(), [
            'names="$(python list.py)"',
            'test -n "$names"',
            'for name in $names; do',
            'cargo test -p "$name" --lib --locked',
            'cargo clippy -p "$name" --all-targets --locked -- -D warnings',
            'done',
        ])

    def test_without_clippy_the_script_keeps_its_loop_and_names_the_skipped_line(self):
        script, skipped = mirror.without_clippy(mirror.step_script(LOOP, 'Library'))
        self.assertEqual(skipped, ['cargo clippy -p "$name" --all-targets --locked -- -D warnings'])
        self.assertNotIn('clippy', script)
        self.assertIn('cargo test -p "$name" --lib --locked', script)
        self.assertTrue(script.rstrip().endswith('done'))

    def test_the_real_workflow_runs_every_crate_through_tests_doctests_and_clippy(self):
        text = mirror.WORKFLOW.read_text(encoding='utf-8')
        script = mirror.step_script(text, mirror.LIBRARY_STEP)
        lines = script.splitlines()
        # The package list comes from the boundary checker, so a crate added to the inventory is covered unedited.
        self.assertIn('check_local_clone_boundaries.py --list-present', script)
        for command in ('cargo test -p "$name" --lib --locked', 'cargo test -p "$name" --doc --locked',
                        'cargo clippy -p "$name" --all-targets --locked -- -D warnings'):
            self.assertIn(command, lines)
        self.assertEqual(lines[-1], 'done')

    def test_the_real_workflow_script_is_valid_shell(self):
        text = mirror.WORKFLOW.read_text(encoding='utf-8')
        checked = subprocess.run(['bash', '-n'], input=mirror.step_script(text, mirror.LIBRARY_STEP), text=True,
                                 capture_output=True, check=False)
        self.assertEqual(checked.returncode, 0, checked.stderr)

    def test_main_runs_the_library_script_once_in_one_shell(self):
        calls = []

        def fake(command, **kwargs):
            calls.append(command)
            return subprocess.CompletedProcess(command, 0)

        with patch.object(mirror.subprocess, 'run', fake), patch.object(mirror.sys, 'argv', ['run_boundary_job.py']), \
                contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(mirror.main(), 0)
        library = [c for c in calls if 'for name in $names' in c[-1]]
        self.assertEqual(len(library), 1)
        # Actions runs a step with no `shell:` as `bash -e {0}`
        self.assertEqual(library[0][:3], ['bash', '-e', '-c'])
        self.assertIn('--doc --locked', library[0][-1])

    def test_skip_library_leaves_the_step_out_and_says_so(self):
        calls = []

        def fake(command, **kwargs):
            calls.append(command)
            return subprocess.CompletedProcess(command, 0)

        with patch.object(mirror.subprocess, 'run', fake), contextlib.redirect_stdout(io.StringIO()), \
                patch.object(mirror.sys, 'argv', ['run_boundary_job.py', '--skip-library']):
            self.assertEqual(mirror.main(), 0)
        self.assertFalse([c for c in calls if 'for name in $names' in c[-1]])

    def test_a_failing_library_script_fails_the_run(self):
        def fake(command, **kwargs):
            return subprocess.CompletedProcess(command, 1 if 'for name in $names' in command[-1] else 0)

        with patch.object(mirror.subprocess, 'run', fake), patch.object(mirror.sys, 'argv', ['run_boundary_job.py']), \
                contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(mirror.main(), 1)


class LibraryStepRunsUnderErrexit(unittest.TestCase):
    """Review P2-1 (gate fixes): the step has no `shell:`, so Actions runs it as `bash -e {0}`: a failing cargo call, an empty
    package list and a failing `--list-present` abort it. Run without -e the script's status was that of its LAST command, so
    a red first crate passed. These run the REAL extracted script through `main` with a stub `python` (the package list) and
    a stub `cargo` on PATH whose first call fails."""

    def stub(self, directory, name, body):
        path = Path(directory) / name
        path.write_text('#!/bin/sh\n' + body, encoding='utf-8')
        path.chmod(path.stat().st_mode | stat.S_IXUSR)
        return path

    def run_main(self, listing, cargo_fails_first=True):
        with tempfile.TemporaryDirectory() as tmp:
            python = self.stub(tmp, 'fake-python', listing)
            log = Path(tmp) / 'cargo.log'
            mark = Path(tmp) / 'failed-once'
            fail = f'if [ ! -e "{mark}" ]; then : > "{mark}"; echo "$@" >> "{log}"; exit 1; fi; ' if cargo_fails_first else ''
            self.stub(tmp, 'cargo', f'{fail}echo "$@" >> "{log}"; exit 0\n')
            env = {**os.environ, 'PATH': f'{tmp}{os.pathsep}{os.environ["PATH"]}'}
            with patch.dict(os.environ, env, clear=True), patch.object(mirror, 'STEPS', ()), \
                    patch.object(mirror.sys, 'executable', str(python)), \
                    patch.object(mirror.sys, 'argv', ['run_boundary_job.py']), \
                    contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                status = mirror.main()
            return status, (log.read_text(encoding='utf-8').splitlines() if log.exists() else [])

    def test_a_failing_first_crate_fails_the_run(self):
        status, calls = self.run_main('echo alpha beta\n')
        self.assertEqual(status, 1)
        self.assertEqual(calls, ['test -p alpha --lib --locked'])  # aborted at once, as Actions does

    def test_the_control_all_crates_passing_runs_every_command_and_passes(self):
        status, calls = self.run_main('echo alpha beta\n', cargo_fails_first=False)
        self.assertEqual(status, 0)
        self.assertEqual(len(calls), 6)  # two crates, each: --lib, --doc, clippy
        self.assertTrue(calls[-1].startswith('clippy -p beta'))

    def test_an_empty_package_list_fails_the_run(self):
        status, calls = self.run_main('exit 0\n')
        self.assertEqual((status, calls), (1, []))

    def test_a_failing_list_present_fails_the_run(self):
        status, calls = self.run_main('exit 3\n')
        self.assertEqual((status, calls), (1, []))


if __name__ == '__main__':
    unittest.main()

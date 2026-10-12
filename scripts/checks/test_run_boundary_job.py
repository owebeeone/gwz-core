"""The local mirror of the boundary job runs the workflow's own commands (run_boundary_job.py)."""
import importlib.util
from pathlib import Path
import unittest

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


if __name__ == '__main__':
    unittest.main()

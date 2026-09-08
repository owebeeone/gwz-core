"""The runner must not inherit a fake filesystem into legacy/native groups."""
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('runner', Path(__file__).resolve().parents[1] / 'run_tests.py')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)

class RunnerModes(unittest.TestCase):
    def test_explicit_filesystem_mode_overrides_environment(self):
        with patch.dict('os.environ', {'GWZ_TEST_FS': 'fake'}):
            with patch.object(runner.subprocess, 'run') as run:
                run.return_value.returncode = 0
                runner.run('real', ['--lib'], ['contract'])
                self.assertEqual(run.call_args.kwargs['env']['GWZ_TEST_FS'], 'real')
                runner.run('fake', ['--lib'], ['contract'], filesystem='fake')
                self.assertEqual(run.call_args.kwargs['env']['GWZ_TEST_FS'], 'fake')
                self.assertEqual(run.call_args.kwargs['env']['GWZ_TEST_GIT'], 'fake')

if __name__ == '__main__':
    unittest.main()

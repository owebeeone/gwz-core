#!/usr/bin/env python3

import subprocess
import sys
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("check_checked_artifact_boundaries.py")


class CheckedArtifactBoundaryTest(unittest.TestCase):
    def test_current_source_inventory_is_classified(self) -> None:
        result = subprocess.run(
            [sys.executable, str(SCRIPT)],
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("checked-artifact boundary: ok", result.stdout)


if __name__ == "__main__":
    unittest.main()

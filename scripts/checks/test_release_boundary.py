#!/usr/bin/env python3

import importlib.util
import subprocess
import sys
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
RELEASE_PATH = ROOT / "scripts" / "release.py"
SPEC = importlib.util.spec_from_file_location("gwz_core_release", RELEASE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load release script")
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


class ReleaseBoundaryTest(unittest.TestCase):
    def test_release_help_exposes_no_compiler_skip(self) -> None:
        result = subprocess.run(
            [sys.executable, str(RELEASE_PATH), "--help"],
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertNotIn("--no-clippy", result.stdout)

    def test_exact_release_gate_reacquires_sha_before_boundary(self) -> None:
        cargo_root = Path("/tmp/exact-release-tree")
        expected = "a" * 40
        calls: list[tuple[str, ...]] = []

        def fake_run(command, **_kwargs):
            calls.append(tuple(str(item) for item in command))
            if command[:3] == ["git", "rev-parse", "HEAD"]:
                return SimpleNamespace(stdout=f"{expected}\n", returncode=0)
            return SimpleNamespace(stdout="", returncode=0)

        with (
            mock.patch.object(release, "run", side_effect=fake_run),
            mock.patch.object(release, "run_checked_boundary_gates") as boundary,
        ):
            release.gate_exact_release_commit(
                cargo_root=cargo_root, expected_head=expected
            )

        self.assertEqual(calls[0], ("git", "reset", "--hard", expected))
        self.assertEqual(calls[1], ("git", "rev-parse", "HEAD"))
        boundary.assert_called_once_with(cargo_root=cargo_root)

    def test_exact_release_gate_rejects_wrong_sha_before_boundary(self) -> None:
        with (
            mock.patch.object(
                release,
                "run",
                return_value=SimpleNamespace(stdout=f"{'b' * 40}\n", returncode=0),
            ),
            mock.patch.object(release, "run_checked_boundary_gates") as boundary,
            self.assertRaises(SystemExit),
        ):
            release.gate_exact_release_commit(
                cargo_root=release.REPO, expected_head="a" * 40
            )
        boundary.assert_not_called()


if __name__ == "__main__":
    unittest.main()

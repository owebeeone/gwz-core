#!/usr/bin/env python3

import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("check_checked_artifact_boundaries.py")
SOURCE = SCRIPT.parents[2] / "src"


def run(source: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), "--source", str(source)],
        check=False,
        capture_output=True,
        text=True,
    )


class CheckedArtifactBoundaryTest(unittest.TestCase):
    def test_current_source_inventory_is_classified(self) -> None:
        result = run(SOURCE)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("checked-artifact boundary: ok", result.stdout)

    def copied_source(self) -> tuple[tempfile.TemporaryDirectory[str], Path]:
        temporary = tempfile.TemporaryDirectory()
        target = Path(temporary.name) / "src"
        shutil.copytree(SOURCE, target)
        return temporary, target

    def test_ordinary_submodule_cannot_call_checked_entry(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "workspace_ops/handle_stash/commands.rs"
        path.write_text(
            path.read_text(encoding="utf-8")
            + "\n// crate::checked_artifact::entry::acquire_merge_root_artifact\n",
            encoding="utf-8",
        )
        self.assertNotEqual(run(source).returncode, 0)

    def test_unclassified_entry_is_rejected(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "checked_artifact/entry.rs"
        path.write_text(
            path.read_text(encoding="utf-8") + "\npub(crate) fn surprise() {}\n",
            encoding="utf-8",
        )
        self.assertNotEqual(run(source).returncode, 0)

    def test_missing_required_merge_reachability_is_rejected(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "workspace_ops/merge/root/artifact_facts.rs"
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                "crate::checked_artifact::entry::acquire_merge_root_artifact",
                "crate::checked_artifact::entry::acquire_merge_preservation_bundle",
            ),
            encoding="utf-8",
        )
        self.assertNotEqual(run(source).returncode, 0)

    def test_raw_entry_outside_the_boundary_is_rejected(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "workspace_ops/handle_stash/shared.rs"
        path.write_text(
            path.read_text(encoding="utf-8") + "\n// CheckedArtifact::acquire(\n",
            encoding="utf-8",
        )
        self.assertNotEqual(run(source).returncode, 0)

    def test_checked_merge_leaf_adapter_cannot_add_raw_successful_bypass(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "workspace_ops/merge/root/artifact_facts.rs"
        path.write_text(
            path.read_text(encoding="utf-8")
            + "\n// crate::artifact::write_atomic(root, bytes)\n",
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("raw filesystem mutation escaped", result.stderr)


if __name__ == "__main__":
    unittest.main()

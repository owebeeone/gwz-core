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

    def append(self, relative: str, text: str) -> subprocess.CompletedProcess[str]:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / relative
        path.write_text(path.read_text(encoding="utf-8") + text, encoding="utf-8")
        return run(source)

    def test_ordinary_unqualified_import_and_call_is_rejected(self) -> None:
        result = self.append(
            "workspace_ops/handle_stash/commands.rs",
            "\nuse crate::checked_artifact::entry::observe_merge_root_artifact;\n"
            "fn bypass(root: &std::path::Path) { let _ = "
            "observe_merge_root_artifact(root, std::path::Path::new(\"x\")); }\n",
        )
        self.assertNotEqual(result.returncode, 0)

    def test_reexported_entry_is_rejected(self) -> None:
        result = self.append(
            "workspace_ops/handle_stash/shared.rs",
            "\npub(crate) use crate::checked_artifact::entry::"
            "replace_merge_root_artifact as unchecked_replace;\n",
        )
        self.assertNotEqual(result.returncode, 0)

    def test_unclassified_visible_item_is_rejected(self) -> None:
        result = self.append(
            "checked_artifact/entry.rs", "\npub(crate) struct Surprise;\n"
        )
        self.assertNotEqual(result.returncode, 0)

    def test_entry_reexport_is_rejected(self) -> None:
        result = self.append(
            "checked_artifact/entry.rs",
            "\npub(crate) use crate::stash::write_bundle as raw_bundle_writer;\n",
        )
        self.assertNotEqual(result.returncode, 0)

    def test_entry_cannot_grow_a_direct_raw_writer(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "checked_artifact/entry.rs"
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                "pub(crate) fn prepare_merge_store_parents(root: &Path) -> ModelResult<()> {",
                "pub(crate) fn prepare_merge_store_parents(root: &Path) -> ModelResult<()> {\n"
                "    std::fs::write(root.join(\"raw\"), b\"bypass\").unwrap();",
            ),
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checked entry call graph changed", result.stderr)

    def test_general_checked_capability_escape_is_rejected(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "checked_artifact/mod.rs"
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                "struct CheckedArtifact {", "pub(crate) struct CheckedArtifact {"
            ),
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)

    def test_allowed_adapter_cannot_reexport_a_new_entry(self) -> None:
        result = self.append(
            "workspace_ops/merge/root/artifact_facts.rs",
            "\npub(in crate::workspace_ops::merge) use "
            "crate::checked_artifact::entry::replace_merge_preservation_bundle;\n",
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("visible-item inventory changed", result.stderr)

    def test_executable_aliased_raw_writer_in_adapter_is_rejected(self) -> None:
        result = self.append(
            "workspace_ops/merge/root/artifact_facts.rs",
            "\nuse std::fs::write as unchecked_write;\n"
            "fn bypass(root: &Path, bytes: &[u8]) { "
            "unchecked_write(root.join(\"raw\"), bytes).unwrap(); }\n",
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checked adapter call graph changed", result.stderr)

    def test_alias_import_is_rejected_even_when_it_reuses_an_allowed_call_name(self) -> None:
        result = self.append(
            "workspace_ops/merge/root/artifact_facts.rs",
            "\nuse std::fs::write as Ok;\n",
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checked adapter import inventory changed", result.stderr)

    def test_transitive_non_std_writer_in_adapter_is_rejected(self) -> None:
        result = self.append(
            "workspace_ops/merge/preserve/checked_bundle.rs",
            "\nfn bypass(root: &Path, bytes: &[u8]) { "
            "crate::stash::write_bundle(root, bytes).unwrap(); }\n",
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checked adapter call graph changed", result.stderr)

    def test_existing_transitive_helper_cannot_grow_a_raw_writer(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "workspace_ops/merge/preserve/artifacts.rs"
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                ") -> ModelResult<StashBundle> {\n"
                "    let stash_id = format!(\"stash_{}\", record.merge_id);",
                ") -> ModelResult<StashBundle> {\n"
                "    std::fs::write(\"raw\", b\"bypass\").unwrap();\n"
                "    let stash_id = format!(\"stash_{}\", record.merge_id);",
            ),
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("transitive helper changed", result.stderr)

    def test_comments_and_strings_do_not_create_false_references(self) -> None:
        result = self.append(
            "workspace_ops/handle_stash/shared.rs",
            "\n// crate::checked_artifact::entry::observe_merge_root_artifact(root, path)\n"
            "const NOTE: &str = \"CheckedArtifact::acquire(\";\n",
        )
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()

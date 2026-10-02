"""Mutation checks for the split checked-entry ownership boundary."""

import importlib.util
from pathlib import Path
import shutil
import tempfile
import unittest


spec = importlib.util.spec_from_file_location(
    "checked_artifact_boundary", Path(__file__).with_name("check_checked_artifact_boundaries.py")
)
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


class EntrySplitBoundaryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.directory = tempfile.TemporaryDirectory(prefix="gwz-entry-boundary-")
        root = Path(cls.directory.name)
        cls.source = root / "src"
        shutil.copytree(checker.ROOT / "src", cls.source)
        shutil.copyfile(checker.ROOT / "Cargo.toml", root / "Cargo.toml")
        # The approved generated corpus edge points outside src/.
        shutil.copytree(checker.ROOT / "protocol", root / "protocol")
        # The embedding test has an approved edge into the integration fixtures.
        fixture = Path("tests/transport_backend/python_embedding.rs")
        (root / fixture).parent.mkdir(parents=True)
        shutil.copyfile(checker.ROOT / fixture, root / fixture)

    @classmethod
    def tearDownClass(cls):
        cls.directory.cleanup()

    def mutate(self, relative, transform, expected):
        path = self.source / relative
        original = path.read_text() if path.exists() else None
        try:
            path.write_text(transform(original or ""))
            findings = checker.check(self.source)
            self.assertTrue(any(expected in finding for finding in findings), findings)
        finally:
            if original is None:
                path.unlink()
            else:
                path.write_text(original)

    def test_current_boundary_passes(self):
        self.assertEqual(checker.check(self.source), [])

    def test_unregistered_child_is_rejected(self):
        self.mutate("checked_artifact/entry/extra.rs", lambda _: "fn extra() {}\n", "ownership changed")

    def test_new_public_function_in_child_is_rejected(self):
        self.mutate(
            "checked_artifact/entry/artifacts.rs",
            lambda text: text + "\npub(crate) fn new_entry() {}\n",
            "visible-item inventory changed",
        )

    def test_wildcard_facade_export_is_rejected(self):
        self.mutate(
            "checked_artifact/entry.rs",
            lambda text: text + "\npub(crate) use observation::*;\n",
            "visible-item inventory changed",
        )

    def test_extra_facade_export_is_rejected(self):
        self.mutate(
            "checked_artifact/entry.rs",
            lambda text: text.replace("{CATALOG_LABEL,", "{extra, CATALOG_LABEL,", 1),
            "facade re-export inventory changed",
        )

    def test_child_cannot_drop_writer_lint_boundary(self):
        self.mutate(
            "checked_artifact/entry/catalog.rs",
            lambda text: text.replace("#![forbid(clippy::disallowed_methods)]", "", 1),
            "writer boundary is not fail-closed",
        )

    def test_neutral_raw_writer_cannot_move_into_child(self):
        self.mutate(
            "checked_artifact/entry/catalog.rs",
            lambda text: text + "\nfn unchecked() { crate::verified_write::write_atomic_verified(); }\n",
            "raw record-write caller outside",
        )


if __name__ == "__main__":
    unittest.main()

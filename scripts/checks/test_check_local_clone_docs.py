#!/usr/bin/env python3
"""Tests for the local-clone documentation consistency gate.

The shape is `test_check_merge_docs.py`'s: the real workspace satisfies the
manifest, a fixture carrying the claims the gate exists to catch fails the
REAL manifest (the negative control -- the only proof the checker catches
anything), and an unreadable source fails closed. Two rows are this gate's
own: every forbidden assertion must fire on the example sentence it carries
(`fires_on`), so no row is vacuous, and the command-line entry point must
print the marker the aggregate driver pins.
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


CHECKS_DIR = Path(__file__).resolve().parent
WORKSPACE_ROOT = CHECKS_DIR.parents[2]
MANIFEST = CHECKS_DIR / "local_clone_docs_manifest.json"
FIXTURE = CHECKS_DIR / "fixtures" / "local_clone_docs_stale_claims.md"
SCRIPT = CHECKS_DIR / "check_local_clone_docs.py"
sys.path.insert(0, str(CHECKS_DIR))

from check_merge_docs import _matches, check_manifest, load_manifest  # noqa: E402


# The claims the fixture makes, and the manifest row each one must trip.
NEGATIVE_CONTROL_IDS = (
    "no_removed_clone_local_spelling",
    "unserved_modes_must_not_be_claimed_working",
    "family_pull_push_must_not_be_claimed_served",
    "deletion_must_not_promise_an_archive",
    # A required statement the fixture lacks: the gate also fails on absence.
    "guide_keep_deletes_nothing",
)


class LocalCloneDocumentConsistencyTests(unittest.TestCase):
    def test_current_workspace_satisfies_manifest(self) -> None:
        manifest = load_manifest(MANIFEST)

        result = check_manifest(manifest, WORKSPACE_ROOT)

        self.assertEqual((), result.findings)
        # The guide, the four command pages it hangs off (local, clone, merge,
        # pull, push), the generated reference, and the five pages that
        # cross-link it (README, Concepts, Workflows, Troubleshooting,
        # Releases). A source added or removed moves this pin and the
        # aggregate driver's marker together.
        self.assertEqual(12, result.source_count)
        self.assertGreaterEqual(result.assertion_count, 90)

    def test_deliberate_stale_claims_fail_the_real_gate(self) -> None:
        manifest = load_manifest(MANIFEST)

        result = check_manifest(
            manifest,
            WORKSPACE_ROOT,
            source_overrides={"local_clones_guide": FIXTURE},
        )

        finding_ids = {
            finding.assertion_id
            for finding in result.findings
            if finding.source_id == "local_clones_guide"
        }
        for expected in NEGATIVE_CONTROL_IDS:
            self.assertIn(expected, finding_ids)
        # Only the overridden source went red; the real pages stay green.
        self.assertEqual(
            {"local_clones_guide"}, {finding.source_id for finding in result.findings}
        )

    def test_every_forbidden_row_fires_on_its_own_example(self) -> None:
        manifest = load_manifest(MANIFEST)
        rows = list(manifest["global_forbidden"])
        for source in manifest["sources"]:
            rows.extend(source.get("forbidden", []))

        self.assertGreater(len(rows), 0)
        for row in rows:
            with self.subTest(row=row["id"]):
                self.assertIn("fires_on", row, "a forbidden row must carry the sentence it catches")
                self.assertTrue(_matches(row["fires_on"], row))

    def test_missing_source_fails_closed(self) -> None:
        manifest = load_manifest(MANIFEST)

        result = check_manifest(
            manifest,
            WORKSPACE_ROOT,
            source_overrides={
                "local_clones_guide": CHECKS_DIR / "fixtures" / "does-not-exist.md"
            },
        )

        self.assertEqual("source_missing", result.findings[0].assertion_id)

    def test_command_line_prints_the_pinned_marker_and_fails_on_the_fixture(self) -> None:
        green = subprocess.run(
            [sys.executable, str(SCRIPT)], check=False, capture_output=True, text=True
        )
        self.assertEqual(0, green.returncode, green.stderr)
        self.assertRegex(
            green.stdout.strip(),
            r"^local-clone document consistency: ok \(12 sources, \d+ assertions\)$",
        )

        # The same manifest with the guide re-pointed at the fixture: the
        # entry point must exit 1 and name the findings, source by source.
        manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
        for source in manifest["sources"]:
            if source["id"] == "local_clones_guide":
                source["path"] = str(FIXTURE.relative_to(WORKSPACE_ROOT))
        with tempfile.TemporaryDirectory() as temporary:
            stale = Path(temporary) / "stale.json"
            stale.write_text(json.dumps(manifest), encoding="utf-8")
            red = subprocess.run(
                [sys.executable, str(SCRIPT), "--manifest", str(stale)],
                check=False,
                capture_output=True,
                text=True,
            )
        self.assertEqual(1, red.returncode, red.stdout)
        self.assertIn("local-clone document consistency: failed", red.stderr)
        self.assertIn("[no_removed_clone_local_spelling] forbidden statement is present", red.stderr)


if __name__ == "__main__":
    unittest.main()

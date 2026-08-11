#!/usr/bin/env python3
"""Tests for the merge documentation consistency gate."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path


CHECKS_DIR = Path(__file__).resolve().parent
WORKSPACE_ROOT = CHECKS_DIR.parents[2]
sys.path.insert(0, str(CHECKS_DIR))

from check_merge_docs import check_manifest, load_manifest  # noqa: E402


class MergeDocumentConsistencyTests(unittest.TestCase):
    def test_current_workspace_satisfies_manifest(self) -> None:
        manifest = load_manifest(CHECKS_DIR / "merge_docs_manifest.json")

        result = check_manifest(manifest, WORKSPACE_ROOT)

        self.assertEqual((), result.findings)
        self.assertEqual(10, result.source_count)
        self.assertGreaterEqual(result.assertion_count, 30)

    def test_deliberate_v0_no_ff_claim_fails_the_real_gate(self) -> None:
        manifest = load_manifest(CHECKS_DIR / "merge_docs_manifest.json")
        fixture = CHECKS_DIR / "fixtures" / "no_ff_in_v0.md"

        result = check_manifest(
            manifest,
            WORKSPACE_ROOT,
            source_overrides={"merge_command": fixture},
        )

        finding_ids = {finding.assertion_id for finding in result.findings}
        self.assertIn("m5a_must_not_release_no_ff", finding_ids)

    def test_missing_source_fails_closed(self) -> None:
        manifest = load_manifest(CHECKS_DIR / "merge_docs_manifest.json")

        result = check_manifest(
            manifest,
            WORKSPACE_ROOT,
            source_overrides={
                "merge_command": CHECKS_DIR / "fixtures" / "does-not-exist.md"
            },
        )

        self.assertEqual("source_missing", result.findings[0].assertion_id)


if __name__ == "__main__":
    unittest.main()

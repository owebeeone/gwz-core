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
        # 11 -> 12 (2026-08-28, R2-E E5 landing window): a6ce8a8 added the
        # recovery guide as the manifest's twelfth source and moved the
        # aggregate driver's marker, but missed this pin. Latent until now
        # because this suite runs only from the real workspace (the J-7
        # blind spot) -- the first real-workspace battery run since exposed it.
        # 12 -> 13 (2026-09-03, M5d step (3)): the SAME miss, one release
        # later. Measured, this pin was already false at gwz-core `57502e4`
        # (ship (1) W5's landing) and at this step's base `69ee990` -- the real
        # workspace has read 13 sources since W5 added one, and neither this
        # pin nor the aggregate driver's marker moved with it. Corrected here
        # rather than left red, because this step touches the manifest and a
        # gate that is red for someone else's reason hides its own findings.
        # This step's own doc delta is two ASSERTIONS on an existing source,
        # not a source: see run_r4bg_aggregate_gates.py's marker.
        self.assertEqual(13, result.source_count)
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

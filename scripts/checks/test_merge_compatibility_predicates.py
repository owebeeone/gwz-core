from __future__ import annotations

import copy
import importlib.util
import inspect
import json
import subprocess
import sys
import unittest
from pathlib import Path


HERE = Path(__file__).resolve().parent
CORE = HERE.parents[1]
REGISTRY = CORE / "dev-docs/GwzM5-8I2CompatibilityPredicates.json"
CHECKER = HERE / "check_merge_compatibility_predicates.py"


def load_checker():
    spec = importlib.util.spec_from_file_location("merge_predicates", CHECKER)
    if spec is None or spec.loader is None:
        raise AssertionError("cannot load compatibility checker")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class CompatibilityPredicateTests(unittest.TestCase):
    def setUp(self) -> None:
        self.checker = load_checker()
        self.document = self.checker.load_json_exact(REGISTRY.read_text(encoding="utf-8"))

    def validate(self) -> None:
        self.checker.validate(self.document)

    def test_repository_registry_is_closed(self) -> None:
        self.validate()

    # --- the class M5d removed (2026-09-05) ------------------------------
    #
    # The five deleted sections addressed Rust test SOURCE FILES by path and
    # the checker resolved them to disk, so deleting the v0 engine turned a
    # correct deletion into a red gate. These two tests hold the class shut
    # from both ends: nothing in the registry may name a source file, and the
    # checker itself may not grow a way to look one up.

    def test_registry_names_no_rust_source_file(self) -> None:
        encoded = json.dumps(self.document)
        self.assertNotIn(".rs", encoded)
        self.assertNotIn("src/workspace_ops", encoded)
        self.assertNotIn("workspace_ops::", encoded)

    def test_checker_takes_no_source_tree(self) -> None:
        # `validate` is handed the document and nothing else, so no registry
        # value has a source tree to be resolved against...
        self.assertEqual(
            ["document"], list(inspect.signature(self.checker.validate).parameters)
        )
        # ...and the `--core` argument that used to supply one is gone from
        # the CLI, so a caller cannot quietly hand one back.
        result = subprocess.run(
            [sys.executable, str(CHECKER), str(REGISTRY), "--core", str(CORE)],
            capture_output=True,
            text=True,
        )
        self.assertEqual(2, result.returncode, result.stderr)
        self.assertIn("unrecognized arguments: --core", result.stderr)

    def test_top_level_sections_are_closed(self) -> None:
        self.document["migration_whitelist"] = []
        with self.assertRaisesRegex(ValueError, "registry fields must be exactly"):
            self.validate()

    def test_registry_schema_version_is_pinned(self) -> None:
        self.document["schema"] = "gwz.merge-i2-compatibility-predicates/v2"
        with self.assertRaisesRegex(ValueError, "registry schema must be"):
            self.validate()

    # --- the closed normalization corpus ---------------------------------

    def test_enum_registry_rejects_extra_or_missing_values(self) -> None:
        self.document["normalization"]["enums"]["mode"].append("invented")
        with self.assertRaisesRegex(ValueError, "closed I2 enum registry"):
            self.validate()

    def test_normative_definition_corpus_cannot_be_weakened(self) -> None:
        self.document["normalization"]["definitions"]["prefix_boundary"] = (
            "Any partially published root is acceptable."
        )
        with self.assertRaisesRegex(ValueError, "normative definition corpus"):
            self.validate()

    def test_canonicalization_rule_cannot_be_weakened(self) -> None:
        self.document["normalization"]["identity"] = "Normalize something."
        with self.assertRaisesRegex(ValueError, "canonicalization rule"):
            self.validate()

    # --- the wire-code reason registry -----------------------------------
    #
    # `GwzM5-8I2ProtocolContract.md` §1 binds codes 48-61 to these exact
    # lists, so this is protocol surface and not v0 scaffolding.

    def test_reason_registry_rejects_extra_or_missing_codes(self) -> None:
        self.document["rejection_reasons"]["InventedCode"] = ["invented"]
        with self.assertRaisesRegex(ValueError, "closed protocol reason corpus"):
            self.validate()

    def test_reason_registry_rejects_a_reworded_reason(self) -> None:
        self.document["rejection_reasons"]["RecordedEvidenceDrift"] = [
            "something about the evidence changed"
        ]
        with self.assertRaisesRegex(ValueError, "closed protocol reason corpus"):
            self.validate()

    # --- the standalone archive corpus (R2-E Phase E5.2, 2026-08-28) ------
    #
    # One test per property the O8 archive-equivalence mechanism rests on, in
    # the same weaken-and-expect-a-raise form the rest of this suite uses.

    def test_archive_corpus_is_exactly_the_ten_table_b_shapes(self) -> None:
        self.document["archive_corpus"].pop()
        with self.assertRaisesRegex(ValueError, "ten Table B archive shapes"):
            self.validate()

    def test_archive_shape_order_is_the_table_b_order(self) -> None:
        corpus = self.document["archive_corpus"]
        corpus[0], corpus[1] = corpus[1], corpus[0]
        with self.assertRaisesRegex(ValueError, "in table order"):
            self.validate()

    def test_a_tier_cannot_reintroduce_a_test_binding(self) -> None:
        # The exactness of `TIER_KEYS` is what now holds the removed class
        # shut: a row cannot name a Rust test again, by any field name.
        row = self.document["archive_corpus"][0]
        row["tier1"]["test"] = (
            "workspace_ops::tests::g23::archive_equivalence_v0::"
            "archived_v0_shapes_are_byte_preserved_from_their_open_records"
        )
        with self.assertRaisesRegex(ValueError, "fields must be exactly"):
            self.validate()

    def test_executed_tier_cannot_also_owe_a_carrier(self) -> None:
        row = next(
            row
            for row in self.document["archive_corpus"]
            if row["tier1"]["status"] == "executed"
        )
        row["tier1"]["carrier"] = "some later lane"
        with self.assertRaisesRegex(ValueError, "must be absent on an executed tier"):
            self.validate()

    def test_unexecuted_tier_must_name_its_carrier(self) -> None:
        row = next(
            row
            for row in self.document["archive_corpus"]
            if row["tier2"]["status"] == "owed"
        )
        row["tier2"]["carrier"] = None
        with self.assertRaisesRegex(ValueError, "carrier"):
            self.validate()

    def test_a_fixtured_row_cannot_leave_tier_one_unexecuted(self) -> None:
        row = next(
            row
            for row in self.document["archive_corpus"]
            if row["disposition"] == "byte-preserved-v0-origin"
        )
        row["tier1"]["status"] = "owed"
        with self.assertRaisesRegex(ValueError, "must be 'executed'"):
            self.validate()

    def test_pending_fixture_pair_is_closed_to_the_two_named_shapes(self) -> None:
        # A third row quietly declaring itself unfixtured would silently shrink
        # the executed denominator; E0 §6.4 names exactly two.
        row = next(
            row
            for row in self.document["archive_corpus"]
            if row["shape"] == "AC-CANDIDATE"
        )
        row["disposition"] = "pending-fixture"
        row["fixture"] = "none"
        for tier in ("tier1", "tier2"):
            row[tier] = {"status": "pending-fixture", "carrier": "invented"}
        with self.assertRaisesRegex(ValueError, "exactly 8 tier-1-executed rows"):
            self.validate()

    def test_pending_fixture_shape_cannot_be_swapped_for_another(self) -> None:
        pending = next(
            row
            for row in self.document["archive_corpus"]
            if row["shape"] == "AP-PRESERVED"
        )
        fixtured = next(
            row
            for row in self.document["archive_corpus"]
            if row["shape"] == "AL-UNKNOWN"
        )
        pending["disposition"] = fixtured["disposition"]
        pending["fixture"] = fixtured["fixture"]
        pending["tier1"] = copy.deepcopy(fixtured["tier1"])
        pending["tier2"] = copy.deepcopy(fixtured["tier2"])
        fixtured["disposition"] = "pending-fixture"
        fixtured["fixture"] = "none"
        for tier in ("tier1", "tier2"):
            fixtured[tier] = {"status": "pending-fixture", "carrier": "invented"}
        with self.assertRaisesRegex(ValueError, "PENDING-FIXTURE rows must be exactly"):
            self.validate()

    def test_archive_clause_must_be_content_anchored(self) -> None:
        self.document["archive_corpus"][0]["clause"] = "see the contract, line 180"
        with self.assertRaisesRegex(ValueError, "content-anchored"):
            self.validate()

    def test_archive_disposition_registry_is_closed(self) -> None:
        row = next(
            row
            for row in self.document["archive_corpus"]
            if row["disposition"] == "byte-preserved-v0-origin"
        )
        row["disposition"] = "byte-preserved-v1-origin"
        with self.assertRaisesRegex(ValueError, "registered archive disposition"):
            self.validate()

    def test_tier_status_registry_is_closed(self) -> None:
        row = next(
            row
            for row in self.document["archive_corpus"]
            if row["tier2"]["status"] == "owed"
        )
        row["tier2"]["status"] = "waived"
        with self.assertRaisesRegex(ValueError, "registered tier status"):
            self.validate()

    def test_nested_duplicate_json_key_is_rejected(self) -> None:
        with self.assertRaisesRegex(ValueError, "duplicate JSON key"):
            self.checker.load_json_exact('{"outer":{"same":1,"same":2}}')


if __name__ == "__main__":
    unittest.main()

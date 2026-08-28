from __future__ import annotations

import copy
import importlib.util
import json
import unittest
from pathlib import Path


HERE = Path(__file__).resolve().parent
CORE = HERE.parents[1]
WORKSPACE = CORE.parent
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
        self.checker.validate(self.document, CORE)

    def test_repository_registry_is_closed_and_fixture_bound(self) -> None:
        self.validate()

    def test_descriptor_digest_is_authoritative(self) -> None:
        self.document["migration_whitelist"][0]["descriptor_sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "descriptor_sha256"):
            self.validate()

    def test_semantic_identity_excludes_fixture_address(self) -> None:
        duplicate = copy.deepcopy(self.document["migration_whitelist"][0])
        duplicate["id"] = "same-descriptor-different-rule"
        self.document["migration_whitelist"].append(duplicate)
        corpus = copy.deepcopy(self.document["fixture_corpus"][0])
        corpus["case_id"] = "changed/duplicate"
        corpus["rule"] = duplicate["id"]
        self.document["fixture_corpus"].append(corpus)
        with self.assertRaisesRegex(ValueError, "duplicate semantic descriptor"):
            self.validate()

    def test_descriptor_cannot_select_a_conflicting_classification(self) -> None:
        duplicate = copy.deepcopy(self.document["migration_whitelist"][0])
        duplicate["id"] = "conflicting-rule"
        duplicate["classification"]["next_action"] = "publish_candidate"
        self.document["migration_whitelist"].append(duplicate)
        corpus = copy.deepcopy(self.document["fixture_corpus"][0])
        corpus["case_id"] = "changed/conflicting"
        corpus["rule"] = duplicate["id"]
        self.document["fixture_corpus"].append(corpus)
        with self.assertRaisesRegex(ValueError, "descriptor|classification"):
            self.validate()

    def test_every_rule_requires_exactly_one_runtime_fixture(self) -> None:
        self.document["fixture_corpus"].pop()
        with self.assertRaisesRegex(ValueError, "fixture coverage"):
            self.validate()

    def test_fixture_subcase_must_be_exported_by_the_rust_test(self) -> None:
        self.document["fixture_corpus"][0]["subcase"] = "not_a_real_window"
        with self.assertRaisesRegex(ValueError, "subcase"):
            self.validate()

    def test_publication_shape_and_next_action_are_executable_rules(self) -> None:
        self.document["migration_whitelist"][0]["classification"]["next_action"] = (
            "publish_candidate"
        )
        with self.assertRaisesRegex(ValueError, "classification"):
            self.validate()

    def test_literal_fixture_member_identity_is_rejected(self) -> None:
        descriptor = self.document["migration_whitelist"][0]["descriptor"]
        descriptor["selection"]["ordered_ids"] = ["mem_remote"]
        descriptor["participants"][0]["id"] = "mem_remote"
        descriptor["observation"]["participants"][0]["id"] = "mem_remote"
        with self.assertRaisesRegex(ValueError, "alpha-normalized"):
            self.validate()

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

    def test_migration_policy_cannot_be_weakened(self) -> None:
        self.document["migration_policy"]["no_match"] = "Zero matches migrates."
        with self.assertRaisesRegex(ValueError, "closed policy corpus"):
            self.validate()

    def test_reason_registry_rejects_extra_or_missing_codes(self) -> None:
        self.document["rejection_reasons"]["InventedCode"] = ["invented"]
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

    def test_executed_tier_cannot_claim_a_test_that_does_not_exist(self) -> None:
        row = next(
            row
            for row in self.document["archive_corpus"]
            if row["tier1"]["status"] == "executed"
        )
        row["tier1"]["subcase"] = "not_a_real_archive_subcase"
        with self.assertRaisesRegex(ValueError, "subcase"):
            self.validate()

    def test_unexecuted_tier_cannot_claim_a_runtime_binding(self) -> None:
        row = next(
            row
            for row in self.document["archive_corpus"]
            if row["tier2"]["status"] == "owed"
        )
        row["tier2"]["test"] = (
            "workspace_ops::tests::g23::archive_equivalence_v0::"
            "archived_v0_shapes_are_byte_preserved_from_their_open_records"
        )
        with self.assertRaisesRegex(ValueError, "must be absent on an unexecuted tier"):
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
            row[tier] = {
                "status": "pending-fixture",
                "test": None,
                "subcase": None,
                "carrier": "invented",
            }
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
            fixtured[tier] = {
                "status": "pending-fixture",
                "test": None,
                "subcase": None,
                "carrier": "invented",
            }
        with self.assertRaisesRegex(ValueError, "PENDING-FIXTURE rows must be exactly"):
            self.validate()

    def test_archive_clause_must_be_content_anchored(self) -> None:
        self.document["archive_corpus"][0]["clause"] = "see the contract, line 180"
        with self.assertRaisesRegex(ValueError, "content-anchored"):
            self.validate()

    def test_nested_duplicate_json_key_is_rejected(self) -> None:
        with self.assertRaisesRegex(ValueError, "duplicate JSON key"):
            self.checker.load_json_exact('{"outer":{"same":1,"same":2}}')


if __name__ == "__main__":
    unittest.main()

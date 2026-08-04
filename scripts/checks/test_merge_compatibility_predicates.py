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

    def test_nested_duplicate_json_key_is_rejected(self) -> None:
        with self.assertRaisesRegex(ValueError, "duplicate JSON key"):
            self.checker.load_json_exact('{"outer":{"same":1,"same":2}}')


if __name__ == "__main__":
    unittest.main()

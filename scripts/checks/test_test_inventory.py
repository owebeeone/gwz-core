"""Negative controls for the generated test census (DR-6)."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("test_inventory", Path(__file__).with_name("test_inventory.py"))
inventory = importlib.util.module_from_spec(spec)
spec.loader.exec_module(inventory)

class InventoryTests(unittest.TestCase):
    def test_cargo_artifacts_select_exactly_one_package_library_test(self):
        import json
        row = {"reason": "compiler-artifact", "target": {"name": "gwz_core", "kind": ["lib"]},
               "profile": {"test": True}, "executable": "/tmp/core-tests"}
        self.assertEqual(inventory.cargo_test_binary(json.dumps(row), "gwz-core"), Path("/tmp/core-tests"))
        for rows in [[], [row, row], [{**row, "executable": None}],
                     [{**row, "target": {"name": "other", "kind": ["lib"]}}]]:
            with self.subTest(rows=rows), self.assertRaises(ValueError):
                inventory.cargo_test_binary("\n".join(map(json.dumps, rows)), "gwz-core")

    def test_baselines_require_the_same_known_build_profile(self):
        identity = {"package": "gwz-core", "platform": "Darwin", "architecture": "arm64", "profile": "debug"}
        inventory.check_baseline_identity(identity, identity)
        for profile in ["release", "unknown", None]:
            with self.subTest(profile=profile), self.assertRaises(ValueError):
                inventory.check_baseline_identity({**identity, "profile": profile}, identity)
        with self.assertRaises(ValueError):
            inventory.check_baseline_identity({**identity, "profile": "unknown"}, {**identity, "profile": "unknown"})

    def test_partition_is_disjoint_and_complete(self):
        names = {"checked_artifact::a", "workspace_ops::merge::v1_lifecycle::b",
                 "workspace_ops::merge::v1_lifecycle::root_fault_matrix", "git::c"}
        groups = inventory.partition(names)
        self.assertEqual(names, set().union(*groups.values()))
        self.assertEqual(len(names), sum(map(len, groups.values())))

    def test_deleted_test_needs_a_specific_reason(self):
        with self.assertRaises(ValueError):
            inventory.check_removals({"old", "kept"}, {"new", "kept"}, {})
        inventory.check_removals({"old", "kept"}, {"new", "kept"}, {"old": "renamed to new; same scenario"})

    def test_omission_duplicate_failure_and_zero_execution_refuse(self):
        for output in ["", "test a ... ok\n", "test a ... ok\ntest a ... ok\ntest b ... ok\n",
                       "test a ... ok\ntest b ... FAILED\n"]:
            with self.subTest(output=output), self.assertRaises(ValueError):
                inventory.check_execution({"a", "b"}, set(), output)

    def test_ignored_tests_are_explicit_and_never_count_as_executed(self):
        inventory.check_execution({"a", "helper"}, {"helper"},
                                  "test a ... ok\ntest helper ... ignored, invoked by parent\n")
        with self.assertRaises(ValueError):
            inventory.check_execution({"a", "helper"}, set(), "test a ... ok\ntest helper ... ignored\n")

    def test_empty_inventory_fails(self):
        with self.assertRaises(ValueError):
            inventory.parse_listing("0 tests, 0 benchmarks")

    def test_overlapping_filters_fail(self):
        with self.assertRaises(ValueError):
            inventory.partition({"checked_artifact::root_fault_matrix"})

    def test_expected_panic_is_a_named_execution(self):
        inventory.check_execution({"a"}, set(), "test a - should panic ... ok\n")

if __name__ == "__main__":
    unittest.main()

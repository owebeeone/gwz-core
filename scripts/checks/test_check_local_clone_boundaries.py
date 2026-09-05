#!/usr/bin/env python3
"""Unit tests and negative fixtures for the local clone boundary gate.

Positive: the real tree passes. Negative fixtures (boundaries §6): an
unclassified crate, a reversed contract edge, an aliased forbidden
dependency, and optional/target/dev dependency violations. Each fixture is a
synthetic crate tree built in a temporary directory so the checker, not a
build, is the rejector.
"""

from __future__ import annotations

import importlib.util
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("check_local_clone_boundaries.py")
INVENTORY = Path(__file__).with_name("local_clone_inventory.json")
ROOT = SCRIPT.parents[2]


def load_gate():
    """The checker as a module, for unit tests over `tier_a_unlocked`."""
    spec = importlib.util.spec_from_file_location("check_local_clone_boundaries", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    sys.modules[spec.name] = module  # dataclasses resolve their module here
    spec.loader.exec_module(module)
    return module


# A Tier A loop in the shape of `.github/workflows/checked-artifact-boundary.yml`,
# with the command itself wrapped across `\` continuations (State S2-P3-2:
# a per-line matcher sees no line holding both `cargo test` and
# `--manifest-path`).
SPLIT_UNLOCKED_WORKFLOW = """\
name: synthetic
on: [push]
jobs:
  libraries:
    runs-on: ubuntu-24.04
    steps:
      - name: Each library's Tier A command
        run: |
          # cargo test --manifest-path crates/x/Cargo.toml --lib --locked
          for directory in alpha \\
              alpha-contract; do
            cargo test \\
              --manifest-path "crates/$directory/Cargo.toml" \\
              --lib
          done
"""

SPLIT_LOCKED_WORKFLOW = SPLIT_UNLOCKED_WORKFLOW.replace(
    "              --lib\n", "              --lib --locked\n"
)

SECOND_LOCKED_WORKFLOW = """\
name: synthetic-two
on: [push]
jobs:
  one-library:
    runs-on: ubuntu-24.04
    steps:
      - run: cargo test --locked --manifest-path crates/alpha/Cargo.toml --lib
"""

SECOND_UNLOCKED_WORKFLOW = SECOND_LOCKED_WORKFLOW.replace(" --locked", "")

NO_TIER_A_WORKFLOW = """\
name: synthetic-none
on: [push]
jobs:
  root-only:
    runs-on: ubuntu-24.04
    steps:
      - run: cargo test --locked --lib
"""


def run(core: Path, inventory: Path = INVENTORY) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), "--core", str(core), "--inventory", str(inventory)],
        check=False,
        capture_output=True,
        text=True,
    )


def manifest(name: str, deps: str = "", extra: str = "") -> str:
    return (
        "[package]\n"
        f'name = "{name}"\n'
        'version = "0.1.0"\n'
        'edition = "2024"\n'
        'rust-version = "1.95"\n'
        "publish = false\n"
        f"{extra}"
        "[dependencies]\n"
        f"{deps}"
    )


class SyntheticTree:
    """A minimal core checkout: a `gwz-core` package plus classified crates."""

    def __init__(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.core = Path(self.temporary.name) / "gwz-core"
        (self.core / "src").mkdir(parents=True)
        (self.core / "src" / "lib.rs").write_text("", encoding="utf-8")
        self.crates = self.core / "crates"
        self.crates.mkdir()
        self.inventory = {
            "inventory_version": 1,
            "crates_dir": "crates",
            "role_edges": {
                "contract": ["contract", "pure"],
                "pure": ["contract", "pure"],
                "implementation": ["contract", "pure"],
                "integration": ["contract", "pure"],
                "harness": ["contract", "pure"],
            },
            "forbidden_dependencies": ["gwz-core", "gwz", "gwz-py"],
            "packages": {},
        }
        self.crate("gwz-alpha-contract", "alpha-contract", "contract", [], [])
        self.crate("gwz-alpha", "alpha", "implementation", ["gwz-alpha-contract"], [])
        self.crate(
            "gwz-fixtures",
            "fixtures",
            "harness",
            ["gwz-alpha-contract"],
            [],
            expected="pending",
        )
        self.write_core_manifest()

    def crate(
        self,
        name: str,
        directory: str,
        role: str,
        first_party: list[str],
        third_party: list[str],
        *,
        expected: str = "present",
        dev_first_party: list[str] | None = None,
        deps: str | None = None,
        extra: str = "",
        classify: bool = True,
        materialize: bool | None = None,
    ) -> Path:
        path = self.crates / directory
        if classify:
            self.inventory["packages"][name] = {
                "directory": directory,
                "role": role,
                "owner": "T",
                "rationale": "synthetic",
                "first_party": first_party,
                "third_party": third_party,
                "dev_first_party": dev_first_party if dev_first_party is not None else first_party,
                "dev_third_party": [],
                "expected": expected,
            }
        if materialize is None:
            materialize = expected == "present"
        if materialize:
            (path / "src").mkdir(parents=True, exist_ok=True)
            (path / "src" / "lib.rs").write_text("", encoding="utf-8")
            if deps is None:
                deps = "".join(
                    f'{dep} = {{ path = "../{self.inventory["packages"][dep]["directory"]}" }}\n'
                    for dep in first_party
                    if dep in self.inventory["packages"]
                )
            (path / "Cargo.toml").write_text(manifest(name, deps, extra), encoding="utf-8")
        return path

    def write_core_manifest(self) -> None:
        deps = "".join(
            f'{name} = {{ path = "crates/{entry["directory"]}" }}\n'
            for name, entry in self.inventory["packages"].items()
            if entry["expected"] == "present" and entry["role"] != "harness"
        )
        (self.core / "Cargo.toml").write_text(
            "[package]\nname = \"gwz-core\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"
            "[dependencies]\n" + deps,
            encoding="utf-8",
        )

    def inventory_path(self) -> Path:
        path = self.core / "inventory.json"
        path.write_text(json.dumps(self.inventory, indent=1), encoding="utf-8")
        return path

    def workflow(self, name: str, text: str) -> None:
        directory = self.core / ".github" / "workflows"
        directory.mkdir(parents=True, exist_ok=True)
        (directory / name).write_text(text, encoding="utf-8")

    def third_party_alpha(self) -> None:
        """`gwz-alpha` declares a third-party dependency (`tempfile`)."""
        self.crate(
            "gwz-alpha",
            "alpha",
            "implementation",
            ["gwz-alpha-contract"],
            ["tempfile"],
            deps='gwz-alpha-contract = { path = "../alpha-contract" }\n'
            'tempfile = "3"\n',
        )

    def check(self) -> subprocess.CompletedProcess[str]:
        self.write_core_manifest()
        return run(self.core, self.inventory_path())

    def cleanup(self) -> None:
        self.temporary.cleanup()


class LocalCloneBoundaryTest(unittest.TestCase):
    def test_real_tree_passes(self) -> None:
        result = run(ROOT)
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertIn("local-clone boundary: ok", result.stdout)

    def test_real_inventory_matches_the_boundary_document(self) -> None:
        inventory = json.loads(INVENTORY.read_text(encoding="utf-8"))
        expected = {
            "gwz-copy-contract": ("C", "contract", []),
            "gwz-refcopy": ("R", "implementation", ["gwz-copy-contract"]),
            "gwz-repo-contract": ("C", "contract", []),
            "gwz-repo-inspect": ("I", "implementation", ["gwz-repo-contract"]),
            "gwz-work-detector": ("W", "pure", ["gwz-repo-contract"]),
            "gwz-history-check": ("H", "integration", ["gwz-repo-contract"]),
            "gwz-family-model": ("F", "pure", []),
            "gwz-family-store-contract": ("C", "contract", ["gwz-family-model"]),
            "gwz-family-store": ("S", "implementation", ["gwz-family-store-contract", "gwz-family-model"]),
            "gwz-local-import": ("X", "integration", ["gwz-repo-contract", "gwz-family-model"]),
            "gwz-workspace-install": (
                "N",
                "integration",
                ["gwz-copy-contract", "gwz-repo-contract", "gwz-family-model", "gwz-family-store-contract"],
            ),
            "gwz-repo-factory": ("B", "integration", ["gwz-repo-contract"]),
            "gwz-local-disposal": (
                "D",
                "integration",
                ["gwz-repo-contract", "gwz-family-model", "gwz-family-store-contract", "gwz-work-detector"],
            ),
            "gwz-local-testrepo": ("T", "harness", ["gwz-repo-contract"]),
        }
        self.assertEqual(set(inventory["packages"]), set(expected))
        for name, (owner, role, first_party) in expected.items():
            entry = inventory["packages"][name]
            self.assertEqual(entry["owner"], owner, name)
            self.assertEqual(entry["role"], role, name)
            self.assertEqual(entry["first_party"], first_party, name)
        self.assertEqual(inventory["packages"]["gwz-local-testrepo"]["expected"], "pending")
        self.assertEqual(
            inventory["policy"]["canonical_policy_sha256"],
            "dcc4fbd2b45caf928978a090208951ef14759ef0589e820b94f8475fccf07c10",
        )

    def test_synthetic_tree_passes(self) -> None:
        tree = SyntheticTree()
        self.addCleanup(tree.cleanup)
        result = tree.check()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("pending", result.stdout)

    def test_unclassified_crate_is_rejected(self) -> None:
        tree = SyntheticTree()
        self.addCleanup(tree.cleanup)
        tree.crate("gwz-rogue", "rogue", "pure", [], [], classify=False, materialize=True)
        result = tree.check()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unclassified crate", result.stderr)

    def test_reversed_contract_edge_is_rejected(self) -> None:
        tree = SyntheticTree()
        self.addCleanup(tree.cleanup)
        tree.crate(
            "gwz-alpha-contract",
            "alpha-contract",
            "contract",
            [],
            [],
            deps='gwz-alpha = { path = "../alpha" }\n',
        )
        result = tree.check()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("gwz-alpha-contract: gwz-alpha [normal]", result.stderr)
        self.assertIn("not in the inventory allowlist", result.stderr)

    def test_role_direction_is_checked_even_when_listed(self) -> None:
        tree = SyntheticTree()
        self.addCleanup(tree.cleanup)
        tree.crate(
            "gwz-alpha-contract",
            "alpha-contract",
            "contract",
            ["gwz-alpha"],
            [],
            deps='gwz-alpha = { path = "../alpha" }\n',
        )
        result = tree.check()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("role contract may not depend on role implementation", result.stderr)

    def test_aliased_forbidden_dependency_is_rejected(self) -> None:
        tree = SyntheticTree()
        self.addCleanup(tree.cleanup)
        tree.crate(
            "gwz-alpha",
            "alpha",
            "implementation",
            ["gwz-alpha-contract"],
            [],
            deps='gwz-alpha-contract = { path = "../alpha-contract" }\n'
            'engine = { package = "gwz-core", path = "../.." }\n',
        )
        result = tree.check()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("gwz-core (as `engine`) [normal]: forbidden dependency", result.stderr)

    def test_aliased_sibling_pointing_at_another_crate_is_rejected(self) -> None:
        tree = SyntheticTree()
        self.addCleanup(tree.cleanup)
        tree.crate("gwz-beta", "beta", "pure", [], [])
        tree.crate(
            "gwz-alpha",
            "alpha",
            "implementation",
            ["gwz-alpha-contract"],
            [],
            deps='gwz-alpha-contract = { package = "gwz-beta", path = "../beta" }\n',
        )
        result = tree.check()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("gwz-beta (as `gwz-alpha-contract`) [normal]", result.stderr)

    def test_optional_third_party_dependency_is_rejected(self) -> None:
        tree = SyntheticTree()
        self.addCleanup(tree.cleanup)
        tree.crate(
            "gwz-alpha",
            "alpha",
            "implementation",
            ["gwz-alpha-contract"],
            [],
            deps='gwz-alpha-contract = { path = "../alpha-contract" }\n'
            'sneaky = { version = "1", optional = true }\n',
            extra="[features]\nfast = [\"dep:sneaky\"]\n",
        )
        result = tree.check()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("sneaky [normal, optional]: third-party edge is not in the inventory allowlist", result.stderr)

    def test_target_specific_dependency_is_rejected(self) -> None:
        tree = SyntheticTree()
        self.addCleanup(tree.cleanup)
        tree.crate(
            "gwz-alpha",
            "alpha",
            "implementation",
            ["gwz-alpha-contract"],
            [],
            deps='gwz-alpha-contract = { path = "../alpha-contract" }\n'
            "[target.'cfg(windows)'.dependencies]\n"
            'windows-sys = "0.61"\n',
        )
        result = tree.check()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("windows-sys [normal, target=cfg(windows)]", result.stderr)

    def test_dev_dependency_on_core_is_rejected(self) -> None:
        tree = SyntheticTree()
        self.addCleanup(tree.cleanup)
        tree.crate(
            "gwz-alpha",
            "alpha",
            "implementation",
            ["gwz-alpha-contract"],
            [],
            deps='gwz-alpha-contract = { path = "../alpha-contract" }\n'
            "[dev-dependencies]\n"
            'gwz-core = { path = "../.." }\n',
        )
        result = tree.check()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("gwz-core [dev]: forbidden dependency", result.stderr)

    def test_harness_as_normal_dependency_is_rejected(self) -> None:
        tree = SyntheticTree()
        self.addCleanup(tree.cleanup)
        tree.crate("gwz-fixtures", "fixtures", "harness", ["gwz-alpha-contract"], [])
        tree.crate(
            "gwz-alpha",
            "alpha",
            "implementation",
            ["gwz-alpha-contract", "gwz-fixtures"],
            [],
            deps='gwz-alpha-contract = { path = "../alpha-contract" }\n'
            'gwz-fixtures = { path = "../fixtures" }\n',
        )
        result = tree.check()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("a harness may only be a dev-dependency", result.stderr)

    def test_test_closure_reaching_core_through_a_sibling_is_rejected(self) -> None:
        tree = SyntheticTree()
        self.addCleanup(tree.cleanup)
        # The sibling declares core; alpha reaches it transitively.
        tree.crate(
            "gwz-alpha-contract",
            "alpha-contract",
            "contract",
            [],
            [],
            deps='gwz-core = { path = "../.." }\n',
        )
        result = tree.check()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("gwz-alpha: test closure reaches forbidden package gwz-core via gwz-alpha-contract", result.stderr)

    def test_expected_package_missing_is_rejected_and_pending_is_not(self) -> None:
        tree = SyntheticTree()
        self.addCleanup(tree.cleanup)
        shutil.rmtree(tree.crates / "alpha")
        result = tree.check()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("gwz-alpha: expected present", result.stderr)

    def test_package_name_must_match_inventory_key(self) -> None:
        tree = SyntheticTree()
        self.addCleanup(tree.cleanup)
        (tree.crates / "alpha" / "Cargo.toml").write_text(
            manifest("gwz-alpha-renamed", 'gwz-alpha-contract = { path = "../alpha-contract" }\n'),
            encoding="utf-8",
        )
        result = tree.check()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("differs from inventory key", result.stderr)

    def test_workspace_table_and_inherited_metadata_are_rejected(self) -> None:
        tree = SyntheticTree()
        self.addCleanup(tree.cleanup)
        (tree.crates / "alpha-contract" / "Cargo.toml").write_text(
            "[package]\nname = \"gwz-alpha-contract\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"
            "publish = false\n[workspace]\n[dependencies]\n",
            encoding="utf-8",
        )
        result = tree.check()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("must not declare a `[workspace]` table", result.stderr)
        self.assertIn("needs an explicit `rust-version`", result.stderr)

    def test_declared_third_party_with_unlocked_tier_a_is_rejected(self) -> None:
        # LCM1.0c-rem1 (State P3-3): a crate that DECLARES a third-party
        # dependency while CI's Tier A step runs unlocked is refused; the same
        # tree with a locked Tier A step passes.
        tree = SyntheticTree()
        self.addCleanup(tree.cleanup)
        tree.crate(
            "gwz-alpha",
            "alpha",
            "implementation",
            ["gwz-alpha-contract"],
            ["tempfile"],
            deps='gwz-alpha-contract = { path = "../alpha-contract" }\n'
            'tempfile = "3"\n',
        )
        tree.inventory["ci_tier_a_unlocked"] = True
        result = tree.check()
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn("declares third-party dependency 'tempfile'", result.stderr)
        self.assertIn("runs unlocked", result.stderr)

        tree.inventory["ci_tier_a_unlocked"] = False
        result = tree.check()
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertIn("locked Tier A step", result.stdout)

    def test_tier_a_command_split_across_continuations_is_unlocked_and_refused(self) -> None:
        # LCM1.0c-fu1 (State S2-P3-2): the workflow-parsing half of the
        # S-P3-3 guard, with no inventory flag. A Tier A command wrapped
        # across `\` continuations and carrying no `--locked` is unlocked --
        # the guard stays armed -- so a declared third-party edge is refused.
        gate = load_gate()
        tree = SyntheticTree()
        self.addCleanup(tree.cleanup)
        tree.third_party_alpha()
        tree.workflow("boundary.yml", SPLIT_UNLOCKED_WORKFLOW)
        self.assertTrue(gate.tier_a_unlocked(tree.core, tree.inventory))
        result = tree.check()
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn("declares third-party dependency 'tempfile'", result.stderr)
        self.assertIn("runs unlocked", result.stderr)

    def test_locked_on_every_tier_a_command_is_locked_and_anything_less_is_not(self) -> None:
        # LCM1.0c-fu1 (State S2-P3-2): "locked" needs affirmative evidence on
        # EVERY `cargo test ... --manifest-path` command in every workflow;
        # anything the parser cannot establish counts as unlocked.
        gate = load_gate()
        tree = SyntheticTree()
        self.addCleanup(tree.cleanup)
        tree.third_party_alpha()
        # No workflow at all while crates/ exists: unknown, hence unlocked.
        self.assertTrue(gate.tier_a_unlocked(tree.core, tree.inventory))
        # A workflow with no recognisable Tier A command: unknown, unlocked.
        tree.workflow("none.yml", NO_TIER_A_WORKFLOW)
        self.assertTrue(gate.tier_a_unlocked(tree.core, tree.inventory))
        # `--locked` on the wrapped command and on a second workflow's command:
        # locked, and the gate admits the third-party edge with its note.
        tree.workflow("boundary.yml", SPLIT_LOCKED_WORKFLOW)
        tree.workflow("second.yml", SECOND_LOCKED_WORKFLOW)
        self.assertFalse(gate.tier_a_unlocked(tree.core, tree.inventory))
        result = tree.check()
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertIn("locked Tier A step", result.stdout)
        # One unlocked command anywhere -- even after a locked one -- unlocks.
        tree.workflow("second.yml", SECOND_UNLOCKED_WORKFLOW)
        self.assertTrue(gate.tier_a_unlocked(tree.core, tree.inventory))
        result = tree.check()
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn("runs unlocked", result.stderr)

    def test_malformed_inventory_is_an_error_not_a_pass(self) -> None:
        tree = SyntheticTree()
        self.addCleanup(tree.cleanup)
        del tree.inventory["packages"]["gwz-alpha"]["rationale"]
        result = tree.check()
        self.assertEqual(result.returncode, 2)
        self.assertIn("lacks `rationale`", result.stderr)


if __name__ == "__main__":
    unittest.main()

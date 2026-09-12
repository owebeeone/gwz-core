#!/usr/bin/env python3
"""Unit tests and negative fixtures for the crate-version lockstep gate.

Positive: a synthetic fourteen-crate tree of the real shape passes, and so
does the real gwz-core checkout this file lives in. Negative fixtures, one per
rule of `check_crate_versions.py`: a product version that is not `X.Y.Z`, a
version that disagrees with `--tag`, an internal version off the `0.0.N` line,
one internal crate off the shared line, a `[dependencies]` edge with no
`version`, one with the wrong version, one with no `path`, a
`[dev-dependencies]` edge carrying a `version`, one with no `path`, a git
dependency, a published crate missing each of `repository`/`readme`/`license`/
`description`, a published crate with `publish = false`, the fixtures crate
without it, and a crate count that drifts from fourteen. Each fixture is a
synthetic manifest tree in a temporary directory, so the checker -- not cargo
-- is the rejector, and each assertion requires the finding to NAME the
offending crate.

Also here, because the ordering lives in the same script: `--print-publish-order`
over the real checkout prints fourteen names ending at `gwz-core`, places every
crate after its dependencies, and agrees with plan section 1's layers; a
synthetic cycle has no order at all.
"""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import tempfile
import unittest
from collections import Counter
from pathlib import Path


SCRIPT = Path(__file__).with_name("check_crate_versions.py")
REAL_ROOT = SCRIPT.resolve().parents[2]

# The fourteen internal crates of plan section 1, in its publish order. The
# directory of each is its name without the `gwz-` prefix, as in the real tree.
INTERNAL = (
    "gwz-repo-contract",
    "gwz-copy-contract",
    "gwz-family-model",
    "gwz-family-store-contract",
    "gwz-work-detector",
    "gwz-history-check",
    "gwz-repo-factory",
    "gwz-repo-inspect",
    "gwz-refcopy",
    "gwz-local-testrepo",
    "gwz-family-store",
    "gwz-local-import",
    "gwz-workspace-install",
    "gwz-local-disposal",
)
FIXTURES_CRATE = "gwz-local-testrepo"
# Plan section 1's dependency layers, which fix the publish order, for the
# fifteen-crate publish set minus the unpublished fixtures crate. The layer of
# a crate must be strictly above every crate it depends on; `gwz-core` is the
# composition root and publishes last.
PLAN_LAYERS = {
    "gwz-repo-contract": 1,
    "gwz-copy-contract": 1,
    "gwz-family-model": 1,
    "gwz-family-store-contract": 2,
    "gwz-work-detector": 2,
    "gwz-history-check": 2,
    "gwz-repo-factory": 2,
    "gwz-repo-inspect": 2,
    "gwz-refcopy": 2,
    "gwz-family-store": 3,
    "gwz-local-import": 3,
    "gwz-workspace-install": 3,
    "gwz-local-disposal": 4,
    "gwz-core": 5,
}
# One crate carries both edge kinds so the dependency rules have a target.
EDGE_CRATE = "gwz-family-store"
EDGE_TARGET = "gwz-family-model"


def load_gate():
    """The checker as a module, for unit tests over its parsers."""
    spec = importlib.util.spec_from_file_location("check_crate_versions", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    sys.modules[spec.name] = module  # dataclasses resolve their module here
    spec.loader.exec_module(module)
    return module


def directory_of(name: str) -> str:
    return name.removeprefix("gwz-")


def core_manifest(version: str, internal_version: str, crates: tuple[str, ...]) -> str:
    """gwz-core's manifest: a versioned edge per published crate, a path-only dev edge."""
    lines = [
        "[package]",
        'name = "gwz-core"',
        f'version = "{version}"',
        'edition = "2024"',
        'license = "GPL-2.0-only"',
        'repository = "https://github.com/owebeeone/gwz-core"',
        "",
        "[dependencies]",
    ]
    for name in sorted(crates):
        if name == FIXTURES_CRATE:
            continue
        lines.append(
            f'{name} = {{ path = "crates/{directory_of(name)}", version = "{internal_version}" }}'
        )
    lines += [
        'serde = { version = "1", features = ["derive"] }',
        "",
        "[dev-dependencies]",
        f'{FIXTURES_CRATE} = {{ path = "crates/{directory_of(FIXTURES_CRATE)}" }}',
        "",
    ]
    return "\n".join(lines)


def crate_manifest(name: str, internal_version: str) -> str:
    """One internal crate's manifest, with the fixtures crate's exception."""
    lines = [
        "[package]",
        f'name = "{name}"',
        f'version = "{internal_version}"',
        'edition = "2024"',
        'license = "GPL-2.0-only"',
        f'description = "An internal component crate of GWZ: {name}."',
    ]
    if name == FIXTURES_CRATE:
        lines.append("publish = false")
    else:
        lines += [
            'repository = "https://github.com/owebeeone/gwz-core"',
            'readme = "README.md"',
            'keywords = ["gwz", "git", "workspace", "internal"]',
        ]
    if name == EDGE_CRATE:
        lines += [
            "",
            "[dependencies]",
            f'{EDGE_TARGET} = {{ path = "../{directory_of(EDGE_TARGET)}", '
            f'version = "{internal_version}" }}',
            "",
            "[dev-dependencies]",
            f'{FIXTURES_CRATE} = {{ path = "../{directory_of(FIXTURES_CRATE)}", '
            'features = ["contract-tests"] }',
        ]
    lines.append("")
    return "\n".join(lines)


def write_tree(
    root: Path,
    *,
    core_version: str = "1.0.12",
    internal_version: str = "0.0.1",
    crates: tuple[str, ...] = INTERNAL,
    edit: dict[str, object] | None = None,
) -> Path:
    """Render the manifest tree under `root`.

    `edit` maps a manifest key -- `"gwz-core"` or a crate name -- to a
    `(old, new)` text substitution applied to that manifest, which is how each
    negative fixture introduces exactly one violation.
    """
    edit = edit or {}

    def apply(key: str, text: str) -> str:
        replacement = edit.get(key)
        if replacement is None:
            return text
        old, new = replacement
        if old not in text:
            raise AssertionError(f"fixture edit for {key!r} does not match: {old!r}")
        return text.replace(old, new, 1)

    root.mkdir(parents=True, exist_ok=True)
    (root / "Cargo.toml").write_text(
        apply("gwz-core", core_manifest(core_version, internal_version, crates)),
        encoding="utf-8",
    )
    for name in crates:
        directory = root / "crates" / directory_of(name)
        directory.mkdir(parents=True, exist_ok=True)
        (directory / "Cargo.toml").write_text(
            apply(name, crate_manifest(name, internal_version)), encoding="utf-8"
        )
    return root


class CrateVersionGateTests(unittest.TestCase):
    """Every rule of the gate, positive first."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.gate = load_gate()

    def findings(self, **kwargs) -> list[str]:
        """The gate's findings over a freshly rendered tree."""
        tag = kwargs.pop("tag", None)
        with tempfile.TemporaryDirectory() as temporary:
            root = write_tree(Path(temporary) / "gwz-core", **kwargs)
            findings, _ = self.gate.run(root, tag)
        return findings

    def assert_names(self, findings: list[str], crate: str, needle: str) -> None:
        """Exactly the expected rule fired, and its finding names the crate."""
        matching = [finding for finding in findings if needle in finding]
        self.assertTrue(matching, f"no finding mentions {needle!r}; got {findings}")
        for finding in matching:
            self.assertIn(crate, finding)

    # --- positive ---

    def test_a_well_formed_tree_passes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = write_tree(Path(temporary) / "gwz-core")
            findings, summary = self.gate.run(root, "1.0.12")
        self.assertEqual([], findings)
        self.assertEqual("1.0.12", summary.core_version)
        self.assertEqual("0.0.1", summary.internal_version)
        self.assertEqual(14, summary.crates)
        self.assertEqual(13, summary.published)
        self.assertEqual(1, summary.unpublished)
        self.assertEqual(14, summary.versioned_edges)  # 13 from core, 1 inside a crate
        self.assertEqual(2, summary.path_only_edges)

    def test_the_real_gwz_core_tree_passes(self) -> None:
        findings, summary = self.gate.run(REAL_ROOT, None)
        self.assertEqual([], findings)
        self.assertEqual(self.gate.EXPECTED_CRATES, summary.crates)
        self.assertEqual(13, summary.published)

    def test_a_release_candidate_version_is_accepted(self) -> None:
        self.assertEqual([], self.findings(core_version="1.0.12-rc.1", tag="1.0.12-rc.1"))

    def test_the_script_runs_green_on_the_real_tree(self) -> None:
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "--root", str(REAL_ROOT)],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("crate versions: ok (", result.stdout)

    # --- the publish order ---

    def real_order(self) -> list[str]:
        """`--print-publish-order` over the real checkout, as a publisher reads it."""
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "--root", str(REAL_ROOT), "--print-publish-order"],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(0, result.returncode, result.stderr)
        return result.stdout.split()

    def real_requirements(self) -> dict[str, set[str]]:
        """Each published crate's internal edges, from the real manifests."""
        core, crates = self.gate.discover(REAL_ROOT)
        published = {crate.name for crate in crates if crate.name not in self.gate.UNPUBLISHED}
        requirements = {
            crate.name: self.gate.internal_requirements(crate, published)
            for crate in crates
            if crate.name in published
        }
        requirements[core.name] = published
        return requirements

    def test_the_publish_order_prints_fourteen_names_ending_at_the_core(self) -> None:
        order = self.real_order()
        self.assertEqual(14, len(order), order)
        self.assertEqual("gwz-core", order[-1])
        self.assertEqual(len(set(order)), len(order), order)
        self.assertNotIn(FIXTURES_CRATE, order)
        self.assertEqual(set(PLAN_LAYERS), set(order))

    def test_the_publish_order_places_every_crate_after_its_dependencies(self) -> None:
        order = self.real_order()
        position = {name: index for index, name in enumerate(order)}
        requirements = self.real_requirements()
        for name in order:
            for dependency in sorted(requirements[name]):
                self.assertLess(
                    position[dependency],
                    position[name],
                    f"{name} publishes before its dependency {dependency}: {order}",
                )

    def test_the_publish_order_matches_the_plan_layers(self) -> None:
        # Plan section 1's layers are the claim; the manifests are the fact.
        # Every declared edge must cross from a higher layer to a lower one,
        # and each layer must hold the crates the plan puts in it.
        requirements = self.real_requirements()
        for name, needs in sorted(requirements.items()):
            for dependency in sorted(needs):
                self.assertGreater(
                    PLAN_LAYERS[name],
                    PLAN_LAYERS[dependency],
                    f"{name} (layer {PLAN_LAYERS[name]}) depends on {dependency} "
                    f"(layer {PLAN_LAYERS[dependency]}), which plan section 1 does not allow",
                )
        sizes = Counter(PLAN_LAYERS.values())
        self.assertEqual({1: 3, 2: 6, 3: 3, 4: 1, 5: 1}, dict(sizes))
        self.assertEqual(set(self.real_order()), set(PLAN_LAYERS))

    def test_a_dependency_cycle_has_no_publish_order(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = write_tree(
                Path(temporary) / "gwz-core",
                edit={
                    EDGE_TARGET: (
                        'keywords = ["gwz", "git", "workspace", "internal"]',
                        'keywords = ["gwz", "git", "workspace", "internal"]\n\n'
                        "[dependencies]\n"
                        f'{EDGE_CRATE} = {{ path = "../{directory_of(EDGE_CRATE)}", '
                        'version = "0.0.1" }',
                    )
                },
            )
            core, crates = self.gate.discover(root)
            with self.assertRaises(self.gate.GateError):
                self.gate.publish_order(core, crates)

    # --- gwz-core's product version ---

    def test_a_non_product_core_version_is_rejected(self) -> None:
        findings = self.findings(core_version="1.0")
        self.assert_names(findings, "gwz-core", "is not a plain X.Y.Z")

    def test_a_core_version_off_the_tag_is_rejected(self) -> None:
        findings = self.findings(core_version="1.0.12", tag="1.0.13")
        self.assert_names(findings, "gwz-core", "does not match the release tag")

    # --- the internal 0.0.N line ---

    def test_an_internal_version_off_the_line_is_rejected(self) -> None:
        # Every crate left the line, so every crate is named in its own finding.
        findings = self.findings(internal_version="0.1.0")
        off_the_line = [
            finding for finding in findings if "is not on the internal 0.0.N line" in finding
        ]
        self.assertEqual(len(INTERNAL), len(off_the_line))
        for name in INTERNAL:
            self.assertTrue(
                any(finding.startswith(f"{name}: ") for finding in off_the_line),
                f"no finding names {name}; got {off_the_line}",
            )

    def test_one_crate_off_the_shared_line_is_rejected(self) -> None:
        findings = self.findings(
            edit={EDGE_TARGET: ('version = "0.0.1"', 'version = "0.0.2"')}
        )
        self.assert_names(findings, EDGE_TARGET, "differs from the internal lockstep version")

    # --- internal dependency edges ---

    def test_a_dependency_edge_without_a_version_is_rejected(self) -> None:
        findings = self.findings(
            edit={
                "gwz-core": (
                    f'{EDGE_TARGET} = {{ path = "crates/{directory_of(EDGE_TARGET)}", '
                    'version = "0.0.1" }',
                    f'{EDGE_TARGET} = {{ path = "crates/{directory_of(EDGE_TARGET)}" }}',
                )
            }
        )
        self.assert_names(findings, "gwz-core", "has no `version`")

    def test_a_dependency_edge_on_the_wrong_version_is_rejected(self) -> None:
        findings = self.findings(
            edit={
                EDGE_CRATE: (
                    f'{EDGE_TARGET} = {{ path = "../{directory_of(EDGE_TARGET)}", '
                    'version = "0.0.1" }',
                    f'{EDGE_TARGET} = {{ path = "../{directory_of(EDGE_TARGET)}", '
                    'version = "0.0.9" }',
                )
            }
        )
        self.assert_names(findings, EDGE_CRATE, "but the internal lockstep version is")

    def test_a_dependency_edge_without_a_path_is_rejected(self) -> None:
        findings = self.findings(
            edit={
                EDGE_CRATE: (
                    f'{EDGE_TARGET} = {{ path = "../{directory_of(EDGE_TARGET)}", '
                    'version = "0.0.1" }',
                    f'{EDGE_TARGET} = "0.0.1"',
                )
            }
        )
        self.assert_names(findings, EDGE_CRATE, "without a `path`")

    # --- internal dev-dependency edges ---

    def test_a_versioned_dev_dependency_is_rejected(self) -> None:
        findings = self.findings(
            edit={
                "gwz-core": (
                    f'{FIXTURES_CRATE} = {{ path = "crates/{directory_of(FIXTURES_CRATE)}" }}',
                    f'{FIXTURES_CRATE} = {{ path = "crates/{directory_of(FIXTURES_CRATE)}", '
                    'version = "0.0.1" }',
                )
            }
        )
        self.assert_names(findings, "gwz-core", "an internal dev-dependency stays path-only")

    def test_a_dev_dependency_without_a_path_is_rejected(self) -> None:
        findings = self.findings(
            edit={
                "gwz-core": (
                    f'{FIXTURES_CRATE} = {{ path = "crates/{directory_of(FIXTURES_CRATE)}" }}',
                    f'{FIXTURES_CRATE} = {{ version = "0.0.1" }}',
                )
            }
        )
        self.assert_names(findings, "gwz-core", "without a `path`")

    # --- git sources ---

    def test_a_git_dependency_is_rejected(self) -> None:
        findings = self.findings(
            edit={
                "gwz-core": (
                    'serde = { version = "1", features = ["derive"] }',
                    'taut-shape = { git = "https://github.com/owebeeone/taut-shape-rs", '
                    'rev = "7fd171b" }',
                )
            }
        )
        self.assert_names(findings, "gwz-core", "uses the git source")

    def test_a_git_dependency_inside_a_crate_is_rejected(self) -> None:
        findings = self.findings(
            edit={
                EDGE_CRATE: (
                    f'{EDGE_TARGET} = {{ path = "../{directory_of(EDGE_TARGET)}", '
                    'version = "0.0.1" }',
                    f'{EDGE_TARGET} = {{ git = "https://example.invalid/{EDGE_TARGET}" }}',
                )
            }
        )
        self.assert_names(findings, EDGE_CRATE, "uses the git source")

    # --- publication metadata ---

    def test_a_published_crate_missing_metadata_is_rejected(self) -> None:
        for field, line in (
            ("repository", 'repository = "https://github.com/owebeeone/gwz-core"\n'),
            ("readme", 'readme = "README.md"\n'),
            ("license", 'license = "GPL-2.0-only"\n'),
            ("description", f'description = "An internal component crate of GWZ: {EDGE_CRATE}."\n'),
        ):
            with self.subTest(field=field):
                findings = self.findings(edit={EDGE_CRATE: (line, "")})
                self.assert_names(findings, EDGE_CRATE, f"has no {field}")

    def test_a_published_crate_withheld_from_the_registry_is_rejected(self) -> None:
        findings = self.findings(
            edit={EDGE_CRATE: ('readme = "README.md"', 'readme = "README.md"\npublish = false')}
        )
        self.assert_names(findings, EDGE_CRATE, "one of the thirteen published internal crates")

    def test_the_fixtures_crate_must_keep_publish_false(self) -> None:
        findings = self.findings(edit={FIXTURES_CRATE: ("publish = false\n", "")})
        self.assert_names(findings, FIXTURES_CRATE, "must keep `publish = false`")

    def test_a_renamed_fixtures_crate_is_reported(self) -> None:
        renamed = tuple(
            "gwz-local-fixtures" if name == FIXTURES_CRATE else name for name in INTERNAL
        )
        findings = self.findings(crates=renamed)
        self.assert_names(findings, FIXTURES_CRATE, "no manifest under")

    # --- the crate set itself ---

    def test_a_crate_count_off_fourteen_is_rejected(self) -> None:
        findings = self.findings(crates=INTERNAL[:-1])
        self.assert_names(findings, "crates/", "but the publish order and")
        self.assertIn(INTERNAL[0], findings[0])

    # --- input errors, never a pass ---

    def test_a_missing_crates_directory_is_an_error(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "gwz-core"
            write_tree(root)
            for manifest in sorted((root / "crates").glob("*/Cargo.toml")):
                manifest.unlink()
                manifest.parent.rmdir()
            (root / "crates").rmdir()
            with self.assertRaises(self.gate.GateError):
                self.gate.run(root, None)

    def test_a_wrong_root_package_is_an_error(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = write_tree(
                Path(temporary) / "gwz-core",
                edit={"gwz-core": ('name = "gwz-core"', 'name = "gwz-cli"')},
            )
            with self.assertRaises(self.gate.GateError):
                self.gate.run(root, None)

    def test_an_unparseable_manifest_is_an_error(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = write_tree(Path(temporary) / "gwz-core")
            (root / "crates" / directory_of(EDGE_CRATE) / "Cargo.toml").write_text(
                "[package\nname =", encoding="utf-8"
            )
            with self.assertRaises(self.gate.GateError):
                self.gate.run(root, None)

    def test_a_malformed_tag_exits_two(self) -> None:
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "--root", str(REAL_ROOT), "--tag", "1.0.12"],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(2, result.returncode)
        self.assertIn("crate versions: error:", result.stderr)


if __name__ == "__main__":
    unittest.main()

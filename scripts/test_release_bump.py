#!/usr/bin/env python3
"""Unit tests for the release script's internal-version bump (plan S1.5, D2).

The bump advances one lockstep line: `[package].version` in the fourteen
crates under `crates/`, and every `gwz-*` edge of a `[dependencies]` or
`[build-dependencies]` table in gwz-core's manifest and in the crates' own.
Dev-dependency edges stay untouched, because cargo drops a path-only dev edge
from a published manifest and a versioned one would have to exist on
crates.io.

Each test copies the real manifests -- root plus `crates/*/Cargo.toml` -- into
a temporary directory and runs the helper against that copy, so the rehearsal
is over the tree that actually ships and the real tree is never written. A
copy is a full crate layout as far as the helper and
`scripts/checks/check_crate_versions.py --root` are concerned: both read
manifests only.

Also here: `read_publish_order` really consumes the gate's
`--print-publish-order` lines rather than a list written down twice, and
`sync_manifests_to_worktree` carries every crate manifest into the standalone
cargo worktree (without it the lock regenerated there would not see the new
internal versions).
"""

from __future__ import annotations

import contextlib
import importlib.util
import io
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RELEASE_PATH = ROOT / "scripts" / "release.py"
CRATE_VERSIONS = ROOT / "scripts" / "checks" / "check_crate_versions.py"
SPEC = importlib.util.spec_from_file_location("gwz_core_release_bump", RELEASE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load release script")
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


def copy_manifests(destination: Path) -> Path:
    """The real manifest tree, copied where a test may rewrite it."""
    destination.mkdir(parents=True, exist_ok=True)
    shutil.copy2(ROOT / "Cargo.toml", destination / "Cargo.toml")
    for manifest in sorted((ROOT / "crates").glob("*/Cargo.toml")):
        crate = destination / "crates" / manifest.parent.name
        crate.mkdir(parents=True, exist_ok=True)
        shutil.copy2(manifest, crate / "Cargo.toml")
    return destination


def package_version(manifest: Path) -> str:
    return tomllib.loads(manifest.read_text(encoding="utf-8"))["package"]["version"]


def edge_versions(manifest: Path, kinds: tuple[str, ...]) -> list[str]:
    """Every version a `gwz-*` edge of the given table kinds requires."""
    tables = tomllib.loads(manifest.read_text(encoding="utf-8"))
    candidates = [tables, *(value for value in tables.get("target", {}).values())]
    found: list[str] = []
    for table in candidates:
        for kind in kinds:
            entries = table.get(kind)
            if not isinstance(entries, dict):
                continue
            for key, value in entries.items():
                spec = value if isinstance(value, dict) else {"version": value}
                package = str(spec.get("package", key))
                if package.startswith("gwz-") and "version" in spec:
                    found.append(str(spec["version"]))
    return found


class InternalBumpTests(unittest.TestCase):
    """The bump, over a copy of the real manifests."""

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.tree = copy_manifests(Path(self.temporary.name) / "gwz-core")
        self.manifests = [self.tree / "Cargo.toml", *sorted((self.tree / "crates").glob("*/Cargo.toml"))]

    def bump_once(self) -> tuple[str, str, bool]:
        current = release.read_internal_version(self.tree)
        following = release.next_internal_version(current)
        changed = release.bump_internal_version(current, following, self.tree)
        return current, following, changed

    def test_the_real_tree_is_on_one_internal_line(self) -> None:
        current = release.read_internal_version(self.tree)
        self.assertRegex(current, r"^0\.0\.[0-9]+$")
        self.assertEqual(14, len(self.manifests) - 1)

    def test_one_bump_advances_every_crate_and_every_publishable_edge(self) -> None:
        current, following, changed = self.bump_once()
        self.assertTrue(changed)
        self.assertEqual(f"0.0.{int(current.rsplit('.', 1)[1]) + 1}", following)
        for manifest in self.manifests[1:]:
            self.assertEqual(following, package_version(manifest), manifest)
        self.assertEqual(following, release.read_internal_version(self.tree))
        edges = 0
        for manifest in self.manifests:
            required = edge_versions(manifest, release.VERSIONED_KINDS)
            self.assertEqual([following] * len(required), required, manifest)
            edges += len(required)
        self.assertEqual(32, edges)

    def test_the_bump_leaves_dev_dependency_edges_alone(self) -> None:
        before = {
            manifest: edge_versions(manifest, ("dev-dependencies",))
            for manifest in self.manifests
        }
        # The internals' dev edges are path-only today, so "untouched" means
        # they still carry no version at all after the bump.
        self.assertEqual([], sorted({version for versions in before.values() for version in versions}))
        self.bump_once()
        for manifest in self.manifests:
            self.assertEqual(
                before[manifest], edge_versions(manifest, ("dev-dependencies",)), manifest
            )
            text = manifest.read_text(encoding="utf-8")
            self.assertNotRegex(text, r"\[dev-dependencies\][^\[]*version\s*=\s*\"0\.0\.")

    def test_the_bump_preserves_comments_and_formatting(self) -> None:
        before = {manifest: manifest.read_text(encoding="utf-8") for manifest in self.manifests}
        _, following, _ = self.bump_once()
        for manifest in self.manifests:
            original = before[manifest]
            after = manifest.read_text(encoding="utf-8")
            self.assertEqual(len(original.splitlines()), len(after.splitlines()), manifest)
            comments = [line for line in original.splitlines() if line.lstrip().startswith("#")]
            for comment in comments:
                self.assertIn(comment, after, manifest)
            # Only version strings moved: every other line is byte-identical.
            for old, new in zip(original.splitlines(), after.splitlines()):
                if old != new:
                    self.assertIn(f'"{following}"', new)
                    self.assertNotIn('"{}"'.format(following), old)

    def test_a_second_bump_reports_no_change(self) -> None:
        current, following, _ = self.bump_once()
        self.assertFalse(release.bump_internal_version(current, following, self.tree))

    def test_a_bumped_tree_still_passes_the_lockstep_gate(self) -> None:
        _, following, _ = self.bump_once()
        result = subprocess.run(
            [sys.executable, str(CRATE_VERSIONS), "--root", str(self.tree)],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn(f"at {following}", result.stdout)

    def test_crates_that_disagree_on_the_version_fail_and_are_named(self) -> None:
        # Break the copy relative to whatever line the real tree is on; a
        # hard-coded version silently stops breaking anything once the line
        # moves past it, and the refusal below is then never exercised.
        current = release.read_internal_version(self.tree)
        odd = self.tree / "crates" / "family-model" / "Cargo.toml"
        before = odd.read_text(encoding="utf-8")
        after = before.replace(f'version = "{current}"', 'version = "0.0.9"', 1)
        self.assertNotEqual(before, after, "the fixture did not change the package version")
        odd.write_text(after, encoding="utf-8")
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr), self.assertRaises(SystemExit):
            release.read_internal_version(self.tree)
        message = stderr.getvalue()
        self.assertIn("disagree on the internal version", message)
        self.assertIn("gwz-family-model", message)
        self.assertIn("gwz-repo-contract", message)

    def test_an_edge_the_bump_cannot_reach_fails_rather_than_being_skipped(self) -> None:
        # A renamed edge (`package = "gwz-..."`) is a real internal edge that
        # the line-targeted rewrite does not match. It must stop the release,
        # not be left behind on the previous version.
        current = release.read_internal_version(self.tree)
        following = release.next_internal_version(current)
        manifest = self.tree / "crates" / "history-check" / "Cargo.toml"
        before = manifest.read_text(encoding="utf-8")
        after = before.replace(
            f'gwz-repo-contract = {{ path = "../repo-contract", version = "{current}" }}',
            'contract = { package = "gwz-repo-contract", path = "../repo-contract", '
            f'version = "{current}" }}',
            1,
        )
        self.assertNotEqual(before, after, "the fixture did not rename the edge")
        manifest.write_text(after, encoding="utf-8")
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr), self.assertRaises(SystemExit):
            release.bump_internal_version(current, following, self.tree)
        self.assertIn("an edge is spelled in a way the bump cannot reach", stderr.getvalue())


class PublishOrderTests(unittest.TestCase):
    """The order the packaging gate follows comes from the gate script."""

    def test_the_release_script_consumes_the_gate_s_publish_order(self) -> None:
        order = release.read_publish_order(cargo_root=ROOT)
        self.assertEqual(14, len(order))
        self.assertEqual("gwz-core", order[-1])
        self.assertNotIn("gwz-local-testrepo", order)
        printed = subprocess.run(
            [sys.executable, str(CRATE_VERSIONS), "--root", str(ROOT), "--print-publish-order"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout.split()
        self.assertEqual(printed, order)

    def test_the_script_holds_no_hand_written_publish_order(self) -> None:
        source = RELEASE_PATH.read_text(encoding="utf-8")
        self.assertIn("--print-publish-order", source)
        internals = re.findall(r'"gwz-(?!core)[a-z-]+"', source)
        self.assertEqual([], internals, internals)


class WorktreeSyncTests(unittest.TestCase):
    """Every bumped manifest reaches the standalone cargo worktree."""

    def test_sync_copies_the_root_manifest_and_every_crate_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            worktree = Path(temporary) / "gwz-core"
            worktree.mkdir()
            release.sync_manifests_to_worktree(worktree)
            copied = sorted(
                path.relative_to(worktree).as_posix() for path in worktree.rglob("Cargo.toml")
            )
        expected = sorted(
            manifest.relative_to(ROOT).as_posix()
            for manifest in [ROOT / "Cargo.toml", *release.crate_manifests()]
        )
        self.assertEqual(expected, copied)
        self.assertEqual(15, len(copied))


if __name__ == "__main__":
    unittest.main()

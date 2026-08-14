#!/usr/bin/env python3

import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("check_checked_artifact_boundaries.py")
ROOT = SCRIPT.parents[2]
SOURCE = ROOT / "src"


def run(source: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), "--source", str(source)],
        check=False,
        capture_output=True,
        text=True,
    )


def run_compiler_probe(mutator) -> subprocess.CompletedProcess[str]:
    temporary = tempfile.TemporaryDirectory()
    target = Path(temporary.name) / "gwz-core"
    for name in (".github", "dev-docs", "scripts", "src", "tests", "protocol"):
        shutil.copytree(ROOT / name, target / name)
    target.mkdir(exist_ok=True)
    for name in ("Cargo.toml", "Cargo.lock", "clippy.toml", "rust-toolchain.toml"):
        shutil.copy2(ROOT / name, target / name)
    mutator(target)
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(ROOT.parent / "target" / "checked-boundary-probe")
    env["CLIPPY_CONF_DIR"] = str(target)
    try:
        return subprocess.run(
            [
                "cargo",
                "+1.95.0",
                "clippy",
                "--manifest-path",
                str(target / "Cargo.toml"),
                "--all-targets",
                "--all-features",
                "--",
                "-D",
                "warnings",
            ],
            check=False,
            capture_output=True,
            text=True,
            env=env,
        )
    finally:
        temporary.cleanup()


class CheckedArtifactBoundaryTest(unittest.TestCase):
    def test_current_source_inventory_is_classified(self) -> None:
        result = run(SOURCE)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("checked-artifact boundary: ok", result.stdout)

    def test_compiler_resolved_writer_gate_is_scoped_to_the_closed_boundary(self) -> None:
        clippy = (ROOT / "clippy.toml").read_text(encoding="utf-8")
        crate = (SOURCE / "lib.rs").read_text(encoding="utf-8")
        protected = [
            "checked_artifact/entry.rs",
            "workspace_ops/merge/root/artifact_facts.rs",
            "git/gitbackend/preservation_root/files.rs",
            "git/gitbackend/preservation_image.rs",
            "workspace_ops/merge/preserve/checked_bundle.rs",
        ]
        self.assertIn("clippy::disallowed_methods", crate)
        self.assertIn("raw writers are isolated from the checked merge boundary", crate)
        self.assertIn('path = "std::fs::write"', clippy)
        self.assertIn('path = "gwz_core::artifact::write_atomic"', clippy)
        self.assertIn('path = "gwz_core::stash::write_bundle"', clippy)
        for relative in protected:
            self.assertIn(
                "#![forbid(clippy::disallowed_methods)]",
                (SOURCE / relative).read_text(encoding="utf-8"),
                relative,
            )

    def test_compiler_resolved_writer_boundary_cannot_be_disabled(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "workspace_ops/merge/preserve/checked_bundle.rs"
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                "#![forbid(clippy::disallowed_methods)]\n", "", 1
            ),
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("compiler-resolved writer boundary is not fail-closed", result.stderr)

    def test_commented_compiler_boundary_is_not_executable_protection(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "workspace_ops/merge/root/artifact_facts.rs"
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                "#![forbid(clippy::disallowed_methods)]",
                "// #![forbid(clippy::disallowed_methods)]",
                1,
            ),
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("compiler-resolved writer boundary is not fail-closed", result.stderr)

    def test_complete_source_allowlist_rejects_unlisted_std_writer_alias(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "workspace_ops/merge/root/artifact_facts.rs"
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                ") -> ModelResult<()> {\n"
                "    crate::checked_artifact::entry::replace_merge_root_artifact(",
                ") -> ModelResult<()> {\n"
                "    let map_transition = std::fs::copy;\n"
                "    let _ = map_transition(relative, \".gwz/raw-copy\");\n"
                "    crate::checked_artifact::entry::replace_merge_root_artifact(",
                1,
            ),
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("protected source allowlist changed", result.stderr)

    def test_complete_source_allowlist_rejects_unlisted_crate_writer_alias(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "workspace_ops/merge/root/artifact_facts.rs"
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                ") -> ModelResult<()> {\n"
                "    crate::checked_artifact::entry::replace_merge_root_artifact(",
                ") -> ModelResult<()> {\n"
                "    let map_transition = "
                "crate::workspace_ops::publish_workspace_exclude_candidate;\n"
                "    map_transition(root, \"unchecked boundary replacement\")?;\n"
                "    crate::checked_artifact::entry::replace_merge_root_artifact(",
                1,
            ),
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("protected source allowlist changed", result.stderr)

    def test_concrete_preservation_observer_is_inside_the_source_allowlist(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "git/gitbackend/preservation_image.rs"
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                ") -> ModelResult<Vec<GitPreservationStashEvidence>> {\n",
                ") -> ModelResult<Vec<GitPreservationStashEvidence>> {\n"
                "    std::fs::write(root.join(\"raw-observer\"), b\"bypass\")\n"
                "        .map_err(crate::git::io_error)?;\n",
                1,
            ),
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("protected source allowlist changed", result.stderr)

    def test_production_observer_delegate_cannot_leave_its_protected_leaf(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "git/gitbackend.rs"
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                "pub(crate) fn observe_preservation_stashes_read_only(",
                "pub(crate) fn unreviewed_preservation_stashes_read_only(",
                1,
            ),
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(
            "production preservation observer no longer terminates in its protected leaf",
            result.stderr,
        )

    def test_checked_bundle_observer_does_not_dispatch_through_open_backend(self) -> None:
        checked_bundle = (
            SOURCE / "workspace_ops/merge/preserve/checked_bundle.rs"
        ).read_text(encoding="utf-8")
        gitbackend = (SOURCE / "git/gitbackend.rs").read_text(encoding="utf-8")
        contract = (SOURCE / "git/gitbackend/contract.rs").read_text(encoding="utf-8")
        self.assertNotIn("backend.preservation_stashes", checked_bundle)
        self.assertNotIn("fn preservation_stashes(", contract)
        self.assertIn(
            "crate::git::observe_preservation_stashes_read_only", checked_bundle
        )
        self.assertIn(
            "pub(crate) fn observe_preservation_stashes_read_only", gitbackend
        )

    def test_open_backend_observer_cannot_reenter_merge_authority(self) -> None:
        result = self.append(
            "workspace_ops/merge/preserve/plan.rs",
            "\nfn unreviewed<B: crate::git::GitBackend>(backend: &B, root: &Path) {\n"
            "    let _ = backend.preservation_stashes(root, \"merge_1\");\n"
            "}\n",
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("reintroduced the open GitBackend preservation observer", result.stderr)

    def test_preservation_observer_cannot_reenter_the_open_trait(self) -> None:
        result = self.append(
            "git/gitbackend/contract.rs",
            "\nfn preservation_stashes(path: &Path, merge_id: &str);\n",
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("reintroduced into the trait contract", result.stderr)

    def copied_source(self) -> tuple[tempfile.TemporaryDirectory[str], Path]:
        temporary = tempfile.TemporaryDirectory()
        target = Path(temporary.name) / "src"
        shutil.copytree(SOURCE, target)
        return temporary, target

    def append(self, relative: str, text: str) -> subprocess.CompletedProcess[str]:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / relative
        path.write_text(path.read_text(encoding="utf-8") + text, encoding="utf-8")
        return run(source)

    def test_ordinary_unqualified_import_and_call_is_rejected(self) -> None:
        result = self.append(
            "workspace_ops/handle_stash/commands.rs",
            "\nuse crate::checked_artifact::entry::observe_merge_root_artifact;\n"
            "fn bypass(root: &std::path::Path) { let _ = "
            "observe_merge_root_artifact(root, std::path::Path::new(\"x\")); }\n",
        )
        self.assertNotEqual(result.returncode, 0)

    def test_reexported_entry_is_rejected(self) -> None:
        result = self.append(
            "workspace_ops/handle_stash/shared.rs",
            "\npub(crate) use crate::checked_artifact::entry::"
            "replace_merge_root_artifact as unchecked_replace;\n",
        )
        self.assertNotEqual(result.returncode, 0)

    def test_unclassified_visible_item_is_rejected(self) -> None:
        result = self.append(
            "checked_artifact/entry.rs", "\npub(crate) struct Surprise;\n"
        )
        self.assertNotEqual(result.returncode, 0)

    def test_entry_reexport_is_rejected(self) -> None:
        result = self.append(
            "checked_artifact/entry.rs",
            "\npub(crate) use crate::stash::write_bundle as raw_bundle_writer;\n",
        )
        self.assertNotEqual(result.returncode, 0)

    def test_entry_cannot_grow_a_direct_raw_writer(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "checked_artifact/entry.rs"
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                "pub(crate) fn prepare_merge_store_parents(root: &Path) -> ModelResult<()> {",
                "pub(crate) fn prepare_merge_store_parents(root: &Path) -> ModelResult<()> {\n"
                "    std::fs::write(root.join(\"raw\"), b\"bypass\").unwrap();",
            ),
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checked entry call graph changed", result.stderr)

    def test_general_checked_capability_escape_is_rejected(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "checked_artifact/mod.rs"
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                "struct CheckedArtifact {", "pub(crate) struct CheckedArtifact {"
            ),
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)

    def test_allowed_adapter_cannot_reexport_a_new_entry(self) -> None:
        result = self.append(
            "workspace_ops/merge/root/artifact_facts.rs",
            "\npub(in crate::workspace_ops::merge) use "
            "crate::checked_artifact::entry::replace_merge_preservation_bundle;\n",
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("visible-item inventory changed", result.stderr)

    def test_executable_aliased_raw_writer_in_adapter_is_rejected(self) -> None:
        result = self.append(
            "workspace_ops/merge/root/artifact_facts.rs",
            "\nuse std::fs::write as unchecked_write;\n"
            "fn bypass(root: &Path, bytes: &[u8]) { "
            "unchecked_write(root.join(\"raw\"), bytes).unwrap(); }\n",
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checked adapter call graph changed", result.stderr)

    def test_alias_import_is_rejected_even_when_it_reuses_an_allowed_call_name(self) -> None:
        result = self.append(
            "workspace_ops/merge/root/artifact_facts.rs",
            "\nuse std::fs::write as Ok;\n",
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checked adapter import inventory changed", result.stderr)

    def test_transitive_non_std_writer_in_adapter_is_rejected(self) -> None:
        result = self.append(
            "workspace_ops/merge/preserve/checked_bundle.rs",
            "\nfn bypass(root: &Path, bytes: &[u8]) { "
            "crate::stash::write_bundle(root, bytes).unwrap(); }\n",
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("checked adapter call graph changed", result.stderr)

    def test_removed_external_bundle_helper_cannot_reappear(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "workspace_ops/merge/preserve/checked_bundle.rs"
        path.write_text(
            path.read_text(encoding="utf-8")
            + "\nuse super::artifacts::expected_bundle as unchecked_expected_bundle;\n",
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)

    def test_external_owner_helper_cannot_reenter_the_checked_adapter_graph(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "workspace_ops/merge/preserve/checked_bundle.rs"
        path.write_text(
            path.read_text(encoding="utf-8")
            + "\nuse super::plan::v1_owner_evidence as unchecked_owner_evidence;\n",
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)

    def assert_compiler_rejects(self, mutator, method: str) -> None:
        result = run_compiler_probe(mutator)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(f"use of a disallowed method `{method}`", result.stderr)

    def test_same_name_function_pointer_raw_writer_is_rejected(self) -> None:
        def mutate(root: Path) -> None:
            path = root / "src/workspace_ops/merge/root/artifact_facts.rs"
            path.write_text(
                path.read_text(encoding="utf-8").replace(
                ") -> ModelResult<()> {\n"
                "    crate::checked_artifact::entry::replace_merge_root_artifact(",
                ") -> ModelResult<()> {\n"
                "    let map_transition = std::fs::write;\n"
                "    let _ = map_transition(relative, bytes);\n"
                "    crate::checked_artifact::entry::replace_merge_root_artifact(",
                1,
                ),
                encoding="utf-8",
            )

        self.assert_compiler_rejects(mutate, "std::fs::write")

    def test_complete_checked_bundle_helper_graph_rejects_a_raw_writer(self) -> None:
        def mutate(root: Path) -> None:
            path = root / "src/workspace_ops/merge/preserve/checked_bundle.rs"
            path.write_text(
                path.read_text(encoding="utf-8").replace(
                    ") -> ModelResult<Option<&'a super::super::PreservationEvidence>> {\n",
                    ") -> ModelResult<Option<&'a super::super::PreservationEvidence>> {\n"
                    "    std::fs::write(\"raw-checked-bypass\", b\"bypass\").unwrap();\n",
                    1,
                ),
                encoding="utf-8",
            )

        self.assert_compiler_rejects(mutate, "std::fs::write")

    def test_compiler_resolves_non_std_writer_aliases_in_checked_adapters(self) -> None:
        def mutate(root: Path) -> None:
            path = root / "src/workspace_ops/merge/preserve/checked_bundle.rs"
            path.write_text(
                path.read_text(encoding="utf-8").replace(
                    "let index = owner_index(plans, owner)?;",
                    "{\n"
                    "        let owner_error = crate::artifact::write_atomic;\n"
                    "        owner_error(root, \"bypass\")?;\n"
                    "    }\n"
                    "    let index = owner_index(plans, owner)?;",
                    1,
                ),
                encoding="utf-8",
            )

        self.assert_compiler_rejects(mutate, "gwz_core::artifact::write_atomic")

    def test_comments_and_strings_do_not_create_false_references(self) -> None:
        result = self.append(
            "workspace_ops/handle_stash/shared.rs",
            "\n// crate::checked_artifact::entry::observe_merge_root_artifact(root, path)\n"
            "const NOTE: &str = \"CheckedArtifact::acquire(\";\n",
        )
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()

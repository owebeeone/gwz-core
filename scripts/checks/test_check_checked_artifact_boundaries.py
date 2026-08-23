#!/usr/bin/env python3

import os
import shutil
import subprocess
import sys
import tempfile
import tomllib
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

    def test_alias_bound_raw_rename_in_catalog_provider_is_rejected(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = (
            source
            / "checked_artifact/capability/pre_catalog/provider/retained.rs"
        )
        path.write_text(
            path.read_text(encoding="utf-8")
            + "\nuse crate::checked_artifact::platform::rename_relative as relocate_entry;\n"
            + "fn probe_alias_publish(dir: &cap_std::fs::Dir) {\n"
            + "    let _ = relocate_entry(\n"
            + "        dir, std::ffi::OsStr::new(\"a\"), dir,\n"
            + "        std::ffi::OsStr::new(\"b\"), false,\n"
            + "        crate::model::ErrorCode::IoError, \"probe\");\n}\n",
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(
            "raw rename caller outside the sealed publication seam", result.stderr
        )

    def test_fn_pointer_bound_raw_rename_in_legacy_interior_is_rejected(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "checked_artifact/cleanup.rs"
        path.write_text(
            path.read_text(encoding="utf-8")
            + "\nfn probe_pointer_cleanup(dir: &cap_std::fs::Dir) {\n"
            + "    let publish = crate::checked_artifact::platform::rename_relative;\n"
            + "    let _ = publish(\n"
            + "        dir, std::ffi::OsStr::new(\"a\"), dir,\n"
            + "        std::ffi::OsStr::new(\"b\"), false,\n"
            + "        crate::model::ErrorCode::IoError, \"probe\");\n}\n",
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(
            "raw rename caller outside the sealed publication seam", result.stderr
        )

    def test_raw_rename_caller_in_catalog_provider_is_rejected(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = (
            source
            / "checked_artifact/capability/pre_catalog/provider/retained.rs"
        )
        path.write_text(
            path.read_text(encoding="utf-8")
            + "\nfn probe_raw_publish(dir: &cap_std::fs::Dir) {\n"
            + "    let _ = crate::checked_artifact::platform::rename_relative(\n"
            + "        dir, std::ffi::OsStr::new(\"a\"), dir,\n"
            + "        std::ffi::OsStr::new(\"b\"), false,\n"
            + "        crate::model::ErrorCode::IoError, \"probe\");\n}\n",
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(
            "raw rename caller outside the sealed publication seam", result.stderr
        )

    def test_raw_rename_caller_in_legacy_interior_is_rejected(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "checked_artifact/cleanup.rs"
        path.write_text(
            path.read_text(encoding="utf-8")
            + "\nfn probe_raw_cleanup(dir: &cap_std::fs::Dir) {\n"
            + "    let _ = crate::checked_artifact::platform::rename_relative(\n"
            + "        dir, std::ffi::OsStr::new(\"a\"), dir,\n"
            + "        std::ffi::OsStr::new(\"b\"), false,\n"
            + "        crate::model::ErrorCode::IoError, \"probe\");\n}\n",
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(
            "raw rename caller outside the sealed publication seam", result.stderr
        )

    def test_provisional_catalog_callback_interface_cannot_return(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "checked_artifact/leaf.rs"
        path.write_text(
            path.read_text(encoding="utf-8")
            + "\nstruct PreCatalogOwnerV1;\n",
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("provisional catalog interface was reintroduced", result.stderr)

    def test_catalog_mutation_lease_cannot_escape_its_exact_reference_set(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "checked_artifact/leaf.rs"
        path.write_text(
            path.read_text(encoding="utf-8")
            + "\nfn unreviewed_catalog_lease(_: CatalogMutationLeaseV1<'_>) {}\n",
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("catalog lease reference set changed", result.stderr)

    def test_catalog_physical_edge_cannot_gain_an_unreviewed_sibling_caller(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "checked_artifact/leaf.rs"
        path.write_text(
            path.read_text(encoding="utf-8")
            + "\nfn unreviewed_catalog_writer() { let _ = prepare_or_rewrite_staging; }\n",
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("catalog lease reference set changed", result.stderr)

    def test_catalog_publication_cannot_bypass_the_shared_source_seam(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "checked_artifact/capability/pre_catalog/provider/mutation.rs"
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                "publish_verified_no_replace(",
                "crate::checked_artifact::platform::rename_relative(",
                1,
            ),
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("catalog publication seam changed", result.stderr)

    def test_catalog_publication_seam_cannot_drop_the_open_source_rename(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "checked_artifact/capability/pre_catalog/provider/publication.rs"
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                "rename_open_source(", "rename_relative(", 1
            ),
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("catalog publication seam changed", result.stderr)

    def test_windows_exact_handle_publication_is_source_protected(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "checked_artifact/platform.rs"
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                "SetFileInformationByHandle(", "unreviewed_path_rename(", 1
            ),
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        # `checked_artifact/platform.rs` is pinned as a TREE, not a flat file:
        # R2-D Step 4.2 split the Windows durability anchor into
        # `platform/anchor.rs`, and its landing converted the pin so the new
        # child could not sit outside any manifest. This assertion named the
        # flat-pin message until R2-D Step 5.1's gate train ran the suite for
        # the first time since that conversion and caught it. Naming the pin as
        # well as the class is deliberate: the generic prefix would have passed
        # on any protected-source finding, which is what let the drift hide.
        self.assertIn(
            "protected source tree changed: checked_artifact/platform.rs",
            result.stderr,
        )

    def test_catalog_lease_tree_rejects_an_unreviewed_target_helper(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = (
            source
            / "checked_artifact/bootstrap/runtime/catalog_lease/unreviewed.rs"
        )
        path.write_text("fn unreviewed_catalog_target() {}\n", encoding="utf-8")
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("protected source tree changed", result.stderr)

    def test_git_lease_target_cannot_return_to_a_caller_selected_directory(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "checked_artifact/bootstrap/runtime/catalog_lease/target.rs"
        text = path.read_text(encoding="utf-8").replace(
            "fn repository_common_git_directory(", "fn git_directory("
        )
        path.write_text(text, encoding="utf-8")
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("must be derived from repository common-directory", result.stderr)

    def test_durable_path_tree_rejects_an_unreviewed_schema_helper(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "checked_artifact/capability/path/unreviewed.rs"
        path.write_text("fn unreviewed_durable_path_shape() {}\n", encoding="utf-8")
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("protected source tree changed", result.stderr)

    def test_compiler_resolved_writer_gate_is_scoped_to_the_closed_boundary(self) -> None:
        clippy = (ROOT / "clippy.toml").read_text(encoding="utf-8")
        crate = (SOURCE / "lib.rs").read_text(encoding="utf-8")
        protected = [
            "checked_artifact/entry.rs",
            "workspace_ops/merge/root/artifact_facts.rs",
            "git/gitbackend/preservation_root/files.rs",
            "git/gitbackend/preservation_image.rs",
            "workspace_ops/merge/preserve/checked_bundle.rs",
            "workspace_ops/merge/preserve/plan.rs",
            "workspace_ops/merge/v1_lifecycle/authority/observe.rs",
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

    def test_commented_authority_tree_boundary_is_not_executable_protection(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "workspace_ops/merge/v1_lifecycle/authority/observe.rs"
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

    def test_preservation_plan_caller_is_inside_the_source_allowlist(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "workspace_ops/merge/preserve/plan.rs"
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                "crate::git::observe_preservation_stashes_read_only(&plan.path, &record.merge_id)",
                "{ std::fs::write(plan.path.join(\"raw-plan-writer\"), b\"bypass\").unwrap(); "
                "crate::git::observe_preservation_stashes_read_only(&plan.path, &record.merge_id) }",
                1,
            ),
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("protected source allowlist changed", result.stderr)

    def test_authority_observer_tree_rejects_a_nested_writer_helper(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = (
            source
            / "workspace_ops/merge/v1_lifecycle/authority/observe/reverse/preservation/phase.rs"
        )
        path.write_text(
            path.read_text(encoding="utf-8")
            + "\nfn unreviewed_authority_writer(path: &std::path::Path) {\n"
            "    let observe_preservation_stashes_read_only = std::fs::write;\n"
            "    observe_preservation_stashes_read_only(path, b\"bypass\").unwrap();\n"
            "}\n",
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("protected source tree changed", result.stderr)

    def test_authority_observer_tree_rejects_a_new_helper_file(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = (
            source
            / "workspace_ops/merge/v1_lifecycle/authority/observe/unreviewed_helper.rs"
        )
        path.write_text(
            "fn write(path: &std::path::Path) { std::fs::write(path, b\"bypass\").unwrap(); }\n",
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("protected source tree changed", result.stderr)

    def test_authority_observer_tree_rejects_a_differently_named_backend_callback(
        self,
    ) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = (
            source
            / "workspace_ops/merge/v1_lifecycle/authority/observe/reverse/preservation/phase/evidence.rs"
        )
        path.write_text(
            path.read_text(encoding="utf-8")
            .replace("    _backend: &B,", "    backend: &B,", 1)
            .replace(
                "    let stashes =\n",
                "    let _unreviewed = backend.status(&plan.path)?;\n"
                "    let stashes =\n",
                1,
            ),
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("protected source tree changed", result.stderr)

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

    def test_concrete_observer_cannot_gain_an_unprotected_merge_caller(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "workspace_ops/merge/preserve/artifacts.rs"
        path.write_text(
            path.read_text(encoding="utf-8")
            + "\n#[cfg(test)]\n#[allow(dead_code)]\n"
            "fn unreviewed_observer_wrapper(path: &std::path::Path) "
            "-> crate::model::ModelResult<Vec<crate::git::GitPreservationStashEvidence>> {\n"
            "    std::fs::write(path.join(\"raw-observer-caller\"), b\"bypass\").unwrap();\n"
            "    crate::git::observe_preservation_stashes_read_only(path, \"merge_probe\")\n"
            "}\n",
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("concrete preservation observer caller set changed", result.stderr)

    def test_non_rs_path_module_cannot_hide_a_concrete_observer_caller(self) -> None:
        compiler = run_compiler_probe(
            lambda root: self.add_non_rs_observer_caller(root / "src", "path")
        )
        self.assertEqual(compiler.returncode, 0, compiler.stderr)
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        self.add_non_rs_observer_caller(source, "path")
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Rust source-loading edge inventory changed", result.stderr)

    def test_non_rs_include_cannot_hide_a_concrete_observer_caller(self) -> None:
        compiler = run_compiler_probe(
            lambda root: self.add_non_rs_observer_caller(root / "src", "include")
        )
        self.assertEqual(compiler.returncode, 0, compiler.stderr)
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        self.add_non_rs_observer_caller(source, "include")
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Rust source-loading edge inventory changed", result.stderr)

    def test_cfg_attr_path_cannot_hide_a_concrete_observer_caller(self) -> None:
        compiler = run_compiler_probe(
            lambda root: self.add_non_rs_observer_caller(root / "src", "cfg_attr")
        )
        self.assertEqual(compiler.returncode, 0, compiler.stderr)
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        self.add_non_rs_observer_caller(source, "cfg_attr")
        result = run(source)
        self.assertNotEqual(result.returncode, 0)

    def test_import_aliased_include_cannot_hide_a_concrete_observer_caller(
        self,
    ) -> None:
        compiler = run_compiler_probe(
            lambda root: self.add_non_rs_observer_caller(
                root / "src", "include_alias"
            )
        )
        self.assertEqual(compiler.returncode, 0, compiler.stderr)
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        self.add_non_rs_observer_caller(source, "include_alias")
        result = run(source)
        self.assertNotEqual(result.returncode, 0)

    def test_approved_outside_source_target_cannot_hide_an_observer_caller(
        self,
    ) -> None:
        compiler = run_compiler_probe(self.add_outside_source_observer_caller)
        self.assertEqual(compiler.returncode, 0, compiler.stderr)
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        self.add_outside_source_observer_caller(source.parent)
        result = run(source)
        self.assertNotEqual(result.returncode, 0)

    def test_compiler_rejects_a_redirected_v1_root(self) -> None:
        compiler = run_compiler_probe(self.redirect_v1_root)
        self.assertEqual(compiler.returncode, 0, compiler.stderr)
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        self.redirect_v1_root(source.parent)
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("compiler root manifest changed", result.stderr)

    def test_compiler_root_manifest_rejects_crate_target_redirection(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        manifest = source.parent / "Cargo.toml"
        manifest.write_text(
            manifest.read_text(encoding="utf-8").replace(
                'path = "src/lib.rs"', 'path = "src/unreviewed_lib.rs"', 1
            ),
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("compiler root manifest changed", result.stderr)

    def test_compiler_root_manifest_allows_non_target_metadata_change(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        manifest = source.parent / "Cargo.toml"
        before = manifest.read_text(encoding="utf-8")
        current = tomllib.loads(before)["package"]["version"]
        replacement = "99.0.0" if current != "99.0.0" else "98.0.0"
        after = before.replace(
            f'version = "{current}"', f'version = "{replacement}"', 1
        )
        self.assertNotEqual(before, after)
        manifest.write_text(after, encoding="utf-8")
        result = run(source)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_compiler_root_manifest_rejects_workspace_ops_redirection(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        crate = source / "lib.rs"
        crate.write_text(
            crate.read_text(encoding="utf-8").replace(
                "pub mod workspace_ops;",
                '#[cfg_attr(all(), path = "unreviewed_workspace_ops.rs")]\n'
                "pub mod workspace_ops;",
                1,
            ),
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("compiler root manifest changed", result.stderr)

    def test_compiler_root_manifest_rejects_merge_redirection(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        workspace_ops = source / "workspace_ops/mod.rs"
        workspace_ops.write_text(
            workspace_ops.read_text(encoding="utf-8").replace(
                "mod merge;",
                '#[cfg_attr(all(), path = "unreviewed_merge.rs")]\nmod merge;',
                1,
            ),
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("compiler root manifest changed", result.stderr)

    def test_v1_compiler_root_has_a_positive_sentinel(self) -> None:
        parent = (SOURCE / "workspace_ops/merge/mod.rs").read_text(encoding="utf-8")
        root = (SOURCE / "workspace_ops/merge/v1_lifecycle/mod.rs").read_text(
            encoding="utf-8"
        )
        self.assertIn(
            "const _: &str = v1_lifecycle::COMPILER_ROOT_SENTINEL;", parent
        )
        self.assertIn(
            'pub(super) const COMPILER_ROOT_SENTINEL: &str = module_path!();', root
        )

    def test_macro_spelled_source_loading_edge_is_rejected(self) -> None:
        result = self.append(
            "workspace_ops/merge/preserve/artifacts.rs",
            "\nmacro_rules! unreviewed_module {\n"
            "    ($target:literal) => { #[path = $target] mod hidden; };\n"
            "}\n"
            "unreviewed_module!(\"unreviewed_observer.inc\");\n",
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Rust source-loading edge inventory changed", result.stderr)

    def test_v1_runtime_cannot_restore_the_open_backend_bound(self) -> None:
        temporary, source = self.copied_source()
        self.addCleanup(temporary.cleanup)
        path = source / "workspace_ops/merge/v1_lifecycle/reverse.rs"
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                "MergeAuthorityBackend", "GitBackend", 1
            ),
            encoding="utf-8",
        )
        result = run(source)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("protected source tree changed", result.stderr)

    def test_merge_authority_backend_has_a_private_compiler_seal(self) -> None:
        path = SOURCE / "git/gitbackend/authority_backend.rs"
        text = path.read_text(encoding="utf-8")
        self.assertIn("mod sealed", text)
        self.assertIn("pub trait MergeAuthorityBackend", text)
        self.assertIn("sealed::Sealed", text)
        self.assertIn("for super::backend::Git2Backend", text)

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
        shutil.copytree(ROOT / "protocol", target.parent / "protocol")
        shutil.copy2(ROOT / "Cargo.toml", target.parent / "Cargo.toml")
        return temporary, target

    def redirect_v1_root(self, root: Path) -> None:
        source = root / "src"
        crate = source / "lib.rs"
        crate.write_text(
            crate.read_text(encoding="utf-8").replace(
                "#![allow(\n",
                "#![cfg_attr(test, allow(dead_code, unused_imports))]\n\n#![allow(\n",
                1,
            ),
            encoding="utf-8",
        )
        parent = source / "workspace_ops/merge/mod.rs"
        parent.write_text(
            parent.read_text(encoding="utf-8").replace(
                "#[cfg(test)]\nmod v1_lifecycle;",
                "#[cfg(test)]\n"
                '#[cfg_attr(all(), path = "unreviewed_v1.rs")]\n'
                "mod v1_lifecycle;",
                1,
            ),
            encoding="utf-8",
        )
        parent.with_name("unreviewed_v1.rs").write_text(
            'pub(super) const COMPILER_ROOT_SENTINEL: &str = "replacement";\n'
            "mod service {\n"
            "    trait ExactObserver {}\n"
            "    trait PhysicalExecutor {}\n"
            "    trait V1Runtime: ExactObserver + PhysicalExecutor {}\n"
            "    impl<T: ExactObserver + PhysicalExecutor> V1Runtime for T {}\n"
            "    struct UnreviewedRuntime;\n"
            "    impl ExactObserver for UnreviewedRuntime {}\n"
            "    impl PhysicalExecutor for UnreviewedRuntime {}\n"
            "    fn accepts_runtime<R: V1Runtime>() {}\n"
            "    #[test]\n"
            "    fn unreviewed_runtime_enters_replacement_service() {\n"
            "        accepts_runtime::<UnreviewedRuntime>();\n"
            "    }\n"
            "}\n",
            encoding="utf-8",
        )

    def add_non_rs_observer_caller(self, source: Path, form: str) -> None:
        path = source / "workspace_ops/merge/preserve/artifacts.rs"
        helper = path.with_name("unreviewed_observer.inc")
        helper.write_text(
            "pub(super) fn observe_after_write(\n"
            "    path: &std::path::Path,\n"
            ") -> crate::model::ModelResult<Vec<crate::git::GitPreservationStashEvidence>> {\n"
            "    let _ = std::fs::write(path.join(\"raw-observer-caller\"), b\"bypass\");\n"
            "    crate::git::observe_preservation_stashes_read_only(path, \"merge_probe\")\n"
            "}\n",
            encoding="utf-8",
        )
        if form == "path":
            edge = (
                "\n#[cfg(test)]\n#[path = \"unreviewed_observer.inc\"]\n"
                "mod unreviewed_observer;\n"
            )
            call = "unreviewed_observer::observe_after_write(&plan.path)?;"
        elif form == "include":
            edge = "\n#[cfg(test)]\ninclude!(\"unreviewed_observer.inc\");\n"
            call = "observe_after_write(&plan.path)?;"
        elif form == "cfg_attr":
            edge = (
                "\n#[cfg(test)]\n"
                "#[cfg_attr(test, path = \"unreviewed_observer.inc\")]\n"
                "mod unreviewed_observer;\n"
            )
            call = "unreviewed_observer::observe_after_write(&plan.path)?;"
        elif form == "include_alias":
            edge = (
                "\n#[cfg(test)]\n"
                "use std::{include as load_unreviewed_source};\n"
                "#[cfg(test)]\n"
                "load_unreviewed_source!(\"unreviewed_observer.inc\");\n"
            )
            call = "observe_after_write(&plan.path)?;"
        else:
            self.fail(f"unsupported source-loading form: {form}")
        text = path.read_text(encoding="utf-8") + edge
        text = text.replace(
            "    match v1_root_preservation_spec(backend, record, plan, attached_commit)? {",
            f"    let _ = {call}\n"
            "    match v1_root_preservation_spec(backend, record, plan, attached_commit)? {",
            1,
        )
        path.write_text(text, encoding="utf-8")

    def add_outside_source_observer_caller(self, root: Path) -> None:
        target = root / "protocol/corpus/rust/vectors.rs"
        target.write_text(
            target.read_text(encoding="utf-8").replace(
                "#[cfg(test)]\nmod conformance {",
                "pub(crate) fn observe_after_write(\n"
                "    path: &std::path::Path,\n"
                ") -> crate::model::ModelResult<Vec<crate::git::GitPreservationStashEvidence>> {\n"
                "    let _ = std::fs::write(path.join(\"raw-observer-caller\"), b\"bypass\");\n"
                "    crate::git::observe_preservation_stashes_read_only(path, \"merge_probe\")\n"
                "}\n\n"
                "#[cfg(test)]\nmod conformance {",
                1,
            ),
            encoding="utf-8",
        )
        path = root / "src/workspace_ops/merge/preserve/artifacts.rs"
        path.write_text(
            path.read_text(encoding="utf-8").replace(
                "    match v1_root_preservation_spec(backend, record, plan, attached_commit)? {",
                "    let _ = crate::protocol_corpus::observe_after_write(&plan.path)?;\n"
                "    match v1_root_preservation_spec(backend, record, plan, attached_commit)? {",
                1,
            ),
            encoding="utf-8",
        )

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

    def test_compiler_rejects_nested_writer_in_authority_observer_tree(self) -> None:
        def mutate(root: Path) -> None:
            path = (
                root
                / "src/workspace_ops/merge/v1_lifecycle/authority/observe/reverse/preservation/phase.rs"
            )
            path.write_text(
                path.read_text(encoding="utf-8")
                + "\nfn unreviewed_authority_writer(path: &std::path::Path) {\n"
                "    let observe_preservation_stashes_read_only = std::fs::write;\n"
                "    observe_preservation_stashes_read_only(path, b\"bypass\").unwrap();\n"
                "}\n",
                encoding="utf-8",
            )

        self.assert_compiler_rejects(mutate, "std::fs::write")

    def test_compiler_rejects_writer_in_preservation_plan_caller(self) -> None:
        def mutate(root: Path) -> None:
            path = root / "src/workspace_ops/merge/preserve/plan.rs"
            path.write_text(
                path.read_text(encoding="utf-8").replace(
                    "crate::git::observe_preservation_stashes_read_only(&plan.path, &record.merge_id)",
                    "{ std::fs::write(plan.path.join(\"raw-plan-writer\"), b\"bypass\").unwrap(); "
                    "crate::git::observe_preservation_stashes_read_only(&plan.path, &record.merge_id) }",
                    1,
                ),
                encoding="utf-8",
            )

        self.assert_compiler_rejects(mutate, "std::fs::write")

    def test_compiler_rejects_writer_in_v1_artifact_observer(self) -> None:
        def mutate(root: Path) -> None:
            path = root / "src/workspace_ops/merge/preserve/artifacts.rs"
            path.write_text(
                path.read_text(encoding="utf-8").replace(
                    "    match v1_root_preservation_spec(backend, record, plan, attached_commit)? {",
                    "    std::fs::write(plan.path.join(\"raw-v1-observer\"), b\"bypass\")"
                    ".unwrap();\n"
                    "    match v1_root_preservation_spec(backend, record, plan, attached_commit)? {",
                    1,
                ),
                encoding="utf-8",
            )

        self.assert_compiler_rejects(mutate, "std::fs::write")

    def test_compiler_rejects_an_alternative_merge_authority_backend(self) -> None:
        def mutate(root: Path) -> None:
            path = root / "src/workspace_ops/tests/g01/tracking_backend.rs"
            path.write_text(
                path.read_text(encoding="utf-8")
                + "\nimpl crate::git::MergeAuthorityBackend for TrackingBackend {}\n",
                encoding="utf-8",
            )

        result = run_compiler_probe(mutate)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("MergeAuthorityBackend", result.stderr)
        self.assertIn("Sealed", result.stderr)

    def test_compiler_rejects_an_unsealed_v1_runtime(self) -> None:
        def mutate(root: Path) -> None:
            path = root / "src/workspace_ops/merge/v1_lifecycle/status.rs"
            path.write_text(
                path.read_text(encoding="utf-8")
                + "\nstruct UnreviewedRuntime;\n"
                "impl super::service::ExactObserver for UnreviewedRuntime {\n"
                "    fn observe(\n"
                "        &mut self,\n"
                "        _: &super::checked::StoredV1Record,\n"
                "        _: &super::authority::BoundObservationRequest,\n"
                "    ) -> crate::model::ModelResult<super::authority::BoundExactObservation> {\n"
                "        unreachable!()\n"
                "    }\n"
                "}\n"
                "impl super::service::PhysicalExecutor for UnreviewedRuntime {\n"
                "    fn execute(\n"
                "        &mut self,\n"
                "        _: &super::checked::V1MutationLease,\n"
                "        _: &super::checked::StoredV1Record,\n"
                "        _: &super::authority::PhysicalActionKind,\n"
                "    ) -> super::authority::ExecutionDiagnostic {\n"
                "        super::authority::ExecutionDiagnostic::Success\n"
                "    }\n"
                "}\n"
                "fn accepts_v1_runtime<R: super::service::V1Runtime>() {}\n"
                "fn prove_unreviewed_runtime_is_admitted() {\n"
                "    accepts_v1_runtime::<UnreviewedRuntime>();\n"
                "}\n",
                encoding="utf-8",
            )

        result = run_compiler_probe(mutate)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("V1Runtime", result.stderr)

    def test_comments_and_strings_do_not_create_false_references(self) -> None:
        result = self.append(
            "workspace_ops/handle_stash/shared.rs",
            "\n// crate::checked_artifact::entry::observe_merge_root_artifact(root, path)\n"
            "const NOTE: &str = \"CheckedArtifact::acquire(\";\n",
        )
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()

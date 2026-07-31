from __future__ import annotations

import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import generate_retained_reader_fixtures as generator
import retained_reader_fixture as fixture_tools
import retained_reader_harness as harness
import retained_reader_evidence as evidence
import retained_reader_matrix as matrix


HERE = Path(__file__).resolve().parent


def git(root: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(root), *args],
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    ).stdout.strip()


class RetainedReaderCaseTests(unittest.TestCase):
    def test_checked_macos_evidence_matches_current_inputs_and_is_path_free(self) -> None:
        evidence_path = HERE / "evidence-macos-aarch64.json"
        checked = json.loads(evidence_path.read_text(encoding="utf-8"))
        cases = json.loads((HERE / "cases.json").read_text(encoding="utf-8"))
        manifest = harness.load_manifest(HERE / "manifest.json")
        contract = json.loads((HERE / "fixture-contract.json").read_text(encoding="utf-8"))
        with tempfile.TemporaryDirectory() as temp:
            fixtures = Path(temp) / "fixtures"
            generator.generate(fixtures)
            evidence.validate_evidence_document(
                checked,
                manifest,
                cases,
                manifest_path=HERE / "manifest.json",
                cases_path=HERE / "cases.json",
                fixture_root=fixtures,
                source_root=HERE,
            )
            self.assertEqual(contract["fixtures"], fixture_tools.fixture_identities(fixtures))
            self.assertEqual(contract["fixture_set_sha256"], fixture_tools.fixture_set_identity(fixtures))
        known_cases = {case["id"] for case in cases["cases"]}
        self.assertEqual("passed", checked["status"])
        evidence.validate_result_set(manifest, cases, "macos-aarch64", checked["results"])
        self.assertEqual("sha1", checked["provenance"]["git"]["object_format"])
        self.assertEqual("separate-attestation", checked["provenance"]["execution"]["identity"])
        self.assertTrue(
            all(
                result["status"] in {"passed", "declared-unsupported"}
                and ("case" not in result or result["case"] in known_cases)
                for result in checked["results"]
            )
        )
        encoded = json.dumps(checked, sort_keys=True)
        self.assertNotIn("/tmp/", encoded)
        self.assertNotIn("/private/", encoded)
        self.assertNotIn("stdout", encoded)
        self.assertEqual(
            json.dumps(checked, ensure_ascii=True, indent=2, sort_keys=True) + "\n",
            evidence_path.read_text(encoding="utf-8"),
        )

    def test_checked_cases_validate_and_cover_every_runnable_reader(self) -> None:
        manifest = harness.load_manifest(HERE / "manifest.json")
        cases = json.loads((HERE / "cases.json").read_text(encoding="utf-8"))

        validated = matrix.validate_cases(cases, {reader["id"] for reader in manifest["readers"]})

        covered = {reader for case in validated for reader in case["readers"]}
        runnable = {
            reader["id"]
            for reader in manifest["readers"]
            if any(artifact["support"] == "required" for artifact in reader["artifacts"])
        }
        self.assertEqual(runnable, covered)

    def test_generation_is_byte_deterministic_and_contains_no_absolute_paths(self) -> None:
        with tempfile.TemporaryDirectory() as first, tempfile.TemporaryDirectory() as second:
            first_root = Path(first) / "fixtures"
            second_root = Path(second) / "fixtures"
            generator.generate(first_root)
            generator.generate(second_root)

            first_snapshot = fixture_tools.snapshot_tree(first_root)
            second_snapshot = fixture_tools.snapshot_tree(second_root)
            self.assertEqual(first_snapshot.sha256, second_snapshot.sha256)
            forbidden = [str(first_root).encode(), str(second_root).encode(), str(Path.home()).encode()]
            for path in first_root.rglob("*"):
                if path.is_file():
                    payload = path.read_bytes()
                    self.assertFalse(any(value in payload for value in forbidden), path)

    def test_generation_refuses_to_replace_an_existing_destination(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            destination = Path(temp) / "fixtures"
            destination.mkdir()
            sentinel = destination / "owned"
            sentinel.write_text("keep\n", encoding="utf-8")

            with self.assertRaisesRegex(generator.GenerationError, "already exists"):
                generator.generate(destination)

            self.assertEqual("keep\n", sentinel.read_text(encoding="utf-8"))

    def test_records_match_live_git_and_workspace_baselines(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "fixtures"
            generator.generate(root)

            pending = root / "custom-message-pending"
            record = (pending / ".gwz/merge/merge_retained.yaml").read_text(encoding="utf-8")
            self.assertIn("custom retained-reader message", record)
            self.assertIn(f"before_commit: {git(pending / 'member', 'rev-parse', 'main')}", record)
            self.assertIn(
                f"source_commit: {git(pending / 'member', 'rev-parse', 'feature/source')}",
                record,
            )
            self.assertIn(
                "lock_sha256: "
                + hashlib.sha256((pending / "gwz.conf/gwz.lock.yml").read_bytes()).hexdigest(),
                record,
            )

            no_ff = root / "no-ff-fast-forwardable"
            self.assertEqual(
                git(no_ff / "member", "rev-parse", "main"),
                git(no_ff / "member", "rev-parse", "feature/source^"),
            )
            self.assertIn("mode: no_ff", (no_ff / ".gwz/merge/merge_retained.yaml").read_text())

            archived = (root / "archived-v0/.gwz/merge/done/merge_retained.yaml").read_text()
            self.assertIn("commit_message: custom retained-reader message", archived)
            self.assertNotIn("pending_action:", archived)
            self.assertNotIn("mode: no_ff", archived)

    def test_normalized_evidence_keeps_digests_and_results_but_not_host_paths(self) -> None:
        manifest = harness.load_manifest(HERE / "manifest.json")
        manifest["readers"] = [
            reader for reader in manifest["readers"] if reader["id"] == "rust-cli-v0.10.0"
        ]
        manifest["runtimes"] = []
        cases = {
            "cases": [{
                "id": "v0-custom-message-pending-continue",
                "readers": ["rust-cli-v0.10.0"],
            }]
        }
        sources = evidence.source_digests(HERE)
        source_set = evidence.source_set_sha256(sources)
        summary = {
            "status": "passed",
            "platform": "macos-aarch64",
            "results": [
                {
                    "reader": "rust-cli-v0.10.0",
                    "case": "v0-custom-message-pending-continue",
                    "status": "passed",
                    "exit_code": 0,
                    "stdout": '{"merge":{"state":"Completed"}}',
                    "stderr": "",
                    "changed_paths": ["text:/private/tmp/not-evidence"],
                    "postconditions": {"status": "passed", "count": 5},
                    "before_sha256": "c" * 64,
                    "after_invariant_sha256": "d" * 64,
                }
            ],
        }

        normalized = evidence.build_evidence(
            manifest,
            cases,
            summary,
            manifest_sha256="a" * 64,
            cases_sha256="b" * 64,
            provenance={
                "fixture_set_sha256": "ac89d68430c3b97ddf57c632a234e6d6e74902196ab651e003e4c152abde529b",
                "fixtures": {"fixture": "f" * 64},
                "generator_sha256": sources["generate_retained_reader_fixtures.py"],
                "evaluator_sha256": source_set,
                "sources": sources,
                "source_set_sha256": source_set,
                "git": {"version": "git version test", "object_format": "sha1"},
                "platform": {"declared": "macos-aarch64", "system": "Darwin", "machine": "arm64"},
                "python": {"implementation": "CPython", "version": "3.12.0", "architecture": "arm64", "pointer_bits": 64, "executable_sha256": "3" * 64},
                "execution": {"identity": "separate-attestation", "required_in_ci": True},
            },
        )
        encoded = json.dumps(normalized, sort_keys=True)

        self.assertEqual("gwz.retained-reader-evidence/v1", normalized["schema"])
        self.assertIn("Completed", encoded)
        self.assertIn("sha256", encoded)
        self.assertNotIn("/private/", encoded)
        self.assertNotIn("stdout", encoded)


if __name__ == "__main__":
    unittest.main()

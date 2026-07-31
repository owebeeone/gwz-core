from __future__ import annotations

import copy
import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import retained_reader_evidence as evidence
import retained_reader_fixture as fixture
import retained_reader_harness as harness
import generate_retained_reader_fixtures as generator
from retained_reader_semantics import merge_record_semantic
from retained_reader_yaml import canonical_yaml_sha256


HERE = Path(__file__).resolve().parent


class ResultSetIntegrityTests(unittest.TestCase):
    def setUp(self) -> None:
        self.manifest = harness.load_manifest(HERE / "manifest.json")
        self.cases = json.loads((HERE / "cases.json").read_text(encoding="utf-8"))
        self.checked = json.loads((HERE / "evidence-macos-aarch64.json").read_text(encoding="utf-8"))

    def test_duplicate_result_cannot_replace_required_result(self) -> None:
        tampered = copy.deepcopy(self.checked["results"])
        tampered[-1] = copy.deepcopy(tampered[0])
        with self.assertRaisesRegex(evidence.EvidenceError, "exact unique expected result"):
            evidence.validate_result_set(self.manifest, self.cases, "macos-aarch64", tampered)

    def test_portable_projection_excludes_only_reviewed_host_fields(self) -> None:
        actual = copy.deepcopy(self.checked)
        actual["provenance"]["git"]["version"] = "git version other"
        actual["provenance"]["python"]["version"] = "3.10.99"
        actual["provenance"]["python"]["executable_sha256"] = "0" * 64
        self.assertEqual(evidence.portable_projection(self.checked), evidence.portable_projection(actual))
        actual["results"][0]["outcome"] = "Corrupt"
        self.assertNotEqual(evidence.portable_projection(self.checked), evidence.portable_projection(actual))

    def test_complete_source_digest_rejects_stale_module(self) -> None:
        tampered = copy.deepcopy(self.checked["provenance"])
        tampered["sources"]["retained_reader_matrix.py"] = "0" * 64
        with self.assertRaisesRegex(evidence.EvidenceError, "source"):
            evidence.validate_source_provenance(tampered, HERE)


class DurableContentTests(unittest.TestCase):
    def test_complete_archive_semantics_reject_unknown_field_change(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            fixtures = Path(temp) / "fixtures"
            generator.generate(fixtures)
            root = fixtures / "archived-v0"
            archive = root / ".gwz/merge/done/merge_retained.yaml"
            spec = [{
                "kind": "yaml-semantic",
                "semantic": "merge-record",
                "path": ".gwz/merge/done/merge_retained.yaml",
                "sha256": merge_record_semantic(root, archive.read_text(encoding="utf-8")),
                "required": {"state": "aborted"},
            }]
            errors, _ = fixture.evaluate_postconditions(spec, root)
            self.assertEqual([], errors)
            archive.write_text(
                archive.read_text(encoding="utf-8").replace(
                    "retained_fixture_generation: canonical-v1",
                    "retained_fixture_generation: corrupt",
                ),
                encoding="utf-8",
            )
            errors, _ = fixture.evaluate_postconditions(spec, root)
            self.assertTrue(errors)

    def test_yaml_semantic_postcondition_rejects_corrupt_lock_member(self) -> None:
        original = (
            "schema: gwz.lock/v0\nworkspace_id: ws\nmembers:\n"
            "  mem_member:\n    path: member\n    commit: abc\n    branch: main\n"
        )
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            lock = root / "gwz.conf/gwz.lock.yml"
            lock.parent.mkdir(parents=True)
            lock.write_text(original, encoding="utf-8")
            spec = [{
                "kind": "yaml-semantic",
                "path": "gwz.conf/gwz.lock.yml",
                "sha256": canonical_yaml_sha256(original),
                "required": {"members.mem_member.commit": "abc"},
            }]
            errors, observations = fixture.evaluate_postconditions(spec, root)
            self.assertEqual([], errors)
            self.assertEqual(canonical_yaml_sha256(original), observations[0]["sha256"])
            lock.write_text(original.replace("commit: abc", "commit: corrupt"), encoding="utf-8")
            errors, _ = fixture.evaluate_postconditions(spec, root)
            self.assertTrue(errors)

    def test_dynamic_marker_identity_normalizes_uuid_but_not_meaning(self) -> None:
        policy = {
            "mode": "contract",
            "exact": [],
            "dynamic": [{
                "pattern": "text:gwz.conf/markers/????????-????-????-????-????????????.yaml",
                "minimum": 1,
                "maximum": 1,
            }],
        }
        identities = []
        for marker_id, workspace_id in [
            ("019fb8b0-74f8-7167-939b-9cf3b9eee108", "ws"),
            ("019fb8b1-1111-7222-8333-444444444444", "ws"),
            ("019fb8b2-1111-7222-8333-444444444444", "corrupt"),
        ]:
            with tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                path = root / f"gwz.conf/markers/{marker_id}.yaml"
                path.parent.mkdir(parents=True)
                path.write_text(
                    f"schema: gwz.marker/v0\ngwz_commit_id: {marker_id}\nworkspace_id: {workspace_id}\n",
                    encoding="utf-8",
                )
                after = fixture.snapshot_tree(root)
                changes = [next(key for key in after.entries if key.endswith(".yaml"))]
                identities.append(fixture.normalized_mutation_identity(policy, changes, after, root))
        self.assertEqual(identities[0], identities[1])
        self.assertNotEqual(identities[0], identities[2])
        self.assertEqual(1, identities[0][0]["count"])
        self.assertIn("content_sha256", identities[0][0])


if __name__ == "__main__":
    unittest.main()

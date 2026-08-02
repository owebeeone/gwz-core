from __future__ import annotations

import copy
import hashlib
import json
import os
import stat
import subprocess
import sys
import tempfile
import time
import unittest
import zipfile
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))

import generate_retained_reader_fixtures as generator
import retained_reader_evidence as evidence
import retained_reader_fixture as fixture
import retained_reader_harness as harness
import retained_reader_matrix as matrix
import retained_reader_semantics as semantics
from test_retained_reader_harness import MANIFEST, complete_manifest
from test_retained_reader_matrix import write_zip


HERE = Path(__file__).resolve().parent


class ManifestAdversarialTests(unittest.TestCase):
    def test_reader_must_declare_exact_platform_cross_product(self) -> None:
        manifest = complete_manifest()
        manifest["platforms"].append(
            {"id": "windows-aarch64", "os": "windows", "arch": "aarch64", "lane": "artifact-smoke"}
        )
        with self.assertRaisesRegex(harness.ManifestError, "exactly one artifact.*windows-aarch64"):
            harness.validate_manifest(manifest)

    def test_frozen_r0_support_cannot_be_reclassified(self) -> None:
        manifest = harness.load_manifest(MANIFEST)
        artifact = manifest["readers"][0]["artifacts"][0]
        artifact.clear()
        artifact.update(
            platform="linux-x86_64",
            support="unsupported",
            reason="silently skipped",
            substitute_evidence=["none"],
        )
        with self.assertRaisesRegex(harness.ManifestError, "frozen support"):
            harness.validate_manifest(manifest)

    def test_unknown_manifest_field_fails_schema_validation(self) -> None:
        manifest = complete_manifest()
        manifest["readers"][0]["artifacts"][0]["sha25_typo"] = "ignored"
        with self.assertRaisesRegex(harness.ManifestError, "sha25_typo"):
            harness.validate_manifest(manifest)

    def test_unreviewed_https_provider_is_not_immutable(self) -> None:
        manifest = complete_manifest()
        artifact = manifest["readers"][0]["artifacts"][0]
        artifact["url"] = "https://example.invalid/releases/download/v0.9.2/gwz-linux.tar.xz"
        with self.assertRaisesRegex(harness.ManifestError, "immutable"):
            harness.validate_manifest(manifest)


class CaseAdversarialTests(unittest.TestCase):
    def setUp(self) -> None:
        self.manifest = harness.load_manifest(MANIFEST)
        self.cases = json.loads((HERE / "cases.json").read_text(encoding="utf-8"))

    def test_misspelled_postconditions_fails_before_execution(self) -> None:
        case = self.cases["cases"][0]
        case["postcondition"] = case.pop("postconditions", [])
        with self.assertRaisesRegex(matrix.MatrixError, "postcondition"):
            matrix.validate_cases(self.cases, self.manifest)

    def test_invalid_stream_mutation_and_duplicate_reader_fail(self) -> None:
        for mutate, expected in [
            (lambda case: case["expected"]["stdout"].update(mode="tokens"), "tokens"),
            (lambda case: case["expected"].update(mutation={"mode": "exact"}), "paths"),
            (lambda case: case["readers"].append(case["readers"][0]), "unique"),
        ]:
            document = copy.deepcopy(self.cases)
            mutate(document["cases"][0])
            with self.assertRaisesRegex(matrix.MatrixError, expected):
                matrix.validate_cases(document, self.manifest)

    def test_required_applicable_command_cannot_lose_its_case(self) -> None:
        self.cases["cases"] = [
            case for case in self.cases["cases"] if case["command"] != "merge-status"
        ]
        with self.assertRaisesRegex(matrix.MatrixError, "merge-status"):
            matrix.validate_cases(self.cases, self.manifest)

    def test_json_contract_rejects_token_only_and_wrong_types(self) -> None:
        expected = {
            "exit_codes": [1],
            "stdout": {
                "mode": "json-contract",
                "value": {
                    "shape": "merge",
                    "outcomes": ["Halted"],
                    "merge_id": "merge_retained",
                    "member_id": "mem_member",
                    "member_outcomes": ["Planned"],
                },
            },
            "mutation": {"mode": "none"},
        }
        snapshot = fixture.TreeSnapshot("0" * 64, {})
        token_only = subprocess.CompletedProcess([], 1, "merge_retained Halted mem_member", "")
        wrong_type = subprocess.CompletedProcess(
            [],
            1,
            json.dumps(
                {
                    "merge": {
                        "merge_id": "merge_retained",
                        "state": "Halted",
                        "repos": [{"target_id": 7, "state": "Planned"}],
                    }
                }
            ),
            "",
        )
        self.assertTrue(fixture.evaluate_expectation(expected, token_only, snapshot, snapshot))
        self.assertTrue(fixture.evaluate_expectation(expected, wrong_type, snapshot, snapshot))

    def test_checked_machine_cases_and_mutations_are_strict(self) -> None:
        for case in self.cases["cases"]:
            self.assertEqual("json-contract", case["expected"]["stdout"]["mode"], case["id"])
            self.assertNotEqual("allow", case["expected"]["mutation"]["mode"], case["id"])
            self.assertRegex(case["fixture_sha256"], r"^[0-9a-f]{64}$")

    def test_v0100_record_rewrites_pin_unknown_baseline_fields(self) -> None:
        fields = {"lock_yaml", "manifest_yaml", "lock_commit_sha256", "manifest_commit_sha256"}
        for case in self.cases["cases"]:
            if "rust-cli-v0.10.0" not in case["readers"]:
                continue
            mutation = case["expected"]["mutation"]
            archives = "text:.gwz/merge/done/merge_retained.yaml" in mutation.get("paths", mutation.get("exact", []))
            if archives and case["command"] != "merge-gc":
                checks = [item for item in case.get("postconditions", []) if item["kind"] == "merge-record-baseline-preserved"]
                self.assertEqual([fields], [set(item["fields"]) for item in checks], case["id"])


class FixtureAdversarialTests(unittest.TestCase):
    def test_git_object_payload_uses_binary_stdin_without_newline_translation(self) -> None:
        payload = "tree deadbeef\nparent cafe1234\n\nmessage\n"
        completed = subprocess.CompletedProcess(
            ["git", "hash-object"], 0, b"0123456789abcdef\n", b""
        )
        with mock.patch.object(generator.subprocess, "run", return_value=completed) as run:
            result = generator._git_input(
                Path("."), payload, "hash-object", "-t", "commit", "-w", "--stdin"
            )

        self.assertEqual("0123456789abcdef", result)
        self.assertEqual(payload.encode("utf-8"), run.call_args.kwargs["input"])
        self.assertNotIn("text", run.call_args.kwargs)

    def test_read_only_cleanup_handler_makes_git_objects_writable_before_retry(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            target = Path(temp) / "object"
            target.write_bytes(b"git object")
            target.chmod(stat.S_IREAD)

            def remove(path: str) -> None:
                self.assertTrue(Path(path).stat().st_mode & stat.S_IWUSR)
                Path(path).unlink()

            error = PermissionError("read-only Git object")
            generator._remove_readonly(remove, str(target), (PermissionError, error, None))

            self.assertFalse(target.exists())

    def test_hostile_git_environment_does_not_change_fixture_identity(self) -> None:
        with tempfile.TemporaryDirectory() as first, tempfile.TemporaryDirectory() as second:
            clean = Path(first) / "fixtures"
            hostile = Path(second) / "fixtures"
            generator.generate(clean)
            with mock.patch.dict(
                os.environ,
                {
                    "GIT_DEFAULT_HASH": "sha256",
                    "GIT_DIR": "/definitely/not/a/repository",
                    "GIT_INDEX_FILE": "/definitely/not/an/index",
                    "GIT_CONFIG_COUNT": "1",
                    "GIT_CONFIG_KEY_0": "core.autocrlf",
                    "GIT_CONFIG_VALUE_0": "true",
                },
            ):
                generator.generate(hostile)
            self.assertEqual(
                fixture.fixture_set_identity(clean),
                fixture.fixture_set_identity(hostile),
            )

    def test_fixture_identity_ignores_non_authoritative_git_bookkeeping(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "fixtures"
            generator.generate(root)
            before = fixture.fixture_set_identity(root)

            for repository in (root / "custom-message-pending", root / "custom-message-pending/member"):
                git_dir = repository / ".git"
                (git_dir / "COMMIT_EDITMSG").write_text("host-specific editor state\r\n")
                (git_dir / "description").write_text("host-specific description\r\n")
                (git_dir / "logs/HEAD").write_text("host-specific reflog\r\n")
                (git_dir / "gc.log").write_text("host-specific maintenance state\r\n")

            self.assertEqual(before, fixture.fixture_set_identity(root))

    def test_fixture_identity_retains_behavior_affecting_git_state(self) -> None:
        mutations = {
            "config": lambda repository: (repository / ".git/config").write_text(
                (repository / ".git/config").read_text() + "[test]\n\tvalue = changed\n"
            ),
            "exclude": lambda repository: (repository / ".git/info/exclude").write_text("/different/\n"),
            "head": lambda repository: (repository / ".git/HEAD").write_text("ref: refs/heads/feature/source\n"),
            "ref": lambda repository: (repository / ".git/refs/heads/main").write_text(
                (repository / ".git/refs/heads/feature/source").read_text()
            ),
            "object": lambda repository: generator._git_input(
                repository, "new object", "hash-object", "-w", "--stdin"
            ),
            "index": lambda repository: generator._git(repository, "rm", "--cached", "base.txt"),
            "worktree": lambda repository: (repository / "base.txt").write_text("changed\n"),
        }
        for label, mutate in mutations.items():
            with self.subTest(label=label), tempfile.TemporaryDirectory() as temp:
                root = Path(temp) / "fixtures"
                generator.generate(root)
                before = fixture.fixture_set_identity(root)
                mutate(root / "custom-message-pending/member")
                self.assertNotEqual(before, fixture.fixture_set_identity(root))

    def test_fixture_identity_normalizes_git_ref_line_endings(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "fixtures"
            generator.generate(root)
            before = fixture.fixture_set_identity(root)
            workspace = root / "custom-message-pending"
            for path in (
                workspace / ".git/HEAD",
                workspace / ".git/refs/heads/main",
                workspace / "member/.git/HEAD",
                workspace / "member/.git/refs/heads/main",
                workspace / "member/.git/refs/heads/feature/source",
            ):
                path.write_bytes(path.read_bytes().replace(b"\n", b"\r\n"))
            self.assertEqual(before, fixture.fixture_set_identity(root))

    def test_fixture_identity_is_stable_across_git_storage_layouts(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "fixtures"
            generator.generate(root)
            before = fixture.fixture_set_identity(root)
            repository = root / "custom-message-pending/member"
            for command in (("repack", "-a", "-d"), ("pack-refs", "--all"), ("update-server-info",), ("commit-graph", "write", "--reachable"), ("commit-graph", "write", "--reachable", "--split"), ("multi-pack-index", "write")):
                generator._git(repository, *command)
                self.assertEqual(before, fixture.fixture_set_identity(root), command)

    def test_fixture_identity_rejects_corrupt_or_unmodeled_git_authority(self) -> None:
        for label in ("corrupt-object", "missing-object", "merge-head", "active-hook", "legacy-branch", "ref-lock", "object-info", "symlink-config", "symlink-exclude", "directory-ref-lock", "directory-index-lock", "directory-log-lock", "directory-hook"):
            with self.subTest(label=label), tempfile.TemporaryDirectory() as temp:
                root = Path(temp) / "fixtures"
                generator.generate(root)
                repository = root / "custom-message-pending/member"
                if label in {"corrupt-object", "missing-object"}:
                    oid = generator._git(repository, "rev-parse", "HEAD")
                    loose = repository / f".git/objects/{oid[:2]}/{oid[2:]}"
                    loose.chmod(stat.S_IREAD | stat.S_IWRITE)
                    if label == "corrupt-object":
                        loose.write_bytes(b"corrupt Git object")
                    else:
                        loose.unlink()
                elif label == "merge-head":
                    (repository / ".git/MERGE_HEAD").write_text(
                        (repository / ".git/refs/heads/feature/source").read_text()
                    )
                else:
                    relative = {"active-hook": "hooks/pre-commit", "legacy-branch": "branches/legacy", "ref-lock": "refs/heads/main.lock", "object-info": "objects/info/unclassified", "symlink-config": "config", "symlink-exclude": "info/exclude", "directory-ref-lock": "refs/heads/main.lock", "directory-index-lock": "index.lock", "directory-log-lock": "logs/refs/heads/main.lock", "directory-hook": "hooks/pre-commit"}[label]
                    authority = repository / ".git" / relative
                    if label.startswith("directory-"):
                        authority.mkdir(parents=True)
                    elif label.startswith("symlink-"):
                        target = Path(temp) / label
                        target.write_bytes(authority.read_bytes())
                        authority.unlink(); authority.symlink_to(target)
                    else:
                        authority.parent.mkdir(parents=True, exist_ok=True)
                        authority.write_text((repository / ".git/refs/heads/main").read_text() if label == "ref-lock" else "#!/bin/sh\nexit 1\n")
                        if label == "active-hook": authority.chmod(0o755)
                with self.assertRaises(fixture.FixtureError):
                    fixture.fixture_set_identity(root)

    def test_fixture_identity_retains_durable_workspace_records(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "fixtures"
            generator.generate(root)
            before = fixture.fixture_set_identity(root)
            record = root / "custom-message-pending/.gwz/merge/merge_retained.yaml"
            record.write_text(record.read_text() + "review_probe: changed\n")
            self.assertNotEqual(before, fixture.fixture_set_identity(root))

    def test_merge_record_semantic_binds_live_boundary_to_candidate_bytes(self) -> None:
        boundary = "/.gwz/\n/member/\n"
        digest = hashlib.sha256(boundary.encode()).hexdigest()
        record = (
            "publication:\n"
            "  candidate:\n"
            "    baseline_boundary_text: |\n"
            "      /.gwz/\n"
            "      /member/\n"
            "    boundary_text: |\n"
            "      /.gwz/\n"
            "      /member/\n"
            f"    baseline_boundary_sha256: {digest}\n"
            f"    boundary_sha256: {digest}\n"
        )
        with tempfile.TemporaryDirectory() as temp:
            workspace = Path(temp) / "workspace"
            generator._init(workspace)
            (workspace / "tracked").write_text("tracked\n")
            generator._commit(workspace, "baseline", "tracked")
            (workspace / ".git/info/exclude").write_bytes(boundary.encode("utf-8"))
            (workspace / "record.yaml").write_text(record)
            expected = semantics.merge_record_semantic(workspace, record)
            matched, _ = semantics.yaml_observation(
                {
                    "path": "record.yaml",
                    "semantic": "merge-record",
                    "sha256": expected,
                    "required": {},
                },
                workspace,
            )
            self.assertTrue(matched)
            (workspace / ".git/info/exclude").write_bytes(b"!/*\n")
            matched, _ = semantics.yaml_observation(
                {
                    "path": "record.yaml",
                    "semantic": "merge-record",
                    "sha256": expected,
                    "required": {},
                },
                workspace,
            )
            self.assertFalse(matched)
            external = Path(temp) / "external-exclude"
            external.write_bytes(boundary.encode("utf-8"))
            (workspace / ".git/info/exclude").unlink()
            (workspace / ".git/info/exclude").symlink_to(external)
            self.assertNotEqual(expected, semantics.merge_record_semantic(workspace, record))

    def test_generated_fixtures_match_reviewed_logical_digests(self) -> None:
        contract = json.loads((HERE / "fixture-contract.json").read_text(encoding="utf-8"))
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / "fixtures"
            generator.generate(root)
            self.assertEqual(contract["fixtures"], fixture.fixture_identities(root))
            self.assertEqual(contract["fixture_set_sha256"], fixture.fixture_set_identity(root))

    def test_unknown_baseline_drop_or_alter_fails_postcondition(self) -> None:
        before = """baseline:\n  lock_sha256: lock\n  manifest_sha256: manifest\n  lock_yaml: |\n    lock body\n  manifest_yaml: |\n    manifest body\n  lock_commit_sha256: lock-commit\n  manifest_commit_sha256: manifest-commit\n"""
        with tempfile.TemporaryDirectory() as temp:
            source = Path(temp) / "source"
            workspace = Path(temp) / "workspace"
            (source / ".gwz/merge").mkdir(parents=True)
            (workspace / ".gwz/merge/done").mkdir(parents=True)
            (source / ".gwz/merge/merge_retained.yaml").write_text(before)
            specification = [{
                "kind": "merge-record-baseline-preserved",
                "before": ".gwz/merge/merge_retained.yaml",
                "after": ".gwz/merge/done/merge_retained.yaml",
                "fields": ["lock_yaml", "manifest_yaml", "lock_commit_sha256", "manifest_commit_sha256"],
            }]
            for after in (before.replace("  manifest_commit_sha256: manifest-commit\n", ""), before.replace("lock body", "changed")):
                (workspace / ".gwz/merge/done/merge_retained.yaml").write_text(after)
                errors, _ = fixture.evaluate_postconditions(specification, workspace, before_root=source)
                self.assertTrue(errors)
            target = workspace / ".gwz/merge/done/merge_retained.yaml"
            external = Path(temp) / "archived-record"
            external.write_text(before)
            target.unlink(); target.symlink_to(external)
            errors, _ = fixture.evaluate_postconditions(specification, workspace, before_root=source)
            self.assertTrue(errors)
            lock = workspace / "gwz.conf/gwz.lock.yml"
            lock.parent.mkdir()
            lock.symlink_to(external)
            checks = [{"kind": "path", "path": "gwz.conf/gwz.lock.yml", "state": "file"}, {"kind": "yaml-semantic", "path": "gwz.conf/gwz.lock.yml", "sha256": semantics.canonical_yaml_sha256(before)}]
            errors, _ = fixture.evaluate_postconditions(checks, workspace)
            self.assertEqual(2, len(errors))


class RuntimeAdversarialTests(unittest.TestCase):
    def test_interpreter_identity_changes_runtime_key(self) -> None:
        runtime = {"id": "python", "python_version": "3.10"}
        artifact = {"sha256": "a" * 64}
        first = {"implementation": "CPython", "version": "3.10.14", "architecture": "x86_64"}
        second = {"implementation": "CPython", "version": "3.12.8", "architecture": "x86_64"}
        self.assertNotEqual(
            matrix._runtime_identity(runtime, artifact, first),
            matrix._runtime_identity(runtime, artifact, second),
        )

    def test_poisoned_cached_derived_tree_is_never_executed(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            archive = root / "reader.zip"
            write_zip(archive, {"reader": b"verified"}, {"reader"})
            digest = hashlib.sha256(archive.read_bytes()).hexdigest()
            cache = root / "cache"
            poisoned = cache / "trees/sha256" / digest
            poisoned.mkdir(parents=True)
            (poisoned / "reader").write_text("poisoned")
            reader = {"id": "reader", "surface": "rust-cli", "runtime": "native"}
            artifact = {
                "sha256": digest,
                "format": "zip",
                "entry_point": "reader",
            }
            entry = matrix._prepare_reader(
                {"runtimes": [], "default_timeout_seconds": 10},
                reader,
                artifact,
                archive,
                cache,
                root / "derived",
                Path(sys.executable),
                True,
            )
            self.assertEqual(b"verified", entry.read_bytes())

    def test_timeout_kills_descendant_process(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            marker = Path(temp) / "survived"
            child = f"import time,pathlib;time.sleep(.25);pathlib.Path({str(marker)!r}).write_text('bad')"
            parent = f"import subprocess,sys,time;subprocess.Popen([sys.executable,'-c',{child!r}]);time.sleep(5)"
            with self.assertRaisesRegex(harness.HarnessError, "timed out"):
                harness.run_command([sys.executable, "-c", parent], timeout_seconds=0.05)
            time.sleep(0.35)
            self.assertFalse(marker.exists())

    def test_windows_unsafe_archive_aliases_are_rejected_everywhere(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            archive = Path(temp) / "bad.zip"
            with zipfile.ZipFile(archive, "w") as output:
                output.writestr("bin/Reader.exe", b"one")
                output.writestr("bin/reader.exe", b"two")
            with self.assertRaisesRegex(matrix.MatrixError, "collision"):
                matrix.extract_archive(archive, "zip", Path(temp) / "tree")


class EvidenceAdversarialTests(unittest.TestCase):
    def test_evidence_rejects_missing_or_unparseable_outcome(self) -> None:
        manifest = harness.load_manifest(MANIFEST)
        for stdout in ("not json", "{}"):
            summary = {
                "status": "passed",
                "platform": "macos-aarch64",
                "results": [{
                    "reader": "rust-cli-v0.10.0",
                    "case": "probe",
                    "status": "passed",
                    "exit_code": 0,
                    "stdout": stdout,
                    "postconditions": {"status": "passed", "count": 0},
                    "before_sha256": "a" * 64,
                    "after_invariant_sha256": "b" * 64,
                }],
            }
            with self.assertRaisesRegex(evidence.EvidenceError, "outcome"):
                evidence.build_evidence(
                    manifest,
                    json.loads((HERE / "cases.json").read_text(encoding="utf-8")),
                    summary,
                    manifest_sha256="c" * 64,
                    cases_sha256="d" * 64,
                    provenance={},
                )

    def test_execution_attestation_is_separate_and_requires_ci_identity(self) -> None:
        with mock.patch.dict(os.environ, {}, clear=True):
            with self.assertRaisesRegex(evidence.EvidenceError, "GITHUB_SHA"):
                evidence.build_execution_attestation(b"evidence", "macos-aarch64")
        with mock.patch.dict(os.environ, {"GITHUB_SHA": "abc", "GITHUB_RUN_ID": "123"}, clear=True):
            result = evidence.build_execution_attestation(b"evidence", "macos-aarch64")
        self.assertEqual(hashlib.sha256(b"evidence").hexdigest(), result["evidence_sha256"])
        self.assertEqual("abc", result["github_commit"])


if __name__ == "__main__":
    unittest.main()

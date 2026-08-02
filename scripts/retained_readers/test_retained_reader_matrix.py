from __future__ import annotations

import hashlib
import json
import os
import stat
import subprocess
import sys
import tarfile
import tempfile
import unittest
import zipfile
from io import BytesIO
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import retained_reader_harness as harness
import retained_reader_matrix as matrix


def write_zip(path: Path, files: dict[str, bytes], executable: set[str] | None = None) -> None:
    executable = executable or set()
    with zipfile.ZipFile(path, "w") as archive:
        for name, payload in files.items():
            info = zipfile.ZipInfo(name)
            mode = 0o755 if name in executable else 0o644
            info.external_attr = (stat.S_IFREG | mode) << 16
            archive.writestr(info, payload)


def cache_artifact(cache: Path, artifact: Path) -> str:
    digest = hashlib.sha256(artifact.read_bytes()).hexdigest()
    destination = harness.cache_path(cache, digest)
    destination.parent.mkdir(parents=True)
    destination.write_bytes(artifact.read_bytes())
    return digest


class ExtractionTests(unittest.TestCase):
    def test_extracts_zip_and_preserves_executable_mode(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            archive = root / "reader.zip"
            write_zip(archive, {"bin/reader": b"reader"}, {"bin/reader"})
            destination = root / "tree"

            matrix.extract_archive(archive, "zip", destination)

            reader = destination / "bin" / "reader"
            self.assertEqual(b"reader", reader.read_bytes())
            if os.name != "nt":
                self.assertTrue(reader.stat().st_mode & stat.S_IXUSR)

    def test_rejects_zip_path_traversal(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            archive = root / "bad.zip"
            write_zip(archive, {"../escape": b"bad"})
            with self.assertRaisesRegex(matrix.MatrixError, "unsafe archive path"):
                matrix.extract_archive(archive, "zip", root / "tree")
            self.assertFalse((root / "escape").exists())

    def test_rejects_tar_links_and_devices(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            archive = root / "bad.tar.xz"
            with tarfile.open(archive, "w:xz") as output:
                member = tarfile.TarInfo("reader-link")
                member.type = tarfile.SYMTYPE
                member.linkname = "outside"
                output.addfile(member)
            with self.assertRaisesRegex(matrix.MatrixError, "unsupported archive member"):
                matrix.extract_archive(archive, "tar.xz", root / "tree")


class SnapshotTests(unittest.TestCase):
    def test_snapshot_hash_ignores_mtime_but_detects_content_and_symlinks(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            tracked = root / "tracked.txt"
            tracked.write_text("before", encoding="utf-8")
            if hasattr(os, "symlink"):
                os.symlink("tracked.txt", root / "link")
            before = matrix.snapshot_tree(root)

            os.utime(tracked, (1, 1))
            same = matrix.snapshot_tree(root)
            self.assertEqual(before.sha256, same.sha256)

            tracked.write_text("after", encoding="utf-8")
            after = matrix.snapshot_tree(root)
            self.assertNotEqual(before.sha256, after.sha256)
            self.assertIn("text:tracked.txt", matrix.changed_paths(before, after))

    def test_snapshot_preserves_non_utf8_path_identity(self) -> None:
        raw = b"non-utf8-\xff"
        path = Path(os.fsdecode(raw))
        key = matrix._path_key(path)
        self.assertTrue(key.startswith("b64:"))


class ExpectationTests(unittest.TestCase):
    def result(self, stdout: str = "ok\n", stderr: str = "") -> subprocess.CompletedProcess[str]:
        return subprocess.CompletedProcess(["reader"], 0, stdout, stderr)

    def test_evaluates_jsonl_and_no_mutation(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "record.yml").write_text("stable", encoding="utf-8")
            before = matrix.snapshot_tree(root)
            after = matrix.snapshot_tree(root)
            expected = {
                "exit_codes": [0],
                "stdout": {"mode": "jsonl", "value": [{"state": "idle"}]},
                "stderr": {"mode": "exact", "value": ""},
                "mutation": {"mode": "none"},
            }
            errors = matrix.evaluate_expectation(
                expected,
                self.result('{"state":"idle"}\n'),
                before,
                after,
            )
            self.assertEqual([], errors)

    def test_unexpected_mutation_is_a_failure(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            before = matrix.snapshot_tree(root)
            (root / "changed").write_text("changed", encoding="utf-8")
            after = matrix.snapshot_tree(root)
            errors = matrix.evaluate_expectation(
                {"exit_codes": [0], "mutation": {"mode": "none"}},
                self.result(),
                before,
                after,
            )
            self.assertTrue(any("unexpected mutation" in error for error in errors))

    def test_optional_boundary_mutation_accepts_zero_or_one_physical_change(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            workspace = root / "workspace"
            boundary = workspace / ".git/info/exclude"
            boundary.parent.mkdir(parents=True)
            boundary.write_text("/member/\n", encoding="utf-8")
            before = matrix.snapshot_tree(workspace)
            mutation = {
                "mode": "contract",
                "exact": [],
                "dynamic": [{
                    "pattern": "text:.git/info/exclude",
                    "minimum": 0,
                    "maximum": 1,
                }],
            }
            expected = {"exit_codes": [0], "mutation": mutation}
            unchanged = matrix.snapshot_tree(workspace)
            self.assertEqual([], matrix.evaluate_expectation(expected, self.result(), before, unchanged))
            zero = matrix.normalized_mutation_identity(mutation, [], unchanged, workspace)

            boundary.chmod(stat.S_IREAD)
            changed = matrix.snapshot_tree(workspace)
            changes = matrix.changed_paths(before, changed)
            self.assertEqual([], matrix.evaluate_expectation(expected, self.result(), before, changed))
            one = matrix.normalized_mutation_identity(mutation, changes, changed, workspace)
            self.assertNotEqual(zero, one)


class PythonBootstrapTests(unittest.TestCase):
    def test_bootstraps_wheel_in_isolated_venv_without_network(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            wheel = root / "tiny_reader-1.0-py3-none-any.whl"
            write_zip(
                wheel,
                {
                    "tiny_reader.py": b"def main():\n    print('tiny-reader')\n",
                    "tiny_reader-1.0.dist-info/METADATA": b"Metadata-Version: 2.1\nName: tiny-reader\nVersion: 1.0\n",
                    "tiny_reader-1.0.dist-info/WHEEL": b"Wheel-Version: 1.0\nGenerator: test\nRoot-Is-Purelib: true\nTag: py3-none-any\n",
                    "tiny_reader-1.0.dist-info/entry_points.txt": b"[console_scripts]\ntiny-reader = tiny_reader:main\n",
                    "tiny_reader-1.0.dist-info/RECORD": b"",
                },
            )
            runtime = {
                "bootstrap": [
                    ["{python}", "-m", "venv", "{runtime_dir}"],
                    [
                        "{runtime_python}",
                        "-m",
                        "pip",
                        "install",
                        "--no-index",
                        "--no-deps",
                        "{artifact}",
                    ],
                ]
            }
            artifact = {
                "sha256": hashlib.sha256(wheel.read_bytes()).hexdigest(),
                "name": wheel.name,
                "entry_point": "tiny-reader.exe" if os.name == "nt" else "tiny-reader",
            }
            cached_object = root / artifact["sha256"]
            wheel.replace(cached_object)

            entry_point = matrix.bootstrap_python_runtime(
                runtime,
                artifact,
                cached_object,
                root / "runtime",
                python_executable=Path(sys.executable),
                timeout_seconds=60,
            )

            completed = harness.run_command([str(entry_point)], timeout_seconds=10)
            self.assertEqual(0, completed.returncode, completed.stderr)
            self.assertEqual("tiny-reader\n", completed.stdout)


class MatrixTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.cache = self.root / "cache"
        archive = self.root / "reader.zip"
        script = b"import pathlib,sys\nroot=pathlib.Path(sys.argv[sys.argv.index('--root')+1])\nprint('ok')\nif 'mutate' in sys.argv: (root/'changed').write_text('changed')\n"
        write_zip(archive, {"reader.py": script})
        digest = cache_artifact(self.cache, archive)
        self.manifest = {
            "schema": "gwz.retained-readers/v1",
            "default_timeout_seconds": 10,
            "decode_generations": [{"id": "test", "release": "v1.0.0", "description": "test"}],
            "platforms": [{"id": "host", "os": "host", "arch": "host", "lane": "behavioral"}],
            "runtimes": [{"id": "native", "kind": "native", "bootstrap": []}],
            "readers": [
                {
                    "id": "reader",
                    "surface": "rust-cli",
                    "release": "v1.0.0",
                    "decode_generation": "test",
                    "runtime": "native",
                    "supported_record_versions": [0],
                    "envelope_behavior": {"v0": "test"},
                    "commands": {"probe": "available"},
                    "projections": ["human"],
                    "invocation": [sys.executable, "{executable}", "--root", "{workspace}"],
                    "artifacts": [
                        {
                            "platform": "host",
                            "support": "required",
                            "status": "verified",
                            "name": "reader.zip",
                            "url": "https://github.com/example/project/releases/download/v1.0.0/reader.zip",
                            "sha256": digest,
                            "format": "zip",
                            "entry_point": "reader.py",
                        }
                    ],
                }
            ],
        }
        fixture = self.root / "fixtures" / "clean"
        fixture.mkdir(parents=True)
        (fixture / "tracked").write_text("stable", encoding="utf-8")

    def tearDown(self) -> None:
        self.temp.cleanup()

    def cases(self, args: list[str] | None = None) -> dict[str, object]:
        return {
            "schema": "gwz.retained-reader-cases/v1",
            "cases": [
                {
                    "id": "probe",
                    "readers": ["reader"],
                    "command": "probe",
                    "args": args or ["probe"],
                    "fixture": "clean",
                    "expected": {
                        "exit_codes": [0],
                        "stdout": {"mode": "exact", "value": "ok\n"},
                        "mutation": {"mode": "none"},
                    },
                }
            ],
        }

    def test_runs_required_tuple_and_evaluates_fixture(self) -> None:
        summary = matrix.run_matrix(
            self.manifest,
            self.cases(),
            platform="host",
            fixture_root=self.root / "fixtures",
            cache_root=self.cache,
            offline=True,
            python_executable=Path(sys.executable),
        )
        self.assertEqual("passed", summary["status"])
        self.assertEqual("passed", summary["results"][0]["status"])

    def test_reports_unexpected_mutation_as_failure(self) -> None:
        summary = matrix.run_matrix(
            self.manifest,
            self.cases(["probe", "mutate"]),
            platform="host",
            fixture_root=self.root / "fixtures",
            cache_root=self.cache,
            offline=True,
            python_executable=Path(sys.executable),
        )
        self.assertEqual("failed", summary["status"])
        self.assertIn("unexpected mutation", summary["results"][0]["errors"][0])

    def test_evaluates_git_postconditions_inside_the_isolated_fixture(self) -> None:
        fixture = self.root / "fixtures" / "clean"
        subprocess.run(["git", "init", "--initial-branch=main", "repo"], cwd=fixture, check=True)
        repo = fixture / "repo"
        subprocess.run(["git", "config", "user.name", "Fixture"], cwd=repo, check=True)
        subprocess.run(["git", "config", "user.email", "fixture@example.test"], cwd=repo, check=True)
        (repo / "tracked").write_text("one\n", encoding="utf-8")
        subprocess.run(["git", "add", "tracked"], cwd=repo, check=True)
        subprocess.run(["git", "commit", "-m", "fixture message"], cwd=repo, check=True)
        cases = self.cases()
        cases["cases"][0]["postconditions"] = [
            {
                "kind": "git-ref-equals",
                "repository": "repo",
                "left": "HEAD",
                "right": "refs/heads/main",
            },
            {
                "kind": "git-commit-message",
                "repository": "repo",
                "ref": "HEAD",
                "value": "fixture message",
            },
            {
                "kind": "git-parent-count",
                "repository": "repo",
                "ref": "HEAD",
                "count": 0,
            },
            {"kind": "path", "path": "repo/tracked", "state": "file"},
        ]

        summary = matrix.run_matrix(
            self.manifest,
            cases,
            platform="host",
            fixture_root=self.root / "fixtures",
            cache_root=self.cache,
            offline=True,
            python_executable=Path(sys.executable),
        )

        self.assertEqual("passed", summary["status"], summary)

    def test_failed_git_postcondition_fails_the_case(self) -> None:
        cases = self.cases()
        cases["cases"][0]["postconditions"] = [
            {"kind": "path", "path": "missing", "state": "file"}
        ]
        summary = matrix.run_matrix(
            self.manifest,
            cases,
            platform="host",
            fixture_root=self.root / "fixtures",
            cache_root=self.cache,
            offline=True,
            python_executable=Path(sys.executable),
        )
        self.assertEqual("failed", summary["status"])
        self.assertIn("postcondition", summary["results"][0]["errors"][0])

    def test_missing_case_fails_instead_of_skipping_reader(self) -> None:
        summary = matrix.run_matrix(
            self.manifest,
            {"schema": "gwz.retained-reader-cases/v1", "cases": []},
            platform="host",
            fixture_root=self.root / "fixtures",
            cache_root=self.cache,
            offline=True,
            python_executable=Path(sys.executable),
        )
        self.assertEqual("failed", summary["status"])
        self.assertEqual("missing-cases", summary["results"][0]["status"])

    def test_matrix_cli_returns_machine_summary(self) -> None:
        manifest_path = self.root / "manifest.json"
        cases_path = self.root / "cases.json"
        manifest_path.write_text(json.dumps(self.manifest), encoding="utf-8")
        cases_path.write_text(json.dumps(self.cases()), encoding="utf-8")
        script = Path(matrix.__file__).resolve()
        completed = harness.run_command(
            [
                sys.executable,
                str(script),
                str(manifest_path),
                str(cases_path),
                "--platform",
                "host",
                "--fixtures",
                str(self.root / "fixtures"),
                "--cache",
                str(self.cache),
            ],
            timeout_seconds=20,
        )
        self.assertEqual(0, completed.returncode, completed.stderr)
        self.assertEqual("passed", json.loads(completed.stdout)["status"])

    def test_failing_matrix_with_evidence_request_still_emits_complete_summary(self) -> None:
        manifest_path = self.root / "manifest.json"
        cases_path = self.root / "cases.json"
        evidence_path = self.root / "failed-evidence.json"
        manifest_path.write_text(json.dumps(self.manifest), encoding="utf-8")
        cases_path.write_text(json.dumps(self.cases(["probe", "mutate"])), encoding="utf-8")
        script = Path(matrix.__file__).resolve()

        completed = harness.run_command(
            [
                sys.executable,
                str(script),
                str(manifest_path),
                str(cases_path),
                "--platform",
                "host",
                "--fixtures",
                str(self.root / "fixtures"),
                "--cache",
                str(self.cache),
                "--evidence-out",
                str(evidence_path),
            ],
            timeout_seconds=20,
        )

        self.assertEqual(1, completed.returncode)
        summary = json.loads(completed.stdout)
        self.assertEqual("failed", summary["status"])
        self.assertEqual("failed", summary["results"][0]["status"])
        self.assertIn("unexpected mutation", summary["results"][0]["errors"][0])
        self.assertFalse(evidence_path.exists())


if __name__ == "__main__":
    unittest.main()

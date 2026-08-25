from __future__ import annotations

import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import retained_reader_harness as harness
import retained_reader_matrix as matrix


HERE = Path(__file__).resolve().parent
MANIFEST = HERE / "manifest.json"


def complete_manifest() -> dict[str, object]:
    payload = b"retained reader"
    digest = hashlib.sha256(payload).hexdigest()
    return {
        "schema": "gwz.retained-readers/v1",
        "default_timeout_seconds": 10,
        "decode_generations": [
            {
                "id": "pre-record-reader",
                "release": "v0.9.2",
                "description": "the reader predates durable merge records",
            }
        ],
        "platforms": [
            {
                "id": "linux-x86_64",
                "os": "linux",
                "arch": "x86_64",
                "lane": "behavioral",
            }
        ],
        "runtimes": [
            {"id": "native", "kind": "native", "bootstrap": []}
        ],
        "readers": [
            {
                "id": "synthetic-pre-record-rust",
                "surface": "rust-cli",
                "release": "v0.9.2",
                "decode_generation": "pre-record-reader",
                "runtime": "native",
                "supported_record_versions": [],
                "envelope_behavior": {
                    "gwz.merge-operation/v0@0": "not decoded",
                    "unsupported": "no durable-record dispatcher",
                },
                "record_envelopes": {
                    "dispatcher": "none",
                    "supported": [],
                    "unsupported": [],
                },
                "commands": {"workspace-status": "available"},
                "projections": ["human"],
                "invocation": ["{executable}", "--root", "{workspace}"],
                "artifacts": [
                    {
                        "platform": "linux-x86_64",
                        "support": "required",
                        "status": "verified",
                        "name": "gwz-linux.tar.xz",
                        "url": "https://github.com/owebeeone/gwz-cli/releases/download/v0.9.2/gwz-linux.tar.xz",
                        "sha256": digest,
                        "format": "tar.xz",
                        "entry_point": "gwz",
                    }
                ],
            }
        ],
    }


class ManifestTests(unittest.TestCase):
    def test_repository_manifest_is_structurally_valid_and_covers_initial_matrix(self) -> None:
        manifest = harness.load_manifest(MANIFEST)
        harness.validate_manifest(manifest)

        generations = {item["id"] for item in manifest["decode_generations"]}
        # Three of these register decode behaviour with no retained-reader
        # artifacts: the two strict-envelope entries (v0.10.4/v0.10.5, whose
        # typed unsupported-record-version outcome is NOT the v0.10.2
        # generation's) and the A1 entry (v0.11.0, unpublished). None of them
        # adds a reader row, so the tuple count asserted below is deliberately
        # unmoved by all three.
        self.assertEqual(
            {
                "pre-v0-record-reader",
                "v0-mode-known-recovery-dormant",
                "v0-strict-envelope-typed-unsupported",
                "v0-strict-envelope-typed-unsupported-narrow",
                "v0-v1-dual-decode-v0-writer-floor",
            },
            generations,
        )

        tuple_ids = set(harness.iter_tuple_ids(manifest))
        for release in ("v0.9.2", "v0.10.2"):
            for surface in ("rust-cli", "gwz-py"):
                for platform in (
                    "linux-x86_64",
                    "windows-x86_64",
                    "linux-aarch64",
                    "macos-x86_64",
                    "macos-aarch64",
                    "windows-aarch64",
                ):
                    self.assertIn(f"{surface}:{release}:{platform}", tuple_ids)

        self.assertEqual(24, len(tuple_ids))
        self.assertEqual([], harness.gate_readiness_errors(manifest))

    def test_repository_manifest_pins_exact_record_envelope_pairs(self) -> None:
        manifest = harness.load_manifest(MANIFEST)
        harness.validate_manifest(manifest)
        readers = {reader["id"]: reader for reader in manifest["readers"]}

        for reader_id in ("rust-cli-v0.9.2", "gwz-py-v0.9.2"):
            envelopes = readers[reader_id]["record_envelopes"]
            self.assertEqual("none", envelopes["dispatcher"])
            self.assertEqual([], envelopes["supported"])
            self.assertEqual([], envelopes["unsupported"])

        expected_unsupported = {
            ("gwz.merge-operation/v1", 1, "record_unreadable"),
            ("gwz.merge-operation/v2", 2, "record_unreadable"),
            ("gwz.merge-operation/v3", 3, "record_unreadable"),
            ("gwz.merge-operation/v4", 4, "record_unreadable"),
            ("gwz.merge-operation/v0", 1, "record_unreadable"),
            ("gwz.merge-operation/future", 99, "record_unreadable"),
        }
        for reader_id in ("rust-cli-v0.10.2", "gwz-py-v0.10.2"):
            envelopes = readers[reader_id]["record_envelopes"]
            self.assertEqual("header_guarded", envelopes["dispatcher"])
            self.assertEqual(
                [{"schema": "gwz.merge-operation/v0", "record_schema_version": 0}],
                envelopes["supported"],
            )
            self.assertEqual(
                expected_unsupported,
                {
                    (
                        pair["schema"],
                        pair["record_schema_version"],
                        pair["classification"],
                    )
                    for pair in envelopes["unsupported"]
                },
            )

    def test_frozen_r0_contract_cannot_be_bypassed_by_renaming_every_reader(self) -> None:
        manifest = harness.load_manifest(MANIFEST)
        cases = json.loads((HERE / "cases.json").read_text(encoding="utf-8"))
        cases["cases"] = matrix.validate_cases(cases, manifest)
        renamed = {
            reader["id"]: f"renamed-reader-{index}"
            for index, reader in enumerate(manifest["readers"])
        }
        for reader in manifest["readers"]:
            reader["id"] = renamed[reader["id"]]
        for case in cases["cases"]:
            case["readers"] = [renamed[reader] for reader in case["readers"]]
        cases["cases"] = [
            case for case in cases["cases"] if case["command"] != "merge-gc"
        ]

        with self.assertRaisesRegex(harness.ManifestError, "frozen reader set"):
            harness.validate_manifest(manifest)
        with self.assertRaisesRegex(harness.ManifestError, "frozen reader set"):
            matrix.validate_cases(cases, manifest)
        with self.assertRaisesRegex(matrix.MatrixError, "manifest"):
            matrix.validate_cases(cases, set(renamed.values()))
        with self.assertRaisesRegex(harness.ManifestError, "frozen reader set"):
            matrix.run_matrix(
                manifest,
                cases,
                platform="macos-aarch64",
                fixture_root=HERE,
                cache_root=HERE,
                offline=True,
                python_executable=Path(sys.executable),
            )

    def test_pending_required_artifact_is_valid_inventory_but_fails_gate(self) -> None:
        manifest = harness.load_manifest(MANIFEST)
        artifact = manifest["readers"][0]["artifacts"][0]
        artifact.update(
            {
                "status": "pending-acquisition",
                "sha256": None,
                "acquisition_note": "digest has not been acquired",
            }
        )

        harness.validate_manifest(manifest)
        errors = harness.gate_readiness_errors(manifest)

        self.assertEqual(1, len(errors))
        self.assertIn("rust-cli:v0.9.2:linux-x86_64", errors[0])
        self.assertIn("pending-acquisition", errors[0])

    def test_unsupported_tuple_requires_reason_and_substitute_evidence(self) -> None:
        manifest = complete_manifest()
        artifact = manifest["readers"][0]["artifacts"][0]
        artifact.clear()
        artifact.update(
            {
                "platform": "linux-x86_64",
                "support": "unsupported",
                "reason": "not distributed",
                "substitute_evidence": ["source-release-build"],
            }
        )
        harness._validate_manifest_shape(manifest)

        artifact["substitute_evidence"] = []
        with self.assertRaisesRegex(harness.ManifestError, "substitute_evidence"):
            harness._validate_manifest_shape(manifest)

    def test_verified_artifact_requires_immutable_url_and_lowercase_digest(self) -> None:
        manifest = complete_manifest()
        artifact = manifest["readers"][0]["artifacts"][0]
        artifact["url"] = "https://github.com/owebeeone/gwz-cli/releases/latest/download/gwz.tar.xz"
        with self.assertRaisesRegex(harness.ManifestError, "immutable"):
            harness._validate_manifest_shape(manifest)

        artifact["url"] = "https://example.invalid/gwz.tar.xz"
        artifact["sha256"] = "A" * 64
        with self.assertRaisesRegex(harness.ManifestError, "sha256"):
            harness._validate_manifest_shape(manifest)

    def test_pending_artifact_cannot_inventory_a_mutable_url(self) -> None:
        manifest = complete_manifest()
        artifact = manifest["readers"][0]["artifacts"][0]
        artifact.update(
            {
                "status": "pending-acquisition",
                "sha256": None,
                "url": "https://github.com/owebeeone/gwz-cli/releases/latest/download/gwz-linux.tar.xz",
                "acquisition_note": "digest pending",
            }
        )
        with self.assertRaisesRegex(harness.ManifestError, "immutable"):
            harness._validate_manifest_shape(manifest)

    def test_unavailable_command_requires_explicit_substitute_evidence(self) -> None:
        manifest = complete_manifest()
        reader = manifest["readers"][0]
        reader["commands"]["merge-status"] = "unavailable"
        with self.assertRaisesRegex(harness.ManifestError, "command_absence_evidence"):
            harness._validate_manifest_shape(manifest)

        reader["command_absence_evidence"] = {
            "merge-status": "released parser rejects it; exercise merge-start and record-byte observation"
        }
        harness._validate_manifest_shape(manifest)

    def test_pre_record_reader_can_support_no_durable_record_versions(self) -> None:
        manifest = complete_manifest()
        manifest["readers"][0]["supported_record_versions"] = []
        harness._validate_manifest_shape(manifest)

    def test_python_wheel_tag_is_required_on_each_platform_tuple(self) -> None:
        manifest = complete_manifest()
        reader = manifest["readers"][0]
        reader["surface"] = "gwz-py"
        reader["python"] = {
            "version": "3.10",
            "abi": "cp310-abi3",
            "native_extension": "gwz._gwz_core",
        }
        artifact = reader["artifacts"][0]
        artifact["wheel_tag"] = "cp310-abi3-manylinux_x86_64"
        harness._validate_manifest_shape(manifest)

        del artifact["wheel_tag"]
        with self.assertRaisesRegex(harness.ManifestError, "wheel_tag"):
            harness._validate_manifest_shape(manifest)

    def test_runtime_companion_artifact_requires_immutable_checksum_pin(self) -> None:
        manifest = complete_manifest()
        manifest["runtimes"][0]["artifacts"] = [
            {
                "id": "dependency",
                "status": "verified",
                "name": "dependency.whl",
                "url": "https://files.pythonhosted.org/packages/content/dependency.whl",
                "sha256": "0" * 64,
            }
        ]
        harness._validate_manifest_shape(manifest)

        manifest["runtimes"][0]["artifacts"][0]["sha256"] = None
        with self.assertRaisesRegex(harness.ManifestError, "sha256"):
            harness._validate_manifest_shape(manifest)

    def test_acquisition_inventory_includes_runtime_companions(self) -> None:
        manifest = complete_manifest()
        manifest["runtimes"][0]["artifacts"] = [
            {
                "id": "dependency",
                "status": "verified",
                "name": "dependency.whl",
                "url": "https://files.pythonhosted.org/packages/content/dependency.whl",
                "sha256": "0" * 64,
            }
        ]
        artifacts = list(harness.iter_acquirable_artifacts(manifest))
        self.assertEqual(2, len(artifacts))


class CacheTests(unittest.TestCase):
    def test_offline_acquisition_uses_content_addressed_cache(self) -> None:
        payload = b"retained reader"
        manifest = complete_manifest()
        artifact = manifest["readers"][0]["artifacts"][0]
        with tempfile.TemporaryDirectory() as temp:
            cache = Path(temp)
            target = harness.cache_path(cache, artifact["sha256"])
            target.parent.mkdir(parents=True)
            target.write_bytes(payload)

            opened = False

            def opener(*_args: object, **_kwargs: object) -> object:
                nonlocal opened
                opened = True
                raise AssertionError("offline acquisition attempted network access")

            actual = harness.acquire_artifact(
                artifact, cache, offline=True, opener=opener
            )

            self.assertEqual(target, actual)
            self.assertFalse(opened)

    def test_offline_missing_artifact_fails_instead_of_skipping(self) -> None:
        artifact = complete_manifest()["readers"][0]["artifacts"][0]
        with tempfile.TemporaryDirectory() as temp:
            with self.assertRaisesRegex(harness.HarnessError, "offline cache miss"):
                harness.acquire_artifact(artifact, Path(temp), offline=True)

    def test_injected_download_is_digest_checked_and_cached_atomically(self) -> None:
        payload = b"retained reader"
        artifact = complete_manifest()["readers"][0]["artifacts"][0]

        class Response:
            def __enter__(self) -> "Response":
                return self

            def __exit__(self, *_args: object) -> None:
                return None

            def read(self, _size: int = -1) -> bytes:
                nonlocal payload
                value, payload = payload, b""
                return value

        with tempfile.TemporaryDirectory() as temp:
            path = harness.acquire_artifact(
                artifact,
                Path(temp),
                offline=False,
                opener=lambda *_args, **_kwargs: Response(),
            )
            self.assertEqual(b"retained reader", path.read_bytes())
            self.assertEqual(artifact["sha256"], path.name)


class InvocationTests(unittest.TestCase):
    def test_unresolved_template_variable_fails(self) -> None:
        with self.assertRaisesRegex(harness.HarnessError, "unknown template"):
            harness.render_command(["{missing}"], {})

    def test_bounded_invocation_times_out_as_failure(self) -> None:
        with self.assertRaisesRegex(harness.HarnessError, "timed out"):
            harness.run_command(
                [sys.executable, "-c", "import time; time.sleep(5)"],
                timeout_seconds=0.05,
            )

    def test_invocation_is_noninteractive_and_captures_outputs(self) -> None:
        code = (
            "import os,sys; "
            "print(os.environ['GIT_TERMINAL_PROMPT']); "
            "print('diagnostic', file=sys.stderr)"
        )
        result = harness.run_command(
            [sys.executable, "-c", code], timeout_seconds=5
        )
        self.assertEqual(0, result.returncode)
        self.assertEqual("0\n", result.stdout)
        self.assertEqual("diagnostic\n", result.stderr)


class CliTests(unittest.TestCase):
    def test_validate_cli_emits_machine_readable_summary(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "manifest.json"
            path.write_bytes(MANIFEST.read_bytes())
            result = harness.run_command(
                [sys.executable, str(HERE / "retained_reader_harness.py"), "validate", str(path)],
                timeout_seconds=5,
            )
        self.assertEqual(0, result.returncode, result.stderr)
        summary = json.loads(result.stdout)
        self.assertEqual("valid", summary["status"])
        self.assertEqual(24, summary["tuple_count"])


class WorkflowTests(unittest.TestCase):
    def test_evidence_bound_inputs_have_portable_lf_checkout_policy(self) -> None:
        root = HERE.parents[1]
        paths = [path.relative_to(root).as_posix() for path in sorted(HERE.iterdir()) if path.is_file()]
        result = harness.run_command(
            ["git", "-C", str(root), "check-attr", "text", "eol", "--", *paths],
            timeout_seconds=5,
        )
        expected = "".join(f"{path}: text: set\n{path}: eol: lf\n" for path in paths)
        self.assertEqual((0, expected), (result.returncode, result.stdout), result.stderr)

    def test_harness_commands_run_in_a_fail_fast_shell_on_windows(self) -> None:
        workflow = (HERE.parents[1] / ".github/workflows/retained-readers.yml").read_text(
            encoding="utf-8"
        )
        step = workflow.split(
            "      - name: Test and validate retained-reader inputs\n", 1
        )[1].split("\n\n", 1)[0]

        self.assertIn("        shell: bash\n", step)
        self.assertLess(step.index("shell: bash"), step.index("run: |"))
        self.assertEqual(2, workflow.split("\njobs:", 1)[0].count('      - ".gitattributes"\n'))


if __name__ == "__main__":
    unittest.main()

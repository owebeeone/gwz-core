import copy
import errno
import json
import pathlib
import sys
import unittest

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import provider  # noqa: E402


def valid_facts():
    return {
        "filesystem": "ext4",
        "filesystem_uuid": "12345678123456781234567812345678",
        "filesystem_uuid_length": 16,
        "handle_provider": "name_to_handle_at-empty-path",
        "handle_type": 1,
        "handle": "ab" * 32,
        "handle_length": 32,
        "path_modes": {
            "sensitive": "Sensitive",
            "casefold": "AsciiCaseFold",
        },
        "mode_query_succeeded": True,
        "mount_id": 101,
    }


def evidence_row(architecture, machine):
    source_digest = provider.probe_source_digest(HERE)
    return {
        "schema_version": 1,
        "core_commit": "a" * 40,
        "workflow_run": "123",
        "architecture": architecture,
        "native_machine": machine,
        "kernel_release": "6.17.0-test",
        "probe_source_sha256": source_digest,
        "provider_table_sha256": provider.provider_table_digest(),
        "tuple": provider.validate_facts(valid_facts()),
        "remount": {
            "identity_equal": True,
            "mount_ids": [101, 202],
            "mount_id_is_non_authoritative": True,
        },
        "substitution": {
            "retained_handle_unchanged": True,
            "replacement_handle_different": True,
        },
        "query_contract": {
            "missing_at_empty_path_errno": "ENOENT",
            "forbidden_flags_rejected_before_syscall": True,
            "pathname_fallback_rejected_before_syscall": True,
            "permission_denial_typed": "IoError",
            "unsupported_empty_path_typed": "UnsupportedOperation",
        },
        "negative_rows": provider.EXPECTED_NEGATIVE_ROWS,
        "diagnostics": {"mount_id_before": 101, "mount_id_after": 202},
    }


class ProviderContractTests(unittest.TestCase):
    def test_support_table_is_ext4_only(self):
        self.assertEqual(
            provider.SUPPORT_TABLE,
            {
                "schema_version": 1,
                "providers": [
                    {
                        "filesystem": "ext4",
                        "provider": "FsIocGetFsUuid",
                        "uuid_bytes": 16,
                        "max_handle_bytes": 128,
                        "handle_query": "retained-fd-empty-path",
                    }
                ],
            },
        )

    def test_valid_tuple_is_normalized(self):
        normalized = provider.validate_facts(valid_facts())
        self.assertEqual(normalized["provider"], "FsIocGetFsUuid")
        self.assertEqual(normalized["handle_length"], 32)
        self.assertNotIn("mount_id", normalized)

    def test_negative_table_is_exact_and_typed(self):
        expected = {
            "handle_overflow": "UnsupportedOperation",
            "handle_fid": "UnsupportedOperation",
            "malformed_uuid_length": "UnsupportedOperation",
            "missing_at_empty_path": "UnsupportedOperation",
            "mode_query_failure": "UnsupportedOperation",
            "network": "UnsupportedOperation",
            "no_uuid": "UnsupportedOperation",
            "overlay": "UnsupportedOperation",
            "pathname_fallback": "UnsupportedOperation",
            "permission_denial": "IoError",
            "symlink_follow": "UnsupportedOperation",
            "tmpfs": "UnsupportedOperation",
            "unknown_handle_provider": "UnsupportedOperation",
            "unsupported_empty_path": "UnsupportedOperation",
            "zero_uuid": "UnsupportedOperation",
        }
        self.assertEqual(provider.EXPECTED_NEGATIVE_ROWS, expected)

        cases = {}
        for name in expected:
            facts = valid_facts()
            if name == "handle_overflow":
                facts["handle"] = "ab" * 129
                facts["handle_length"] = 129
            elif name == "handle_fid":
                cases[name] = self.query_rejection(b"", provider.AT_EMPTY_PATH | provider.AT_HANDLE_FID)
                continue
            elif name == "malformed_uuid_length":
                facts["filesystem_uuid_length"] = 15
                facts["filesystem_uuid"] = "ab" * 15
            elif name == "missing_at_empty_path":
                cases[name] = self.query_rejection(b"", 0)
                continue
            elif name == "mode_query_failure":
                facts["mode_query_succeeded"] = False
            elif name == "network":
                facts["filesystem"] = "nfs"
            elif name == "no_uuid":
                facts["filesystem_uuid_length"] = 0
                facts["filesystem_uuid"] = ""
            elif name == "overlay":
                facts["filesystem"] = "overlay"
            elif name == "pathname_fallback":
                cases[name] = self.query_rejection(b"object", provider.AT_EMPTY_PATH)
                continue
            elif name == "permission_denial":
                cases[name] = provider.handle_query_error(errno.EACCES).code
                continue
            elif name == "symlink_follow":
                cases[name] = self.query_rejection(
                    b"", provider.AT_EMPTY_PATH | provider.AT_SYMLINK_FOLLOW
                )
                continue
            elif name == "tmpfs":
                facts["filesystem"] = "tmpfs"
            elif name == "unknown_handle_provider":
                facts["handle_provider"] = "pathname"
            elif name == "unsupported_empty_path":
                cases[name] = provider.handle_query_error(errno.EOPNOTSUPP).code
                continue
            elif name == "zero_uuid":
                facts["filesystem_uuid"] = "00" * 16
            with self.assertRaises(provider.ProbeError) as caught:
                provider.validate_facts(facts)
            cases[name] = caught.exception.code
        self.assertEqual(cases, expected)

    def query_rejection(self, path, flags):
        with self.assertRaises(provider.ProbeError) as caught:
            provider.validate_handle_query(path=path, flags=flags)
        return caught.exception.code

    def test_handle_query_errno_mapping_is_typed(self):
        self.assertEqual(provider.handle_query_error(errno.EACCES).code, "IoError")
        self.assertEqual(provider.handle_query_error(errno.EPERM).code, "IoError")
        self.assertEqual(
            provider.handle_query_error(errno.EOPNOTSUPP).code,
            "UnsupportedOperation",
        )

    def test_handle_query_contract_forbids_fallback_flags_and_paths(self):
        provider.validate_handle_query(path=b"", flags=provider.AT_EMPTY_PATH)
        invalid = [
            (b"object", provider.AT_EMPTY_PATH),
            (b"", 0),
            (b"", provider.AT_EMPTY_PATH | provider.AT_SYMLINK_FOLLOW),
            (b"", provider.AT_EMPTY_PATH | provider.AT_HANDLE_FID),
        ]
        for path, flags in invalid:
            with self.subTest(path=path, flags=flags):
                with self.assertRaises(provider.ProbeError) as caught:
                    provider.validate_handle_query(path=path, flags=flags)
                self.assertEqual(caught.exception.code, "UnsupportedOperation")

    def test_remount_comparison_ignores_mount_id_only(self):
        before = valid_facts()
        after = copy.deepcopy(before)
        after["mount_id"] = 202
        result = provider.compare_remount(before, after)
        self.assertTrue(result["identity_equal"])
        self.assertEqual(result["mount_ids"], [101, 202])

        for key in ("filesystem_uuid", "handle_type", "handle", "path_modes"):
            changed = copy.deepcopy(after)
            changed[key] = "different" if key != "path_modes" else {}
            with self.subTest(key=key):
                with self.assertRaises(provider.ProbeError) as caught:
                    provider.compare_remount(before, changed)
                self.assertEqual(caught.exception.code, "Ambiguity")

    def test_aggregate_requires_both_architectures_and_exact_binding(self):
        rows = [
            evidence_row("linux-x86_64", "x86_64"),
            evidence_row("linux-aarch64", "aarch64"),
        ]
        arguments = {
            "expected_core_commit": "a" * 40,
            "expected_workflow_run": "123",
            "expected_source_sha256": provider.probe_source_digest(HERE),
        }
        aggregate = provider.aggregate_evidence(rows, **arguments)
        self.assertEqual(
            [row["architecture"] for row in aggregate["architectures"]],
            ["linux-aarch64", "linux-x86_64"],
        )
        self.assertEqual(aggregate["core_commit"], "a" * 40)

        with self.assertRaises(provider.ProbeError):
            provider.aggregate_evidence(rows[:1], **arguments)
        mismatched = copy.deepcopy(rows)
        mismatched[1]["core_commit"] = "c" * 40
        with self.assertRaises(provider.ProbeError):
            provider.aggregate_evidence(mismatched, **arguments)

    def test_aggregate_rejects_missing_false_and_unknown_evidence(self):
        rows = [
            evidence_row("linux-x86_64", "x86_64"),
            evidence_row("linux-aarch64", "aarch64"),
        ]
        arguments = {
            "expected_core_commit": "a" * 40,
            "expected_workflow_run": "123",
            "expected_source_sha256": provider.probe_source_digest(HERE),
        }

        required_fields = set(rows[0])
        for field in required_fields:
            changed = copy.deepcopy(rows)
            del changed[0][field]
            with self.subTest(missing=field):
                with self.assertRaises(provider.ProbeError):
                    provider.aggregate_evidence(changed, **arguments)

        nested_fields = ("tuple", "substitution", "query_contract", "remount", "diagnostics")
        for section in nested_fields:
            for field in rows[0][section]:
                changed = copy.deepcopy(rows)
                del changed[0][section][field]
                with self.subTest(missing=f"{section}.{field}"):
                    with self.assertRaises(provider.ProbeError):
                        provider.aggregate_evidence(changed, **arguments)
            changed = copy.deepcopy(rows)
            changed[0][section]["unexpected"] = True
            with self.subTest(unknown=section):
                with self.assertRaises(provider.ProbeError):
                    provider.aggregate_evidence(changed, **arguments)

        mutations = [
            ("tuple", None),
            ("substitution.retained_handle_unchanged", False),
            ("substitution.replacement_handle_different", False),
            ("query_contract.missing_at_empty_path_errno", "SUCCESS"),
            ("query_contract.forbidden_flags_rejected_before_syscall", False),
            ("query_contract.pathname_fallback_rejected_before_syscall", False),
            ("query_contract.permission_denial_typed", "UnsupportedOperation"),
            ("query_contract.unsupported_empty_path_typed", "IoError"),
            ("remount.mount_id_is_non_authoritative", False),
            ("native_machine", "wrong-machine"),
            ("kernel_release", ""),
            ("probe_source_sha256", "b" * 64),
        ]
        for path, value in mutations:
            changed = copy.deepcopy(rows)
            target = changed[0]
            components = path.split(".")
            for component in components[:-1]:
                target = target[component]
            target[components[-1]] = value
            with self.subTest(mutation=path):
                with self.assertRaises(provider.ProbeError):
                    provider.aggregate_evidence(changed, **arguments)

        changed = copy.deepcopy(rows)
        changed[0]["unexpected"] = True
        with self.assertRaises(provider.ProbeError):
            provider.aggregate_evidence(changed, **arguments)

    def test_aggregate_rejects_invalid_values_for_every_evidence_field(self):
        rows = [
            evidence_row("linux-x86_64", "x86_64"),
            evidence_row("linux-aarch64", "aarch64"),
        ]
        arguments = {
            "expected_core_commit": "a" * 40,
            "expected_workflow_run": "123",
            "expected_source_sha256": provider.probe_source_digest(HERE),
        }
        invalid_values = {
            "schema_version": 2,
            "core_commit": "b" * 40,
            "workflow_run": "456",
            "architecture": "linux-mips64",
            "native_machine": "armv7l",
            "kernel_release": "",
            "probe_source_sha256": "b" * 64,
            "provider_table_sha256": "b" * 64,
            "tuple.provider": "PathLookup",
            "tuple.filesystem": "xfs",
            "tuple.filesystem_uuid": "0" * 32,
            "tuple.handle_type": 0,
            "tuple.handle": "zz" * 32,
            "tuple.handle_length": 0,
            "tuple.path_modes": {},
            "remount.identity_equal": False,
            "remount.mount_ids": [101, "202"],
            "remount.mount_id_is_non_authoritative": False,
            "substitution.retained_handle_unchanged": False,
            "substitution.replacement_handle_different": False,
            "query_contract.missing_at_empty_path_errno": "SUCCESS",
            "query_contract.forbidden_flags_rejected_before_syscall": False,
            "query_contract.pathname_fallback_rejected_before_syscall": False,
            "query_contract.permission_denial_typed": "UnsupportedOperation",
            "query_contract.unsupported_empty_path_typed": "IoError",
            "negative_rows.permission_denial": "UnsupportedOperation",
            "diagnostics.mount_id_before": 999,
            "diagnostics.mount_id_after": 999,
        }
        for path, value in invalid_values.items():
            changed = copy.deepcopy(rows)
            target = changed[0]
            components = path.split(".")
            for component in components[:-1]:
                target = target[component]
            target[components[-1]] = value
            with self.subTest(invalid=path):
                with self.assertRaises(provider.ProbeError):
                    provider.aggregate_evidence(changed, **arguments)

        for invalid in (None, [], "not-an-object"):
            with self.subTest(row=invalid):
                with self.assertRaises(provider.ProbeError):
                    provider.aggregate_evidence([invalid, rows[1]], **arguments)

        invalid_arguments = {
            "expected_core_commit": "A" * 40,
            "expected_workflow_run": "run-123",
            "expected_source_sha256": "g" * 64,
        }
        for field, value in invalid_arguments.items():
            changed_arguments = dict(arguments)
            changed_arguments[field] = value
            with self.subTest(invalid_argument=field):
                with self.assertRaises(provider.ProbeError):
                    provider.aggregate_evidence(rows, **changed_arguments)

    def test_canonical_json_is_byte_stable(self):
        value = {"b": 2, "a": [3, 1]}
        self.assertEqual(provider.canonical_json(value), b'{"a":[3,1],"b":2}\n')
        self.assertEqual(json.loads(provider.canonical_json(value)), value)


if __name__ == "__main__":
    unittest.main()

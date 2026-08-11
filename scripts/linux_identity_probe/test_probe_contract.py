import copy
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
            "mode_query_failure": "UnsupportedOperation",
            "network": "UnsupportedOperation",
            "no_uuid": "UnsupportedOperation",
            "overlay": "UnsupportedOperation",
            "tmpfs": "UnsupportedOperation",
            "unknown_handle_provider": "UnsupportedOperation",
            "zero_uuid": "UnsupportedOperation",
        }
        self.assertEqual(provider.EXPECTED_NEGATIVE_ROWS, expected)

        cases = {}
        for name in expected:
            facts = valid_facts()
            if name == "handle_overflow":
                facts["handle"] = "ab" * 129
                facts["handle_length"] = 129
            elif name == "mode_query_failure":
                facts["mode_query_succeeded"] = False
            elif name == "network":
                facts["filesystem"] = "nfs"
            elif name == "no_uuid":
                facts["filesystem_uuid_length"] = 0
                facts["filesystem_uuid"] = ""
            elif name == "overlay":
                facts["filesystem"] = "overlay"
            elif name == "tmpfs":
                facts["filesystem"] = "tmpfs"
            elif name == "unknown_handle_provider":
                facts["handle_provider"] = "pathname"
            elif name == "zero_uuid":
                facts["filesystem_uuid"] = "00" * 16
            with self.assertRaises(provider.ProbeError) as caught:
                provider.validate_facts(facts)
            cases[name] = caught.exception.code
        self.assertEqual(cases, expected)

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
        common = {
            "schema_version": 1,
            "core_commit": "a" * 40,
            "workflow_run": "123",
            "probe_source_sha256": "b" * 64,
            "provider_table_sha256": provider.provider_table_digest(),
        }
        rows = []
        for architecture in ("linux-x86_64", "linux-aarch64"):
            row = dict(common)
            row.update(
                {
                    "architecture": architecture,
                    "tuple": provider.validate_facts(valid_facts()),
                    "remount": {"identity_equal": True},
                    "negative_rows": provider.EXPECTED_NEGATIVE_ROWS,
                }
            )
            rows.append(row)

        aggregate = provider.aggregate_evidence(rows)
        self.assertEqual(
            [row["architecture"] for row in aggregate["architectures"]],
            ["linux-aarch64", "linux-x86_64"],
        )
        self.assertEqual(aggregate["core_commit"], "a" * 40)

        with self.assertRaises(provider.ProbeError):
            provider.aggregate_evidence(rows[:1])
        mismatched = copy.deepcopy(rows)
        mismatched[1]["core_commit"] = "c" * 40
        with self.assertRaises(provider.ProbeError):
            provider.aggregate_evidence(mismatched)

    def test_canonical_json_is_byte_stable(self):
        value = {"b": 2, "a": [3, 1]}
        self.assertEqual(provider.canonical_json(value), b'{"a":[3,1],"b":2}\n')
        self.assertEqual(json.loads(provider.canonical_json(value)), value)


if __name__ == "__main__":
    unittest.main()

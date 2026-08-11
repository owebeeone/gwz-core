"""Pure contract for the R4b Linux durable-identity capability probe."""

from __future__ import annotations

import errno
import hashlib
import json
import pathlib
from typing import Any, Iterable

AT_SYMLINK_FOLLOW = 0x400
AT_EMPTY_PATH = 0x1000
AT_HANDLE_FID = 0x200

SUPPORT_TABLE = {
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
}

EXPECTED_NEGATIVE_ROWS = {
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

REQUIRED_ARCHITECTURES = {"linux-x86_64", "linux-aarch64"}
ARCHITECTURE_MACHINES = {
    "linux-x86_64": "x86_64",
    "linux-aarch64": "aarch64",
}
SHARED_EVIDENCE_FIELDS = (
    "schema_version",
    "core_commit",
    "workflow_run",
    "probe_source_sha256",
    "provider_table_sha256",
)
ROW_FIELDS = {
    "schema_version",
    "core_commit",
    "workflow_run",
    "architecture",
    "native_machine",
    "kernel_release",
    "probe_source_sha256",
    "provider_table_sha256",
    "tuple",
    "remount",
    "substitution",
    "query_contract",
    "negative_rows",
    "diagnostics",
}
TUPLE_FIELDS = {
    "provider",
    "filesystem",
    "filesystem_uuid",
    "handle_type",
    "handle",
    "handle_length",
    "path_modes",
}
QUERY_CONTRACT = {
    "missing_at_empty_path_errno": "ENOENT",
    "forbidden_flags_rejected_before_syscall": True,
    "pathname_fallback_rejected_before_syscall": True,
    "permission_denial_typed": "IoError",
    "unsupported_empty_path_typed": "UnsupportedOperation",
}


class ProbeError(RuntimeError):
    def __init__(self, code: str, reason: str):
        super().__init__(reason)
        self.code = code
        self.reason = reason


def canonical_json(value: Any) -> bytes:
    return (
        json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True)
        + "\n"
    ).encode("ascii")


def provider_table_digest() -> str:
    return hashlib.sha256(canonical_json(SUPPORT_TABLE)).hexdigest()


def probe_source_digest(directory: pathlib.Path) -> str:
    digest = hashlib.sha256()
    sources = sorted(
        path
        for path in directory.iterdir()
        if path.is_file() and path.suffix in {".py", ".md"}
    )
    for path in sources:
        relative = path.relative_to(directory).as_posix().encode("utf-8")
        payload = path.read_bytes()
        digest.update(len(relative).to_bytes(4, "big"))
        digest.update(relative)
        digest.update(len(payload).to_bytes(8, "big"))
        digest.update(payload)
    return digest.hexdigest()


def unsupported(reason: str) -> ProbeError:
    return ProbeError("UnsupportedOperation", reason)


def handle_query_error(error_number: int) -> ProbeError:
    if error_number in {errno.EOPNOTSUPP, getattr(errno, "ENOTSUP", errno.EOPNOTSUPP)}:
        return unsupported("retained empty-path handle lookup is unsupported")
    return ProbeError("IoError", f"retained handle query failed with errno {error_number}")


def validate_handle_query(*, path: bytes, flags: int) -> None:
    if path != b"":
        raise unsupported("persistent handle query must use an empty path")
    if flags != AT_EMPTY_PATH:
        raise unsupported("persistent handle query must use only AT_EMPTY_PATH")


def _hex_bytes(value: Any, field: str) -> bytes:
    if not isinstance(value, str) or len(value) % 2:
        raise unsupported(f"{field} is not an even-length hexadecimal string")
    try:
        return bytes.fromhex(value)
    except ValueError as error:
        raise unsupported(f"{field} is not hexadecimal") from error


def validate_facts(facts: dict[str, Any]) -> dict[str, Any]:
    if facts.get("filesystem") != "ext4":
        raise unsupported("filesystem is not the admitted ext4 provider")

    if facts.get("filesystem_uuid_length") != 16:
        raise unsupported("FS_IOC_GETFSUUID did not return a 16-byte UUID")
    uuid = _hex_bytes(facts.get("filesystem_uuid"), "filesystem_uuid")
    if len(uuid) != 16 or not any(uuid):
        raise unsupported("filesystem UUID is absent or all zero")

    if facts.get("handle_provider") != "name_to_handle_at-empty-path":
        raise unsupported("persistent handle did not come from the retained fd")
    handle_type = facts.get("handle_type")
    if not isinstance(handle_type, int) or isinstance(handle_type, bool) or handle_type <= 0:
        raise unsupported("filesystem returned an invalid handle type")
    handle = _hex_bytes(facts.get("handle"), "handle")
    handle_length = facts.get("handle_length")
    if (
        not isinstance(handle_length, int)
        or isinstance(handle_length, bool)
        or handle_length != len(handle)
        or not 1 <= handle_length <= 128
    ):
        raise unsupported("filesystem returned an unsupported handle length")

    if facts.get("mode_query_succeeded") is not True:
        raise unsupported("filesystem path-equivalence query failed")
    modes = facts.get("path_modes")
    if modes != {"sensitive": "Sensitive", "casefold": "AsciiCaseFold"}:
        raise unsupported("ext4 sensitive/casefold capability rows are incomplete")

    return {
        "provider": "FsIocGetFsUuid",
        "filesystem": "ext4",
        "filesystem_uuid": uuid.hex(),
        "handle_type": handle_type,
        "handle": handle.hex(),
        "handle_length": handle_length,
        "path_modes": dict(modes),
    }


def compare_remount(before: dict[str, Any], after: dict[str, Any]) -> dict[str, Any]:
    before_tuple = validate_facts(before)
    try:
        after_tuple = validate_facts(after)
    except ProbeError as error:
        raise ProbeError(
            "Ambiguity",
            f"remount no longer satisfies the admitted tuple: {error.reason}",
        ) from error
    for field in ("filesystem_uuid", "handle_type", "handle", "path_modes"):
        if before_tuple[field] != after_tuple[field]:
            raise ProbeError("Ambiguity", f"remount changed durable field {field}")
    return {
        "identity_equal": True,
        "mount_ids": [before.get("mount_id"), after.get("mount_id")],
        "mount_id_is_non_authoritative": True,
    }


def validate_negative_rows(rows: dict[str, str]) -> None:
    if rows != EXPECTED_NEGATIVE_ROWS:
        raise ProbeError(
            "EvidenceInvalid",
            "negative provider rows do not exactly match the support table",
        )


def _require_exact_keys(value: Any, expected: set[str], label: str) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != expected:
        raise ProbeError("EvidenceInvalid", f"{label} does not have the exact schema")
    return value


def _require_hex(value: Any, length: int, label: str) -> str:
    if (
        not isinstance(value, str)
        or len(value) != length
        or any(character not in "0123456789abcdef" for character in value)
    ):
        raise ProbeError("EvidenceInvalid", f"{label} has the wrong length")
    return value


def _validate_normalized_tuple(value: Any) -> None:
    value = _require_exact_keys(value, TUPLE_FIELDS, "durable tuple")
    if value.get("provider") != "FsIocGetFsUuid" or value.get("filesystem") != "ext4":
        raise ProbeError("EvidenceInvalid", "durable tuple uses an unsupported provider")
    uuid = _require_hex(value.get("filesystem_uuid"), 32, "filesystem UUID")
    if not any(bytes.fromhex(uuid)):
        raise ProbeError("EvidenceInvalid", "filesystem UUID is all zero")
    handle_type = value.get("handle_type")
    handle_length = value.get("handle_length")
    if not isinstance(handle_type, int) or isinstance(handle_type, bool) or handle_type <= 0:
        raise ProbeError("EvidenceInvalid", "handle type is not positive")
    if (
        not isinstance(handle_length, int)
        or isinstance(handle_length, bool)
        or not 1 <= handle_length <= 128
    ):
        raise ProbeError("EvidenceInvalid", "handle length is out of bounds")
    _require_hex(value.get("handle"), handle_length * 2, "persistent handle")
    if value.get("path_modes") != {
        "sensitive": "Sensitive",
        "casefold": "AsciiCaseFold",
    }:
        raise ProbeError("EvidenceInvalid", "path-mode evidence is incomplete")


def _validate_evidence_row(
    row: Any,
    *,
    expected_core_commit: str,
    expected_workflow_run: str,
    expected_source_sha256: str,
) -> None:
    row = _require_exact_keys(row, ROW_FIELDS, "architecture evidence")
    if row.get("schema_version") != 1:
        raise ProbeError("EvidenceInvalid", "unsupported evidence schema")
    if row.get("core_commit") != expected_core_commit:
        raise ProbeError("EvidenceInvalid", "core commit does not match the workflow")
    if row.get("workflow_run") != expected_workflow_run:
        raise ProbeError("EvidenceInvalid", "workflow run does not match the workflow")
    architecture = row.get("architecture")
    if row.get("native_machine") != ARCHITECTURE_MACHINES.get(architecture):
        raise ProbeError("EvidenceInvalid", "architecture does not match native machine")
    if not isinstance(row.get("kernel_release"), str) or not row["kernel_release"]:
        raise ProbeError("EvidenceInvalid", "kernel release is missing")
    if row.get("probe_source_sha256") != expected_source_sha256:
        raise ProbeError("EvidenceInvalid", "probe source digest mismatch")
    if row.get("provider_table_sha256") != provider_table_digest():
        raise ProbeError("EvidenceInvalid", "provider table digest mismatch")

    _validate_normalized_tuple(row.get("tuple"))
    validate_negative_rows(row.get("negative_rows"))
    if row.get("query_contract") != QUERY_CONTRACT:
        raise ProbeError("EvidenceInvalid", "query contract evidence is incomplete")

    substitution = _require_exact_keys(
        row.get("substitution"),
        {"retained_handle_unchanged", "replacement_handle_different"},
        "substitution evidence",
    )
    if substitution != {
        "retained_handle_unchanged": True,
        "replacement_handle_different": True,
    }:
        raise ProbeError("EvidenceInvalid", "descriptor substitution proof failed")

    remount = _require_exact_keys(
        row.get("remount"),
        {"identity_equal", "mount_ids", "mount_id_is_non_authoritative"},
        "remount evidence",
    )
    mount_ids = remount.get("mount_ids")
    if (
        remount.get("identity_equal") is not True
        or remount.get("mount_id_is_non_authoritative") is not True
        or not isinstance(mount_ids, list)
        or len(mount_ids) != 2
        or any(not isinstance(value, int) or isinstance(value, bool) for value in mount_ids)
    ):
        raise ProbeError("EvidenceInvalid", "remount evidence is incomplete")

    diagnostics = _require_exact_keys(
        row.get("diagnostics"),
        {"mount_id_before", "mount_id_after"},
        "mount diagnostics",
    )
    if [diagnostics["mount_id_before"], diagnostics["mount_id_after"]] != mount_ids:
        raise ProbeError("EvidenceInvalid", "mount diagnostics disagree with remount evidence")


def aggregate_evidence(
    rows: Iterable[dict[str, Any]],
    *,
    expected_core_commit: str,
    expected_workflow_run: str,
    expected_source_sha256: str,
) -> dict[str, Any]:
    rows = list(rows)
    if any(not isinstance(row, dict) for row in rows):
        raise ProbeError("EvidenceInvalid", "architecture evidence is not an object")
    architectures = {row.get("architecture") for row in rows}
    if architectures != REQUIRED_ARCHITECTURES or len(rows) != 2:
        raise ProbeError("EvidenceInvalid", "both Linux release architectures are required")

    _require_hex(expected_core_commit, 40, "expected core commit")
    if not isinstance(expected_workflow_run, str) or not expected_workflow_run.isdigit():
        raise ProbeError("EvidenceInvalid", "expected workflow run is not numeric")
    _require_hex(expected_source_sha256, 64, "expected source digest")

    reference = rows[0]
    for row in rows:
        _validate_evidence_row(
            row,
            expected_core_commit=expected_core_commit,
            expected_workflow_run=expected_workflow_run,
            expected_source_sha256=expected_source_sha256,
        )
        for field in SHARED_EVIDENCE_FIELDS:
            if row.get(field) != reference.get(field):
                raise ProbeError("EvidenceInvalid", f"evidence disagrees on {field}")

    return {
        "schema_version": 1,
        "core_commit": reference["core_commit"],
        "workflow_run": reference["workflow_run"],
        "probe_source_sha256": reference["probe_source_sha256"],
        "provider_table_sha256": reference["provider_table_sha256"],
        "architectures": sorted(rows, key=lambda row: row["architecture"]),
    }

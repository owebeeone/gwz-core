"""Pure contract for the R4b Linux durable-identity capability probe."""

from __future__ import annotations

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
    "mode_query_failure": "UnsupportedOperation",
    "network": "UnsupportedOperation",
    "no_uuid": "UnsupportedOperation",
    "overlay": "UnsupportedOperation",
    "tmpfs": "UnsupportedOperation",
    "unknown_handle_provider": "UnsupportedOperation",
    "zero_uuid": "UnsupportedOperation",
}

REQUIRED_ARCHITECTURES = {"linux-x86_64", "linux-aarch64"}
SHARED_EVIDENCE_FIELDS = (
    "schema_version",
    "core_commit",
    "workflow_run",
    "probe_source_sha256",
    "provider_table_sha256",
)


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


def aggregate_evidence(rows: Iterable[dict[str, Any]]) -> dict[str, Any]:
    rows = list(rows)
    architectures = {row.get("architecture") for row in rows}
    if architectures != REQUIRED_ARCHITECTURES or len(rows) != 2:
        raise ProbeError("EvidenceInvalid", "both Linux release architectures are required")

    reference = rows[0]
    for row in rows:
        for field in SHARED_EVIDENCE_FIELDS:
            if row.get(field) != reference.get(field):
                raise ProbeError("EvidenceInvalid", f"evidence disagrees on {field}")
        if row.get("schema_version") != 1:
            raise ProbeError("EvidenceInvalid", "unsupported evidence schema")
        if row.get("provider_table_sha256") != provider_table_digest():
            raise ProbeError("EvidenceInvalid", "provider table digest mismatch")
        if row.get("negative_rows") != EXPECTED_NEGATIVE_ROWS:
            raise ProbeError("EvidenceInvalid", "negative row evidence mismatch")
        if row.get("remount", {}).get("identity_equal") is not True:
            raise ProbeError("EvidenceInvalid", "remount identity was not preserved")

    return {
        "schema_version": 1,
        "core_commit": reference["core_commit"],
        "workflow_run": reference["workflow_run"],
        "probe_source_sha256": reference["probe_source_sha256"],
        "provider_table_sha256": reference["provider_table_sha256"],
        "architectures": sorted(rows, key=lambda row: row["architecture"]),
    }

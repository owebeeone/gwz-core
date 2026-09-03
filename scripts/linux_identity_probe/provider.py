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

EVIDENCE_SCHEMA_VERSION = 2

# The names of the filesystems whose contents do not survive power loss. The
# production provider refuses them by superblock magic before it asks for
# identity at all (`platform/linux.rs::refuse_volatile_filesystem`); this
# package mirrors that test by NAME because `run_probe.py` derives the name
# from the observed magic, one to one. Volatility is the one name-shaped test
# the identity contract keeps, and it is a CATALOG ADMISSION refusal only: a
# merge on tmpfs still starts, warns once, and runs without crash recovery
# (DR-1 ship (1) charter §0.1).
VOLATILE_FILESYSTEM_NAMES = ("ramfs", "tmpfs")

# Evidence vocabulary, NOT a production denylist. The gate admits a volume on
# what it can PROVE — a nonzero 16-byte `FS_IOC_GETFSUUID` UUID plus a
# persistent `name_to_handle_at` handle — and records the filesystem's name
# rather than testing it. This table only documents which Linux filesystems
# are known to satisfy that contract today, i.e. which ones
# `--filesystem-strict` admits: they are the three that call `super_set_uuid`
# in the kernel (ext4 `fs/ext4/super.c:5344`, f2fs `fs/f2fs/super.c:4433`,
# xfs `fs/xfs/xfs_mount.c:65`, all at v6.9). A filesystem outside this table
# that answers the contract is admitted by the provider and by this gate;
# nothing anywhere consults the table to refuse.
STRONG_TABLE = ("ext4", "f2fs", "xfs")

# The row the gate proves in full — fixed external UUID, remount stability,
# descriptor substitution, the whole negative table.
REFERENCE_POSITIVE_FILESYSTEM = "ext4"

# The strong-table rows the native probe additionally builds and remounts.
# `ext4` is proved by the reference row above and is not repeated here.
STRONG_TABLE_ROWS = ("xfs",)

# Filesystems that cannot carry an `FS_CASEFOLD_FL` directory, so their
# path-mode evidence is two `Sensitive` rows rather than one of each.
CASEFOLD_CAPABLE = frozenset({"ext4", "f2fs"})

SUPPORT_TABLE = {
    "schema_version": 2,
    "identity_contract": {
        "provider": "FsIocGetFsUuid",
        "uuid_bytes": 16,
        "max_handle_bytes": 128,
        "handle_query": "retained-fd-empty-path",
        "volatile_filesystems_refused": list(VOLATILE_FILESYSTEM_NAMES),
    },
    "reference_positive": REFERENCE_POSITIVE_FILESYSTEM,
    "strong_table": list(STRONG_TABLE),
    "strong_table_is_a_denylist": False,
}

# The closed negative table. Every verdict here is unchanged by DR-1 ship (1);
# what changed is WHY three of the rows refuse, now that the gate no longer
# tests the filesystem name:
#   * `tmpfs` refuses as a VOLATILE filesystem (the provider's magic test),
#     not as "not ext4". `run_probe.py` proves it on a real tmpfs mount.
#   * `overlay` refuses because `name_to_handle_at` answers `EOPNOTSUPP` on an
#     overlay without `nfs_export`. `run_probe.py` proves it on a real overlay
#     mount by attempting the contract for real.
#   * `network` refuses because a remote volume publishes no filesystem UUID
#     (NFS does not call `super_set_uuid`, so the ioctl cannot answer). The
#     name `nfs` is a warning REASON — `remote filesystem` — never a denylist
#     entry; the row asserts the evidence, and carries the name only to say
#     which substrate the evidence describes.
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
    "strong_table",
    "remount",
    "substitution",
    "query_contract",
    "negative_rows",
    "diagnostics",
}
STRONG_TABLE_ROW_FIELDS = {"tuple", "remount"}
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


def expected_path_modes(filesystem: str) -> dict[str, str]:
    """The path-equivalence evidence a positive row must carry.

    `FS_CASEFOLD_FL` is an ext4/f2fs feature; xfs has no case-folding mode, so
    an xfs row proves two `Sensitive` directories rather than one of each. The
    capability under test is that the mode query ANSWERS for both directories,
    not that the substrate can fold.
    """

    casefold = "AsciiCaseFold" if filesystem in CASEFOLD_CAPABLE else "Sensitive"
    return {"sensitive": "Sensitive", "casefold": casefold}


def _filesystem_name(value: Any) -> str:
    if not isinstance(value, str) or not value:
        raise unsupported("filesystem name is missing")
    return value


def validate_facts(facts: dict[str, Any]) -> dict[str, Any]:
    """Admit a volume on the identity contract, recording the name it reports.

    The evaluation order mirrors `platform/linux.rs::identity` exactly:
    volatility, then the external UUID, then the persistent handle. There is
    no filesystem-name test: ext4, xfs, f2fs and any other filesystem that
    answers `FS_IOC_GETFSUUID` with a nonzero 16-byte UUID and a persistent
    `name_to_handle_at` handle are admitted alike, and the name is carried
    into the tuple as evidence rather than consulted as a gate.
    """

    name = _filesystem_name(facts.get("filesystem"))
    if name in VOLATILE_FILESYSTEM_NAMES:
        raise unsupported("volatile filesystem: contents do not survive power loss")

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
    if modes != expected_path_modes(name):
        raise unsupported("sensitive/casefold capability rows are incomplete")

    return {
        "provider": "FsIocGetFsUuid",
        "filesystem": name,
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


def _validate_normalized_tuple(value: Any, *, expected_filesystem: str | None = None) -> None:
    value = _require_exact_keys(value, TUPLE_FIELDS, "durable tuple")
    if value.get("provider") != "FsIocGetFsUuid":
        raise ProbeError("EvidenceInvalid", "durable tuple uses an unsupported provider")
    filesystem = value.get("filesystem")
    if not isinstance(filesystem, str) or not filesystem:
        raise ProbeError("EvidenceInvalid", "durable tuple does not name its filesystem")
    if filesystem in VOLATILE_FILESYSTEM_NAMES:
        raise ProbeError("EvidenceInvalid", "durable tuple names a volatile filesystem")
    if expected_filesystem is not None and filesystem != expected_filesystem:
        raise ProbeError(
            "EvidenceInvalid", f"durable tuple is not the declared {expected_filesystem} row"
        )
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
    if value.get("path_modes") != expected_path_modes(filesystem):
        raise ProbeError("EvidenceInvalid", "path-mode evidence is incomplete")


def _validate_remount(value: Any, label: str) -> list[int]:
    remount = _require_exact_keys(
        value,
        {"identity_equal", "mount_ids", "mount_id_is_non_authoritative"},
        label,
    )
    mount_ids = remount.get("mount_ids")
    if (
        remount.get("identity_equal") is not True
        or remount.get("mount_id_is_non_authoritative") is not True
        or not isinstance(mount_ids, list)
        or len(mount_ids) != 2
        or any(not isinstance(value, int) or isinstance(value, bool) for value in mount_ids)
    ):
        raise ProbeError("EvidenceInvalid", f"{label} is incomplete")
    return mount_ids


def _validate_strong_table(value: Any) -> None:
    """Every strong-table filesystem the native probe could build must appear.

    The rows are evidence that `--filesystem-strict` admits more than ext4:
    each one is a real loop-mounted filesystem whose UUID came through the
    same ioctl, whose handle came from the same retained-descriptor query, and
    which reproduced both across a remount.
    """

    rows = _require_exact_keys(value, set(STRONG_TABLE_ROWS), "strong table evidence")
    for filesystem, row in sorted(rows.items()):
        if filesystem not in STRONG_TABLE:
            raise ProbeError("EvidenceInvalid", "strong table row is not in the strong table")
        row = _require_exact_keys(
            row, STRONG_TABLE_ROW_FIELDS, f"{filesystem} strong table row"
        )
        _validate_normalized_tuple(row.get("tuple"), expected_filesystem=filesystem)
        _validate_remount(row.get("remount"), f"{filesystem} remount evidence")


def _validate_evidence_row(
    row: Any,
    *,
    expected_core_commit: str,
    expected_workflow_run: str,
    expected_source_sha256: str,
) -> None:
    row = _require_exact_keys(row, ROW_FIELDS, "architecture evidence")
    if row.get("schema_version") != EVIDENCE_SCHEMA_VERSION:
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

    _validate_normalized_tuple(
        row.get("tuple"), expected_filesystem=REFERENCE_POSITIVE_FILESYSTEM
    )
    _validate_strong_table(row.get("strong_table"))
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

    mount_ids = _validate_remount(row.get("remount"), "remount evidence")

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
        "schema_version": EVIDENCE_SCHEMA_VERSION,
        "core_commit": reference["core_commit"],
        "workflow_run": reference["workflow_run"],
        "probe_source_sha256": reference["probe_source_sha256"],
        "provider_table_sha256": reference["provider_table_sha256"],
        "architectures": sorted(rows, key=lambda row: row["architecture"]),
    }

#!/usr/bin/env python3
"""Fail-not-skip native probe for the Linux durable-identity provider.

The gate is the identity contract, not a filesystem name. ext4 remains the
REFERENCE positive row — it is the one built with a fixed external UUID and
carried through remount, descriptor substitution and the whole negative
table — and every other strong-table filesystem the runner can build is
proved to satisfy the same contract (`strong_table`). The negative rows for
tmpfs and overlay are taken from REAL mounts by attempting the contract for
real, so their verdicts rest on what the kernel answered rather than on the
name the mount reports.
"""

from __future__ import annotations

import argparse
import array
import ctypes
import errno
import fcntl
import json
import os
import pathlib
import platform
import shutil
import subprocess
import tempfile
import uuid

import provider

EXT4_SUPER_MAGIC = 0xEF53
XFS_SUPER_MAGIC = 0x58465342
OVERLAYFS_SUPER_MAGIC = 0x794C7630
TMPFS_MAGIC = 0x01021994
RAMFS_MAGIC = 0x858458F6
FS_CASEFOLD_FL = 0x40000000
FIXED_UUID = uuid.UUID("718c918e-3cc3-43c9-ae9e-27f5cecc8a17")
MAX_HANDLE_BYTES = 128

# How each strong-table filesystem is built on a loop device. `casefold` says
# whether the fixture can carry an `FS_CASEFOLD_FL` directory; xfs cannot, and
# `provider.expected_path_modes` expects two `Sensitive` rows there.
STRONG_TABLE_BUILDS = {
    "xfs": {
        "tools": ("mkfs.xfs",),
        # xfsprogs refuses to format anything smaller than 300MB.
        "size": "512M",
        "mkfs": ("mkfs.xfs", "-q", "-f"),
        "casefold": False,
    },
}


def _ior(type_value: int, number: int, size: int) -> int:
    return (2 << 30) | (size << 16) | (type_value << 8) | number


FS_IOC_GETFSUUID = _ior(0x15, 0, 17)
FS_IOC_GETFLAGS = _ior(ord("f"), 1, ctypes.sizeof(ctypes.c_long))


class FileHandle(ctypes.Structure):
    _fields_ = [
        ("handle_bytes", ctypes.c_uint32),
        ("handle_type", ctypes.c_int32),
        ("value", ctypes.c_ubyte * MAX_HANDLE_BYTES),
    ]


class Fsid(ctypes.Structure):
    _fields_ = [("value", ctypes.c_int32 * 2)]


class StatFs(ctypes.Structure):
    _fields_ = [
        ("f_type", ctypes.c_long),
        ("f_bsize", ctypes.c_long),
        ("f_blocks", ctypes.c_ulong),
        ("f_bfree", ctypes.c_ulong),
        ("f_bavail", ctypes.c_ulong),
        ("f_files", ctypes.c_ulong),
        ("f_ffree", ctypes.c_ulong),
        ("f_fsid", Fsid),
        ("f_namelen", ctypes.c_long),
        ("f_frsize", ctypes.c_long),
        ("f_flags", ctypes.c_long),
        ("f_spare", ctypes.c_long * 4),
    ]


LIBC = ctypes.CDLL(None, use_errno=True)
LIBC.name_to_handle_at.argtypes = [
    ctypes.c_int,
    ctypes.c_char_p,
    ctypes.POINTER(FileHandle),
    ctypes.POINTER(ctypes.c_int),
    ctypes.c_int,
]
LIBC.name_to_handle_at.restype = ctypes.c_int
LIBC.fstatfs.argtypes = [ctypes.c_int, ctypes.POINTER(StatFs)]
LIBC.fstatfs.restype = ctypes.c_int


def command(*args: str) -> None:
    subprocess.run(args, check=True)


def filesystem_magic(fd: int) -> int:
    value = StatFs()
    if LIBC.fstatfs(fd, ctypes.byref(value)) != 0:
        raise OSError(ctypes.get_errno(), os.strerror(ctypes.get_errno()))
    return value.f_type & 0xFFFFFFFF


def filesystem_name(magic: int) -> str:
    """Name the substrate from its superblock magic, one to one.

    `provider.validate_facts` mirrors the production provider's volatility
    refusal by NAME; this map is what makes that mirror faithful, because the
    name a fact set carries is always derived from the magic the kernel
    reported for that mount.
    """

    return {
        EXT4_SUPER_MAGIC: "ext4",
        XFS_SUPER_MAGIC: "xfs",
        OVERLAYFS_SUPER_MAGIC: "overlay",
        TMPFS_MAGIC: "tmpfs",
        RAMFS_MAGIC: "ramfs",
    }.get(magic, f"unknown-{magic:08x}")


def filesystem_uuid(fd: int) -> tuple[int, bytes]:
    payload = bytearray(17)
    fcntl.ioctl(fd, FS_IOC_GETFSUUID, payload, True)
    return payload[0], bytes(payload[1:17])


def inode_flags(fd: int) -> int:
    payload = array.array("l", [0])
    fcntl.ioctl(fd, FS_IOC_GETFLAGS, payload, True)
    return int(payload[0])


def retained_handle(fd: int, *, path: bytes = b"", flags: int = provider.AT_EMPTY_PATH):
    provider.validate_handle_query(path=path, flags=flags)
    handle = FileHandle(handle_bytes=MAX_HANDLE_BYTES)
    mount_id = ctypes.c_int()
    result = LIBC.name_to_handle_at(
        fd, path, ctypes.byref(handle), ctypes.byref(mount_id), flags
    )
    if result != 0:
        error = ctypes.get_errno()
        raise provider.handle_query_error(error)
    length = int(handle.handle_bytes)
    if not 1 <= length <= MAX_HANDLE_BYTES:
        raise provider.unsupported("name_to_handle_at returned an invalid length")
    return int(handle.handle_type), bytes(handle.value[:length]), mount_id.value


def mount_id(fd: int) -> int:
    text = pathlib.Path(f"/proc/self/fdinfo/{fd}").read_text(encoding="ascii")
    for line in text.splitlines():
        if line.startswith("mnt_id:"):
            return int(line.split(":", 1)[1].strip())
    raise RuntimeError("fdinfo did not expose mnt_id")


def open_nofollow(path: pathlib.Path, *, directory: bool = False) -> int:
    flags = os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW
    if directory:
        flags |= os.O_DIRECTORY
    return os.open(path, flags)


def path_mode(path: pathlib.Path) -> str:
    fd = open_nofollow(path, directory=True)
    try:
        return "AsciiCaseFold" if inode_flags(fd) & FS_CASEFOLD_FL else "Sensitive"
    finally:
        os.close(fd)


def snapshot(root: pathlib.Path) -> dict[str, object]:
    fd = open_nofollow(root / "sensitive" / "stable")
    try:
        magic = filesystem_magic(fd)
        uuid_length, uuid_bytes = filesystem_uuid(fd)
        handle_type, handle, syscall_mount_id = retained_handle(fd)
        observed_mount_id = mount_id(fd)
    finally:
        os.close(fd)
    if syscall_mount_id != observed_mount_id:
        raise RuntimeError("name_to_handle_at and fdinfo mount IDs disagree")
    return {
        "filesystem": filesystem_name(magic),
        "filesystem_uuid": uuid_bytes.hex(),
        "filesystem_uuid_length": uuid_length,
        "handle_provider": "name_to_handle_at-empty-path",
        "handle_type": handle_type,
        "handle": handle.hex(),
        "handle_length": len(handle),
        "path_modes": {
            "sensitive": path_mode(root / "sensitive"),
            "casefold": path_mode(root / "casefold"),
        },
        "mode_query_succeeded": True,
        "mount_id": observed_mount_id,
    }


def descriptor_substitution(root: pathlib.Path) -> dict[str, object]:
    original = root / "sensitive" / "descriptor-object"
    replacement = root / "sensitive" / "descriptor-replacement"
    retained_name = root / "sensitive" / "descriptor-retained"
    original.write_bytes(b"A")
    replacement.write_bytes(b"B")
    fd = open_nofollow(original)
    try:
        before = retained_handle(fd)
        original.rename(retained_name)
        replacement.rename(original)
        after = retained_handle(fd)
        replacement_fd = open_nofollow(original)
        try:
            replacement_value = retained_handle(replacement_fd)
        finally:
            os.close(replacement_fd)
    finally:
        os.close(fd)
    if before[:2] != after[:2] or after[:2] == replacement_value[:2]:
        raise RuntimeError("empty-path handle did not remain descriptor-bound")
    return {
        "retained_handle_unchanged": True,
        "replacement_handle_different": True,
    }


def create_fixture(root: pathlib.Path, *, casefold: bool = True) -> None:
    (root / "sensitive").mkdir()
    (root / "casefold").mkdir()
    if casefold:
        command("chattr", "+F", str(root / "casefold"))
    (root / "sensitive" / "stable").write_bytes(b"gwz-r4b-linux-probe\n")


def observed_facts(sample: pathlib.Path, modes_root: pathlib.Path) -> dict[str, object]:
    """Ask a real mount for the identity contract, in the provider's order.

    Raises `provider.ProbeError` at the first step the substrate cannot
    answer, exactly as `platform/linux.rs::identity` fails at the first
    syscall that refuses.
    """

    fd = open_nofollow(sample)
    try:
        name = filesystem_name(filesystem_magic(fd))
        try:
            uuid_length, uuid_bytes = filesystem_uuid(fd)
        except OSError as error:
            raise provider.unsupported(
                f"FS_IOC_GETFSUUID refused with errno {error.errno}"
            ) from error
        handle_type, handle, _ = retained_handle(fd)
    finally:
        os.close(fd)
    try:
        modes = {
            "sensitive": path_mode(modes_root / "sensitive"),
            "casefold": path_mode(modes_root / "casefold"),
        }
        mode_query_succeeded = True
    except OSError:
        modes = {}
        mode_query_succeeded = False
    return {
        "filesystem": name,
        "filesystem_uuid": uuid_bytes.hex(),
        "filesystem_uuid_length": uuid_length,
        "handle_provider": "name_to_handle_at-empty-path",
        "handle_type": handle_type,
        "handle": handle.hex(),
        "handle_length": len(handle),
        "path_modes": modes,
        "mode_query_succeeded": mode_query_succeeded,
    }


def reject_real_mount(sample: pathlib.Path, modes_root: pathlib.Path, expected: str) -> str:
    """Prove a real mount is below the identity bar, and return its typed code.

    Nothing here consults a name list. tmpfs answers the UUID ioctl and the
    handle query on every kernel that has the ioctl at all, and is refused
    for being VOLATILE; overlay without `nfs_export` is refused by
    `name_to_handle_at` itself.
    """

    fd = open_nofollow(modes_root, directory=True)
    try:
        observed = filesystem_name(filesystem_magic(fd))
    finally:
        os.close(fd)
    if observed != expected:
        raise RuntimeError(f"expected {expected}, observed {observed}")
    try:
        facts = observed_facts(sample, modes_root)
    except provider.ProbeError as error:
        return error.code
    try:
        provider.validate_facts(facts)
    except provider.ProbeError as error:
        return error.code
    raise RuntimeError(f"{expected} unexpectedly satisfied the identity contract")


def synthetic_negative_rows(valid: dict[str, object]) -> dict[str, str]:
    rows = {
        "overlay": "pending-real",
        "tmpfs": "pending-real",
    }
    variants = {
        # A remote volume publishes no filesystem UUID: NFS never calls
        # `super_set_uuid`, so `FS_IOC_GETFSUUID` cannot answer for it. The
        # name is carried to say which substrate the evidence describes; it is
        # a warning REASON, not a denylist entry.
        "network": {
            "filesystem": "nfs",
            "filesystem_uuid": "",
            "filesystem_uuid_length": 0,
        },
        "zero_uuid": {"filesystem_uuid": "00" * 16},
        "no_uuid": {"filesystem_uuid": "", "filesystem_uuid_length": 0},
        "malformed_uuid_length": {
            "filesystem_uuid": "aa" * 15,
            "filesystem_uuid_length": 15,
        },
        "handle_overflow": {"handle": "aa" * 129, "handle_length": 129},
        "unknown_handle_provider": {"handle_provider": "pathname"},
        "mode_query_failure": {"mode_query_succeeded": False},
    }
    for name, changes in variants.items():
        facts = dict(valid)
        facts.update(changes)
        try:
            provider.validate_facts(facts)
        except provider.ProbeError as error:
            rows[name] = error.code
        else:
            raise RuntimeError(f"negative row {name} unexpectedly succeeded")

    queries = {
        "missing_at_empty_path": (b"", 0),
        "symlink_follow": (
            b"",
            provider.AT_EMPTY_PATH | provider.AT_SYMLINK_FOLLOW,
        ),
        "handle_fid": (b"", provider.AT_EMPTY_PATH | provider.AT_HANDLE_FID),
        "pathname_fallback": (b"stable", provider.AT_EMPTY_PATH),
    }
    for name, (path, flags) in queries.items():
        try:
            provider.validate_handle_query(path=path, flags=flags)
        except provider.ProbeError as error:
            rows[name] = error.code
        else:
            raise RuntimeError(f"query negative row {name} unexpectedly succeeded")
    rows["permission_denial"] = provider.handle_query_error(errno.EACCES).code
    rows["unsupported_empty_path"] = provider.handle_query_error(errno.EOPNOTSUPP).code
    return rows


def strong_table_row(
    temporary: pathlib.Path, filesystem: str, mounted: list[pathlib.Path]
) -> dict[str, object]:
    """Build one strong-table filesystem on a loop device and prove the contract.

    This is the evidence that `--filesystem-strict` is identity-based rather
    than an ext4 name test: the UUID comes through the same
    `FS_IOC_GETFSUUID`, the handle through the same retained-descriptor query,
    and both survive an unmount/remount cycle.
    """

    build = STRONG_TABLE_BUILDS[filesystem]
    image = temporary / f"{filesystem}.img"
    mountpoint = temporary / filesystem
    image.touch()
    command("truncate", "-s", build["size"], str(image))
    command(*build["mkfs"], str(image))
    mountpoint.mkdir()
    command("mount", "-o", "loop", str(image), str(mountpoint))
    mounted.append(mountpoint)
    create_fixture(mountpoint, casefold=build["casefold"])
    before = snapshot(mountpoint)
    if before["filesystem"] != filesystem:
        raise RuntimeError(f"expected {filesystem}, observed {before['filesystem']}")
    command("sync")
    command("umount", str(mountpoint))
    mounted.remove(mountpoint)
    command("mount", "-o", "loop", str(image), str(mountpoint))
    mounted.append(mountpoint)
    after = snapshot(mountpoint)
    return {
        "tuple": provider.validate_facts(before),
        "remount": provider.compare_remount(before, after),
    }


def raw_missing_empty_path_error(fd: int) -> int:
    handle = FileHandle(handle_bytes=MAX_HANDLE_BYTES)
    value = ctypes.c_int()
    result = LIBC.name_to_handle_at(fd, b"", ctypes.byref(handle), ctypes.byref(value), 0)
    if result == 0:
        raise RuntimeError("empty path without AT_EMPTY_PATH unexpectedly succeeded")
    return ctypes.get_errno()


def run(args: argparse.Namespace) -> dict[str, object]:
    if platform.system() != "Linux":
        raise RuntimeError("native Linux identity probe cannot run on this host")
    required = ["mkfs.ext4", "mount", "umount", "chattr"]
    for filesystem in provider.STRONG_TABLE_ROWS:
        required.extend(STRONG_TABLE_BUILDS[filesystem]["tools"])
    for executable in required:
        if shutil.which(executable) is None:
            raise RuntimeError(f"required executable is unavailable: {executable}")
    native_machine = platform.machine()
    if provider.ARCHITECTURE_MACHINES[args.architecture] != native_machine:
        raise RuntimeError(
            f"declared architecture {args.architecture} does not match {native_machine}"
        )

    temporary = pathlib.Path(tempfile.mkdtemp(prefix="gwz-r4b-linux-"))
    image = temporary / "ext4.img"
    mountpoint = temporary / "ext4"
    tmpfs = temporary / "tmpfs"
    overlay = temporary / "overlay"
    mounted: list[pathlib.Path] = []
    try:
        image.touch()
        command("truncate", "-s", "256M", str(image))
        command("mkfs.ext4", "-q", "-F", "-O", "casefold", "-U", str(FIXED_UUID), str(image))
        mountpoint.mkdir()
        command("mount", "-o", "loop", str(image), str(mountpoint))
        mounted.append(mountpoint)
        create_fixture(mountpoint)
        before = snapshot(mountpoint)
        if before["filesystem_uuid"] != FIXED_UUID.hex:
            raise RuntimeError("FS_IOC_GETFSUUID differs from the mkfs external UUID")
        substitution = descriptor_substitution(mountpoint)
        stable_fd = open_nofollow(mountpoint / "sensitive" / "stable")
        try:
            missing_empty_path_errno = raw_missing_empty_path_error(stable_fd)
        finally:
            os.close(stable_fd)
        if missing_empty_path_errno != errno.ENOENT:
            raise RuntimeError("missing AT_EMPTY_PATH did not fail with ENOENT")
        command("sync")
        command("umount", str(mountpoint))
        mounted.remove(mountpoint)
        command("mount", "-o", "loop", str(image), str(mountpoint))
        mounted.append(mountpoint)
        after = snapshot(mountpoint)
        remount = provider.compare_remount(before, after)

        strong_table = {
            filesystem: strong_table_row(temporary, filesystem, mounted)
            for filesystem in provider.STRONG_TABLE_ROWS
        }

        tmpfs.mkdir()
        command("mount", "-t", "tmpfs", "tmpfs", str(tmpfs))
        mounted.append(tmpfs)
        create_fixture(tmpfs, casefold=False)

        for name in ("lower", "upper", "work", "merged"):
            (overlay / name).mkdir(parents=True, exist_ok=True)
        (overlay / "lower" / "sentinel").write_text("overlay\n", encoding="ascii")
        options = ",".join(
            f"{name}dir={overlay / name}" for name in ("lower", "upper", "work")
        )
        command("mount", "-t", "overlay", "overlay", "-o", options, str(overlay / "merged"))
        mounted.append(overlay / "merged")
        create_fixture(overlay / "merged", casefold=False)

        negative_rows = synthetic_negative_rows(before)
        negative_rows["tmpfs"] = reject_real_mount(
            tmpfs / "sensitive" / "stable", tmpfs, "tmpfs"
        )
        negative_rows["overlay"] = reject_real_mount(
            overlay / "merged" / "sensitive" / "stable", overlay / "merged", "overlay"
        )
        provider.validate_negative_rows(negative_rows)
        query_contract = {
            "missing_at_empty_path_errno": errno.errorcode[missing_empty_path_errno],
            "forbidden_flags_rejected_before_syscall": all(
                negative_rows[name] == "UnsupportedOperation"
                for name in ("missing_at_empty_path", "symlink_follow", "handle_fid")
            ),
            "pathname_fallback_rejected_before_syscall": (
                negative_rows["pathname_fallback"] == "UnsupportedOperation"
            ),
            "permission_denial_typed": negative_rows["permission_denial"],
            "unsupported_empty_path_typed": negative_rows["unsupported_empty_path"],
        }
        if query_contract != provider.QUERY_CONTRACT:
            raise RuntimeError("query contract did not match the closed evidence schema")

        source_directory = pathlib.Path(__file__).resolve().parent
        return {
            "schema_version": provider.EVIDENCE_SCHEMA_VERSION,
            "core_commit": args.core_commit,
            "workflow_run": args.workflow_run,
            "architecture": args.architecture,
            "native_machine": native_machine,
            "kernel_release": platform.release(),
            "probe_source_sha256": provider.probe_source_digest(source_directory),
            "provider_table_sha256": provider.provider_table_digest(),
            "tuple": provider.validate_facts(before),
            "strong_table": strong_table,
            "remount": remount,
            "substitution": substitution,
            "query_contract": query_contract,
            "negative_rows": negative_rows,
            "diagnostics": {
                "mount_id_before": before["mount_id"],
                "mount_id_after": after["mount_id"],
            },
        }
    finally:
        for path in reversed(mounted):
            subprocess.run(("umount", str(path)), check=False)
        shutil.rmtree(temporary, ignore_errors=True)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--architecture", required=True, choices=sorted(provider.REQUIRED_ARCHITECTURES))
    parser.add_argument("--core-commit", required=True)
    parser.add_argument("--workflow-run", required=True)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    evidence = run(args)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(provider.canonical_json(evidence))
    print(json.dumps(evidence, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Fail-not-skip native ext4 probe for the R4b Linux identity provider."""

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
OVERLAYFS_SUPER_MAGIC = 0x794C7630
TMPFS_MAGIC = 0x01021994
FS_CASEFOLD_FL = 0x40000000
FIXED_UUID = uuid.UUID("718c918e-3cc3-43c9-ae9e-27f5cecc8a17")
MAX_HANDLE_BYTES = 128


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
    return {
        EXT4_SUPER_MAGIC: "ext4",
        OVERLAYFS_SUPER_MAGIC: "overlay",
        TMPFS_MAGIC: "tmpfs",
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
        raise OSError(error, os.strerror(error))
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


def create_fixture(root: pathlib.Path) -> None:
    (root / "sensitive").mkdir()
    (root / "casefold").mkdir()
    command("chattr", "+F", str(root / "casefold"))
    (root / "sensitive" / "stable").write_bytes(b"gwz-r4b-linux-probe\n")


def reject_filesystem(path: pathlib.Path, expected: str) -> str:
    fd = open_nofollow(path, directory=True)
    try:
        observed = filesystem_name(filesystem_magic(fd))
    finally:
        os.close(fd)
    if observed != expected:
        raise RuntimeError(f"expected {expected}, observed {observed}")
    facts = {
        "filesystem": observed,
        "filesystem_uuid": FIXED_UUID.hex,
        "filesystem_uuid_length": 16,
        "handle_provider": "name_to_handle_at-empty-path",
        "handle_type": 1,
        "handle": "aa",
        "handle_length": 1,
        "path_modes": {"sensitive": "Sensitive", "casefold": "AsciiCaseFold"},
        "mode_query_succeeded": True,
    }
    try:
        provider.validate_facts(facts)
    except provider.ProbeError as error:
        return error.code
    raise RuntimeError(f"{expected} unexpectedly passed the ext4 support table")


def synthetic_negative_rows(valid: dict[str, object]) -> dict[str, str]:
    rows = {
        "overlay": "pending-real",
        "tmpfs": "pending-real",
    }
    variants = {
        "network": {"filesystem": "nfs"},
        "zero_uuid": {"filesystem_uuid": "00" * 16},
        "no_uuid": {"filesystem_uuid": "", "filesystem_uuid_length": 0},
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
    return rows


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
    for executable in ("mkfs.ext4", "mount", "umount", "chattr"):
        if shutil.which(executable) is None:
            raise RuntimeError(f"required executable is unavailable: {executable}")

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

        tmpfs.mkdir()
        command("mount", "-t", "tmpfs", "tmpfs", str(tmpfs))
        mounted.append(tmpfs)

        for name in ("lower", "upper", "work", "merged"):
            (overlay / name).mkdir(parents=True, exist_ok=True)
        (overlay / "lower" / "sentinel").write_text("overlay\n", encoding="ascii")
        options = ",".join(
            f"{name}dir={overlay / name}" for name in ("lower", "upper", "work")
        )
        command("mount", "-t", "overlay", "overlay", "-o", options, str(overlay / "merged"))
        mounted.append(overlay / "merged")

        negative_rows = synthetic_negative_rows(before)
        negative_rows["tmpfs"] = reject_filesystem(tmpfs, "tmpfs")
        negative_rows["overlay"] = reject_filesystem(overlay / "merged", "overlay")
        provider.validate_negative_rows(negative_rows)

        source_directory = pathlib.Path(__file__).resolve().parent
        return {
            "schema_version": 1,
            "core_commit": args.core_commit,
            "workflow_run": args.workflow_run,
            "architecture": args.architecture,
            "kernel_release": platform.release(),
            "probe_source_sha256": provider.probe_source_digest(source_directory),
            "provider_table_sha256": provider.provider_table_digest(),
            "tuple": provider.validate_facts(before),
            "remount": remount,
            "substitution": substitution,
            "query_contract": {
                "missing_at_empty_path_errno": errno.errorcode[missing_empty_path_errno],
                "forbidden_flags_rejected_before_syscall": True,
                "pathname_fallback_rejected_before_syscall": True,
            },
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

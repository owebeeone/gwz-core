"""Archive provenance and extraction tests for the isolated consumer proof."""

import importlib.util
import io
import json
import tarfile
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parent
PROOF_PATH = ROOT / "package_proof.py"
SPEC = importlib.util.spec_from_file_location("package_proof", PROOF_PATH)
assert SPEC is not None and SPEC.loader is not None
PROOF = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROOF)

REVISION = "e8b9a1c5408cc9ea9528939b3a602acbeb697814"
ARCHIVE = ROOT.parents[2] / "gwz-transport" / "target" / "package" / "gwz-transport-0.1.0.crate"


def _archive(
    path: Path,
    *,
    dirty: bool = False,
    link: bool = False,
    unsafe_name: str | None = None,
) -> None:
    root = "gwz-transport-0.1.0"
    with tarfile.open(path, "w:gz") as bundle:
        files = {
            f"{root}/Cargo.toml": b'[package]\nname = "gwz-transport"\nversion = "0.1.0"\n',
            f"{root}/.cargo_vcs_info.json": json.dumps(
                {"git": {"sha1": REVISION}, "dirty": dirty}
            ).encode(),
            f"{root}/src/lib.rs": b"pub fn proof() {}\n",
        }
        for name, content in files.items():
            member = tarfile.TarInfo(name)
            member.size = len(content)
            bundle.addfile(member, io.BytesIO(content))
        if link:
            member = tarfile.TarInfo(f"{root}/src/link")
            member.type = tarfile.SYMTYPE
            member.linkname = "../Cargo.toml"
            bundle.addfile(member)
        if unsafe_name is not None:
            content = b"unsafe"
            member = tarfile.TarInfo(unsafe_name)
            member.size = len(content)
            bundle.addfile(member, io.BytesIO(content))


def test_actual_archive_has_pinned_identity_and_revision():
    if not ARCHIVE.is_file():
        pytest.skip("run package_proof.py for the explicit archive success proof")
    assert PROOF.package_identity(ARCHIVE, REVISION) == ("gwz-transport", "0.1.0")


def test_dirty_archive_is_rejected(tmp_path):
    archive = tmp_path / "dirty.crate"
    _archive(archive, dirty=True)
    with pytest.raises(SystemExit, match="dirty"):
        PROOF.package_identity(archive, REVISION)


def test_wrong_archive_revision_is_rejected(tmp_path):
    archive = tmp_path / "wrong-revision.crate"
    _archive(archive)
    with pytest.raises(SystemExit, match="source revision mismatch"):
        PROOF.package_identity(archive, "0" * 40)


def test_links_are_rejected_before_extraction(tmp_path):
    archive = tmp_path / "link.crate"
    _archive(archive, link=True)
    with pytest.raises(SystemExit, match="non-regular"):
        PROOF.package_identity(archive, REVISION)


@pytest.mark.parametrize(
    "unsafe_name",
    [
        "gwz-transport-0.1.0/src/../escape",
        "gwz-transport-0.1.0/src/./escape",
        "gwz-transport-0.1.0\\src\\escape",
        "../gwz-transport-0.1.0/escape",
        "C:/gwz-transport-0.1.0/escape",
    ],
)
def test_path_traversal_and_windows_names_are_rejected_before_extraction(
    tmp_path, unsafe_name
):
    archive = tmp_path / "unsafe.crate"
    _archive(archive, unsafe_name=unsafe_name)
    with pytest.raises(SystemExit, match="unsafe or duplicate|escapes"):
        PROOF.package_identity(archive, REVISION)

#!/usr/bin/env python3
"""Generate canonical retained-reader Git workspaces from pinned source inputs."""

from __future__ import annotations

import argparse
import hashlib
import os
import shutil
import subprocess
import tempfile
from pathlib import Path
from typing import Sequence


IDENTITY_NAME = "GWZ Retained Fixture"
IDENTITY_EMAIL = "retained-fixture@example.test"
GIT_DATE = "2000-01-01T00:00:00+0000"
SIGNATURE_SECONDS = 946684800
MERGE_ID = "merge_retained"
CUSTOM_MESSAGE = "custom retained-reader message"
ROOT_EXCLUDE = (
    ".gwz/\n# BEGIN GWZ managed member repositories\n/.gwz/\n"
    "/gwz.conf/.tmp/\n/member/\n# END GWZ managed member repositories\n"
)


class GenerationError(RuntimeError):
    """A canonical retained-reader fixture could not be generated."""


def _environment() -> dict[str, str]:
    environment = {
        "PATH": os.environ.get("PATH", os.defpath),
        **{
            key: os.environ[key]
            for key in ("SystemRoot", "WINDIR", "COMSPEC", "PATHEXT", "TEMP", "TMP")
            if key in os.environ
        },
        "GIT_AUTHOR_NAME": IDENTITY_NAME,
        "GIT_AUTHOR_EMAIL": IDENTITY_EMAIL,
        "GIT_AUTHOR_DATE": GIT_DATE,
        "GIT_COMMITTER_NAME": IDENTITY_NAME,
        "GIT_COMMITTER_EMAIL": IDENTITY_EMAIL,
        "GIT_COMMITTER_DATE": GIT_DATE,
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": os.devnull,
        "LC_ALL": "C",
        "LANG": "C",
        "TZ": "UTC",
    }
    return environment


def _git(repository: Path, *args: str) -> str:
    try:
        completed = subprocess.run(
            ["git", *args],
            cwd=repository,
            env=_environment(),
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
    except OSError as error:
        raise GenerationError(f"cannot execute Git: {error}") from error
    if completed.returncode:
        raise GenerationError(f"git {' '.join(args)} failed: {completed.stderr.strip()}")
    return completed.stdout.strip()


def _git_input(repository: Path, input_text: str, *args: str) -> str:
    completed = subprocess.run(
        ["git", *args], cwd=repository, env=_environment(), input=input_text,
        text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False,
    )
    if completed.returncode:
        raise GenerationError(f"git {' '.join(args)} failed: {completed.stderr.strip()}")
    return completed.stdout.strip()


def _configure(repository: Path) -> None:
    for key, value in [
        ("user.name", IDENTITY_NAME),
        ("user.email", IDENTITY_EMAIL),
        ("commit.gpgsign", "false"),
        ("core.autocrlf", "false"),
        ("core.filemode", "false"),
        ("core.ignorecase", "false"),
        ("core.precomposeunicode", "false"),
        ("core.symlinks", "false"),
    ]:
        _git(repository, "config", key, value)


def _init(repository: Path) -> None:
    repository.mkdir(parents=True)
    _git(repository, "init", "--quiet", "--object-format=sha1", "--initial-branch=main")
    _configure(repository)


def _write(path: Path, contents: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(contents.encode("utf-8"))


def _commit(repository: Path, message: str, *paths: str) -> str:
    _git(repository, "add", "--", *paths)
    _git(repository, "commit", "--quiet", "-m", message)
    return _git(repository, "rev-parse", "HEAD")


def _canonicalize_git_dir(repository: Path, exclude: str = "") -> None:
    git_dir = repository / ".git"
    shutil.rmtree(git_dir / "hooks", ignore_errors=True)
    _write(git_dir / "info/exclude", exclude)
    _write(
        git_dir / "config",
        "[core]\n"
        "\trepositoryformatversion = 0\n"
        "\tfilemode = false\n"
        "\tbare = false\n"
        "\tlogallrefupdates = true\n"
        "\tautocrlf = false\n"
        "\tignorecase = false\n"
        "\tprecomposeunicode = false\n"
        "\tsymlinks = false\n"
        "[commit]\n\tgpgsign = false\n"
        f"[user]\n\tname = {IDENTITY_NAME}\n\temail = {IDENTITY_EMAIL}\n",
    )
    entries = _git(repository, "ls-tree", "-r", "HEAD")
    payload = "\n".join(line.replace(" blob ", " ", 1) for line in entries.splitlines()) + "\n"
    (git_dir / "index").unlink(missing_ok=True)
    completed = subprocess.run(
        ["git", "update-index", "--index-info"],
        cwd=repository,
        env=_environment(),
        input=payload,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode:
        raise GenerationError(f"cannot canonicalize Git index: {completed.stderr.strip()}")


def _member(repository: Path, true_merge: bool) -> tuple[str, str, str | None]:
    _init(repository)
    _write(repository / "base.txt", "base\n")
    base = _commit(repository, "base", "base.txt")
    _git(repository, "branch", "feature/source")
    _git(repository, "switch", "--quiet", "feature/source")
    _write(repository / "feature.txt", "feature\n")
    source = _commit(repository, "feature", "feature.txt")
    _git(repository, "switch", "--quiet", "main")
    if true_merge:
        _write(repository / "main.txt", "main\n")
        before = _commit(repository, "main", "main.txt")
        tree = _git(
            repository,
            "merge-tree",
            "--write-tree",
            "--no-messages",
            before,
            source,
        ).splitlines()[0]
    else:
        before = base
        tree = None
    _canonicalize_git_dir(repository)
    return before, source, tree


def _workspace_artifacts(before: str) -> tuple[str, str]:
    manifest = (
        "schema: gwz.workspace/v0\n"
        "workspace:\n  id: ws_retained\n"
        "members:\n"
        "- id: mem_member\n  path: member\n  type: git\n"
        "  source_id: src_member\n  active: true\n"
        "  desired:\n    branch: main\n  remotes: []\n"
    )
    lock = (
        "schema: gwz.lock/v0\nworkspace_id: ws_retained\n"
        "manifest_schema: gwz.workspace/v0\nmembers:\n"
        "  mem_member:\n    path: member\n    source_id: src_member\n"
        "    source_kind: git\n"
        f"    commit: {before}\n"
        "    branch: main\n    detached: false\n    dirty: false\n    materialized: true\n"
    )
    return manifest, lock


def _sha256(contents: str) -> str:
    return hashlib.sha256(contents.encode("utf-8")).hexdigest()


def _literal(name: str, contents: str) -> str:
    return f"  {name}: |\n" + "".join(f"    {line}\n" for line in contents.splitlines())


def _record(
    manifest: str,
    lock: str,
    root_head: str,
    before: str,
    source: str,
    *,
    mode: str | None,
    pending_tree: str | None,
) -> str:
    message = CUSTOM_MESSAGE if pending_tree else "frozen no-ff message"
    baseline = (
        "baseline:\n"
        f"  lock_sha256: {_sha256(lock)}\n"
        f"  manifest_sha256: {_sha256(manifest)}\n"
        + _literal("lock_yaml", lock)
        + _literal("manifest_yaml", manifest)
        + f"  lock_commit_sha256: {_sha256(lock)}\n"
        + f"  manifest_commit_sha256: {_sha256(manifest)}\n"
        + f"  root_head: {root_head}\n  root_branch: main\n"
    )
    pending = ""
    if pending_tree:
        signature = (
            f"name: {IDENTITY_NAME}, email: {IDENTITY_EMAIL}, "
            f"time_seconds: {SIGNATURE_SECONDS}, timezone_offset_minutes: 0"
        )
        pending = (
            "    pending_action:\n      kind: true_merge\n      target_branch: main\n"
            f"      before_commit: {before}\n      source_commit: {source}\n"
            f"      commit_message: {message}\n      expected_result: commit\n"
            f"      commit_spec:\n        tree_oid: {pending_tree}\n"
            f"        author: {{{signature}}}\n        committer: {{{signature}}}\n"
        )
    mode_line = f"mode: {mode}\n" if mode else ""
    return (
        "schema: gwz.merge-operation/v0\nrecord_schema_version: 0\n"
        "writer_version: 0.10.2\nworkspace_id: ws_retained\n"
        f"merge_id: {MERGE_ID}\noperation_id: op_retained\nstate: halted\n"
        f"source_ref: feature/source\n{mode_line}created_at: '946684800000'\n"
        f"{baseline}selected_targets:\n- mem_member\nparticipants:\n"
        "  mem_member:\n    path: member\n    target_kind: member\n"
        "    target_branch: main\n"
        f"    before_commit: {before}\n    source_commit: {source}\n"
        f"    commit_message: {message}\n    state: planned\n{pending}"
        "retained_fixture_generation: canonical-v1\n"
    )


def _create_fixture(root: Path, *, true_merge: bool, mode: str | None) -> None:
    _init(root)
    before, source, tree = _member(root / "member", true_merge)
    manifest, lock = _workspace_artifacts(before)
    _write(root / ".gitignore", "member/\n")
    _write(root / "gwz.conf/gwz.yml", manifest)
    _write(root / "gwz.conf/gwz.lock.yml", lock)
    root_head = _commit(root, "workspace baseline", ".gitignore", "gwz.conf")
    _write(
        root / f".gwz/merge/{MERGE_ID}.yaml",
        _record(manifest, lock, root_head, before, source, mode=mode, pending_tree=tree),
    )
    _write(root / ".gwz/locks/workspace-mutator.lock", "")
    _canonicalize_git_dir(root, ROOT_EXCLUDE)


def _adopt_pending_commit(root: Path, message: str) -> str:
    member = root / "member"
    before = _git(member, "rev-parse", "main")
    source = _git(member, "rev-parse", "feature/source")
    tree = _git(member, "merge-tree", "--write-tree", "--no-messages", before, source).splitlines()[0]
    payload = (
        f"tree {tree}\nparent {before}\nparent {source}\n"
        f"author {IDENTITY_NAME} <{IDENTITY_EMAIL}> {SIGNATURE_SECONDS} +0000\n"
        f"committer {IDENTITY_NAME} <{IDENTITY_EMAIL}> {SIGNATURE_SECONDS} +0000\n\n{message}"
    )
    commit = _git_input(member, payload, "hash-object", "-t", "commit", "-w", "--stdin")
    _git(member, "update-ref", "refs/heads/main", commit)
    _git(member, "reset", "--hard", "--quiet", commit)
    _canonicalize_git_dir(member)
    return commit


def _archive_fixture(root: Path) -> None:
    source = root / f".gwz/merge/{MERGE_ID}.yaml"
    target = root / f".gwz/merge/done/{MERGE_ID}.yaml"
    text = source.read_text(encoding="utf-8")
    text = text.replace("state: halted\n", "state: aborted\n", 1)
    text = text.replace("    state: planned\n", "    state: aborted\n", 1)
    lines = text.splitlines(keepends=True)
    pending = next(
        (position for position, line in enumerate(lines) if line.startswith("    pending_action:")),
        None,
    )
    if pending is not None:
        end = pending + 1
        while end < len(lines) and (not lines[end].strip() or lines[end].startswith("    ")):
            end += 1
        del lines[pending:end]
        text = "".join(lines)
    _write(target, text)
    source.unlink()


def generate(destination: Path) -> None:
    """Atomically create ``destination`` with all canonical workspaces."""

    destination = destination.resolve()
    if destination.exists():
        raise GenerationError(f"fixture destination already exists: {destination}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = Path(tempfile.mkdtemp(prefix=f".{destination.name}-", dir=destination.parent))
    try:
        _create_fixture(temporary / "custom-message-pending", true_merge=True, mode=None)
        shutil.copytree(temporary / "custom-message-pending", temporary / "custom-message-pending-completed")
        _adopt_pending_commit(temporary / "custom-message-pending-completed", CUSTOM_MESSAGE)
        shutil.copytree(temporary / "custom-message-pending", temporary / "custom-message-pending-wrong-message")
        _adopt_pending_commit(temporary / "custom-message-pending-wrong-message", "wrong retained-reader message")
        _create_fixture(temporary / "no-ff-fast-forwardable", true_merge=False, mode="no_ff")
        shutil.copytree(
            temporary / "no-ff-fast-forwardable",
            temporary / "pre-record-open-v0",
        )
        shutil.copytree(temporary / "custom-message-pending", temporary / "archived-v0")
        _archive_fixture(temporary / "archived-v0")
        os.replace(temporary, destination)
        temporary = Path()
    finally:
        if temporary != Path() and temporary.exists():
            shutil.rmtree(temporary)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args(argv)
    try:
        generate(args.destination)
    except GenerationError as error:
        parser.error(str(error))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

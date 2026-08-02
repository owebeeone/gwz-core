"""Bounded, non-interactive retained-reader process execution."""

from __future__ import annotations

import os
import re
import signal
import subprocess
from pathlib import Path
from typing import Mapping, Sequence

from retained_reader_errors import HarnessError


TEMPLATE_RE = re.compile(r"\{([a-z][a-z0-9_]*)\}")


def regular_tree_inventory(root: Path) -> tuple[set[str], set[str]]:
    """List real files and directories below a directory, rejecting indirection."""

    if root.is_symlink() or not root.is_dir():
        raise ValueError(f"not a real directory: {root}")
    files: set[str] = set()
    directories: set[str] = set()
    for path in root.rglob("*"):
        relative = path.relative_to(root).as_posix()
        if path.is_symlink() or not (path.is_file() or path.is_dir()):
            raise ValueError(f"non-regular filesystem entry: {relative}")
        if path.is_file():
            files.add(relative)
        else:
            directories.add(relative)
    return files, directories


def is_regular_file(path: Path) -> bool:
    return not path.is_symlink() and path.is_file()


def read_regular_text(path: Path, *, encoding: str = "utf-8", errors: str = "strict") -> str:
    if not is_regular_file(path):
        raise ValueError(f"not a regular file: {path}")
    return path.read_text(encoding=encoding, errors=errors)


def render_command(command: Sequence[str], variables: Mapping[str, str]) -> list[str]:
    rendered: list[str] = []
    for argument in command:
        unknown = [name for name in TEMPLATE_RE.findall(argument) if name not in variables]
        if unknown:
            raise HarnessError(f"unknown template variable(s): {', '.join(sorted(set(unknown)))}")
        rendered.append(TEMPLATE_RE.sub(lambda match: variables[match.group(1)], argument))
    return rendered


def run_command(
    command: Sequence[str], *, timeout_seconds: float,
    cwd: Path | None = None, env: Mapping[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    if timeout_seconds <= 0:
        raise HarnessError("timeout_seconds must be positive")
    invocation_env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    invocation_env.update({"CI": "1", "GIT_DEFAULT_HASH": "sha1", "GIT_TERMINAL_PROMPT": "0", "GWZ_RETAINED_READER": "1"})
    if env is not None:
        invocation_env.update(env)
    process: subprocess.Popen[str] | None = None
    try:
        creationflags = getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0) if os.name == "nt" else 0
        process = subprocess.Popen(
            list(command), cwd=cwd, env=invocation_env, stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding="utf-8",
            errors="surrogateescape", start_new_session=os.name != "nt",
            creationflags=creationflags,
        )
        stdout, stderr = process.communicate(timeout=timeout_seconds)
        return subprocess.CompletedProcess(list(command), process.returncode, stdout, stderr)
    except subprocess.TimeoutExpired as error:
        if process is not None:
            if os.name == "nt":
                subprocess.run(
                    ["taskkill", "/PID", str(process.pid), "/T", "/F"],
                    stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL, check=False,
                )
                process.kill()
            else:
                os.killpg(process.pid, signal.SIGKILL)
            process.communicate()
        raise HarnessError(f"retained reader timed out after {timeout_seconds} seconds") from error
    except OSError as error:
        raise HarnessError(f"retained reader could not execute: {error}") from error

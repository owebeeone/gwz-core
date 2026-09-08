#!/usr/bin/env python3
"""Run migrated tests with fake Git and the remaining suite with native Git.

Use --compare for the migrated tests against both backends. Other arguments
are passed to Cargo (for example --lib or --no-fail-fast).
"""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import subprocess
import time
import sys

ROOT = Path(__file__).resolve().parents[1]
FILESYSTEM_CONTRACTS = (
    "filesystem::filesystem_contract_tests::",
    "store::rewrite::filesystem_tests::",
)
CONTRACTS = (
    "git::gitbackend::repository_contract_tests::",
    "reverse::preservation::tests::factory_contract::",
)
MATRICES = (
    "reverse::preservation::tests::root_fault_matrix::",
    "reverse::preservation::tests::root_ambiguity_matrix::",
)
MIGRATED_GROUPS = (
    "reverse::preservation::tests::root_successor_matrix::",
    "reverse::preservation::tests::root_durability::",
    "reverse::rollback::tests::",
)
NATIVE_CROSSCHECKS = (
    "reverse::rollback::tests::real_git::",
)


def run(mode: str, cargo_args: list[str], test_args: list[str], *, filesystem: str = "real") -> int:
    command = ["cargo", "test", "--locked", *cargo_args, "--", *test_args]
    env = {**os.environ, "GWZ_TEST_GIT": mode, "GWZ_TEST_FS": filesystem}
    print(f"Git backend: {mode}; filesystem: {filesystem}; {' '.join(command)}", flush=True)
    start = time.monotonic()
    result = subprocess.run(command, cwd=ROOT, env=env, check=False)
    print(f"Git backend {mode}: {time.monotonic() - start:.2f}s", flush=True)
    return result.returncode


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compare", action="store_true")
    options, cargo_args = parser.parse_known_args()
    library_args = ["--lib", *[arg for arg in cargo_args if arg != "--lib"]]
    subprocess.run([sys.executable, str(ROOT / "scripts/checks/check_filesystem_boundary.py")], check=True)
    filesystem_result = run("fake", library_args, list(FILESYSTEM_CONTRACTS), filesystem="fake")
    if filesystem_result and "--no-fail-fast" not in cargo_args:
        raise SystemExit(filesystem_result)
    migrated = CONTRACTS + MATRICES + MIGRATED_GROUPS
    fake_result = run("fake", library_args, list(migrated), filesystem="fake")
    if fake_result and "--no-fail-fast" not in cargo_args:
        raise SystemExit(fake_result)
    if options.compare:
        real_result = run("real", library_args, list(migrated + FILESYSTEM_CONTRACTS))
    else:
        native_crosscheck_result = run("real", library_args, list(NATIVE_CROSSCHECKS))
        if native_crosscheck_result and "--no-fail-fast" not in cargo_args:
            raise SystemExit(native_crosscheck_result)
        skipped = MATRICES + MIGRATED_GROUPS
        real_result = run("real", cargo_args, [arg for name in skipped for arg in ("--skip", name)])
        real_result = native_crosscheck_result or real_result
    raise SystemExit(filesystem_result or fake_result or real_result)


if __name__ == "__main__":
    main()

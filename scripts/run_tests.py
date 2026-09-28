#!/usr/bin/env python3
"""Run migrated tests with fake Git and the remaining suite with native Git.

Use --compare for the migrated tests against both backends. Other arguments
are passed to Cargo (for example --lib or --no-fail-fast).

The source checks run first. gwz-transport's process-global check runs over
the checkout that GWZ_TRANSPORT_CHECKOUT names, or else the one beside
gwz-core; with neither, the run fails. --skip-transport-globals skips that
check and prints SKIPPED GATE instead; only a CI job that has no gwz-transport
checkout passes it (GwzCoreSessionDesign §5.7).

The conditional-compilation boundary check covers gwz-core and the gwz-cli and
gwz-py checkouts beside it; with either missing, the run fails.
--skip-cfg-siblings checks gwz-core alone and prints SKIPPED GATE for the two
siblings; only a CI job that has neither checkout passes it (CS1.7).
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
    "catalog_batch_uses_its_world_through_reopen_and_contention",
    "service_reopens_and_commits_in_the_supplied_world",
    "operation_context::",
    "checked::context_tests::",
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


TRANSPORT_CHECKOUT = "GWZ_TRANSPORT_CHECKOUT"
SKIP_TRANSPORT_GLOBALS = "--skip-transport-globals"


def transport_checkout() -> Path:
    """The gwz-transport checkout that gwz-core builds against.

    GWZ_TRANSPORT_CHECKOUT names it, or else it is the checkout beside
    gwz-core, which tests/transport_backend/prepare.py links into the
    transport build. A named checkout that is missing is an error, never a
    reason to look elsewhere, and so is finding neither.
    """
    sibling = ROOT.parent / "gwz-transport"
    ways = (
        "gwz-transport's process-global check needs a gwz-transport checkout: "
        f"set {TRANSPORT_CHECKOUT}=<path>, or check gwz-transport out beside gwz-core at {sibling}"
    )
    named = os.environ.get(TRANSPORT_CHECKOUT)
    if named:
        checkout = Path(named)
        if not checkout.is_dir():
            raise SystemExit(f"{TRANSPORT_CHECKOUT} names {named}, which is not a directory. {ways}")
        return checkout
    if sibling.is_dir():
        return sibling
    raise SystemExit(f"no gwz-transport checkout found. {ways}")


def check_transport_process_globals(skip: bool) -> None:
    # gwz-core, the consumer, checks the gwz-transport it builds against;
    # gwz-transport's own CI carries no such gate (GwzCoreSessionDesign §5.7).
    if skip:
        print(
            f"SKIPPED GATE: gwz-transport process-global check ({SKIP_TRANSPORT_GLOBALS}): this run "
            "has no gwz-transport checkout; gwz-core's boundary CI job runs the check at the "
            "gwz-transport commit its allowlist records as reconciled_commit",
            flush=True,
        )
        return
    transport = transport_checkout()
    print(f"gwz-transport process-global check: {transport}", flush=True)
    subprocess.run(
        [
            sys.executable,
            str(ROOT / "scripts/checks/check_process_globals.py"),
            "--repo",
            str(transport),
            "--allowlist",
            str(ROOT / "scripts/checks/process_globals_allowlist_gwz_transport.json"),
        ],
        check=True,
    )


SKIP_CFG_SIBLINGS = "--skip-cfg-siblings"
CFG_SIBLINGS = ("gwz-cli", "gwz-py")


def check_cfg_boundaries(skip_siblings: bool) -> None:
    # The checker fails when a sibling named in its allowlist is not checked
    # out; skipping one makes it print SKIPPED GATE instead.
    skips = [arg for name in CFG_SIBLINGS for arg in ("--skip-repo", name)] if skip_siblings else []
    subprocess.run([sys.executable, str(ROOT / "scripts/checks/check_cfg_boundaries.py"), *skips], check=True)


def main(argv: list[str] | None = None) -> None:
    # No abbreviations: a prefix such as --skip must never turn a gate off.
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--compare", action="store_true")
    parser.add_argument(
        SKIP_TRANSPORT_GLOBALS,
        action="store_true",
        help="skip gwz-transport's process-global check and print SKIPPED GATE; "
        "only for a CI job that has no gwz-transport checkout",
    )
    parser.add_argument(
        SKIP_CFG_SIBLINGS,
        action="store_true",
        help="check conditional-compilation boundaries in gwz-core alone and print SKIPPED GATE "
        "for gwz-cli and gwz-py; only for a CI job that has neither checkout",
    )
    options, cargo_args = parser.parse_known_args(argv)
    library_args = ["--lib", *[arg for arg in cargo_args if arg != "--lib"]]
    subprocess.run([sys.executable, str(ROOT / "scripts/checks/check_filesystem_boundary.py")], check=True)
    subprocess.run([sys.executable, str(ROOT / "scripts/checks/check_process_globals.py")], check=True)
    check_cfg_boundaries(options.skip_cfg_siblings)
    check_transport_process_globals(options.skip_transport_globals)
    subprocess.run([sys.executable, str(ROOT / "scripts/checks/check_crate_versions.py")], check=True)
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

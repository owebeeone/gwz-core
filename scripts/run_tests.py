#!/usr/bin/env python3
"""Run migrated tests with fake Git and the remaining suite with native Git.

Use --compare for the migrated tests against both backends. Other arguments
are passed to Cargo (for example --lib or --no-fail-fast).

The source checks run first. gwz-transport's process-global check runs over
the checkout that GWZ_TRANSPORT_CHECKOUT names, or else the one beside
gwz-core; with neither, the run fails. --skip-transport-globals skips that
check and prints SKIPPED GATE instead; only a CI job that has no gwz-transport
checkout passes it (GwzCoreSessionDesign §5.7).

The gwz-transport checkout must be at the commit CI builds against, the first
40-hex line of .github/gwz-transport.commit: a local run against a newer
sibling can pass what CI cannot compile. --allow-transport-pin-mismatch lets a
lane that develops gwz-core and gwz-transport at once run anyway; it prints
both commits, and the pin must be moved before gwz-core is pushed.

gwz-git, which gwz-core builds against by path (step 5.2 moved native commit and
tag there), is checked the same way: its process-global check runs over the
checkout that GWZ_GIT_CHECKOUT names, or else the one beside gwz-core, with the
allowlist gwz-git carries (scripts/process_globals_allowlist.json), and the
checkout must be at the first 40-hex line of .github/gwz-git.commit.
--allow-git-pin-mismatch lets a lane that develops gwz-core and gwz-git at once
run anyway, as the transport flag does. Every job that builds gwz-core has gwz-git
beside it, so there is no skip flag.

The conditional-compilation boundary check covers gwz-core and the gwz-cli and
gwz-py checkouts beside it; with either missing, the run fails.
--skip-cfg-siblings checks gwz-core alone and prints SKIPPED GATE for the two
siblings; only a CI job that has neither checkout passes it (CS1.7).
--skip-cfg-sibling NAME skips one sibling the same way, for a CI job that has
the other checked out.

The candidate switch inventory check covers gwz-core alone: gwz-cli and gwz-py
run it over themselves in their own tests (TR2.12).

--test-filter NAME runs, in each group, only that group's tests whose path
contains NAME. Do not pass a bare test name instead: Cargo hands it to libtest
as one more filter, libtest ORs filters, and every group (the fake-filesystem
ones included) would also run every matching test of the other groups.

--skip-exact NAME (repeatable) leaves one test out of the libtest runs: NAME is the test's full path, as `cargo test
-- --list` prints it, and each skipped name is printed with the reason "run in the privileged step". For a CI job
whose unprivileged step cannot run a row that needs root and runs it in a step of its own (the sandbox rule's
mount-namespace and pid-reuse rows, plan 2.3a). It is libtest's `--skip`, a substring match, so only a full path
(which names one test) is accepted. The rows keep failing loudly for anyone who runs them without root and without
this option.

The source checks are read-only scans, so they are started side by side and
judged in the order main() names them: the first that fails, in that order, is
the failure the run reports, after the output of every check before it, and no
Cargo command runs (scripts/checks/test_run_tests_parallel_checks.py). A notice
the runner prints itself between checks (SKIPPED GATE, PIN OVERRIDE) can come
before the output of a check started ahead of it. The gwz-git adoption check and
the crate-version check run after the others have passed, one at a time.
"""
from __future__ import annotations

import argparse
import concurrent.futures
import os
from pathlib import Path
import re
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
# The core session's crates (every gwz-session-* package under crates/; the unit test in
# scripts/checks/test_run_tests_session_crates.py keeps the list equal to them). Their own tests and doctests run,
# and clippy with warnings denied, as the hosted checked-artifact-boundary job runs them for every library, so the
# lane gate matrix covers them too (step 2.2b review, State S-P3-5). They run after the core groups whether those
# passed or not, and their status joins the run's. A --test-filter run leaves them out: the filter narrows the
# core groups.
SESSION_CRATES = ("gwz-session-channel", "gwz-session-contract", "gwz-session-host")


# The characters of exact test names one filtered command carries; a Windows command line holds 32,767.
EXACT_COMMAND_BUDGET = 24_000


class FilterTally:
    """How many tests a `--test-filter` run will execute across its groups (a run that executes none fails), and
    how many groups could not be listed."""

    def __init__(self) -> None:
        self.matched = 0
        self.listing_failures = 0


def run(
    mode: str,
    cargo_args: list[str],
    test_args: list[str],
    *,
    filesystem: str = "real",
    test_filter: str | None = None,
    tally: FilterTally | None = None,
    skip_exact: tuple[str, ...] = (),
) -> int:
    env = {**os.environ, "GWZ_TEST_GIT": mode, "GWZ_TEST_FS": filesystem}
    if test_filter is None:
        skips = [arg for name in skip_exact for arg in ("--skip", name)]
        return run_command(mode, filesystem, ["cargo", "test", "--locked", *cargo_args, "--", *test_args, *skips], env)
    # libtest has no AND of filters: list the group, intersect here, and run the exact names.
    # `--list` names #[ignore] tests too, and an exact run only reports them ignored: list those separately and
    # leave them out, so a filter that matches only ignored tests counts as matching nothing.
    listings = []
    for extra in ([], ["--ignored"]):
        listing = ["cargo", "test", "--locked", *cargo_args, "--", *test_args, *extra, "--list", "--format", "terse"]
        listed = subprocess.run(listing, cwd=ROOT, env=env, check=False, stdout=subprocess.PIPE, text=True)
        if listed.returncode:
            print(f"Git backend {mode}: listing the group failed ({listed.returncode})", flush=True)
            if tally is not None:
                tally.listing_failures += 1
            return listed.returncode
        listings.append([line[: -len(": test")] for line in listed.stdout.splitlines() if line.endswith(": test")])
    every, ignored = listings[0], set(listings[1])
    names = [name for name in every if test_filter in name and name not in ignored and name not in skip_exact]
    if not names:
        print(f"Git backend: {mode}; filesystem: {filesystem}; no test of this group matches {test_filter!r}", flush=True)
        return 0
    if tally is not None:
        tally.matched += len(names)
    result = 0
    for batch in exact_batches(names):
        code = run_command(mode, filesystem, ["cargo", "test", "--locked", *cargo_args, "--", "--exact", *batch], env)
        result = result or code
    return result


def skip_exact_name(value: str) -> str:
    """A test's full path. libtest's `--skip` matches substrings, so a bare word would skip every test containing it."""
    if "::" not in value or re.search(r"\s", value):
        raise argparse.ArgumentTypeError(f"{value!r} is not a full test path such as 'module::tests::name'")
    return value


def exact_batches(names: list[str]) -> list[list[str]]:
    batches: list[list[str]] = [[]]
    size = 0
    for name in names:
        if batches[-1] and size + len(name) + 1 > EXACT_COMMAND_BUDGET:
            batches.append([])
            size = 0
        batches[-1].append(name)
        size += len(name) + 1
    return batches


def run_command(mode: str, filesystem: str, command: list[str], env: dict[str, str]) -> int:
    print(f"Git backend: {mode}; filesystem: {filesystem}; {' '.join(command)}", flush=True)
    start = time.monotonic()
    result = subprocess.run(command, cwd=ROOT, env=env, check=False)
    print(f"Git backend {mode}: {time.monotonic() - start:.2f}s", flush=True)
    return result.returncode


def run_check(command: list[str], header: str | None = None) -> None:
    """Runs one source check to its end; a check that fails raises CalledProcessError."""
    if header is not None:
        print(header, flush=True)
    subprocess.run(command, check=True)


class SourceChecks:
    """The source checks of one run, started side by side and judged in the order they were started.

    `start` has run_check's signature. `finish` prints each check's header and output in order and raises
    CalledProcessError for the first check, in that order, that failed, as run_check would have raised it.
    """

    def __init__(self) -> None:
        self.pool = concurrent.futures.ThreadPoolExecutor()
        self.started: list[tuple[list[str], str | None, concurrent.futures.Future]] = []

    def start(self, command: list[str], header: str | None = None) -> None:
        captured = self.pool.submit(subprocess.run, command, check=False, capture_output=True, text=True)
        self.started.append((command, header, captured))

    def finish(self) -> None:
        try:
            for command, header, captured in self.started:
                result = captured.result()
                if header is not None:
                    print(header, flush=True)
                if isinstance(result.stdout, str):
                    print(result.stdout, end="", flush=True)
                if isinstance(result.stderr, str):
                    print(result.stderr, end="", file=sys.stderr, flush=True)
                if result.returncode:
                    raise subprocess.CalledProcessError(result.returncode, command)
        finally:
            self.pool.shutdown(wait=True)


def run_source_checks(steps) -> None:
    """Runs `steps(start)`, which starts the source checks in order through `start`, then judges them.

    A step that fails between checks (a checkout off its pin, or missing) ends the run only once every check
    started before it has passed: one of those that failed is the failure the run reports, as it was when each
    check ran to its end before the next step.
    """
    checks = SourceChecks()
    try:
        steps(checks.start)
    except BaseException:
        try:
            checks.finish()
        except subprocess.CalledProcessError as failed:
            raise failed from None
        raise
    checks.finish()


TRANSPORT_CHECKOUT = "GWZ_TRANSPORT_CHECKOUT"
SKIP_TRANSPORT_GLOBALS = "--skip-transport-globals"


def transport_checkout() -> Path:
    """The gwz-transport checkout that gwz-core builds against.

    GWZ_TRANSPORT_CHECKOUT names it, or else it is the checkout beside
    gwz-core, which gwz-core's Cargo.toml names by path behind the
    transport switch. A named checkout that is missing is an error, never a
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


def check_transport_process_globals(skip: bool, start=run_check) -> None:
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
    start(
        [
            sys.executable,
            str(ROOT / "scripts/checks/check_process_globals.py"),
            "--repo",
            str(transport),
            "--allowlist",
            str(ROOT / "scripts/checks/process_globals_allowlist_gwz_transport.json"),
        ],
        f"gwz-transport process-global check: {transport}",
    )


TRANSPORT_PIN_FILE = ".github/gwz-transport.commit"
ALLOW_PIN_MISMATCH = "--allow-transport-pin-mismatch"


def pinned_transport_commit() -> str:
    """The commit CI builds against: the first 40-hex line of the pin file."""
    path = ROOT / TRANSPORT_PIN_FILE
    if not path.is_file():
        raise SystemExit(f"{path} is missing: it pins the gwz-transport commit CI builds against")
    for line in path.read_text().splitlines():
        if re.fullmatch(r"[0-9a-f]{40}", line.strip()):
            return line.strip()
    raise SystemExit(f"{path} names no gwz-transport commit (a 40-hex line)")


def transport_head(checkout: Path) -> str:
    result = subprocess.run(
        ["git", "-C", str(checkout), "rev-parse", "HEAD"], capture_output=True, text=True, check=False
    )
    if result.returncode != 0:
        raise SystemExit(f"cannot read {checkout}'s HEAD: {result.stderr.strip()}")
    return result.stdout.strip()


def check_transport_pin(skip: bool, allow_mismatch: bool) -> None:
    """Fail unless the gwz-transport checkouts are at CI's pin (P2-1 of the idle-loss State review).

    Checks the checkout transport_checkout() names and the one beside gwz-core,
    which the candidate build links by path.
    """
    if skip:
        print(
            f"SKIPPED GATE: gwz-transport pin check ({SKIP_TRANSPORT_GLOBALS}): this run has no "
            "gwz-transport checkout",
            flush=True,
        )
        return
    pinned = pinned_transport_commit()
    checkouts = [transport_checkout()]
    sibling = ROOT.parent / "gwz-transport"
    if sibling.is_dir() and sibling not in checkouts:
        checkouts.append(sibling)
    for checkout in checkouts:
        head = transport_head(checkout)
        if head == pinned:
            continue
        if allow_mismatch:
            print(
                f"PIN OVERRIDE ({ALLOW_PIN_MISMATCH}): {checkout} is at {head}, not the pin {pinned} "
                f"({TRANSPORT_PIN_FILE}); move the pin before gwz-core is pushed",
                flush=True,
            )
            continue
        raise SystemExit(
            f"{checkout} is at {head}, but {TRANSPORT_PIN_FILE} pins {pinned}, the commit CI builds against. "
            f"Check the pin out, or move the pin in the commit that needs the newer gwz-transport; a lane "
            f"developing both at once passes {ALLOW_PIN_MISMATCH}"
        )


SKIP_CFG_SIBLINGS = "--skip-cfg-siblings"
SKIP_CFG_SIBLING = "--skip-cfg-sibling"
CFG_SIBLINGS = ("gwz-cli", "gwz-py")


def run_session_crates() -> int:
    """Each session crate's tests, doctests included, and its clippy with warnings denied."""
    result = 0
    for crate in SESSION_CRATES:
        for command in (
            ["cargo", "test", "--locked", "-p", crate],
            ["cargo", "clippy", "--locked", "-p", crate, "--all-targets", "--", "-D", "warnings"],
        ):
            print(f"session crate {crate}: {' '.join(command)}", flush=True)
            code = subprocess.run(command, cwd=ROOT, check=False).returncode
            result = result or code
    return result


def check_cfg_boundaries(skipped: list[str], start=run_check) -> None:
    # The checker fails when a sibling named in its allowlist is not checked
    # out; skipping one makes it print SKIPPED GATE instead.
    skips = [arg for name in skipped for arg in ("--skip-repo", name)]
    start([sys.executable, str(ROOT / "scripts/checks/check_cfg_boundaries.py"), *skips])


GIT_CHECKOUT = "GWZ_GIT_CHECKOUT"
GIT_PIN_FILE = ".github/gwz-git.commit"
ALLOW_GIT_PIN_MISMATCH = "--allow-git-pin-mismatch"
GIT_ALLOWLIST = "scripts/process_globals_allowlist.json"


def git_checkout() -> Path:
    """The gwz-git checkout that gwz-core builds against: GWZ_GIT_CHECKOUT, or else the one beside gwz-core.

    A named checkout that is missing is an error, never a reason to look elsewhere, and so is finding neither.
    """
    sibling = ROOT.parent / "gwz-git"
    ways = (
        "gwz-git's process-global check needs a gwz-git checkout: "
        f"set {GIT_CHECKOUT}=<path>, or check gwz-git out beside gwz-core at {sibling}"
    )
    named = os.environ.get(GIT_CHECKOUT)
    if named:
        checkout = Path(named)
        if not checkout.is_dir():
            raise SystemExit(f"{GIT_CHECKOUT} names {named}, which is not a directory. {ways}")
        return checkout
    if sibling.is_dir():
        return sibling
    raise SystemExit(f"no gwz-git checkout found. {ways}")


def pinned_git_commit() -> str:
    """The gwz-git commit CI builds against: the first 40-hex line of the pin file."""
    path = ROOT / GIT_PIN_FILE
    if not path.is_file():
        raise SystemExit(f"{path} is missing: it pins the gwz-git commit CI builds against")
    for line in path.read_text().splitlines():
        if re.fullmatch(r"[0-9a-f]{40}", line.strip()):
            return line.strip()
    raise SystemExit(f"{path} names no gwz-git commit (a 40-hex line)")


def check_git_process_globals(allow_pin_mismatch: bool, start=run_check) -> None:
    """gwz-core, the consumer, checks the gwz-git it builds against, at the pin, with gwz-git's own allowlist."""
    checkout = git_checkout()
    pinned = pinned_git_commit()
    head = transport_head(checkout)
    if head != pinned:
        if not allow_pin_mismatch:
            raise SystemExit(
                f"{checkout} is at {head}, but {GIT_PIN_FILE} pins {pinned}, the commit CI builds against. "
                f"Check the pin out, or move the pin in the commit that needs the newer gwz-git; a lane "
                f"developing both at once passes {ALLOW_GIT_PIN_MISMATCH}"
            )
        print(
            f"PIN OVERRIDE ({ALLOW_GIT_PIN_MISMATCH}): {checkout} is at {head}, not the pin {pinned} "
            f"({GIT_PIN_FILE}); move the pin before gwz-core is pushed",
            flush=True,
        )
    start(
        [
            sys.executable,
            str(ROOT / "scripts/checks/check_process_globals.py"),
            "--repo",
            str(checkout),
            "--allowlist",
            str(checkout / GIT_ALLOWLIST),
        ],
        f"gwz-git process-global check: {checkout}",
    )


GIT_ADOPTION = "scripts/checks/check_gwz_git_adoption.py"


def check_git_adoption() -> None:
    """Every public capability of the gwz-git checkout (the one check_git_process_globals pinned) has a gwz-core caller."""
    checkout = git_checkout()
    print(f"gwz-git adoption check: {checkout}", flush=True)
    subprocess.run([sys.executable, str(ROOT / GIT_ADOPTION), "--gwz-git", str(checkout)], check=True)


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
        ALLOW_PIN_MISMATCH,
        action="store_true",
        help="run although the gwz-transport checkout is not at .github/gwz-transport.commit; "
        "for a lane that develops gwz-core and gwz-transport at once; prints both commits",
    )
    parser.add_argument(
        ALLOW_GIT_PIN_MISMATCH,
        action="store_true",
        help="run although the gwz-git checkout is not at .github/gwz-git.commit; "
        "for a lane that develops gwz-core and gwz-git at once; prints both commits",
    )
    parser.add_argument(
        SKIP_CFG_SIBLINGS,
        action="store_true",
        help="check conditional-compilation boundaries in gwz-core alone and print SKIPPED GATE "
        "for gwz-cli and gwz-py; only for a CI job that has neither checkout",
    )
    parser.add_argument(
        SKIP_CFG_SIBLING,
        action="append",
        choices=CFG_SIBLINGS,
        default=[],
        metavar="NAME",
        help="skip the named sibling's conditional-compilation boundary check and print SKIPPED GATE "
        "for it; only for a CI job that has no checkout of it",
    )
    parser.add_argument(
        "--test-filter",
        metavar="NAME",
        help="run, in each group, only that group's tests whose path contains NAME",
    )
    parser.add_argument(
        "--skip-exact",
        action="append",
        default=[],
        metavar="NAME",
        type=skip_exact_name,
        help="leave out the test with this full path ('a::b::name'); repeatable; for a CI step that runs it "
        "elsewhere, with the privilege it needs",
    )
    options, cargo_args = parser.parse_known_args(argv)
    for name in options.skip_exact:
        print(f"SKIPPED TEST {name}: run in the privileged step", flush=True)
    tally = FilterTally()
    narrow = {"test_filter": options.test_filter, "tally": tally, "skip_exact": tuple(options.skip_exact)}
    library_args = ["--lib", *[arg for arg in cargo_args if arg != "--lib"]]

    def source_checks(start) -> None:
        start([sys.executable, str(ROOT / "scripts/checks/check_filesystem_boundary.py")])
        start([sys.executable, str(ROOT / "scripts/checks/check_process_globals.py")])
        start([sys.executable, str(ROOT / "scripts/checks/check_spawn_gate_minting.py")])
        check_cfg_boundaries(
            [name for name in CFG_SIBLINGS if options.skip_cfg_siblings or name in options.skip_cfg_sibling],
            start,
        )
        start([sys.executable, str(ROOT / "scripts/checks/check_candidate_switches.py")])
        start([sys.executable, str(ROOT / "scripts/checks/check_carrier_peer_rule.py")])
        start([sys.executable, str(ROOT / "scripts/checks/check_driver_only_boundary.py")])
        start([sys.executable, str(ROOT / "scripts/checks/check_windows_parity.py")])
        check_transport_process_globals(options.skip_transport_globals, start)
        check_transport_pin(options.skip_transport_globals, options.allow_transport_pin_mismatch)
        check_git_process_globals(options.allow_git_pin_mismatch, start)

    run_source_checks(source_checks)
    # After every check above has passed, in the order the runner has always named them; these two run one at a
    # time, so their output and failures keep that order too.
    check_git_adoption()
    subprocess.run([sys.executable, str(ROOT / "scripts/checks/check_crate_versions.py")], check=True)
    filesystem_result = run("fake", library_args, list(FILESYSTEM_CONTRACTS), filesystem="fake", **narrow)
    if filesystem_result and "--no-fail-fast" not in cargo_args:
        raise SystemExit(filesystem_result)
    migrated = CONTRACTS + MATRICES + MIGRATED_GROUPS
    fake_result = run("fake", library_args, list(migrated), filesystem="fake", **narrow)
    if fake_result and "--no-fail-fast" not in cargo_args:
        raise SystemExit(fake_result)
    if options.compare:
        real_result = run("real", library_args, list(migrated + FILESYSTEM_CONTRACTS), **narrow)
    else:
        native_crosscheck_result = run("real", library_args, list(NATIVE_CROSSCHECKS), **narrow)
        if native_crosscheck_result and "--no-fail-fast" not in cargo_args:
            raise SystemExit(native_crosscheck_result)
        skipped = MATRICES + MIGRATED_GROUPS
        real_result = run("real", cargo_args, [arg for name in skipped for arg in ("--skip", name)], **narrow)
        real_result = native_crosscheck_result or real_result
    result = filesystem_result or fake_result or real_result
    if options.test_filter is None:
        # Always run: `result or run_session_crates()` would skip the crates once a core group had failed, and the
        # failing run would hide what they report.
        session_result = run_session_crates()
        result = result or session_result
    if options.test_filter is not None and tally.matched == 0:
        # A filter that selects nothing would otherwise pass having run no test.
        if tally.listing_failures:
            print(f"--test-filter {options.test_filter!r}: no test ran because {tally.listing_failures} group "
                  "listing failed", flush=True)
        else:
            print(f"--test-filter {options.test_filter!r} matches no test that runs in any group (ignored tests "
                  "are not run)", flush=True)
        raise SystemExit(result or 1)
    raise SystemExit(result)


if __name__ == "__main__":
    main()

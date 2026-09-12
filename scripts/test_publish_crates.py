#!/usr/bin/env python3
"""Unit tests for the crates.io publisher (plan S2.1, D5).

`scripts/publish_crates.py` walks one publish order and makes one decision per
crate: skip a version crates.io already holds, publish an absent one, wait out
the new-crate rate limit once, poll until the new version is visible, and stop
the run at the first unrelated failure. Every one of those decisions is
checked here through the script's injectable `Runner`, so no test opens a
socket, runs a cargo or sleeps for real.

The trees are synthetic manifest trees in a temporary directory -- a root
`gwz-core` manifest plus one `crates/<dir>/Cargo.toml` per internal -- because
what the publisher reads from a checkout is exactly the `[package]` name and
version of each manifest. One test reads the real gwz-core checkout this file
lives in, which is the honest check that the real manifests still give the
publisher a version for every crate the gate orders.

Style follows `scripts/test_release_bump.py`: load the script by path, drive
its helpers directly, and assert on the ledger lines a CI reader would see.
"""

from __future__ import annotations

import contextlib
import importlib.util
import io
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT_PATH = ROOT / "scripts" / "publish_crates.py"
CRATE_VERSIONS = ROOT / "scripts" / "checks" / "check_crate_versions.py"
SPEC = importlib.util.spec_from_file_location("gwz_core_publish_crates", SCRIPT_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load publisher script")
publish = importlib.util.module_from_spec(SPEC)
# Registered before execution because the script's dataclasses resolve their
# own annotations through `sys.modules[__module__]` under
# `from __future__ import annotations`.
sys.modules[SPEC.name] = publish
SPEC.loader.exec_module(publish)


CORE_VERSION = "1.0.12"
INTERNAL_VERSION = "0.0.2"
TAG = f"v{CORE_VERSION}"
# Two internals and the core: enough to prove the order is walked and that a
# failure stops the crates behind it, without restating the real fourteen.
INTERNALS = ("gwz-copy-contract", "gwz-family-model")
ORDER = (*INTERNALS, "gwz-core")
RATE_LIMIT_OUTPUT = (
    "    Uploading gwz-copy-contract v0.0.2\n"
    "error: failed to publish to registry at https://crates.io\n"
    "Caused by:\n"
    "  the remote server responded with an error (status 429 Too Many Requests): "
    "You have published too many new crates in a short period of time. "
    "Please try again after Wed, 13 Sep 2026 04:20:00 GMT or email help@crates.io "
    "to have your limit increased.\n"
)
OTHER_FAILURE_OUTPUT = (
    "error: failed to publish to registry at https://crates.io\n"
    "Caused by:\n"
    "  the remote server responded with an error (status 403 Forbidden): "
    "this crate exists but you don't seem to be an owner.\n"
)


def make_tree(root: Path, *, internals: tuple[str, ...] = INTERNALS) -> Path:
    """A synthetic checkout: the root manifest plus one manifest per internal."""
    (root / "crates").mkdir(parents=True)
    (root / "Cargo.toml").write_text(
        f'[package]\nname = "gwz-core"\nversion = "{CORE_VERSION}"\n', encoding="utf-8"
    )
    for name in internals:
        directory = root / "crates" / name.removeprefix("gwz-")
        directory.mkdir()
        (directory / "Cargo.toml").write_text(
            f'[package]\nname = "{name}"\nversion = "{INTERNAL_VERSION}"\n', encoding="utf-8"
        )
    return root


class FakeRunner:
    """The publisher's whole outside world, scripted per crate.

    `statuses` and `publishes` map a crate name to the answers it gets, in
    order; the last entry of a list repeats, so a test states only what it
    cares about. Unlisted crates are absent from crates.io (404) and publish
    successfully, which is the boring path every test varies from.
    """

    def __init__(
        self,
        *,
        order: tuple[str, ...] = ORDER,
        statuses: dict[str, list[int]] | None = None,
        publishes: dict[str, list[tuple[int, str]]] | None = None,
        gate: int = 0,
    ) -> None:
        self.order = list(order)
        self.statuses = {name: list(values) for name, values in (statuses or {}).items()}
        self.publishes = {name: list(values) for name, values in (publishes or {}).items()}
        self.gate = gate
        self.commands: list[list[str]] = []
        self.cwds: list[Path] = []
        self.probes: list[str] = []
        self.sleeps: list[float] = []
        self.clock = 0.0

    @staticmethod
    def _next(queue: list, fallback):
        if not queue:
            return fallback
        return queue.pop(0) if len(queue) > 1 else queue[0]

    def run(self, command, *, cwd, echo: bool = True) -> publish.Completed:
        text = [str(part) for part in command]
        self.commands.append(text)
        self.cwds.append(Path(cwd))
        if "--print-publish-order" in text:
            return publish.Completed(returncode=0, output="\n".join(self.order) + "\n")
        if any(part.endswith("check_crate_versions.py") for part in text):
            return publish.Completed(returncode=self.gate, output="crate versions: ok\n")
        if text[:2] == ["cargo", "publish"]:
            name = text[text.index("-p") + 1]
            code, output = self._next(self.publishes.get(name, []), (0, ""))
            return publish.Completed(returncode=code, output=output)
        raise AssertionError(f"unexpected command {text}")

    def probe(self, url: str) -> int:
        self.probes.append(url)
        name = url.rsplit("/", 2)[-2]
        return self._next(self.statuses.get(name, []), 404)

    def sleep(self, seconds: float) -> None:
        self.sleeps.append(seconds)
        self.clock += seconds

    def monotonic(self) -> float:
        return self.clock

    # -- what the assertions read -------------------------------------------
    def published(self) -> list[str]:
        return [
            command[command.index("-p") + 1]
            for command in self.commands
            if command[:2] == ["cargo", "publish"]
        ]

    def gate_commands(self) -> list[list[str]]:
        return [
            command
            for command in self.commands
            if any(part.endswith("check_crate_versions.py") for part in command)
        ]


class PublisherTestCase(unittest.TestCase):
    """A synthetic checkout, a scripted runner, and the captured ledger."""

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.tree = make_tree(Path(self.temporary.name) / "gwz-core")

    def drive(self, runner: FakeRunner, **options) -> tuple[int, str]:
        """Run the publisher over the synthetic tree; return (status, ledger)."""
        publisher = publish.Publisher(root=self.tree, runner=runner, **options)
        ledger = io.StringIO()
        with contextlib.redirect_stdout(ledger):
            try:
                status = publisher.run(TAG)
            except publish.PublishError as error:
                status = 1
                self.failure = str(error)
        return status, ledger.getvalue()

    def expect_failure(self, runner: FakeRunner, **options) -> tuple[str, str]:
        """Run the publisher expecting it to stop; return (message, ledger)."""
        self.failure = ""
        status, ledger = self.drive(runner, **options)
        self.assertEqual(1, status, ledger)
        self.assertTrue(self.failure, "a stopped run must say why")
        return self.failure, ledger


class PlanTests(PublisherTestCase):
    """The order and the versions both come from the checkout, not from here."""

    def test_the_gate_runs_with_the_tag_before_the_order_is_read(self) -> None:
        runner = FakeRunner(statuses={name: [200] for name in ORDER})
        status, ledger = self.drive(runner)
        self.assertEqual(0, status, ledger)
        gates = runner.gate_commands()
        self.assertEqual(2, len(gates), gates)
        self.assertIn("--tag", gates[0])
        self.assertEqual(TAG, gates[0][gates[0].index("--tag") + 1])
        self.assertNotIn("--print-publish-order", gates[0])
        self.assertIn("--print-publish-order", gates[1])
        for command in gates:
            self.assertEqual(sys.executable, command[0])
            self.assertEqual(str(self.tree / publish.CRATE_VERSIONS), command[1])
            self.assertEqual(str(self.tree), command[command.index("--root") + 1])

    def test_a_refused_gate_stops_before_any_lookup_or_publish(self) -> None:
        runner = FakeRunner(gate=1)
        message, _ledger = self.expect_failure(runner)
        self.assertIn("lockstep gate refused", message)
        self.assertEqual([], runner.probes)
        self.assertEqual([], runner.published())

    def test_the_plan_pairs_the_gate_s_order_with_the_manifest_versions(self) -> None:
        runner = FakeRunner(statuses={name: [200] for name in ORDER})
        _status, ledger = self.drive(runner)
        self.assertIn(f"tag {TAG}: 3 crate(s) to publish in dependency order", ledger)
        self.assertIn(f"1. gwz-copy-contract {INTERNAL_VERSION}", ledger)
        self.assertIn(f"2. gwz-family-model {INTERNAL_VERSION}", ledger)
        self.assertIn(f"3. gwz-core {CORE_VERSION}", ledger)

    def test_an_order_naming_an_absent_crate_stops_the_run(self) -> None:
        runner = FakeRunner(order=("gwz-copy-contract", "gwz-not-here", "gwz-core"))
        message, _ledger = self.expect_failure(runner)
        self.assertIn("gwz-not-here", message)
        self.assertEqual([], runner.published())

    def test_the_real_checkout_declares_every_crate_the_gate_orders(self) -> None:
        versions = publish.read_versions(ROOT)
        names = subprocess.run(
            [sys.executable, str(CRATE_VERSIONS), "--root", str(ROOT), "--print-publish-order"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout.split()
        self.assertEqual(14, len(names))
        self.assertEqual("gwz-core", names[-1])
        self.assertEqual([], [name for name in names if name not in versions], versions)
        internals = {versions[name] for name in names if name != "gwz-core"}
        self.assertEqual(1, len(internals), internals)
        self.assertRegex(internals.pop(), r"^0\.0\.[0-9]+$")
        self.assertRegex(versions["gwz-core"], r"^[0-9]+\.[0-9]+\.[0-9]+")


class SkipTests(PublisherTestCase):
    """A version crates.io already holds is what makes a retry idempotent."""

    def test_a_present_version_is_skipped_and_never_published(self) -> None:
        runner = FakeRunner(statuses={name: [200] for name in ORDER})
        status, ledger = self.drive(runner)
        self.assertEqual(0, status, ledger)
        self.assertEqual([], runner.published())
        for name in ORDER:
            self.assertIn(f"{name} ", ledger)
        self.assertIn("skip (already on crates.io)", ledger)
        self.assertIn("ok (0 published, 3 already on crates.io", ledger)
        self.assertEqual([], runner.sleeps)

    def test_a_partial_run_resumes_at_the_first_absent_crate(self) -> None:
        runner = FakeRunner(
            statuses={
                "gwz-copy-contract": [200],
                "gwz-family-model": [404, 200],
                "gwz-core": [404, 200],
            }
        )
        status, ledger = self.drive(runner)
        self.assertEqual(0, status, ledger)
        self.assertEqual(["gwz-family-model", "gwz-core"], runner.published())
        self.assertIn("ok (2 published, 1 already on crates.io", ledger)

    def test_an_unexpected_status_stops_the_run_rather_than_guessing(self) -> None:
        runner = FakeRunner(statuses={"gwz-copy-contract": [503]})
        message, _ledger = self.expect_failure(runner)
        self.assertIn("answered 503", message)
        self.assertEqual([], runner.published())


class PublishTests(PublisherTestCase):
    """An absent version is published with the exact command the plan names."""

    def test_an_absent_version_is_published_locked_and_unverified(self) -> None:
        runner = FakeRunner(statuses={name: [404, 200] for name in ORDER})
        status, ledger = self.drive(runner)
        self.assertEqual(0, status, ledger)
        self.assertEqual(list(ORDER), runner.published())
        for command, cwd in zip(runner.commands, runner.cwds):
            if command[:2] != ["cargo", "publish"]:
                continue
            self.assertEqual(
                ["cargo", "publish", "-p", command[3], "--locked", "--no-verify"], command
            )
            self.assertEqual(self.tree, cwd)
        self.assertIn("ok (3 published, 0 already on crates.io", ledger)
        self.assertIn("0 rate-limit wait(s)", ledger)

    def test_an_unrelated_failure_stops_the_run_at_that_crate(self) -> None:
        runner = FakeRunner(
            statuses={name: [404, 200] for name in ORDER},
            publishes={"gwz-family-model": [(101, OTHER_FAILURE_OUTPUT)]},
        )
        message, _ledger = self.expect_failure(runner)
        self.assertIn("gwz-family-model", message)
        self.assertIn("cargo publish failed with exit 101", message)
        self.assertIn("resumes here", message)
        # The first crate published, the failing one stopped the walk, and the
        # crate behind it was never attempted.
        self.assertEqual(["gwz-copy-contract", "gwz-family-model"], runner.published())
        self.assertEqual([], runner.sleeps)


class RateLimitTests(PublisherTestCase):
    """crates.io's new-crate limit is waited out once per crate (plan D5)."""

    def test_a_rate_limit_refusal_is_waited_out_and_the_retry_publishes(self) -> None:
        runner = FakeRunner(
            statuses={name: [404, 200] for name in ORDER},
            publishes={"gwz-copy-contract": [(101, RATE_LIMIT_OUTPUT), (0, "")]},
        )
        status, ledger = self.drive(runner, wait_seconds=620)
        self.assertEqual(0, status, ledger)
        self.assertEqual([620], runner.sleeps)
        self.assertEqual(
            ["gwz-copy-contract", "gwz-copy-contract", "gwz-family-model", "gwz-core"],
            runner.published(),
        )
        self.assertIn("sleeping 620s before the one retry", ledger)
        self.assertIn("waited 620s after the new-crate rate limit; retrying", ledger)
        self.assertIn("1 rate-limit wait(s)", ledger)

    def test_a_second_refusal_exhausts_this_crate_s_wait_budget(self) -> None:
        runner = FakeRunner(
            statuses={name: [404, 200] for name in ORDER},
            publishes={"gwz-copy-contract": [(101, RATE_LIMIT_OUTPUT)]},
        )
        message, _ledger = self.expect_failure(runner, wait_seconds=620)
        self.assertIn("still refuses new crates after one 620s wait", message)
        self.assertIn("one wait per crate", message)
        self.assertEqual([620], runner.sleeps)
        self.assertEqual(["gwz-copy-contract", "gwz-copy-contract"], runner.published())

    def test_each_crate_gets_its_own_wait(self) -> None:
        runner = FakeRunner(
            statuses={name: [404, 200] for name in ORDER},
            publishes={
                "gwz-copy-contract": [(101, RATE_LIMIT_OUTPUT), (0, "")],
                "gwz-family-model": [(101, RATE_LIMIT_OUTPUT), (0, "")],
            },
        )
        status, ledger = self.drive(runner, wait_seconds=620)
        self.assertEqual(0, status, ledger)
        self.assertEqual([620, 620], runner.sleeps)
        self.assertIn("2 rate-limit wait(s)", ledger)


class IndexTests(PublisherTestCase):
    """A published version must be visible before the next crate is built."""

    def test_the_poll_reports_how_long_the_version_took_to_appear(self) -> None:
        runner = FakeRunner(
            statuses={
                "gwz-copy-contract": [404, 404, 404, 200],
                "gwz-family-model": [404, 200],
                "gwz-core": [404, 200],
            }
        )
        status, ledger = self.drive(runner)
        self.assertEqual(0, status, ledger)
        # Three polls at ten seconds each: absent, absent, then visible.
        self.assertEqual(
            [publish.INDEX_POLL_SECONDS, publish.INDEX_POLL_SECONDS], runner.sleeps
        )
        self.assertIn("gwz-copy-contract 0.0.2: visible after 20 seconds", ledger)
        self.assertIn("gwz-family-model 0.0.2: visible after 0 seconds", ledger)

    def test_an_index_that_never_shows_the_version_is_an_error(self) -> None:
        runner = FakeRunner(statuses={"gwz-copy-contract": [404]})
        message, _ledger = self.expect_failure(runner, index_timeout=60)
        self.assertIn("still not visible on crates.io", message)
        self.assertIn("after 60s", message)
        self.assertEqual([publish.INDEX_POLL_SECONDS] * 6, runner.sleeps)
        # The crate behind the timeout is not published: it could not resolve
        # a dependency the registry has not shown yet.
        self.assertEqual(["gwz-copy-contract"], runner.published())


class DryRunTests(PublisherTestCase):
    """`--dry-run` decides everything and publishes nothing."""

    def test_the_dry_run_reads_crates_io_but_calls_no_cargo(self) -> None:
        runner = FakeRunner()
        ledger = io.StringIO()
        with contextlib.redirect_stdout(ledger):
            status = publish.main(
                ["--tag", TAG, "--root", str(self.tree), "--dry-run"], runner=runner
            )
        report = ledger.getvalue()
        self.assertEqual(0, status, report)
        self.assertEqual([], runner.published())
        self.assertEqual([], runner.sleeps)
        self.assertEqual(3, len(runner.probes))
        self.assertIn("dry run: nothing will be published", report)
        for name in ORDER:
            self.assertIn(f"{name} ", report)
        self.assertIn("publish (dry run: not published)", report)
        self.assertIn("dry run: nothing published (3 would publish, 0 already", report)
        # The lockstep gate still runs: a dry run that skipped it would report
        # a plan the real run would refuse.
        self.assertEqual(2, len(runner.gate_commands()))

    def test_a_malformed_tag_is_refused_before_anything_runs(self) -> None:
        runner = FakeRunner()
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            status = publish.main(["--tag", "1.0.12", "--root", str(self.tree)], runner=runner)
        self.assertEqual(2, status)
        self.assertIn("is not `vX.Y.Z`", stderr.getvalue())
        self.assertEqual([], runner.commands)
        self.assertEqual([], runner.probes)


class ScriptTests(unittest.TestCase):
    """What the publisher must not do, and what the workflow calls."""

    def test_the_script_never_reads_the_registry_token(self) -> None:
        source = SCRIPT_PATH.read_text(encoding="utf-8")
        self.assertIn("CARGO_REGISTRY_TOKEN", source, "the docstring explains the credential")
        # cargo inherits the credential from the process environment; nothing
        # here reads it, passes an environment to a subprocess, or could print
        # it, which is why the script imports no `os` at all.
        self.assertNotIn("os.environ", source)
        self.assertNotIn("getenv", source)
        self.assertNotIn("env=", source)
        self.assertNotRegex(source, r"(?m)^import os$")

    def test_the_script_holds_no_hand_written_publish_order(self) -> None:
        source = SCRIPT_PATH.read_text(encoding="utf-8")
        self.assertIn("--print-publish-order", source)
        internals = re.findall(r'"gwz-(?!core)[a-z-]+"', source)
        self.assertEqual([], internals, internals)

    def test_help_names_the_defaults_the_workflow_relies_on(self) -> None:
        result = subprocess.run(
            [sys.executable, str(SCRIPT_PATH), "--help"],
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertIn("--dry-run", result.stdout)
        self.assertEqual(620, publish.DEFAULT_WAIT_SECONDS)
        self.assertEqual(600, publish.DEFAULT_INDEX_TIMEOUT)


if __name__ == "__main__":
    unittest.main()

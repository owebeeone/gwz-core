#!/usr/bin/env python3
"""Publish gwz-core's fourteen crates to crates.io, in order, idempotently.

Authority: gwz-core `dev-docs/GwzCratesIoPlan.md` (ADOPTED 2026-09-13) D5 and
its step S2.1. This is the script the `publish` job of
`.github/workflows/release.yml` runs, and nothing else runs it: publishing
never happens from a laptop (D5).

What it does, in one pass over the publish order:

- Runs `scripts/checks/check_crate_versions.py --tag <tag>` first (plan S1.2)
  and stops if it fails, so nothing is uploaded from a tree whose manifests
  disagree with the tag. A publish is permanent; only the version check is
  cheap.
- Takes the order from the same script's `--print-publish-order` (the thirteen
  published internals in dependency order, then `gwz-core`) and each crate's
  version from that crate's own manifest, so a new crate or a new internal
  edge cannot leave a hand-written list here publishing the wrong set.
- Asks crates.io whether each version is already there
  (`GET /api/v1/crates/<name>/<version>`; 200 means published, 404 means
  absent). A present version is skipped, which is what makes a
  `workflow_dispatch` retry resume at the crate that failed rather than start
  over. Anything but 200 or 404 stops the run instead of guessing.
- Publishes an absent version with `cargo publish -p <name> --locked
  --no-verify` from the checkout root. `--no-verify` is honest because the
  release script already packaged the whole workspace before the tag existed
  (plan S1.4/S1.5: `cargo package --workspace --no-verify --locked`, because
  packaging one internal crate alone resolves its published manifest against
  the registry, where the new internal versions do not exist yet).
- On crates.io's new-crate rate-limit refusal ("You have published too many
  new crates in a short period of time"; section 1: a burst of 5, then one
  more every 10 minutes) it sleeps `--wait-seconds` -- a little over ten
  minutes, so the run is correct whether the refill counts from the last
  success or the last refusal (plan U7) -- and retries that same crate once.
  A second refusal for the same crate gives up with a message; one wait per
  crate still to publish is the whole budget (D5), which is what makes the
  first run's thirteen new names finish unattended in about eighty minutes
  (S2.2) with no rate-limit increase requested.
- Any other publish failure exits non-zero immediately, at that crate.
- After a successful publish it polls the same endpoint until the version is
  visible, up to `--index-timeout` (plan U4: cargo's own sixty-second index
  wait was not enough for uv, so the wait lives here at ten minutes). A
  timeout is an error: the next crate could not resolve this one anyway.

Every decision prints one line, so the CI log reads as a ledger of what was
skipped, published, waited for and seen.

Authentication is whatever `CARGO_REGISTRY_TOKEN` cargo finds in the
environment -- the `crates-io` environment secret for the first publication of
a new name, the Trusted Publishing token afterwards (D5, S2.2, S2.3). This
script never reads that variable and never prints it.

The network and the subprocesses live behind one injectable `Runner`, so the
unit tests in `scripts/test_publish_crates.py` exercise the decisions without
a network or a cargo.

Usage (from the gwz-core checkout):
    python scripts/publish_crates.py --tag v1.0.12
    python scripts/publish_crates.py --tag v1.0.12 --dry-run
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import time
import tomllib
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path


DEFAULT_ROOT = Path(__file__).resolve().parents[1]
CRATE_VERSIONS = Path("scripts") / "checks" / "check_crate_versions.py"
MANIFEST = "Cargo.toml"
CRATES_DIR = "crates"
API = "https://crates.io/api/v1/crates/{name}/{version}"
# crates.io refuses a request without a User-Agent that identifies the client,
# so name the script and the repository rather than urllib's default.
USER_AGENT = "gwz-core-publish-crates (scripts/publish_crates.py; https://github.com/owebeeone/gwz-core)"
RELEASE_TAG = re.compile(r"^v(?P<version>[0-9]+\.[0-9]+\.[0-9]+(?:-rc\.[0-9]+)?)$")
# The distinguishing fragment of "You have published too many new crates in a
# short period of time" (plan section 1). Matched on the captured output, not
# on an exit code, because cargo reports every API refusal the same way.
RATE_LIMIT = "too many new crates"
DEFAULT_WAIT_SECONDS = 620
DEFAULT_INDEX_TIMEOUT = 600
INDEX_POLL_SECONDS = 10
HTTP_TIMEOUT = 30
PUBLISHED = 200
ABSENT = 404


class PublishError(Exception):
    """The run cannot go on: a gate failed, a publish failed, or the index never showed."""


@dataclass(frozen=True)
class Completed:
    """One finished subprocess: its exit status and everything it printed."""

    returncode: int
    output: str


@dataclass(frozen=True)
class Crate:
    """One crate to publish, at the version its own manifest declares."""

    name: str
    version: str


class Runner:
    """Every effect the publisher has on the world, in one injectable object."""

    def run(self, command: list[str], *, cwd: Path, echo: bool = True) -> Completed:
        """Run a command, stream its output when asked, and capture all of it."""
        process = subprocess.Popen(
            [str(part) for part in command],
            cwd=str(cwd),
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            bufsize=1,
        )
        chunks: list[str] = []
        with process:
            for line in process.stdout or ():
                chunks.append(line)
                if echo:
                    sys.stdout.write(line)
                    sys.stdout.flush()
        return Completed(returncode=process.returncode, output="".join(chunks))

    def probe(self, url: str) -> int:
        """The HTTP status of a read-only crates.io lookup; 0 when it did not answer."""
        request = urllib.request.Request(
            url, headers={"User-Agent": USER_AGENT}, method="GET"
        )
        try:
            with urllib.request.urlopen(request, timeout=HTTP_TIMEOUT) as response:
                return int(response.status)
        except urllib.error.HTTPError as error:
            return int(error.code)
        except (urllib.error.URLError, TimeoutError, OSError):
            return 0

    def sleep(self, seconds: float) -> None:
        time.sleep(seconds)

    def monotonic(self) -> float:
        return time.monotonic()


def read_versions(root: Path) -> dict[str, str]:
    """Every package this checkout declares, mapped to its manifest version.

    Read with `tomllib` from gwz-core's own manifest and each `crates/*`
    manifest, keyed by the `[package]` name rather than by the directory name,
    because the directories drop the `gwz-` prefix and a rename must not be
    able to silently repoint a publish.
    """
    versions: dict[str, str] = {}
    for path in [root / MANIFEST, *sorted((root / CRATES_DIR).glob(f"*/{MANIFEST}"))]:
        try:
            table = tomllib.loads(path.read_text(encoding="utf-8"))
        except OSError as error:
            raise PublishError(f"cannot read {path}: {error}") from error
        except tomllib.TOMLDecodeError as error:
            raise PublishError(f"cannot parse {path}: {error}") from error
        package = table.get("package")
        if not isinstance(package, dict):
            continue
        name = package.get("name")
        version = package.get("version")
        if isinstance(name, str) and name and isinstance(version, str) and version:
            versions[name] = version
    if not versions:
        raise PublishError(f"{root}: no crate manifest to publish")
    return versions


class Publisher:
    """The publish order, walked once, with one printed line per decision."""

    def __init__(
        self,
        *,
        root: Path,
        runner: Runner,
        dry_run: bool = False,
        wait_seconds: float = DEFAULT_WAIT_SECONDS,
        index_timeout: float = DEFAULT_INDEX_TIMEOUT,
    ) -> None:
        self.root = root
        self.runner = runner
        self.dry_run = dry_run
        self.wait_seconds = wait_seconds
        self.index_timeout = index_timeout
        self.waits = 0

    def say(self, message: str) -> None:
        print(f"publish crates: {message}", flush=True)

    def url(self, crate: Crate) -> str:
        return API.format(name=crate.name, version=crate.version)

    def gate(self, tag: str) -> None:
        """The lockstep gate on this tag (plan S1.2), before anything is uploaded."""
        result = self.runner.run(
            [sys.executable, str(self.root / CRATE_VERSIONS), "--root", str(self.root), "--tag", tag],
            cwd=self.root,
        )
        if result.returncode != 0:
            raise PublishError(
                f"the crate-version lockstep gate refused {tag} (exit {result.returncode}); "
                "nothing is uploaded from a tree whose manifests disagree with the tag, because "
                "a published version can only be yanked, never replaced"
            )

    def order(self) -> list[str]:
        """The publish order, derived from the manifests by the lockstep gate."""
        result = self.runner.run(
            [
                sys.executable,
                str(self.root / CRATE_VERSIONS),
                "--root",
                str(self.root),
                "--print-publish-order",
            ],
            cwd=self.root,
            echo=False,
        )
        names = result.output.split()
        if result.returncode != 0 or not names or names[-1] != "gwz-core":
            raise PublishError(
                f"--print-publish-order did not end at gwz-core (exit {result.returncode}): "
                f"{names}"
            )
        return names

    def plan(self, tag: str) -> list[Crate]:
        """What will be published, in order, at which version."""
        self.gate(tag)
        names = self.order()
        versions = read_versions(self.root)
        missing = [name for name in names if name not in versions]
        if missing:
            raise PublishError(
                "the publish order names crate(s) no manifest in this checkout declares: "
                + ", ".join(missing)
            )
        crates = [Crate(name=name, version=versions[name]) for name in names]
        self.say(
            f"tag {tag}: {len(crates)} crate(s) to publish in dependency order"
            + (" (dry run: nothing will be published)" if self.dry_run else "")
        )
        for index, crate in enumerate(crates, start=1):
            self.say(f"  {index:2}. {crate.name} {crate.version}")
        return crates

    def is_published(self, crate: Crate) -> bool:
        """Whether crates.io already holds this exact version."""
        url = self.url(crate)
        status = self.runner.probe(url)
        if status == PUBLISHED:
            return True
        if status == ABSENT:
            return False
        raise PublishError(
            f"{crate.name} {crate.version}: crates.io answered {status} for {url}; only 200 "
            "(published) and 404 (absent) decide whether to publish, so the run stops rather "
            "than guess"
        )

    def publish_one(self, crate: Crate) -> None:
        """Publish one crate, waiting out one new-crate rate-limit refusal."""
        waited = False
        while True:
            self.say(f"{crate.name} {crate.version}: publish")
            result = self.runner.run(
                ["cargo", "publish", "-p", crate.name, "--locked", "--no-verify"],
                cwd=self.root,
            )
            if result.returncode == 0:
                return
            if RATE_LIMIT not in result.output:
                raise PublishError(
                    f"{crate.name} {crate.version}: cargo publish failed with exit "
                    f"{result.returncode}; the run stops at this crate, and a workflow_dispatch "
                    "retry on the same tag resumes here because every version already on "
                    "crates.io is skipped"
                )
            if waited:
                raise PublishError(
                    f"{crate.name} {crate.version}: crates.io still refuses new crates after one "
                    f"{self.wait_seconds:.0f}s wait; the budget is one wait per crate still to "
                    "publish (plan D5), so the run stops here and a workflow_dispatch retry on "
                    "the same tag resumes at this crate"
                )
            waited = True
            self.waits += 1
            self.say(
                f"{crate.name} {crate.version}: crates.io refused a new crate (its new-crate "
                f"rate limit); sleeping {self.wait_seconds:.0f}s before the one retry this crate "
                "is allowed"
            )
            self.runner.sleep(self.wait_seconds)
            self.say(
                f"{crate.name} {crate.version}: waited {self.wait_seconds:.0f}s after the "
                "new-crate rate limit; retrying"
            )

    def wait_for_index(self, crate: Crate) -> None:
        """Poll crates.io until the just-published version is visible (plan U4)."""
        url = self.url(crate)
        start = self.runner.monotonic()
        while True:
            status = self.runner.probe(url)
            elapsed = self.runner.monotonic() - start
            if status == PUBLISHED:
                self.say(f"{crate.name} {crate.version}: visible after {elapsed:.0f} seconds")
                return
            if elapsed >= self.index_timeout:
                raise PublishError(
                    f"{crate.name} {crate.version}: published but still not visible on crates.io "
                    f"after {elapsed:.0f}s (last status {status}); the crates that depend on it "
                    "are built against the registry and could not resolve it, so the run stops "
                    "(plan U4)"
                )
            self.runner.sleep(INDEX_POLL_SECONDS)

    def run(self, tag: str) -> int:
        """Walk the publish order once; return the process exit status."""
        crates = self.plan(tag)
        published = 0
        present = 0
        for crate in crates:
            if self.is_published(crate):
                self.say(f"{crate.name} {crate.version}: skip (already on crates.io)")
                present += 1
                continue
            if self.dry_run:
                self.say(f"{crate.name} {crate.version}: publish (dry run: not published)")
                published += 1
                continue
            self.publish_one(crate)
            self.wait_for_index(crate)
            published += 1
        if self.dry_run:
            self.say(
                f"dry run: nothing published ({published} would publish, {present} already on "
                f"crates.io, of {len(crates)} crate(s))"
            )
        else:
            self.say(
                f"ok ({published} published, {present} already on crates.io, of {len(crates)} "
                f"crate(s); {self.waits} rate-limit wait(s))"
            )
        return 0


def main(argv: list[str] | None = None, runner: Runner | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "--tag",
        required=True,
        help="the release tag `vX.Y.Z` or `vX.Y.Z-rc.N` being published; gwz-core's version "
        "must equal its version",
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=DEFAULT_ROOT,
        help="the gwz-core checkout holding Cargo.toml and crates/",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="print the plan and every decision and publish nothing; the read-only crates.io "
        "version lookups still happen, nothing else touches the network",
    )
    parser.add_argument(
        "--wait-seconds",
        type=float,
        default=DEFAULT_WAIT_SECONDS,
        help="how long to sleep after crates.io refuses a new crate, before the one retry that "
        f"crate is allowed (default {DEFAULT_WAIT_SECONDS}, a little over its ten-minute refill)",
    )
    parser.add_argument(
        "--index-timeout",
        type=float,
        default=DEFAULT_INDEX_TIMEOUT,
        help="how long to wait for a published version to become visible on crates.io before "
        f"failing (default {DEFAULT_INDEX_TIMEOUT} seconds)",
    )
    args = parser.parse_args(argv)
    if RELEASE_TAG.match(args.tag) is None:
        print(
            f"publish crates: error: tag {args.tag!r} is not `vX.Y.Z` or `vX.Y.Z-rc.N`",
            file=sys.stderr,
        )
        return 2
    publisher = Publisher(
        root=args.root.resolve(),
        runner=runner if runner is not None else Runner(),
        dry_run=args.dry_run,
        wait_seconds=args.wait_seconds,
        index_timeout=args.index_timeout,
    )
    try:
        return publisher.run(args.tag)
    except PublishError as error:
        print(f"publish crates: failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())

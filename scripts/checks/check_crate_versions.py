#!/usr/bin/env python3
"""Lockstep gate: the crate versions and dependency edges a crates.io publish needs.

Authority: gwz-core `dev-docs/GwzCratesIoPlan.md` (ADOPTED 2026-09-13) D1, D2
and D8, implemented by its step S1.2. Fifteen crates publish; the fourteen
crates under `crates/` share their own `0.0.N` internal line while gwz-core
carries the product version, and `gwz-local-testrepo` alone stays
`publish = false` because it is a path-only dev-dependency that cargo drops
from the published package. crates.io refuses a published crate that depends
on a git source, or on a path dependency without a `version`, so every
internal edge must name the internal version beside its `path`. A comment in
each manifest cannot check any of that; this gate is the check those comments
could not be.

What the gate checks (pure manifest metadata, read with `tomllib`, no build):

- gwz-core's `[package].version` is a plain `X.Y.Z` or `X.Y.Z-rc.N` (the shape
  the release script accepts) and, with `--tag vX.Y.Z`, equals the tag's
  version -- the assertion the publish job makes before it uploads anything.
- All fourteen crates under `crates/` carry one and the same version, on the
  internal `^0\\.0\\.[0-9]+$` line (D2). A crate whose version differs is named;
  so is one whose version is off the line entirely.
- Every `gwz-*` entry of a `[dependencies]` or `[build-dependencies]` table --
  gwz-core's and each crate's, target-specific tables included -- has both a
  `path` (so a local build stays local) and a `version` equal to that internal
  version (so the registry build resolves). Build dependencies are kept in a
  published manifest exactly as normal ones are, hence the same rule; there
  are none today.
- Every `gwz-*` entry of a `[dev-dependencies]` table has a `path` and NO
  `version`. cargo drops a path-only dev-dependency from the published
  manifest, and a versioned one would have to exist on crates.io -- which
  `gwz-local-testrepo`, being unpublished, never will.
- No dependency of any kind in any of these manifests uses `git`.
- Each of the thirteen published crates carries `repository`, `readme`,
  `license` and `description`, and is not `publish = false`;
  `gwz-local-testrepo` is (D1).

Not checked here, deliberately: gwz-core's own registry metadata and the
`include` lists (plan S1.4), the crates' roles and dependency edges
(`check_local_clone_boundaries.py`), and anything that needs a build.

Also derived here, because the manifests are the only honest source for it:
`--print-publish-order` prints the order a publisher must follow -- the
thirteen published internals in dependency order, then `gwz-core` -- one name
per line. The release script (plan S1.5) and the CI publish job (S2.1) consume
those lines instead of carrying a hand-written list that a new crate or a new
edge would silently invalidate.

Usage (from gwz-core):
    python3 scripts/checks/check_crate_versions.py
    python3 scripts/checks/check_crate_versions.py --root <gwz-core>
    python3 scripts/checks/check_crate_versions.py --tag v1.0.12
    python3 scripts/checks/check_crate_versions.py --print-publish-order
"""

from __future__ import annotations

import argparse
import re
import sys
import tomllib
from collections import Counter
from dataclasses import dataclass
from pathlib import Path


DEFAULT_ROOT = Path(__file__).resolve().parents[2]
CRATES_DIR = "crates"
MANIFEST = "Cargo.toml"
CORE_NAME = "gwz-core"
# The one crate of `crates/` that does not publish (plan D1). It still tracks
# the internal version line so the release bump stays uniform.
UNPUBLISHED = ("gwz-local-testrepo",)
# Section 1 of the plan names the fourteen internal crates. A fifteenth (or a
# thirteenth) is a publish-order and release-script change, so it is drift this
# gate reports rather than absorbs.
EXPECTED_CRATES = 14
REQUIRED_METADATA = ("repository", "readme", "license", "description")
VERSIONED_KINDS = ("dependencies", "build-dependencies")
PATH_ONLY_KINDS = ("dev-dependencies",)
DEPENDENCY_KINDS = VERSIONED_KINDS + PATH_ONLY_KINDS
INTERNAL_PREFIX = "gwz-"
INTERNAL_VERSION = re.compile(r"^0\.0\.[0-9]+$")
PRODUCT_VERSION = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+(?:-rc\.[0-9]+)?$")
RELEASE_TAG = re.compile(r"^v(?P<version>[0-9]+\.[0-9]+\.[0-9]+(?:-rc\.[0-9]+)?)$")


class GateError(Exception):
    """An input is missing or cannot be read at all."""


@dataclass(frozen=True)
class Manifest:
    """One parsed manifest: its package name, where it is, and its tables."""

    name: str
    where: str
    table: dict


@dataclass(frozen=True)
class Summary:
    """What the gate compared, for the one-line pass report."""

    core_version: str
    internal_version: str
    crates: int
    published: int
    unpublished: int
    versioned_edges: int
    path_only_edges: int


def _read(path: Path, what: str) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError as error:
        raise GateError(f"cannot read {what} {path}: {error}") from error


def load_manifest(path: Path, root: Path) -> Manifest:
    """Parse one manifest; a missing `[package]` name is an error, not a finding."""
    try:
        table = tomllib.loads(_read(path, "manifest"))
    except tomllib.TOMLDecodeError as error:
        raise GateError(f"cannot parse {path}: {error}") from error
    package = table.get("package")
    if not isinstance(package, dict):
        raise GateError(f"{path}: no [package] table")
    name = package.get("name")
    if not isinstance(name, str) or not name:
        raise GateError(f"{path}: no [package] name")
    try:
        where = path.relative_to(root).as_posix()
    except ValueError:
        where = path.as_posix()
    return Manifest(name=name, where=where, table=table)


def discover(root: Path) -> tuple[Manifest, list[Manifest]]:
    """gwz-core's manifest and every manifest under `crates/`."""
    core = load_manifest(root / MANIFEST, root)
    if core.name != CORE_NAME:
        raise GateError(
            f"{root / MANIFEST}: expected [package] name {CORE_NAME!r}, found {core.name!r}"
        )
    crates_dir = root / CRATES_DIR
    if not crates_dir.is_dir():
        raise GateError(f"{root}: no {CRATES_DIR}/ directory")
    crates = [load_manifest(path, root) for path in sorted(crates_dir.glob(f"*/{MANIFEST}"))]
    if not crates:
        raise GateError(f"{crates_dir}: no crate manifest")
    return core, crates


def dependency_tables(table: dict):
    """`(kind, where, entries)` for every dependency table, target ones included."""
    for kind in DEPENDENCY_KINDS:
        entries = table.get(kind)
        if isinstance(entries, dict):
            yield kind, f"[{kind}]", entries
    targets = table.get("target")
    if isinstance(targets, dict):
        for cfg in sorted(targets):
            cfg_table = targets[cfg]
            if not isinstance(cfg_table, dict):
                continue
            for kind in DEPENDENCY_KINDS:
                entries = cfg_table.get(kind)
                if isinstance(entries, dict):
                    yield kind, f"[target.'{cfg}'.{kind}]", entries


def internal_requirements(crate: Manifest, published: set[str]) -> set[str]:
    """The published internal crates one crate needs at publish time."""
    needs: set[str] = set()
    for kind, _where, entries in dependency_tables(crate.table):
        if kind not in VERSIONED_KINDS:
            # A dev edge is dropped from the published manifest, so it does not
            # constrain the publish order the way a normal or build edge does.
            continue
        for key in sorted(entries):
            value = entries[key]
            spec = value if isinstance(value, dict) else {"version": value}
            package = str(spec.get("package", key))
            if package in published:
                needs.add(package)
    return needs


def publish_order(core: Manifest, crates: list[Manifest]) -> list[str]:
    """The thirteen published internals in dependency order, then `gwz-core`.

    Kahn's algorithm over the internal edges of the versioned tables, taking
    the alphabetically first ready crate at every step, so the order is a
    function of the manifests alone and not of directory iteration. `gwz-core`
    is last because it is the composition root that depends on all thirteen;
    `gwz-local-testrepo` is absent because it does not publish (plan D1) and
    reaches nothing but dev-dependency tables. A cycle is an error, not a
    finding: there is no order to print.
    """
    published = {crate.name for crate in crates if crate.name not in UNPUBLISHED}
    remaining = {
        crate.name: internal_requirements(crate, published)
        for crate in crates
        if crate.name in published
    }
    order: list[str] = []
    placed: set[str] = set()
    while remaining:
        ready = sorted(name for name, needs in remaining.items() if needs <= placed)
        if not ready:
            blocked = ", ".join(
                f"{name} needs {', '.join(sorted(needs - placed))}"
                for name, needs in sorted(remaining.items())
            )
            raise GateError(
                f"the internal dependency edges have no publish order: {blocked}; a cycle "
                "cannot be published, since every crate is built against the registry"
            )
        chosen = ready[0]
        order.append(chosen)
        placed.add(chosen)
        del remaining[chosen]
    order.append(core.name)
    return order


def check_core_version(core: Manifest, tag: str | None, findings: list[str]) -> str:
    """gwz-core's product version, and its agreement with the release tag."""
    version = core.table["package"].get("version")
    if not isinstance(version, str) or not PRODUCT_VERSION.match(version):
        findings.append(
            f"{core.name} ({core.where}): version {version!r} is not a plain X.Y.Z or "
            "X.Y.Z-rc.N product version (plan D2)"
        )
        return version if isinstance(version, str) else ""
    if tag is not None and version != tag:
        findings.append(
            f"{core.name} ({core.where}): version {version!r} does not match the release tag's "
            f"version {tag!r}; the publish job refuses to upload a tag that disagrees with the "
            "manifest"
        )
    return version


def check_internal_versions(crates: list[Manifest], findings: list[str]) -> str:
    """The one internal `0.0.N` version the fourteen crates share (plan D2)."""
    versions: dict[str, str] = {}
    for crate in crates:
        version = crate.table["package"].get("version")
        if not isinstance(version, str) or not version:
            findings.append(f"{crate.name} ({crate.where}): [package] has no version")
            continue
        versions[crate.name] = version
    if not versions:
        return ""
    counts = Counter(versions.values())
    # The modal version is the lockstep line, so a finding names the crate that
    # left it rather than the thirteen that did not. Ties resolve by value, so
    # the report is deterministic.
    expected = min(counts, key=lambda version: (-counts[version], version))
    for name in sorted(versions):
        version = versions[name]
        if not INTERNAL_VERSION.match(version):
            findings.append(
                f"{name}: version {version!r} is not on the internal 0.0.N line "
                "(^0\\.0\\.[0-9]+$); the internals carry their own lockstep version, never the "
                "product version (plan D2)"
            )
        elif version != expected:
            findings.append(
                f"{name}: version {version!r} differs from the internal lockstep version "
                f"{expected!r} that the other {counts[expected]} crate(s) carry; the release "
                "script bumps all fourteen together (plan D2)"
            )
    return expected


def check_dependencies(
    manifest: Manifest, internal_version: str, findings: list[str]
) -> tuple[int, int]:
    """Internal edges and git sources of one manifest; returns the edge counts."""
    versioned = 0
    path_only = 0
    for kind, where, entries in dependency_tables(manifest.table):
        for key in sorted(entries):
            value = entries[key]
            spec = value if isinstance(value, dict) else {"version": value}
            if "git" in spec:
                findings.append(
                    f"{manifest.name} ({manifest.where}): {where} {key!r} uses the git source "
                    f"{spec['git']!r}; a published crate may not depend on git, so the "
                    "dependency goes to crates.io first (plan section 1)"
                )
            package = str(spec.get("package", key))
            if not package.startswith(INTERNAL_PREFIX):
                continue
            path = spec.get("path")
            declared = spec.get("version")
            if not isinstance(path, str) or not path:
                findings.append(
                    f"{manifest.name} ({manifest.where}): {where} {key!r} names the internal "
                    f"crate {package!r} without a `path`; a local build must resolve it from "
                    "this checkout, never from the registry"
                )
            if kind in PATH_ONLY_KINDS:
                if declared is not None:
                    findings.append(
                        f"{manifest.name} ({manifest.where}): {where} {key!r} carries "
                        f"version = {declared!r}; an internal dev-dependency stays path-only, "
                        "because cargo drops it from the published manifest and a versioned one "
                        "would have to exist on crates.io"
                    )
                else:
                    path_only += 1
                continue
            if declared is None:
                findings.append(
                    f"{manifest.name} ({manifest.where}): {where} {key!r} has no `version`; "
                    "crates.io refuses a path dependency without one, so every internal edge "
                    f"names the internal version {internal_version!r} (plan D2)"
                )
            elif declared != internal_version:
                findings.append(
                    f"{manifest.name} ({manifest.where}): {where} {key!r} requires "
                    f"version = {declared!r} but the internal lockstep version is "
                    f"{internal_version!r}; the release script bumps every edge with the crates "
                    "(plan D2)"
                )
            else:
                versioned += 1
    return versioned, path_only


def check_publication(crate: Manifest, findings: list[str]) -> bool:
    """The registry metadata of one internal crate; returns True when it publishes."""
    package = crate.table["package"]
    publish = package.get("publish")
    withheld = publish is False or (isinstance(publish, list) and not publish)
    if crate.name in UNPUBLISHED:
        if not withheld:
            findings.append(
                f"{crate.name} ({crate.where}): is the dev-only fixtures crate and must keep "
                "`publish = false`; it is a path-only dev-dependency that cargo drops from the "
                "published package, so nothing downstream could use it (plan D1)"
            )
        return False
    if withheld:
        findings.append(
            f"{crate.name} ({crate.where}): has publish = {publish!r} but is one of the thirteen "
            "published internal crates (plan D1)"
        )
    for field in REQUIRED_METADATA:
        value = package.get(field)
        if not isinstance(value, str) or not value.strip():
            findings.append(
                f"{crate.name} ({crate.where}): [package] has no {field}; every published crate "
                "carries repository, readme, license and description (plan S1.1, D8)"
            )
    return not withheld


def run(root: Path, tag: str | None) -> tuple[list[str], Summary]:
    core, crates = discover(root)
    findings: list[str] = []
    if len(crates) != EXPECTED_CRATES:
        names = ", ".join(sorted(crate.name for crate in crates))
        findings.append(
            f"{CRATES_DIR}/: holds {len(crates)} crate(s) ({names}) but the publish order and "
            f"the release bump are written for {EXPECTED_CRATES}; adding or removing one is a "
            "deliberate change to the plan's section 1 order and to EXPECTED_CRATES here"
        )
    present = {crate.name for crate in crates}
    for name in UNPUBLISHED:
        if name not in present:
            findings.append(
                f"{name}: is named as the unpublished fixtures crate but no manifest under "
                f"{CRATES_DIR}/ declares it; a rename must move the UNPUBLISHED entry with it"
            )

    core_version = check_core_version(core, tag, findings)
    internal_version = check_internal_versions(crates, findings)
    published = 0
    for crate in crates:
        if check_publication(crate, findings):
            published += 1
    versioned = 0
    path_only = 0
    for manifest in [core, *crates]:
        crate_versioned, crate_path_only = check_dependencies(
            manifest, internal_version, findings
        )
        versioned += crate_versioned
        path_only += crate_path_only
    summary = Summary(
        core_version=core_version,
        internal_version=internal_version,
        crates=len(crates),
        published=published,
        unpublished=len(crates) - published,
        versioned_edges=versioned,
        path_only_edges=path_only,
    )
    return findings, summary


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=DEFAULT_ROOT,
        help="the gwz-core checkout holding Cargo.toml and crates/",
    )
    parser.add_argument(
        "--tag",
        help="release tag `vX.Y.Z` or `vX.Y.Z-rc.N`; gwz-core's version must equal its version",
    )
    parser.add_argument(
        "--print-publish-order",
        action="store_true",
        help="print the publish order (the published internals, then gwz-core), one name per "
        "line, and exit without running the gate",
    )
    args = parser.parse_args()
    if args.print_publish_order:
        try:
            core, crates = discover(args.root.resolve())
            names = publish_order(core, crates)
        except GateError as error:
            print(f"crate versions: error: {error}", file=sys.stderr)
            return 2
        for name in names:
            print(name)
        return 0
    tag_version = None
    if args.tag is not None:
        match = RELEASE_TAG.match(args.tag)
        if match is None:
            print(
                f"crate versions: error: tag {args.tag!r} is not `vX.Y.Z` or `vX.Y.Z-rc.N`",
                file=sys.stderr,
            )
            return 2
        tag_version = match.group("version")
    try:
        findings, summary = run(args.root.resolve(), tag_version)
    except GateError as error:
        print(f"crate versions: error: {error}", file=sys.stderr)
        return 2
    if findings:
        print("crate versions: failed", file=sys.stderr)
        for finding in findings:
            print(f"ERROR: {finding}", file=sys.stderr)
        return 1
    print(
        f"crate versions: ok ({CORE_NAME} {summary.core_version}"
        + (f" == tag v{tag_version}" if tag_version else "")
        + f"; {summary.crates} internal crate(s) at {summary.internal_version}, "
        f"{summary.published} published, {summary.unpublished} publish = false; "
        f"{summary.versioned_edges} versioned internal edge(s), "
        f"{summary.path_only_edges} path-only dev edge(s); no git dependency)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

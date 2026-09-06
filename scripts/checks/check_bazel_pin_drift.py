#!/usr/bin/env python3
"""Bazel pin-drift gate: the hand-pinned `@crates` hub must track `//:Cargo.lock`.

Authority: gwz-dev `MODULE.bazel` (workspace layout Option A, operator ruling
2026-09-05; `dev-docs/GwzLocalCloneDesign.md` §11 item 14). gwz-core is its
own Cargo workspace, so `crate.from_cargo` cannot splice the outer virtual
workspace and gwz-cli's direct crates.io dependencies are pinned BY HAND in
`MODULE.bazel` (`crate.spec(..., repositories = ["crates"])`), each at the
exact version `//:Cargo.lock` resolved it to and with the feature set
`gwz-cli/Cargo.toml` requests. Cargo and Bazel agree only while those pins are
kept in step; a comment in `MODULE.bazel` says so, and this gate is the check
the comment could not be.

What the gate compares (three inputs, plus the consumer's BUILD file):

- `MODULE.bazel`: every `crate.spec` scoped to the hub -- package, version,
  features, `default_features` -- and the `crate.from_specs(name = <hub>)`
  declaration. Starlark is a syntactic subset of Python, so the file is read
  with `ast` (no evaluation) and a file `ast` cannot parse is an error.
- `gwz-cli/Cargo.toml`: the direct `[dependencies]` that come from a registry
  (a `path`/`git`/`workspace` entry is not a hub member; a rename resolves to
  its `package`), with each one's `features` and `default-features`.
  `[dev-dependencies]` and `[build-dependencies]` are not hub members: the
  gwz-cli BUILD file declares no `rust_test`, so the hub mirrors exactly what
  the `rust_library` consumes.
- `Cargo.lock` (the OUTER workspace's, at the gwz-dev root): the version each
  direct dependency resolved to, read off the cli package's own dependency
  edges (`name` or `name version` when the lock holds two versions of it).
- `gwz-cli/BUILD.bazel` (optional): the `@<hub>//:<name>` labels the crate
  consumes must be exactly the pinned set.

Refuses (exit 1) when a spec's version is not the lock's, when it is not an
exact `=x.y.z` pin, when a feature set or `default_features` differs from the
manifest, when a direct registry dependency has no spec (added), when a spec
names no direct dependency (removed), when a spec is unscoped or shared with
another hub (it would inject itself into gwz-core's splice), and when the
BUILD labels drift from the spec set. A missing or unparseable input is an
error (exit 2), never a pass.

Home: this is a WORKSPACE-ROOT gate. Two of its three inputs are root-repo
files and the third is gwz-cli's, so it resolves the gwz-dev root from this
script's location exactly as `check_merge_docs.py` does (`parents[3]`) and
cannot run on a single-repo gwz-core checkout -- its unit tests can, from
synthetic fixtures. See the LCM1.0c checkpoint §15 for where it should move.

Usage (from gwz-core, inside the gwz-dev workspace):
    python3 scripts/checks/check_bazel_pin_drift.py
    python3 scripts/checks/check_bazel_pin_drift.py --workspace-root <gwz-dev>
    python3 scripts/checks/check_bazel_pin_drift.py --module-bazel M --cargo-lock L \\
        --cli-manifest T [--cli-build B | --no-cli-build] [--hub crates]
"""

from __future__ import annotations

import argparse
import ast
import re
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path


DEFAULT_WORKSPACE_ROOT = Path(__file__).resolve().parents[3]
DEFAULT_HUB = "crates"
MODULE_BAZEL = "MODULE.bazel"
CARGO_LOCK = "Cargo.lock"
CLI_DIR = "gwz-cli"
CLI_MANIFEST = "Cargo.toml"
CLI_BUILD = "BUILD.bazel"
EXACT_PIN = re.compile(r"^=(\d+)\.(\d+)\.(\d+)(?:[-+][0-9A-Za-z.-]+)?$")


class GateError(Exception):
    """An input is missing or cannot be read at all."""


@dataclass(frozen=True)
class Spec:
    """One `crate.spec(...)` in `MODULE.bazel`."""

    package: str
    version: str
    features: tuple[str, ...]
    default_features: bool
    repositories: tuple[str, ...] | None
    line: int


@dataclass(frozen=True)
class DirectDependency:
    """One registry entry of the cli manifest's `[dependencies]`."""

    package: str
    requirement: str
    features: tuple[str, ...]
    default_features: bool


def _read(path: Path, what: str) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError as error:
        raise GateError(f"cannot read {what} {path}: {error}") from error


def _literal(node: ast.AST, context: str) -> object:
    try:
        return ast.literal_eval(node)
    except ValueError as error:
        raise GateError(f"{context}: argument is not a literal ({error})") from error


def _call_target(node: ast.Call) -> tuple[str, str] | None:
    """`<receiver>.<method>` of a call, or `None` for a bare call."""
    if isinstance(node.func, ast.Attribute) and isinstance(node.func.value, ast.Name):
        return node.func.value.id, node.func.attr
    return None


def parse_module_bazel(text: str, path: Path) -> tuple[list[Spec], set[str]]:
    """Every `crate.spec` and the names of every `crate.from_specs` hub."""
    try:
        tree = ast.parse(text, filename=str(path))
    except SyntaxError as error:
        raise GateError(f"{path} is not Python-parseable Starlark: {error}") from error
    specs: list[Spec] = []
    hubs: set[str] = set()
    for node in ast.walk(tree):
        if not isinstance(node, ast.Call):
            continue
        target = _call_target(node)
        if target is None or target[0] != "crate":
            continue
        keywords = {keyword.arg: keyword.value for keyword in node.keywords if keyword.arg}
        context = f"{path}:{node.lineno} crate.{target[1]}"
        if target[1] == "from_specs":
            name = keywords.get("name")
            if name is None:
                raise GateError(f"{context}: needs `name`")
            hubs.add(str(_literal(name, context)))
        elif target[1] == "spec":
            package = keywords.get("package")
            version = keywords.get("version")
            if package is None or version is None:
                raise GateError(f"{context}: needs `package` and `version`")
            features = keywords.get("features")
            default_features = keywords.get("default_features")
            repositories = keywords.get("repositories")
            specs.append(
                Spec(
                    package=str(_literal(package, context)),
                    version=str(_literal(version, context)),
                    features=tuple(
                        str(feature) for feature in (_literal(features, context) if features else [])
                    ),
                    default_features=(
                        bool(_literal(default_features, context)) if default_features else True
                    ),
                    repositories=(
                        tuple(str(repo) for repo in _literal(repositories, context))
                        if repositories is not None
                        else None
                    ),
                    line=node.lineno,
                )
            )
    return specs, hubs


def parse_cli_manifest(text: str, path: Path) -> tuple[str, list[DirectDependency]]:
    """The cli package name and its direct registry dependencies."""
    try:
        table = tomllib.loads(text)
    except tomllib.TOMLDecodeError as error:
        raise GateError(f"cannot parse {path}: {error}") from error
    name = table.get("package", {}).get("name")
    if not isinstance(name, str) or not name:
        raise GateError(f"{path}: no [package] name")
    dependencies: list[DirectDependency] = []
    for key, value in table.get("dependencies", {}).items():
        if isinstance(value, str):
            dependencies.append(DirectDependency(key, value, (), True))
            continue
        if not isinstance(value, dict):
            raise GateError(f"{path}: dependency {key!r} has an unrecognised shape")
        if "path" in value or "git" in value or value.get("workspace"):
            continue  # not a crates.io dependency: not a hub member
        package = str(value.get("package", key))
        requirement = value.get("version")
        if not isinstance(requirement, str):
            raise GateError(f"{path}: registry dependency {key!r} has no version requirement")
        dependencies.append(
            DirectDependency(
                package=package,
                requirement=requirement,
                features=tuple(str(feature) for feature in value.get("features", [])),
                default_features=bool(value.get("default-features", True)),
            )
        )
    return name, dependencies


def parse_cargo_lock(text: str, path: Path) -> dict:
    try:
        lock = tomllib.loads(text)
    except tomllib.TOMLDecodeError as error:
        raise GateError(f"cannot parse {path}: {error}") from error
    if not isinstance(lock.get("package"), list):
        raise GateError(f"{path}: no [[package]] entries")
    return lock


def resolved_versions(lock: dict, root_package: str, path: Path) -> dict[str, str]:
    """`{dependency name: version}` for every edge of `root_package` in the lock.

    A lock edge is `name` when the graph holds one version of it and
    `name version` (optionally `name version (source)`) when it holds more,
    so the version is read from the edge when spelled and from the unique
    package otherwise; an edge that names no package or an ambiguous one is
    an error, not a guess.
    """
    by_name: dict[str, list[dict]] = {}
    for package in lock["package"]:
        by_name.setdefault(str(package.get("name")), []).append(package)
    roots = by_name.get(root_package, [])
    if len(roots) != 1:
        raise GateError(
            f"{path}: expected exactly one [[package]] named {root_package!r}, found {len(roots)}"
        )
    versions: dict[str, str] = {}
    for edge in roots[0].get("dependencies", []):
        parts = str(edge).split()
        name = parts[0]
        if len(parts) >= 2:
            versions[name] = parts[1]
            continue
        candidates = by_name.get(name, [])
        if len(candidates) != 1:
            raise GateError(
                f"{path}: edge {edge!r} of {root_package!r} resolves to {len(candidates)} "
                "packages; the lock is inconsistent"
            )
        versions[name] = str(candidates[0].get("version"))
    return versions


def build_labels(text: str, hub: str) -> set[str]:
    """Every `@<hub>//:<name>` label in a BUILD file."""
    return set(re.findall(rf'"@{re.escape(hub)}//:([A-Za-z0-9_.-]+)"', text))


def compare(
    specs: list[Spec],
    hubs: set[str],
    hub: str,
    dependencies: list[DirectDependency],
    versions: dict[str, str],
    labels: set[str] | None,
) -> tuple[list[str], list[str]]:
    findings: list[str] = []
    notes: list[str] = []
    if hub not in hubs:
        raise GateError(f"MODULE.bazel declares no `crate.from_specs(name = \"{hub}\")` hub")

    pinned: dict[str, Spec] = {}
    for spec in specs:
        if spec.repositories is None:
            findings.append(
                f"MODULE.bazel:{spec.line}: crate.spec {spec.package!r} is unscoped "
                "(no `repositories`); it would apply to every hub and inject itself into "
                "gwz-core's splice"
            )
            # It applies to this hub too, so it is compared like a scoped pin.
            pinned.setdefault(spec.package, spec)
            continue
        if hub not in spec.repositories:
            continue  # another hub's spec
        if len(spec.repositories) != 1:
            findings.append(
                f"MODULE.bazel:{spec.line}: crate.spec {spec.package!r} is shared with "
                f"{sorted(set(spec.repositories) - {hub})}; a `{hub}` pin must be scoped to "
                f"`{hub}` alone"
            )
        if spec.package in pinned:
            findings.append(
                f"MODULE.bazel:{spec.line}: crate.spec {spec.package!r} is pinned twice for "
                f"hub `{hub}`"
            )
            continue
        pinned[spec.package] = spec

    direct: dict[str, DirectDependency] = {}
    for dependency in dependencies:
        if dependency.package in direct:
            findings.append(
                f"gwz-cli/Cargo.toml: {dependency.package!r} is declared twice as a direct "
                "dependency"
            )
        direct[dependency.package] = dependency

    for name in sorted(set(direct) - set(pinned)):
        findings.append(
            f"gwz-cli/Cargo.toml: direct dependency {name!r} has no crate.spec in hub "
            f"`{hub}` (added dependency; pin it in MODULE.bazel at the Cargo.lock version)"
        )
    for name in sorted(set(pinned) - set(direct)):
        findings.append(
            f"MODULE.bazel:{pinned[name].line}: crate.spec {name!r} is not a direct registry "
            "dependency of gwz-cli/Cargo.toml (removed dependency; drop the pin)"
        )

    for name in sorted(set(direct) & set(pinned)):
        spec = pinned[name]
        dependency = direct[name]
        locked = versions.get(name)
        if locked is None:
            findings.append(
                f"Cargo.lock: no edge from the cli package to {name!r}; the lock does not "
                "describe the manifest (`cargo --locked` would refuse)"
            )
        match = EXACT_PIN.match(spec.version)
        if match is None:
            findings.append(
                f"MODULE.bazel:{spec.line}: crate.spec {name!r} version {spec.version!r} is "
                "not an exact `=x.y.z` pin"
            )
        elif locked is not None and spec.version[1:] != locked:
            findings.append(
                f"MODULE.bazel:{spec.line}: crate.spec {name!r} pins {spec.version[1:]} but "
                f"Cargo.lock resolved {locked} (version drift)"
            )
        if sorted(set(spec.features)) != sorted(set(dependency.features)):
            findings.append(
                f"MODULE.bazel:{spec.line}: crate.spec {name!r} features "
                f"{sorted(set(spec.features))} differ from gwz-cli/Cargo.toml's "
                f"{sorted(set(dependency.features))} (feature drift)"
            )
        if spec.default_features != dependency.default_features:
            findings.append(
                f"MODULE.bazel:{spec.line}: crate.spec {name!r} default_features="
                f"{spec.default_features} but gwz-cli/Cargo.toml says "
                f"default-features={dependency.default_features}"
            )

    if labels is not None:
        for name in sorted(labels - set(pinned)):
            findings.append(
                f"gwz-cli/BUILD.bazel: consumes `@{hub}//:{name}` but hub `{hub}` pins no "
                f"crate.spec {name!r}"
            )
        for name in sorted(set(pinned) - labels):
            findings.append(
                f"gwz-cli/BUILD.bazel: does not consume `@{hub}//:{name}` although hub "
                f"`{hub}` pins it (stale pin, or a missing dep label)"
            )

    for name in sorted(pinned):
        spec = pinned[name]
        features = ",".join(spec.features) or "-"
        notes.append(
            f"{name} {spec.version} features [{features}] default_features={spec.default_features} "
            f"<-> lock {versions.get(name, '?')}"
        )
    notes.append(
        f"{len(pinned)} pin(s) in hub `{hub}` compared with {len(direct)} direct registry "
        f"dependenc{'y' if len(direct) == 1 else 'ies'} of gwz-cli/Cargo.toml"
        + ("" if labels is None else f" and {len(labels)} BUILD label(s)")
    )
    return findings, notes


def run(
    module_bazel: Path,
    cargo_lock: Path,
    cli_manifest: Path,
    cli_build: Path | None,
    hub: str,
) -> tuple[list[str], list[str]]:
    specs, hubs = parse_module_bazel(_read(module_bazel, "MODULE.bazel"), module_bazel)
    root_package, dependencies = parse_cli_manifest(
        _read(cli_manifest, "gwz-cli/Cargo.toml"), cli_manifest
    )
    lock = parse_cargo_lock(_read(cargo_lock, "Cargo.lock"), cargo_lock)
    versions = resolved_versions(lock, root_package, cargo_lock)
    labels = None if cli_build is None else build_labels(_read(cli_build, "gwz-cli/BUILD.bazel"), hub)
    findings, notes = compare(specs, hubs, hub, dependencies, versions, labels)
    notes.append(
        f"Cargo.lock version {lock.get('version')} with {len(lock['package'])} packages; cli "
        f"package {root_package!r}"
    )
    return findings, notes


def main() -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "--workspace-root",
        type=Path,
        default=DEFAULT_WORKSPACE_ROOT,
        help="the gwz-dev workspace root holding MODULE.bazel, Cargo.lock and gwz-cli/",
    )
    parser.add_argument("--module-bazel", type=Path, help="override <root>/MODULE.bazel")
    parser.add_argument("--cargo-lock", type=Path, help="override <root>/Cargo.lock")
    parser.add_argument("--cli-manifest", type=Path, help="override <root>/gwz-cli/Cargo.toml")
    parser.add_argument("--cli-build", type=Path, help="override <root>/gwz-cli/BUILD.bazel")
    parser.add_argument(
        "--no-cli-build",
        action="store_true",
        help="do not compare the BUILD file's `@<hub>//:` labels",
    )
    parser.add_argument("--hub", default=DEFAULT_HUB, help="the from_specs hub name")
    args = parser.parse_args()
    root = args.workspace_root.resolve()
    module_bazel = (args.module_bazel or root / MODULE_BAZEL).resolve()
    cargo_lock = (args.cargo_lock or root / CARGO_LOCK).resolve()
    cli_manifest = (args.cli_manifest or root / CLI_DIR / CLI_MANIFEST).resolve()
    cli_build = None if args.no_cli_build else (args.cli_build or root / CLI_DIR / CLI_BUILD).resolve()
    try:
        findings, notes = run(module_bazel, cargo_lock, cli_manifest, cli_build, args.hub)
    except GateError as error:
        print(f"bazel pin drift: error: {error}", file=sys.stderr)
        return 2
    for note in notes:
        print(f"  note: {note}")
    if findings:
        print("bazel pin drift: failed", file=sys.stderr)
        for finding in findings:
            print(f"- {finding}", file=sys.stderr)
        return 1
    print("bazel pin drift: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

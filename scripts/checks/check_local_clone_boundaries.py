#!/usr/bin/env python3
"""Architecture gate for the local clone family libraries under `crates/`.

Authority: gwz-dev `dev-docs/GwzLocalCloneLibraryBoundaries.md` §2/§6
(revision 1), adopting LBT-001..LBT-012. The inventory beside this script,
`local_clone_inventory.json`, is the machine-readable classification: every
package's role, owner, rationale and complete allowed dependency edges.

What the gate checks (all through Cargo metadata, no build):

- LBT-001: every package directory under `crates/` is classified; a classified
  package that must be present exists, its Cargo name equals its inventory
  key, it is `publish = false`, carries explicit `edition`/`rust-version`
  (no workspace inheritance) and is not a workspace root of its own.
- LBT-003/004/005: every DECLARED dependency edge -- normal, build, dev,
  optional and target-specific, with renames resolved to the real package
  name -- is inside the package's allowlist. First-party edges must be in
  `first_party` (dev edges in `dev_first_party`); third-party edges must be in
  `third_party` (`dev_third_party` for dev). Nothing may depend on a
  forbidden package (`gwz-core`, the drivers, generated-protocol or
  checked-artifact crates) in any dependency kind.
- Role direction: a package may only depend on roles its role admits
  (`role_edges`); a harness is dev-only everywhere.
- Test closure: the transitive closure of first-party edges reachable from a
  package's `--lib` test build (dev edges at the root, normal/build edges
  below) never reaches a forbidden package.
- Each package has one `lib` target with tests enabled, so
  `cargo test -p <name> --lib` is a real fast command.

Coverage limits, stated honestly: this is a declared-edge gate. It does not
audit the transitive third-party graph, expand macros, parse Rust, prove trait
implementations or detect public type leakage; conformance suites and
compiler witnesses in the crates do the behavioral half, and API review does
the rest (policy §5). It uses `cargo metadata --no-deps --manifest-path` per
package, which reads manifests and resolves path dependencies without a
lockfile, network or build.

Usage (from gwz-core):
    python3 scripts/checks/check_local_clone_boundaries.py
    python3 scripts/checks/check_local_clone_boundaries.py --core <path> --inventory <json>
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tomllib
from dataclasses import dataclass, field
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_INVENTORY = Path(__file__).with_name("local_clone_inventory.json")
FIRST_PARTY_PREFIX = "gwz-"
ROLES = ("contract", "pure", "implementation", "integration", "harness")


class GateError(Exception):
    """The inventory or the tree cannot be inspected at all."""


@dataclass(frozen=True)
class Dependency:
    name: str
    rename: str | None
    kind: str  # normal | dev | build
    optional: bool
    target: str | None
    path: Path | None

    def describe(self) -> str:
        bits = [self.kind]
        if self.optional:
            bits.append("optional")
        if self.target:
            bits.append(f"target={self.target}")
        alias = f" (as `{self.rename}`)" if self.rename else ""
        return f"{self.name}{alias} [{', '.join(bits)}]"


@dataclass
class Package:
    name: str
    manifest_path: Path
    dependencies: list[Dependency]
    lib_target_tested: bool
    publish: object
    edition: object
    rust_version: object
    has_workspace_table: bool
    findings: list[str] = field(default_factory=list)


def load_inventory(path: Path) -> dict:
    try:
        inventory = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise GateError(f"cannot read inventory {path}: {error}") from error
    if inventory.get("inventory_version") != 1:
        raise GateError("inventory_version must be 1")
    packages = inventory.get("packages")
    if not isinstance(packages, dict) or not packages:
        raise GateError("inventory must classify at least one package")
    for name, entry in packages.items():
        for key in ("directory", "role", "owner", "rationale", "first_party", "third_party",
                    "dev_first_party", "dev_third_party", "expected"):
            if key not in entry:
                raise GateError(f"inventory entry {name} lacks `{key}`")
        if entry["role"] not in ROLES:
            raise GateError(f"inventory entry {name} has unknown role {entry['role']!r}")
        if entry["expected"] not in ("present", "pending"):
            raise GateError(f"inventory entry {name}: expected must be present|pending")
        if not str(entry["rationale"]).strip():
            raise GateError(f"inventory entry {name} needs a non-empty rationale (LBT-001)")
    for role, targets in inventory.get("role_edges", {}).items():
        if role not in ROLES or any(target not in ROLES for target in targets):
            raise GateError(f"role_edges names an unknown role: {role} -> {targets}")
    return inventory


def cargo_metadata(manifest: Path) -> dict:
    command = [
        "cargo",
        "metadata",
        "--no-deps",
        "--format-version",
        "1",
        "--manifest-path",
        str(manifest),
    ]
    result = subprocess.run(command, capture_output=True, text=True, check=False)
    if result.returncode != 0:
        raise GateError(f"cargo metadata failed for {manifest}:\n{result.stderr.strip()}")
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise GateError(f"cargo metadata returned invalid JSON for {manifest}: {error}") from error


def read_package(manifest: Path) -> Package:
    metadata = cargo_metadata(manifest)
    resolved = manifest.resolve()
    entries = [
        package
        for package in metadata["packages"]
        if Path(package["manifest_path"]).resolve() == resolved
    ]
    if len(entries) != 1:
        raise GateError(f"cargo metadata did not report exactly one package for {manifest}")
    entry = entries[0]
    dependencies = [
        Dependency(
            name=dependency["name"],
            rename=dependency.get("rename"),
            kind=dependency.get("kind") or "normal",
            optional=bool(dependency.get("optional")),
            target=dependency.get("target"),
            path=Path(dependency["path"]).resolve() if dependency.get("path") else None,
        )
        for dependency in entry["dependencies"]
    ]
    lib_tested = any(
        "lib" in target["kind"] and target.get("test", True) for target in entry["targets"]
    )
    try:
        manifest_table = tomllib.loads(manifest.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise GateError(f"cannot parse {manifest}: {error}") from error
    package_table = manifest_table.get("package", {})
    return Package(
        name=entry["name"],
        manifest_path=resolved,
        dependencies=dependencies,
        lib_target_tested=lib_tested,
        publish=package_table.get("publish", True),
        edition=package_table.get("edition"),
        rust_version=package_table.get("rust-version"),
        has_workspace_table="workspace" in manifest_table,
    )


def is_first_party(name: str) -> bool:
    return name.startswith(FIRST_PARTY_PREFIX)


def check_package(
    package: Package,
    entry: dict,
    inventory: dict,
    inventory_dirs: dict[Path, str],
) -> list[str]:
    findings: list[str] = []
    role = entry["role"]
    forbidden = set(inventory.get("forbidden_dependencies", []))
    packages = inventory["packages"]
    role_edges = inventory.get("role_edges", {})

    if package.publish is not False:
        findings.append(f"{package.name}: must set `publish = false` (private path crate)")
    if not isinstance(package.edition, str):
        findings.append(f"{package.name}: needs an explicit `edition` (no workspace inheritance)")
    if not isinstance(package.rust_version, str):
        findings.append(f"{package.name}: needs an explicit `rust-version` (no workspace inheritance)")
    if package.has_workspace_table:
        findings.append(f"{package.name}: must not declare a `[workspace]` table")
    if not package.lib_target_tested:
        findings.append(f"{package.name}: needs a `lib` target with tests enabled (`cargo test -p {package.name} --lib`)")

    for dependency in package.dependencies:
        label = f"{package.name}: {dependency.describe()}"
        if dependency.name in forbidden:
            findings.append(f"{label}: forbidden dependency")
            continue
        dev = dependency.kind == "dev"
        if is_first_party(dependency.name):
            allowed = set(entry["dev_first_party"] if dev else entry["first_party"])
            if dependency.name not in allowed:
                findings.append(f"{label}: first-party edge is not in the inventory allowlist")
                continue
            target_entry = packages.get(dependency.name)
            if target_entry is None:
                findings.append(f"{label}: first-party dependency is not a classified package")
                continue
            if dependency.path is not None and dependency.path.resolve() not in inventory_dirs:
                findings.append(f"{label}: path does not point at the classified crate directory")
            elif dependency.path is not None and inventory_dirs[dependency.path.resolve()] != dependency.name:
                findings.append(
                    f"{label}: path points at {inventory_dirs[dependency.path.resolve()]}, "
                    "a different package (aliased edge)"
                )
            target_role = target_entry["role"]
            if target_role == "harness" and not dev:
                findings.append(f"{label}: a harness may only be a dev-dependency")
            elif not dev and target_role not in role_edges.get(role, []):
                findings.append(f"{label}: role {role} may not depend on role {target_role}")
        else:
            allowed = set(entry["dev_third_party"] if dev else entry["third_party"])
            if dependency.name not in allowed:
                findings.append(f"{label}: third-party edge is not in the inventory allowlist")
    return findings


def test_closure(
    name: str,
    packages: dict[str, Package],
    inventory: dict,
) -> tuple[set[str], list[str]]:
    """First-party names reachable from `name`'s `--lib` test build."""
    forbidden = set(inventory.get("forbidden_dependencies", []))
    reached: set[str] = set()
    findings: list[str] = []
    frontier = [(name, True)]
    while frontier:
        current, at_root = frontier.pop()
        package = packages.get(current)
        if package is None:
            continue
        for dependency in package.dependencies:
            if dependency.kind == "dev" and not at_root:
                continue
            if dependency.name in forbidden:
                findings.append(
                    f"{name}: test closure reaches forbidden package {dependency.name} via {current}"
                )
                continue
            if not is_first_party(dependency.name):
                continue
            if dependency.name not in reached:
                reached.add(dependency.name)
                frontier.append((dependency.name, False))
    return reached, findings


TIER_A_COMMAND = "cargo test"
MANIFEST_FLAG = "--manifest-path"
LOCKED_FLAG = "--locked"
CONTINUATION = re.compile(r"\\\r?\n[ \t]*")


def tier_a_commands(text: str) -> list[str]:
    """Every `cargo test ... --manifest-path ...` command in a workflow text.

    Comment lines are dropped and `\\`-newline continuations joined first, so
    a command wrapped across lines (as the Tier A loop's own `for` header
    already is) is matched whole rather than missed (State S2-P3-2).
    """
    kept = [line for line in text.splitlines() if not line.lstrip().startswith("#")]
    joined = CONTINUATION.sub(" ", "\n".join(kept))
    return [
        line for line in joined.splitlines() if TIER_A_COMMAND in line and MANIFEST_FLAG in line
    ]


def tier_a_unlocked(core: Path, inventory: dict) -> bool:
    """Whether CI may run a library's Tier A command WITHOUT `--locked`.

    LCM1.0c-rem1 (State P3-3): an unlocked `--manifest-path` build resolves a
    library's third-party dependencies fresh, so once any crate declares one
    (e.g. lane I's `git2`) CI may resolve a different version from the product
    lock -- a false green. This answer arms the guard in `run`, so it fails
    toward "unlocked" (LCM1.0c-fu1, State S2-P3-2): `False` (locked) needs
    affirmative evidence -- the explicit `ci_tier_a_unlocked: false` inventory
    flag, or `--locked` on EVERY `cargo test ... --manifest-path` command in
    every `.github/workflows/*.yml`/`*.yaml`. Anything the function cannot
    establish counts as unlocked: no workflow at all while `crates_dir` exists,
    a workflow it cannot read, workflows with no recognisable Tier A command,
    or one such command anywhere without the flag. No workflow AND no
    `crates_dir` is the only "nothing to build" answer.
    """
    flag = inventory.get("ci_tier_a_unlocked")
    if isinstance(flag, bool):
        return flag
    workflows_dir = core / ".github" / "workflows"
    workflows = sorted(
        path for path in workflows_dir.glob("*.y*ml") if path.suffix in (".yml", ".yaml")
    ) if workflows_dir.is_dir() else []
    if not workflows:
        return (core / inventory.get("crates_dir", "crates")).is_dir()
    commands: list[str] = []
    for workflow in workflows:
        try:
            commands.extend(tier_a_commands(workflow.read_text(encoding="utf-8")))
        except OSError:
            return True
    if not commands:
        return True
    return any(LOCKED_FLAG not in command for command in commands)


def run(core: Path, inventory_path: Path) -> tuple[list[str], list[str]]:
    inventory = load_inventory(inventory_path)
    crates_dir = core / inventory.get("crates_dir", "crates")
    entries = inventory["packages"]
    notes: list[str] = []
    findings: list[str] = []

    inventory_dirs = {
        (crates_dir / entry["directory"]).resolve(): name for name, entry in entries.items()
    }
    present_dirs = sorted(
        path for path in crates_dir.iterdir() if path.is_dir() and (path / "Cargo.toml").exists()
    ) if crates_dir.is_dir() else []
    for directory in present_dirs:
        if directory.resolve() not in inventory_dirs:
            findings.append(f"{directory.relative_to(core)}: unclassified crate (LBT-001)")
    for path in (crates_dir.iterdir() if crates_dir.is_dir() else []):
        if path.is_dir() and not (path / "Cargo.toml").exists() and path.resolve() not in inventory_dirs:
            findings.append(f"{path.relative_to(core)}: directory without a manifest under crates/")

    packages: dict[str, Package] = {}
    for name, entry in entries.items():
        manifest = crates_dir / entry["directory"] / "Cargo.toml"
        if not manifest.exists():
            if entry["expected"] == "present":
                findings.append(f"{name}: expected present at {manifest.relative_to(core)} but absent")
            else:
                notes.append(f"{name}: pending (lane {entry['owner']}); not present yet")
            continue
        try:
            package = read_package(manifest)
        except GateError as error:
            findings.append(str(error))
            continue
        if package.name != name:
            findings.append(
                f"{manifest.relative_to(core)}: Cargo package name {package.name!r} differs "
                f"from inventory key {name!r}"
            )
            continue
        packages[name] = package

    for name, package in packages.items():
        findings.extend(check_package(package, entries[name], inventory, inventory_dirs))
    for name in packages:
        reached, closure_findings = test_closure(name, packages, inventory)
        findings.extend(closure_findings)
        unclassified = sorted(dep for dep in reached if dep not in entries)
        if unclassified:
            findings.append(f"{name}: test closure reaches unclassified packages {unclassified}")

    # LBT-012 fail-closed guard (LCM1.0c-rem1, State P3-3). A declared
    # third-party edge resolved by an unlocked Tier A CI step may diverge from
    # the product lock. Refuse the combination until the retirement condition
    # holds (record §7.6: commit per-crate locks and restore `--locked`, or
    # land the nested-workspace layout so the product resolution is measured).
    forbidden = set(inventory.get("forbidden_dependencies", []))
    declared_third_party = sorted(
        (name, dependency.name)
        for name, package in packages.items()
        for dependency in package.dependencies
        if not is_first_party(dependency.name) and dependency.name not in forbidden
    )
    if declared_third_party and tier_a_unlocked(core, inventory):
        for name, dependency in declared_third_party:
            findings.append(
                f"{name}: declares third-party dependency {dependency!r} while the CI Tier A "
                "step runs unlocked (LBT-012, State P3-3); commit per-crate locks and restore "
                "`--locked`, or land the nested-workspace layout, before a crate declares a "
                "third-party dependency"
            )
    elif declared_third_party:
        notes.append(
            f"{len(declared_third_party)} declared third-party edge(s) with a locked Tier A step"
        )

    notes.append(
        f"{len(packages)} classified package(s) inspected, "
        f"{sum(len(p.dependencies) for p in packages.values())} declared edge(s) checked "
        "(normal/build/dev/optional/target, renames resolved)"
    )
    notes.append(
        "coverage: declared Cargo edges and test closures only; no transitive third-party audit, "
        "macro expansion, Rust parsing, trait proof or public-type leakage analysis"
    )
    return findings, notes


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--core", type=Path, default=ROOT, help="gwz-core checkout root")
    parser.add_argument("--inventory", type=Path, default=DEFAULT_INVENTORY)
    args = parser.parse_args()
    try:
        findings, notes = run(args.core.resolve(), args.inventory.resolve())
    except GateError as error:
        print(f"local-clone boundary: error: {error}", file=sys.stderr)
        return 2
    for note in notes:
        print(f"  note: {note}")
    if findings:
        print("local-clone boundary: failed", file=sys.stderr)
        for finding in findings:
            print(f"- {finding}", file=sys.stderr)
        return 1
    print("local-clone boundary: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

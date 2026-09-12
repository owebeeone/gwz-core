#!/usr/bin/env python3
"""Cut a gwz-core release off ``main``, per RELEASE.md.

gwz-core has no release branch — tags are cut directly on ``main``. This script
automates RELEASE.md steps 1-4 for a given tag:

  1. Gate the tree: protocol regeneration, formatting, the crate-version lockstep
     check, the structural checked-artifact source boundary scan, tests, and Clippy.
     Compiler-mutation suites are manual-only.
  2. Bump ``version`` in ``Cargo.toml``, advance the internal ``0.0.N`` line across
     the fourteen crates under ``crates/`` and every internal dependency edge
     (dev-docs/GwzCratesIoPlan.md D2), and refresh ``Cargo.lock`` via
     ``cargo generate-lockfile``.
  3. Commit on ``main``: ``chore(release): gwz-core X.Y.Z``.
  4. On that exact commit, re-run the lockstep check against the tag and package
     every publishable crate, then tag it ``vX.Y.Z`` (lightweight). An existing tag
     is NEVER moved — if ``vX.Y.Z`` already points elsewhere the script aborts.
     Publishing to crates.io happens in CI, never here (plan D5).

Requires a clean working tree (land feature work first). The commit is skipped when
``Cargo.toml`` already carries the target version. Re-running after a successful release
is an idempotent no-op (and will create the tag if a prior run stopped before tagging).
Pushing is left to you unless ``--push`` is given.

When gwz-core is checked out inside the gwz-dev umbrella workspace, ``cargo`` commands
run in a temporary detached worktree under ``/tmp`` so they use gwz-core's own
``Cargo.lock`` (not ``../Cargo.lock``). CI and standalone checkouts are unaffected.

This operates on your LOCAL ``main`` ref and does not fetch; it warns if ``main`` is
behind its upstream. Pull first if you want the latest.

Usage:
    python scripts/release.py vX.Y.Z              # verify + bump + commit + tag (no push)
    python scripts/release.py vX.Y.Z --push       # also push main + tag to origin
    python scripts/release.py vX.Y.Z --no-test      # skip full `cargo test` only
    python scripts/release.py vX.Y.Z --skip-regen-check
    python scripts/release.py vX.Y.Z --keep-worktree
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

# scripts/release.py -> the gwz-core repo root is one level up.
REPO = Path(__file__).resolve().parent.parent
REGEN = REPO / "protocol" / "regen.py"
REGEN_VENV = REPO / "protocol" / ".regen-venv"
CHECKED_ARTIFACT_BOUNDARY = Path("scripts/checks/check_checked_artifact_boundaries.py")
CRATE_VERSIONS = Path("scripts/checks/check_crate_versions.py")
CRATES_DIR = "crates"
MANIFEST = "Cargo.toml"
# The internal crates share their own lockstep line, `0.0.N`, bumped by one at
# every gwz-core release while gwz-core carries the product version
# (dev-docs/GwzCratesIoPlan.md D2). Every `0.0.x` version is semver-incompatible
# with every other, so a caret edge on the line resolves exactly one version and
# the number itself tells a reader the crate is not a supported API.
INTERNAL_VERSION = re.compile(r"^0\.0\.([0-9]+)$")
# A dependency table whose entries survive into the published manifest. A
# `[dev-dependencies]` edge does not: cargo drops a path-only dev edge, so the
# bump must leave those alone or it would demand a version that never publishes.
VERSIONED_KINDS = ("dependencies", "build-dependencies")
TABLE_HEADER = re.compile(r"^\s*\[([^\]]+)\]")


def fail(msg: str):
    print(f"release: error: {msg}", file=sys.stderr)
    raise SystemExit(1)


def log(msg: str):
    print(f"release: {msg}")


def run(cmd, *, cwd=None, capture=False, check=True, env=None) -> subprocess.CompletedProcess:
    printable = " ".join(str(c) for c in cmd)
    log(f"$ {printable}")
    result = subprocess.run(
        [str(c) for c in cmd],
        cwd=str(cwd) if cwd is not None else None,
        capture_output=capture,
        text=True,
        env=env,
    )
    if check and result.returncode != 0:
        if capture and result.stderr:
            print(result.stderr, file=sys.stderr)
        fail(f"command failed ({result.returncode}): {printable}")
    return result


def regen_python() -> Path:
    """Python from protocol/.regen-venv (PyPI taut-proto), matching CI's taut install."""
    suffix = ".exe" if os.name == "nt" else ""
    bindir = "Scripts" if os.name == "nt" else "bin"
    py = REGEN_VENV / bindir / f"python{suffix}"
    if not py.is_file():
        fail(
            "protocol/.regen-venv not found -- run `python protocol/regen.py --check` "
            "(or drop --skip-regen-check) so taut-proto is available for cargo tests"
        )
    return py


def cargo_env() -> dict[str, str]:
    """Env for cargo test/clippy: TAUT_PYTHON must see taut-proto (see tests/protocol.rs)."""
    env = os.environ.copy()
    env["TAUT_PYTHON"] = str(regen_python())
    return env


def run_fmt_check(*, cargo_root: Path):
    result = run(["cargo", "fmt", "--check"], cwd=cargo_root, check=False)
    if result.returncode != 0:
        print(
            "\nrelease: rustfmt check failed. Run this from the gwz-core repo root, "
            "then stage the resulting formatting changes:\n"
            "  cargo fmt\n",
            file=sys.stderr,
        )
        fail(f"command failed ({result.returncode}): cargo fmt --check")


def parent_cargo_workspace_root(start: Path) -> Path | None:
    """Return the nearest ancestor Cargo workspace root, if any."""
    for directory in (start, *start.parents):
        manifest = directory / "Cargo.toml"
        if not manifest.is_file():
            continue
        text = manifest.read_text(encoding="utf-8")
        if re.search(r"^\[workspace\]", text, flags=re.M):
            return directory
    return None


def make_standalone_worktree(label: str) -> Path:
    """Checkout HEAD in /tmp so cargo does not join a parent gwz-dev workspace."""
    git(["worktree", "prune"], check=False)
    base = Path(tempfile.gettempdir()) / f"gwz-core-{label}-{os.getpid()}"
    path = base / "gwz-core"
    if base.exists():
        fail(f"standalone worktree path already exists: {base}")
    base.mkdir(parents=True)
    head = git(["rev-parse", "HEAD"], capture=True).stdout.strip()
    git(["worktree", "add", "--detach", path, head])
    log(f"standalone cargo worktree -> {path}")
    return path


def remove_standalone_worktree(path: Path):
    base = path.parent
    result = git(["worktree", "remove", "--force", path], capture=True, check=False)
    if result.returncode != 0:
        log(f"WARNING: `git worktree remove` failed for {path}: {result.stderr.strip()}")
        shutil.rmtree(path, ignore_errors=True)
    git(["worktree", "prune"], check=False)
    if base.exists():
        shutil.rmtree(base, ignore_errors=True)


def sync_manifests_to_worktree(worktree: Path):
    """Carry every bumped manifest across, not just the root one.

    The lock is regenerated in the worktree, and it pins the internal crates by
    the version their own manifests declare; a worktree that still held the
    previous `0.0.N` would produce a lock that disagrees with the committed
    manifests (plan D2).
    """
    for manifest in [REPO / MANIFEST, *crate_manifests()]:
        destination = worktree / manifest.relative_to(REPO)
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(manifest, destination)


def git(args, **kw) -> subprocess.CompletedProcess:
    return run(["git", "-C", REPO, *args], **kw)


def current_branch() -> str:
    result = git(["branch", "--show-current"], capture=True)
    branch = result.stdout.strip()
    if not branch:
        fail("detached HEAD -- switch to main before releasing")
    return branch


def warn_if_behind_upstream(branch: str):
    upstream = git(
        ["rev-parse", "--abbrev-ref", "--symbolic-full-name", f"{branch}@{{u}}"],
        capture=True,
        check=False,
    )
    if upstream.returncode != 0 or not upstream.stdout.strip():
        return
    name = upstream.stdout.strip()
    behind = git(
        ["rev-list", "--count", f"{branch}..{name}"],
        capture=True,
        check=False,
    ).stdout.strip()
    if behind and behind != "0":
        log(
            f"WARNING: local {branch} is {behind} commit(s) behind {name} "
            f"(tracking ref; run `git fetch` for current state) -- releasing local {branch}"
        )


def working_tree_clean():
    status = git(["status", "--porcelain"], capture=True).stdout
    if status.strip():
        fail(
            "working tree is not clean -- commit or stash changes first:\n"
            + status.rstrip()
        )


def assert_no_external_path_dependencies():
    """A released gwz-core tag must be buildable when Cargo fetches only this repo."""
    manifest = REPO / "Cargo.toml"
    data = tomllib.loads(manifest.read_text(encoding="utf-8"))
    repo_root = REPO.resolve()
    violations: list[str] = []

    def scan(table: object, context: str = ""):
        if not isinstance(table, dict):
            return
        for section in ("dependencies", "dev-dependencies", "build-dependencies"):
            deps = table.get(section)
            if not isinstance(deps, dict):
                continue
            for name, spec in deps.items():
                if not isinstance(spec, dict) or "path" not in spec:
                    continue
                raw_path = str(spec["path"])
                dep_path = (REPO / raw_path).resolve()
                try:
                    dep_path.relative_to(repo_root)
                except ValueError:
                    violations.append(f"{context}{section}.{name} -> {raw_path}")

        targets = table.get("target")
        if isinstance(targets, dict):
            for target_name, target_table in targets.items():
                scan(target_table, f'target."{target_name}".')

    scan(data)
    if violations:
        fail(
            "release manifest has path dependencies outside the gwz-core repo, "
            "which breaks downstream Git dependencies:\n  "
            + "\n  ".join(violations)
        )


def read_package_version() -> str:
    toml = (REPO / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'^version\s*=\s*"([^"]+)"', toml, flags=re.M)
    if not match:
        fail("no top-level `version = \"...\"` found in Cargo.toml")
    return match.group(1)


def crate_manifests(root: Path = REPO) -> list[Path]:
    """Every internal crate manifest under `crates/`, in a stable order."""
    manifests = sorted((root / CRATES_DIR).glob(f"*/{MANIFEST}"))
    if not manifests:
        fail(f"no crate manifest under {root / CRATES_DIR} -- is this a gwz-core checkout?")
    return manifests


def read_internal_version(root: Path = REPO) -> str:
    """The one `0.0.N` version the crates under `crates/` share (plan D2).

    Read with `tomllib`, the way `scripts/checks/check_crate_versions.py` reads
    it, so this and the gate cannot disagree about what the tree says. The
    rewriting below is textual; only the reading is structural.
    """
    seen: dict[str, list[str]] = {}
    for manifest in crate_manifests(root):
        package = tomllib.loads(manifest.read_text(encoding="utf-8")).get("package")
        name = package.get("name") if isinstance(package, dict) else None
        version = package.get("version") if isinstance(package, dict) else None
        if not isinstance(name, str) or not isinstance(version, str):
            fail(f"{manifest}: [package] needs both a name and a version")
        seen.setdefault(version, []).append(name)
    if len(seen) != 1:
        detail = "; ".join(
            f"{version}: {', '.join(sorted(names))}" for version, names in sorted(seen.items())
        )
        fail(
            "the crates under crates/ disagree on the internal version, and the release bump "
            f"advances them as one lockstep line (plan D2) -- {detail}"
        )
    version = next(iter(seen))
    if not INTERNAL_VERSION.match(version):
        fail(
            f"the internal crates are at {version}, which is not on the 0.0.N line the release "
            "bump advances (plan D2)"
        )
    return version


def next_internal_version(current: str) -> str:
    """`0.0.N` -> `0.0.N+1`."""
    match = INTERNAL_VERSION.match(current)
    if match is None:
        fail(f"internal version {current!r} is not `0.0.N`")
    return f"0.0.{int(match.group(1)) + 1}"


def internal_edges_at(text: str, version: str) -> int:
    """How many `gwz-*` edges of a publishable table require `version`.

    Structural count, used only to check the textual rewrite below against what
    the manifest actually declares: a `gwz-*` edge spelled some way the line
    regex does not reach (a multi-line inline table, say) would otherwise be
    left behind silently, and the release would carry one stale edge.
    """
    total = 0
    tables = tomllib.loads(text)
    candidates = [tables]
    targets = tables.get("target")
    if isinstance(targets, dict):
        candidates.extend(table for table in targets.values() if isinstance(table, dict))
    for table in candidates:
        for kind in VERSIONED_KINDS:
            entries = table.get(kind)
            if not isinstance(entries, dict):
                continue
            for key, value in entries.items():
                spec = value if isinstance(value, dict) else {"version": value}
                package = str(spec.get("package", key))
                if package.startswith("gwz-") and spec.get("version") == version:
                    total += 1
    return total


def rewrite_internal_edges(text: str, current: str, version: str) -> tuple[str, int]:
    """Move every `gwz-*` edge of a publishable table onto `version`.

    Line-targeted, like `bump_cargo_version`, so comments, ordering and inline
    formatting survive and the manifest is never re-serialized. The enclosing
    table is tracked so a `[dev-dependencies]` edge is left alone.
    """
    edge = re.compile(
        r'^(?P<head>gwz-[A-Za-z0-9_-]*\s*=\s*\{.*?version\s*=\s*)"' + re.escape(current) + '"'
    )
    lines = text.split("\n")
    kind = ""
    rewritten = 0
    for index, line in enumerate(lines):
        header = TABLE_HEADER.match(line)
        if header is not None:
            kind = header.group(1).rsplit(".", 1)[-1].strip()
            continue
        if kind not in VERSIONED_KINDS:
            continue
        updated, count = edge.subn(rf'\g<head>"{version}"', line, count=1)
        if count:
            lines[index] = updated
            rewritten += 1
    return "\n".join(lines), rewritten


def bump_internal_version(current: str, version: str, root: Path = REPO) -> bool:
    """Advance the internal line in the fourteen crates and on every edge.

    Rewrites `[package].version` in each crate manifest and every `gwz-*` edge
    of a `[dependencies]` or `[build-dependencies]` table -- gwz-core's and the
    crates' own, target-specific tables included. Returns True when anything
    changed, so a second run on an already-bumped tree is a no-op the caller
    can see. `scripts/checks/check_crate_versions.py` is the gate that proves
    the result; this only has to produce it. `root` is the checkout to rewrite,
    which the tests point at a copy of the manifests.
    """
    package_version = re.compile(r'^(version\s*=\s*)"' + re.escape(current) + '"', flags=re.M)
    changed = False
    for manifest in [root / MANIFEST, *crate_manifests(root)]:
        text = manifest.read_text(encoding="utf-8")
        expected = internal_edges_at(text, current)
        updated = text
        if manifest != root / MANIFEST:
            updated = package_version.sub(rf'\g<1>"{version}"', updated, count=1)
        updated, rewritten = rewrite_internal_edges(updated, current, version)
        if rewritten != expected:
            fail(
                f"{manifest}: rewrote {rewritten} internal edge(s) at {current} but the manifest "
                f"declares {expected}; an edge is spelled in a way the bump cannot reach, so fix "
                "it by hand before releasing"
            )
        if updated == text:
            continue
        manifest.write_text(updated, encoding="utf-8", newline="\n")
        changed = True
    if changed:
        log(f"bumped the internal crate line {current} -> {version} (14 crates and their edges)")
    else:
        log(f"internal crate line already at {version}")
    return changed


def release_commit_message(version: str) -> str:
    return f"chore(release): gwz-core {version}"


def assert_internal_line_is_not_a_hand_bump(version: str) -> None:
    """Refuse to tag a tree whose product version was bumped without the internals.

    Reached only when `Cargo.toml` already carries the target version and no
    tag exists. This script writes the product version and the internal `0.0.N`
    line in one commit, so such a tree is either the work of an earlier run
    that stopped before tagging -- in which case the internal line moved with
    it -- or a hand bump, in which case the internal line is still the previous
    release's and two releases would share it (plan D2). Nothing in the tree
    tells the two apart, so the release commit this script writes has to be in
    history; otherwise this refuses rather than guessing.
    """
    message = release_commit_message(version)
    found = git(
        ["log", "--format=%s", "-n", "200", "HEAD"], capture=True, check=False
    ).stdout.splitlines()
    if message in found:
        log(f"release commit for {version} is already in history; internal line came with it")
        return
    fail(
        f"Cargo.toml is already at {version} but no `{message}` commit exists, so the internal "
        f"0.0.N line (now {read_internal_version()}) cannot be shown to have been advanced for "
        "this release; reset the hand-written version bump and re-run, or advance the internal "
        "line deliberately first"
    )


def bump_cargo_version(version: str) -> bool:
    """Set the package version. Returns True if Cargo.toml changed."""
    path = REPO / "Cargo.toml"
    text = path.read_text(encoding="utf-8")
    updated = re.sub(
        r'^(version\s*=\s*)"[^"]*"',
        rf'\g<1>"{version}"',
        text,
        count=1,
        flags=re.M,
    )
    if updated == text:
        if f'version = "{version}"' in text:
            log(f"Cargo.toml already at version {version}")
            return False
        fail("Cargo.toml version bump changed nothing and the expected line is absent")
    path.write_text(updated, encoding="utf-8", newline="\n")
    log(f"bumped Cargo.toml version -> {version}")
    return True


def bump_bazel_version(version: str) -> bool:
    """Keep the Bazel artifact's package version aligned with Cargo."""
    path = REPO / "BUILD.bazel"
    text = path.read_text(encoding="utf-8")
    updated, count = re.subn(
        r'^(\s*version\s*=\s*)"[^"]*"',
        rf'\g<1>"{version}"',
        text,
        flags=re.M,
    )
    if count != 1:
        fail("expected one package version in BUILD.bazel")
    if updated == text:
        return False
    path.write_text(updated, encoding="utf-8", newline="\n")
    log(f"bumped BUILD.bazel version -> {version}")
    return True


def assert_lock_current(*, cargo_root: Path):
    """Fail fast when Cargo.lock does not match Cargo.toml (e.g. new dep without lock update)."""
    result = run(
        ["cargo", "metadata", "--format-version", "1", "--locked"],
        cwd=cargo_root,
        capture=True,
        check=False,
    )
    if result.returncode == 0:
        return
    stderr = result.stderr or ""
    if "lock file" not in stderr and "Cargo.lock" not in stderr:
        if stderr:
            print(stderr, file=sys.stderr)
        fail("`cargo metadata --locked` failed before release gates")
    fail(
        "Cargo.lock is out of sync with Cargo.toml.\n"
        "  Fix: from a standalone gwz-core checkout (outside the gwz-dev workspace), run\n"
        "       `cargo generate-lockfile`, commit Cargo.lock, then re-run this script."
    )


def refresh_cargo_lock(*, cargo_root: Path) -> bool:
    """Regenerate Cargo.lock from Cargo.toml. Returns True if the lock file changed."""
    lock = cargo_root / "Cargo.lock"
    before = lock.read_text(encoding="utf-8") if lock.is_file() else ""
    run(["cargo", "generate-lockfile"], cwd=cargo_root)
    after = lock.read_text(encoding="utf-8")
    if after == before:
        log("Cargo.lock already matches Cargo.toml")
        return False
    log("refreshed Cargo.lock from Cargo.toml")
    return True


def copy_lock_from_cargo_root(cargo_root: Path):
    shutil.copy2(cargo_root / "Cargo.lock", REPO / "Cargo.lock")


def ensure_tag(tag: str, target: str):
    """Create the lightweight tag ``tag`` at commit ``target``, or no-op if it already points there.
    NEVER moves an existing tag -- released tags are immutable."""
    existing = git(
        ["rev-parse", "-q", "--verify", f"refs/tags/{tag}^{{commit}}"],
        capture=True,
        check=False,
    )
    if existing.returncode == 0:
        if existing.stdout.strip() == target:
            log(f"tag {tag} already points at {target[:10]} -- leaving it")
            return
        fail(
            f"tag {tag} already exists at {existing.stdout.strip()[:10]}, not the release commit "
            f"{target[:10]} -- refusing to move a release tag (delete it yourself if this is intentional)"
        )
    git(["tag", tag, target])
    log(f"created tag {tag} -> {target[:10]}")


def push_release(branch: str, tag: str, *, expected_head: str):
    """Push the branch + tag together, atomically (both land or neither)."""
    result = run(
        [
            "git",
            "-C",
            REPO,
            "push",
            "--atomic",
            "origin",
            f"{expected_head}:refs/heads/{branch}",
            f"{expected_head}:refs/tags/{tag}",
        ],
        capture=True,
        check=False,
    )
    if result.returncode != 0:
        if result.stderr:
            print(result.stderr, file=sys.stderr)
        fail(
            f"atomic push of gated commit {expected_head[:10]} to {branch} and {tag} failed -- "
            "with --atomic the remote is left "
            "unchanged; inspect `git ls-remote origin` and retry"
        )
    log(f"pushed {branch} + {tag} to origin (atomic)")


def run_checked_boundary_gates(*, cargo_root: Path):
    """Check the real source tree; mutation/compiler suites are manual-only."""
    run(
        [sys.executable, CHECKED_ARTIFACT_BOUNDARY],
        cwd=cargo_root,
    )
    test_env = cargo_env()
    test_env["CLIPPY_CONF_DIR"] = str(cargo_root)
    run(
        ["cargo", "clippy", "--all-targets", "--all-features", "--", "-D", "warnings"],
        cwd=cargo_root,
        env=test_env,
    )


def gate_exact_release_commit(*, cargo_root: Path, expected_head: str):
    """Reacquire the exact commit that can be tagged, then run mandatory gates."""
    if cargo_root != REPO:
        run(["git", "reset", "--hard", expected_head], cwd=cargo_root)
    require_exact_clean_release_tree(
        cargo_root=cargo_root, expected_head=expected_head, phase="before"
    )
    run_checked_boundary_gates(cargo_root=cargo_root)
    require_exact_clean_release_tree(
        cargo_root=cargo_root, expected_head=expected_head, phase="after"
    )


def require_exact_clean_release_tree(
    *, cargo_root: Path, expected_head: str, phase: str
):
    observed = run(
        ["git", "rev-parse", "HEAD"], cwd=cargo_root, capture=True
    ).stdout.strip()
    if observed != expected_head:
        fail(
            f"release gate tree is {observed[:10]}, expected exact tag target "
            f"{expected_head[:10]}"
        )
    dirty = run(
        ["git", "status", "--porcelain"], cwd=cargo_root, capture=True
    ).stdout.strip()
    if dirty:
        fail(f"release gate tree is dirty {phase} the exact-target gate:\n{dirty}")


def read_publish_order(*, cargo_root: Path) -> list[str]:
    """The publish order, derived from the manifests by the lockstep gate.

    Read from `check_crate_versions.py --print-publish-order` rather than
    written down here, so a new internal crate or a new internal edge cannot
    leave this script packaging the wrong set in the wrong order.
    """
    result = run(
        [sys.executable, CRATE_VERSIONS, "--root", cargo_root, "--print-publish-order"],
        cwd=cargo_root,
        capture=True,
    )
    names = result.stdout.split()
    if not names or names[-1] != "gwz-core":
        fail(f"--print-publish-order did not end at gwz-core: {names}")
    return names


def gate_release_publication(*, cargo_root: Path, expected_head: str, tag: str):
    """What a crates.io publish needs, on the exact commit about to be tagged.

    The lockstep gate with the tag (plan S1.2: gwz-core's version equals the
    tag's, the internals share their `0.0.N` line, every edge names it, nothing
    comes from git, every published crate carries its registry metadata), then
    one packaging pass over every crate in publish order, so `--no-verify` at
    publish time (plan D5) rests on a package cargo has actually assembled
    here.

    The pass is `cargo package --workspace`, not one `cargo package -p <crate>`
    per crate in the order: even with `--no-verify`, packaging one crate alone
    resolves its *published* manifest against the registry, and the internal
    versions a release introduces are not there yet, so every internal that has
    an internal edge fails outright (plan U5, measured in S1.4). `--workspace`
    resolves the siblings against each other. The publish order still decides
    which `.crate` files must exist when the pass finishes, so a crate that
    quietly packages nothing is a failed gate.

    Separate from `gate_exact_release_commit` because it needs the tag, and it
    re-asserts the exact clean tree around itself so the fence is the same.
    """
    require_exact_clean_release_tree(
        cargo_root=cargo_root, expected_head=expected_head, phase="before"
    )
    run([sys.executable, CRATE_VERSIONS, "--root", cargo_root, "--tag", tag], cwd=cargo_root)
    order = read_publish_order(cargo_root=cargo_root)
    product = read_package_version()
    internal = read_internal_version()
    expected = {
        name: cargo_root
        / "target"
        / "package"
        / f"{name}-{product if name == 'gwz-core' else internal}.crate"
        for name in order
    }
    for archive in expected.values():
        archive.unlink(missing_ok=True)
    run(["cargo", "package", "--workspace", "--no-verify", "--locked"], cwd=cargo_root)
    missing = [name for name, archive in expected.items() if not archive.is_file()]
    if missing:
        fail(
            "the packaging pass produced no archive for "
            + ", ".join(missing)
            + f" of the {len(order)} crate(s) that publish; crates.io is fed exactly these "
            "packages, so a missing one is a release that cannot complete (plan S1.4, D5)"
        )
    log(f"packaged {len(order)} publishable crate(s) in publish order: {', '.join(order)}")
    require_exact_clean_release_tree(
        cargo_root=cargo_root, expected_head=expected_head, phase="after"
    )


def finalize_new_release(
    *, cargo_root: Path, expected_head: str, branch: str, tag: str, push: bool
):
    """Gate the exact immutable target immediately before tag publication."""
    gate_exact_release_commit(cargo_root=cargo_root, expected_head=expected_head)
    gate_release_publication(cargo_root=cargo_root, expected_head=expected_head, tag=tag)
    ensure_tag(tag, expected_head)
    if push:
        push_release(branch, tag, expected_head=expected_head)
    else:
        log("next step (not done without --push):")
        log(f"  git -C {REPO} push origin {branch} {tag}")


def run_gates(*, cargo_root: Path, skip_regen: bool, no_test: bool):
    if not skip_regen:
        if not REGEN.is_file():
            fail(f"protocol regen script not found at {REGEN}")
        run([sys.executable, str(REGEN), "--check"], cwd=REPO)
    elif not no_test:
        # generated_protocol_is_current needs taut-proto even when --skip-regen-check.
        regen_python()

    run_fmt_check(cargo_root=cargo_root)
    assert_lock_current(cargo_root=cargo_root)
    # The lockstep line before anything expensive: a crate off the internal
    # 0.0.N line, an unversioned internal edge or a git dependency makes the
    # release unpublishable, and none of it needs a build to see (plan S1.2).
    # Without the tag here -- the tag is checked on the exact commit, once the
    # version bump has landed (gate_release_publication).
    run([sys.executable, CRATE_VERSIONS, "--root", cargo_root], cwd=cargo_root)
    run_checked_boundary_gates(cargo_root=cargo_root)
    test_env = cargo_env()

    if not no_test:
        run([sys.executable, str(cargo_root / "scripts" / "run_tests.py")], cwd=cargo_root, env=test_env)
    else:
        log("skipping `cargo test`")

def release_version(tag: str) -> str:
    """Accept stable releases and numbered release candidates, with no leading zeroes."""
    number = r"(?:0|[1-9][0-9]*)"
    if not re.fullmatch(rf"v{number}\.{number}\.{number}(?:-rc\.[1-9][0-9]*)?", tag):
        fail(f"tag must look like vX.Y.Z or vX.Y.Z-rc.N, got '{tag}'")
    return tag[1:]


def main():
    parser = argparse.ArgumentParser(
        description="Cut a gwz-core release tag off main (verify, bump, commit, tag)."
    )
    parser.add_argument("tag", help="release tag, e.g. v0.3.0")
    parser.add_argument("--branch", default="main", help="branch to release from (default: main)")
    parser.add_argument("--no-test", action="store_true", help="skip `cargo test --locked`")
    parser.add_argument(
        "--skip-regen-check",
        action="store_true",
        help="skip `python protocol/regen.py --check`",
    )
    parser.add_argument("--push", action="store_true", help="also push the branch + tag to origin")
    parser.add_argument(
        "--keep-worktree",
        action="store_true",
        help="leave the temp cargo worktree in place (you must `git worktree remove` it before re-running)",
    )
    args = parser.parse_args()

    tag = args.tag
    version = release_version(tag)

    for tool in ("git", "cargo"):
        if not shutil.which(tool):
            fail(f"`{tool}` not found on PATH")
    if not args.skip_regen_check and not shutil.which(sys.executable):
        fail(f"`{sys.executable}` not found on PATH")

    branch = current_branch()
    if branch != args.branch:
        fail(f"on branch '{branch}' but releases are cut from '{args.branch}' -- switch first")

    warn_if_behind_upstream(args.branch)
    working_tree_clean()
    assert_no_external_path_dependencies()

    head = git(["rev-parse", "HEAD"], capture=True).stdout.strip()
    existing = git(
        ["rev-parse", "-q", "--verify", f"refs/tags/{tag}^{{commit}}"],
        capture=True,
        check=False,
    )
    release_already_cut = existing.returncode == 0
    if release_already_cut:
        if existing.stdout.strip() != head:
            fail(
                f"tag {tag} already exists at {existing.stdout.strip()[:10]} but {args.branch} HEAD is "
                f"{head[:10]} -- inconsistent; resolve the tag manually before re-running"
            )
        current = read_package_version()
        if current != version:
            fail(
                f"tag {tag} already points at HEAD but Cargo.toml version is {current}, not {version}"
            )
        log(f"{tag} already exists at {args.branch} HEAD ({head[:10]}); release already cut")

    umbrella = parent_cargo_workspace_root(REPO)
    cargo_root = REPO
    worktree: Path | None = None
    if umbrella is not None and umbrella != REPO:
        log(
            f"gwz-core checkout sits under umbrella workspace {umbrella} -- "
            "cargo gates will run in a detached /tmp worktree"
        )
        worktree = make_standalone_worktree(tag)
        cargo_root = worktree

    try:
        if release_already_cut:
            gate_exact_release_commit(cargo_root=cargo_root, expected_head=head)
            gate_release_publication(cargo_root=cargo_root, expected_head=head, tag=tag)
            if args.push:
                push_release(args.branch, tag, expected_head=head)
            return

        run_gates(
            cargo_root=cargo_root,
            skip_regen=args.skip_regen_check,
            no_test=args.no_test,
        )

        toml_changed = bump_cargo_version(version)
        bazel_changed = bump_bazel_version(version)
        if toml_changed or bazel_changed:
            # The product version and the internal `0.0.N` line move together,
            # in one commit, so a release is never split across the two lines
            # (plan D2).
            internal = read_internal_version()
            bump_internal_version(internal, next_internal_version(internal))
            if worktree is not None:
                sync_manifests_to_worktree(worktree)
            refresh_cargo_lock(cargo_root=cargo_root)
            if worktree is not None:
                copy_lock_from_cargo_root(cargo_root)
            if not args.no_test:
                run([sys.executable, str(cargo_root / "scripts" / "run_tests.py")], cwd=cargo_root, env=cargo_env())
            staged = [
                str(manifest.relative_to(REPO).as_posix()) for manifest in crate_manifests()
            ]
            git(["add", "Cargo.toml", "Cargo.lock", "BUILD.bazel", *staged])
            # No AI co-author trailer. The operator's attribution rule is
            # absolute and applies to every commit in every repo, including
            # commits authored by tooling — and the settings-level enforcement
            # that covers agent-authored commits does not reach this script.
            message = release_commit_message(version)
            git(["commit", "-m", message])
            head = git(["rev-parse", "HEAD"], capture=True).stdout.strip()
            log(f"release commit -> {head[:10]}  (gwz-core {version})")
        else:
            current = read_package_version()
            if current != version:
                fail(f"Cargo.toml version is {current}, expected {version} for {tag}")
            assert_internal_line_is_not_a_hand_bump(version)
            log(f"{args.branch} already at version {version}; no new commit needed")

        finalize_new_release(
            cargo_root=cargo_root,
            expected_head=head,
            branch=args.branch,
            tag=tag,
            push=args.push,
        )
    finally:
        if worktree is not None:
            if args.keep_worktree:
                log(
                    f"left cargo worktree at {worktree} "
                    "(remove it before the next run: git worktree remove)"
                )
            else:
                remove_standalone_worktree(worktree)


if __name__ == "__main__":
    main()

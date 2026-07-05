#!/usr/bin/env python3
"""Cut a gwz-core release off ``main``, per RELEASE.md.

gwz-core has no release branch — tags are cut directly on ``main``. This script
automates RELEASE.md steps 1-4 for a given tag:

  1. Gate the tree: ``python protocol/regen.py --check``, ``cargo fmt --check``,
     ``cargo test --locked``, ``cargo clippy --all-targets -- -D warnings`` (same bar as CI).
  2. Bump ``version`` in ``Cargo.toml`` and refresh ``Cargo.lock`` via ``cargo generate-lockfile``.
  3. Commit on ``main``: ``chore(release): gwz-core X.Y.Z``.
  4. Tag that commit ``vX.Y.Z`` (lightweight). An existing tag is NEVER moved — if
     ``vX.Y.Z`` already points elsewhere the script aborts.

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
    python scripts/release.py vX.Y.Z --no-test      # skip `cargo test` (still runs clippy)
    python scripts/release.py vX.Y.Z --no-clippy    # skip `cargo clippy`
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
from pathlib import Path

# scripts/release.py -> the gwz-core repo root is one level up.
REPO = Path(__file__).resolve().parent.parent
REGEN = REPO / "protocol" / "regen.py"
REGEN_VENV = REPO / "protocol" / ".regen-venv"


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
    add_path_dependency_worktrees(base)
    log(f"standalone cargo worktree -> {path}")
    return path


def add_path_dependency_worktrees(base: Path):
    """Mirror sibling path dependencies needed by Cargo.toml in the temp parent."""
    dependencies = [
        ("taut-shape-rs", REPO.parent / "taut-shape-rs"),
    ]
    for name, source in dependencies:
        if not source.is_dir():
            fail(f"path dependency checkout missing: {source}")
        run(["git", "-C", source, "worktree", "prune"], check=False)
        head = run(["git", "-C", source, "rev-parse", "HEAD"], capture=True).stdout.strip()
        target = base / name
        run(["git", "-C", source, "worktree", "add", "--detach", target, head])
        log(f"path dependency worktree {name} -> {target}")


def remove_standalone_worktree(path: Path):
    base = path.parent
    dependency_worktrees = [
        (REPO.parent / "taut-shape-rs", base / "taut-shape-rs"),
    ]
    result = git(["worktree", "remove", "--force", path], capture=True, check=False)
    if result.returncode != 0:
        log(f"WARNING: `git worktree remove` failed for {path}: {result.stderr.strip()}")
        shutil.rmtree(path, ignore_errors=True)
    git(["worktree", "prune"], check=False)
    for source, target in dependency_worktrees:
        if not target.exists():
            continue
        result = run(
            ["git", "-C", source, "worktree", "remove", "--force", target],
            capture=True,
            check=False,
        )
        if result.returncode != 0:
            log(f"WARNING: `git worktree remove` failed for {target}: {result.stderr.strip()}")
            shutil.rmtree(target, ignore_errors=True)
        run(["git", "-C", source, "worktree", "prune"], check=False)
    if base.exists():
        shutil.rmtree(base, ignore_errors=True)


def sync_manifests_to_worktree(worktree: Path):
    shutil.copy2(REPO / "Cargo.toml", worktree / "Cargo.toml")


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


def read_package_version() -> str:
    toml = (REPO / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'^version\s*=\s*"([^"]+)"', toml, flags=re.M)
    if not match:
        fail("no top-level `version = \"...\"` found in Cargo.toml")
    return match.group(1)


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


def push_release(branch: str, tag: str):
    """Push the branch + tag together, atomically (both land or neither)."""
    result = run(
        ["git", "-C", REPO, "push", "--atomic", "origin", branch, tag],
        capture=True,
        check=False,
    )
    if result.returncode != 0:
        if result.stderr:
            print(result.stderr, file=sys.stderr)
        fail(
            f"`git push --atomic origin {branch} {tag}` failed -- with --atomic the remote is left "
            "unchanged; inspect `git ls-remote origin` and retry"
        )
    log(f"pushed {branch} + {tag} to origin (atomic)")


def run_gates(*, cargo_root: Path, skip_regen: bool, no_test: bool, no_clippy: bool):
    if not skip_regen:
        if not REGEN.is_file():
            fail(f"protocol regen script not found at {REGEN}")
        run([sys.executable, str(REGEN), "--check"], cwd=REPO)
    elif not no_test:
        # generated_protocol_is_current needs taut-proto even when --skip-regen-check.
        regen_python()

    run_fmt_check(cargo_root=cargo_root)
    assert_lock_current(cargo_root=cargo_root)
    test_env = cargo_env()

    if not no_test:
        run(["cargo", "test", "--locked"], cwd=cargo_root, env=test_env)
    else:
        log("skipping `cargo test`")

    if not no_clippy:
        run(["cargo", "clippy", "--all-targets", "--", "-D", "warnings"], cwd=cargo_root, env=test_env)
    else:
        log("skipping `cargo clippy`")


def main():
    parser = argparse.ArgumentParser(
        description="Cut a gwz-core release tag off main (verify, bump, commit, tag)."
    )
    parser.add_argument("tag", help="release tag, e.g. v0.3.0")
    parser.add_argument("--branch", default="main", help="branch to release from (default: main)")
    parser.add_argument("--no-test", action="store_true", help="skip `cargo test --locked`")
    parser.add_argument("--no-clippy", action="store_true", help="skip `cargo clippy`")
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
    if not re.fullmatch(r"v\d+\.\d+\.\d+", tag):
        fail(f"tag must look like vX.Y.Z, got '{tag}'")
    version = tag[1:]

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

    head = git(["rev-parse", "HEAD"], capture=True).stdout.strip()
    existing = git(
        ["rev-parse", "-q", "--verify", f"refs/tags/{tag}^{{commit}}"],
        capture=True,
        check=False,
    )
    if existing.returncode == 0:
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
        if args.push:
            push_release(args.branch, tag)
        return

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
        run_gates(
            cargo_root=cargo_root,
            skip_regen=args.skip_regen_check,
            no_test=args.no_test,
            no_clippy=args.no_clippy,
        )

        toml_changed = bump_cargo_version(version)
        if toml_changed:
            if worktree is not None:
                sync_manifests_to_worktree(worktree)
            refresh_cargo_lock(cargo_root=cargo_root)
            if worktree is not None:
                copy_lock_from_cargo_root(cargo_root)
            run(["cargo", "test", "--locked"], cwd=cargo_root, env=cargo_env())
            git(["add", "Cargo.toml", "Cargo.lock"])
            message = (
                f"chore(release): gwz-core {version}\n\n"
                "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
            )
            git(["commit", "-m", message])
            head = git(["rev-parse", "HEAD"], capture=True).stdout.strip()
            log(f"release commit -> {head[:10]}  (gwz-core {version})")
        else:
            current = read_package_version()
            if current != version:
                fail(f"Cargo.toml version is {current}, expected {version} for {tag}")
            log(f"{args.branch} already at version {version}; no new commit needed")

        ensure_tag(tag, head)

        if args.push:
            push_release(args.branch, tag)
        else:
            log("next step (not done without --push):")
            log(f"  git -C {REPO} push origin {args.branch} {tag}")
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

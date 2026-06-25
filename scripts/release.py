#!/usr/bin/env python3
"""Cut a gwz-core release off ``main``, per RELEASE.md.

gwz-core has no release branch — tags are cut directly on ``main``. This script
automates RELEASE.md steps 1-4 for a given tag:

  1. Gate the tree: ``python protocol/regen.py --check``, ``cargo test --locked``,
     ``cargo clippy --all-targets -- -D warnings`` (same bar as CI).
  2. Bump ``version`` in ``Cargo.toml`` to match the tag (``vX.Y.Z`` -> ``X.Y.Z``).
  3. Commit on ``main``: ``chore(release): gwz-core X.Y.Z``.
  4. Tag that commit ``vX.Y.Z`` (lightweight). An existing tag is NEVER moved — if
     ``vX.Y.Z`` already points elsewhere the script aborts.

Requires a clean working tree (land feature work first). The commit is skipped when
``Cargo.toml`` already carries the target version. Re-running after a successful release
is an idempotent no-op (and will create the tag if a prior run stopped before tagging).
Pushing is left to you unless ``--push`` is given.

This operates on your LOCAL ``main`` ref and does not fetch; it warns if ``main`` is
behind its upstream. Pull first if you want the latest.

Usage:
    python scripts/release.py vX.Y.Z              # verify + bump + commit + tag (no push)
    python scripts/release.py vX.Y.Z --push       # also push main + tag to origin
    python scripts/release.py vX.Y.Z --no-test      # skip `cargo test` (still runs clippy)
    python scripts/release.py vX.Y.Z --no-clippy    # skip `cargo clippy`
    python scripts/release.py vX.Y.Z --skip-regen-check
"""

from __future__ import annotations

import argparse
import re
import shutil
import subprocess
import sys
from pathlib import Path

# scripts/release.py -> the gwz-core repo root is one level up.
REPO = Path(__file__).resolve().parent.parent
REGEN = REPO / "protocol" / "regen.py"


def fail(msg: str):
    print(f"release: error: {msg}", file=sys.stderr)
    raise SystemExit(1)


def log(msg: str):
    print(f"release: {msg}")


def run(cmd, *, cwd=None, capture=False, check=True) -> subprocess.CompletedProcess:
    printable = " ".join(str(c) for c in cmd)
    log(f"$ {printable}")
    result = subprocess.run(
        [str(c) for c in cmd],
        cwd=str(cwd) if cwd is not None else None,
        capture_output=capture,
        text=True,
    )
    if check and result.returncode != 0:
        if capture and result.stderr:
            print(result.stderr, file=sys.stderr)
        fail(f"command failed ({result.returncode}): {printable}")
    return result


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


def run_gates(*, skip_regen: bool, no_test: bool, no_clippy: bool):
    if not skip_regen:
        if not REGEN.is_file():
            fail(f"protocol regen script not found at {REGEN}")
        run([sys.executable, str(REGEN), "--check"], cwd=REPO)
    else:
        log("skipping protocol/regen.py --check")

    if not no_test:
        run(["cargo", "test", "--locked"], cwd=REPO)
    else:
        log("skipping `cargo test`")

    if not no_clippy:
        run(["cargo", "clippy", "--all-targets", "--", "-D", "warnings"], cwd=REPO)
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

    run_gates(
        skip_regen=args.skip_regen_check,
        no_test=args.no_test,
        no_clippy=args.no_clippy,
    )

    changed = bump_cargo_version(version)
    if changed:
        git(["add", "Cargo.toml"])
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


if __name__ == "__main__":
    main()

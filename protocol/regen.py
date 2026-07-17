#!/usr/bin/env python3
"""Regenerate gwz-core's taut protocol bindings from the *released* taut-proto.

This is the canonical, cross-platform "release" generator. It provisions a `uv`
venv pinned to a PyPI ``taut-proto`` release and drives that release's ``tautc``
to regenerate the checked-in Rust protocol artifacts.

Why PyPI and not the vendored ``taut/src``:
  * CI (``tests/protocol.rs::generated_protocol_is_current``) regenerates and
    byte-compares against whatever ``taut`` it can import. The release workflow
    installs ``taut-proto`` from PyPI, so generating the committed output with the
    *same* PyPI release keeps the tree in lock-step with what CI verifies.
  * The editable vendored ``taut/src`` trips a setuptools-scm version bug on the
    ``gwztag/tag-done-p4`` git tag (needs ``SETUPTOOLS_SCM_PRETEND_VERSION``). The
    PyPI wheel ships a baked version, so this path is immune -- no workaround, and
    the git tag is left untouched.

Artifacts (all paths relative to the gwz-core crate root; commands run from there):
    tautc gen    protocol/gwz.taut.py -o <tmp> -l rust --api-only --with-runtime
        <tmp>/rust/api.rs  -> src/protocol/generated.rs
        <tmp>/rust/cbor.rs -> src/cbor.rs
        (<tmp>/rust/ext.rs is emitted but intentionally NOT vendored -- gwz-core
         does not use the forward-compat extension runtime; copying it would add
         an untracked, unverified file the build never references.)
    tautc corpus protocol/gwz.taut.py -o protocol/corpus -l rust
        writes protocol/corpus/golden.json and protocol/corpus/rust/vectors.rs in place

Usage:
    python protocol/regen.py                 # provision venv if needed, regenerate + write
    python protocol/regen.py --check         # verify only: no writes; nonzero exit on drift (CI-style gate)
    python protocol/regen.py --recreate      # rebuild the uv venv from scratch
    python protocol/regen.py --taut-version 0.8.1   # explicit interface-checkpoint override
    python protocol/regen.py --venv PATH     # override the venv location
"""

from __future__ import annotations

import argparse
import filecmp
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

# This file lives at gwz-core/protocol/regen.py -> the crate root is two levels up.
GWZ_CORE = Path(__file__).resolve().parent.parent
SCHEMA = "protocol/gwz.taut.py"            # relative to GWZ_CORE
CORPUS_DIR = "protocol/corpus"             # relative to GWZ_CORE
# tautc emits these into <out>/rust/; map each to its destination under GWZ_CORE.
GEN_COPIES = [
    ("api.rs", "src/protocol/generated.rs"),
    ("cbor.rs", "src/cbor.rs"),
    # "ext.rs" is emitted by --with-runtime but deliberately not vendored (see module docstring).
]
DEFAULT_VENV = GWZ_CORE / "protocol" / ".regen-venv"
TAUT_GENERATOR_VERSION = "0.8.1"


def fail(msg: str):
    print(f"regen: error: {msg}", file=sys.stderr)
    raise SystemExit(1)


def log(msg: str):
    print(f"regen: {msg}")


def _bindir(venv: Path) -> Path:
    # uv/std venvs: Scripts/ on Windows, bin/ on POSIX.
    return venv / ("Scripts" if os.name == "nt" else "bin")


def venv_exe(venv: Path, name: str) -> Path:
    suffix = ".exe" if os.name == "nt" else ""
    return _bindir(venv) / f"{name}{suffix}"


def run(cmd, **kwargs) -> subprocess.CompletedProcess:
    printable = " ".join(str(c) for c in cmd)
    log(f"$ {printable}")
    return subprocess.run([str(c) for c in cmd], **kwargs)


def ensure_venv(venv: Path, taut_version: str, recreate: bool) -> None:
    """Create the uv venv (if needed) and install taut-proto into it."""
    uv = shutil.which("uv")
    if not uv:
        fail(
            "`uv` not found on PATH. Install it:\n"
            "  macOS/Linux: curl -LsSf https://astral.sh/uv/install.sh | sh\n"
            "  Windows:     powershell -ExecutionPolicy ByPass -c \"irm https://astral.sh/uv/install.ps1 | iex\"\n"
            "  or:          pipx install uv  /  pip install uv  /  brew install uv"
        )
    if recreate and venv.exists():
        log(f"removing existing venv {venv}")
        shutil.rmtree(venv)
    if not venv.exists():
        if run([uv, "venv", venv]).returncode != 0:
            fail(f"failed to create uv venv at {venv}")

    spec = f"taut-proto=={taut_version}"
    install = [uv, "pip", "install", "--python", venv, spec]
    if run(install).returncode != 0:
        fail("failed to install taut-proto into the venv")


def installed_version(venv: Path) -> str:
    py = venv_exe(venv, "python")
    result = run(
        [py, "-c", "import importlib.metadata as m; print(m.version('taut-proto'))"],
        capture_output=True,
        text=True,
    )
    return result.stdout.strip() or "unknown"


def taut_env() -> dict:
    """Child env: drop PYTHONPATH (so vendored taut/src can't shadow the wheel) and
    force UTF-8 so generated source bytes match the committed files on every OS."""
    env = dict(os.environ)
    env.pop("PYTHONPATH", None)
    env["PYTHONUTF8"] = "1"
    env["PYTHONIOENCODING"] = "utf-8"
    return env


def tautc(venv: Path) -> list:
    """The tautc console script, or the layout-agnostic module fallback."""
    exe = venv_exe(venv, "tautc")
    if exe.exists():
        return [exe]
    return [venv_exe(venv, "python"), "-m", "taut.cli"]


def gen_into(venv: Path, out_dir: Path) -> None:
    cmd = tautc(venv) + ["gen", SCHEMA, "-o", out_dir, "-l", "rust", "--api-only", "--with-runtime"]
    if run(cmd, cwd=GWZ_CORE, env=taut_env()).returncode != 0:
        fail("tautc gen failed")


def corpus_write(venv: Path) -> None:
    cmd = tautc(venv) + ["corpus", SCHEMA, "-o", CORPUS_DIR, "-l", "rust"]
    if run(cmd, cwd=GWZ_CORE, env=taut_env()).returncode != 0:
        fail("tautc corpus (write) failed")


def corpus_is_current(venv: Path) -> bool:
    cmd = tautc(venv) + ["corpus", SCHEMA, "-o", CORPUS_DIR, "-l", "rust", "--check"]
    return run(cmd, cwd=GWZ_CORE, env=taut_env()).returncode == 0


def _emitted(out_dir: Path, name: str) -> Path:
    path = out_dir / "rust" / name
    if not path.exists():
        fail(f"expected generated file missing: {path}")
    return path


def do_write(venv: Path) -> list:
    """Regenerate and write the tracked artifacts. Returns the list that changed."""
    changed = []
    tmp = Path(tempfile.mkdtemp(prefix="gwz-taut-gen-"))
    try:
        gen_into(venv, tmp)
        for name, dest in GEN_COPIES:
            src = _emitted(tmp, name)
            dest_path = GWZ_CORE / dest
            if not (dest_path.exists() and filecmp.cmp(src, dest_path, shallow=False)):
                changed.append(dest)
            dest_path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(src, dest_path)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)

    if not corpus_is_current(venv):
        changed.append(f"{CORPUS_DIR}/")
    corpus_write(venv)
    if not corpus_is_current(venv):
        fail("corpus still reports drift after regeneration -- unexpected")
    return changed


def do_check(venv: Path) -> int:
    """Verify the committed artifacts match the generator; write nothing."""
    drift = []
    tmp = Path(tempfile.mkdtemp(prefix="gwz-taut-check-"))
    try:
        gen_into(venv, tmp)
        for name, dest in GEN_COPIES:
            src = _emitted(tmp, name)
            dest_path = GWZ_CORE / dest
            if not (dest_path.exists() and filecmp.cmp(src, dest_path, shallow=False)):
                drift.append(dest)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    if not corpus_is_current(venv):
        drift.append(CORPUS_DIR)

    if drift:
        print("regen: DRIFT -- committed files do not match the generator:", file=sys.stderr)
        for path in drift:
            print(f"  - {path}", file=sys.stderr)
        print("regen: run `python protocol/regen.py` to update them.", file=sys.stderr)
        return 1
    log("OK -- committed protocol artifacts are current.")
    return 0


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Regenerate gwz-core's taut protocol bindings from PyPI taut-proto via uv."
    )
    parser.add_argument("--check", action="store_true",
                        help="verify only; write nothing; nonzero exit on drift")
    parser.add_argument("--recreate", action="store_true",
                        help="rebuild the uv venv from scratch")
    parser.add_argument("--taut-version", default=TAUT_GENERATOR_VERSION,
                        help=f"taut-proto interface checkpoint (default: {TAUT_GENERATOR_VERSION})")
    parser.add_argument("--venv", type=Path, default=DEFAULT_VENV,
                        help=f"venv location (default: {DEFAULT_VENV})")
    args = parser.parse_args()

    ensure_venv(args.venv, args.taut_version, args.recreate)
    version = installed_version(args.venv)
    log(f"using taut-proto {version}  (venv: {args.venv})")

    if args.check:
        raise SystemExit(do_check(args.venv))

    changed = do_write(args.venv)
    if changed:
        log("updated:")
        for path in changed:
            print(f"  - {path}")
        log(f"done. Review the diff and commit; protocol output was generated by taut-proto {version}.")
    else:
        log("no changes -- already current.")


if __name__ == "__main__":
    main()

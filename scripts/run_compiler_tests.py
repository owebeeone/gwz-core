#!/usr/bin/env python3
"""Run slow source-mutation/compiler probes explicitly, outside release and CI gates."""
import argparse
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
SUITES = {
    "boundary": "scripts/manual_tests/boundary_probes.py",
    "privacy": "scripts/manual_tests/privacy_probes.py",
}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("suite", nargs="?", choices=["all", *SUITES], default="all")
    args = parser.parse_args()
    suites = list(SUITES.values()) if args.suite == "all" else [SUITES[args.suite]]
    return subprocess.call([sys.executable, "-m", "unittest", *suites, "-v"], cwd=ROOT)


if __name__ == "__main__":
    raise SystemExit(main())

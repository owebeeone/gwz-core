#!/usr/bin/env python3
"""Check the local clone family's user documentation against its manifest.

The same instrument as ``check_merge_docs.py`` -- the manifest schema, the
matcher and the fail-closed handling of an unreadable source are imported
from it, not copied -- read against a second manifest,
``local_clone_docs_manifest.json``. Two manifests rather than more rows in
one: the merge gate pins its source and assertion counts in its own suite and
in the aggregate driver, and those must not move when a local-clone sentence
changes; and the local-clone rows retire on their own schedule, one at a time,
as the modes the current build refuses start to be served.

Standard-library only, like its sibling, so release gates can run it offline.
Paths in the manifest are relative to the GWZ development workspace (the
directory holding ``gwz-core`` and ``gwz-cli``), which is why this checker,
exactly like the merge one, cannot run on a single-repository CI checkout.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path
from typing import Sequence


CHECKS_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(CHECKS_DIR))

from check_merge_docs import (  # noqa: E402
    DEFAULT_WORKSPACE_ROOT,
    ManifestError,
    check_manifest,
    load_manifest,
)

DEFAULT_MANIFEST = CHECKS_DIR / "local_clone_docs_manifest.json"
LABEL = "local-clone document consistency"


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--workspace-root",
        type=Path,
        default=DEFAULT_WORKSPACE_ROOT,
        help="GWZ development workspace containing gwz-core and gwz-cli",
    )
    parser.add_argument(
        "--manifest",
        type=Path,
        default=DEFAULT_MANIFEST,
        help="JSON assertion manifest",
    )
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    try:
        manifest = load_manifest(args.manifest)
        result = check_manifest(manifest, args.workspace_root.resolve())
    except ManifestError as error:
        print(f"{LABEL}: invalid manifest: {error}", file=sys.stderr)
        return 2

    if result.ok:
        print(
            f"{LABEL}: ok "
            f"({result.source_count} sources, {result.assertion_count} assertions)"
        )
        return 0

    print(f"{LABEL}: failed ({len(result.findings)} finding(s))", file=sys.stderr)
    for finding in result.findings:
        print(
            f"{finding.path}: [{finding.assertion_id}] {finding.message}",
            file=sys.stderr,
        )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())

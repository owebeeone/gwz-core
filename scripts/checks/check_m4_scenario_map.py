#!/usr/bin/env python3
"""Fail-closed check of the M4 scenario -> equivalence-evidence map.

R4b-G F-1 / inventory W4 / evidence rows 2.3b and O8. O8 obliges "byte-
equivalent lock/candidate/root/archive output and identical restart actions
for every M4 and all seven adapted-v0 scenarios"
(`GwzM5-8Refactor.md:2265`). Until the M4 scenario set was enumerated and
mapped, the clause was not checkable at all. The enumeration is
`dev-docs/GwzM5-8R4bG-Evidence.md` §12, delimited by the `m4-map` markers;
this script is the machine half of it, and answers three questions the prose
cannot answer about itself:

  1. does every test the map names still exist?
  2. is every registry row claimed by exactly one scenario, so a bound case
     cannot silently drop out of the map?
  3. does every registry case id the map cites actually exist in the frozen
     predicate registry?

It deliberately does NOT re-run the tests -- `check_merge_compatibility_
predicates.py` and the g23 suite do that. This closes the gap between them:
the registry validates the rows that exist, and cannot notice a scenario
nobody filed; the map names the scenarios, and cannot notice a test that was
renamed away.

Only Tables A and B (between the markers) are in scope. §12.5's feature axis
is O9's, not O8's.

**Scope disclosure (J-7 class).** The map lives in the gwz-dev workspace's
`dev-docs/`, one level above this checkout, so this checker resolves it the
same way `check_merge_docs.py` resolves its workspace root -- and inherits the
same limitation: **it cannot run in a bare gwz-core CI checkout, and it is not
wired into any workflow.** It fails, never passes, when the map is absent.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MAP_DOC = ROOT.parent / "dev-docs" / "GwzM5-8R4bG-Evidence.md"
REGISTRY = ROOT / "dev-docs" / "GwzM5-8I2CompatibilityPredicates.json"
REGION = re.compile(r"<!-- m4-map:begin -->(.*?)<!-- m4-map:end -->", re.S)
TICKED = re.compile(r"`([^`]+)`")
TEST_PATH = re.compile(r"^[a-z_][a-z0-9_]*(?:::[a-z_][a-z0-9_]*)+$")
CASE_ID = re.compile(r"^[a-z][a-z-]*/[a-z0-9-]+$")
IMPLICIT_PREFIX = "workspace_ops::tests::g23::"


def listed_tests(test_list: Path | None) -> set[str]:
    if test_list is not None:
        text = test_list.read_text(encoding="utf-8")
    else:
        text = subprocess.run(
            ["cargo", "test", "--lib", "-p", "gwz-core", "--", "--list"],
            check=True,
            capture_output=True,
            text=True,
            cwd=ROOT,
        ).stdout
    return {
        line.removesuffix(": test").strip()
        for line in text.splitlines()
        if line.endswith(": test")
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--doc", type=Path, default=MAP_DOC)
    parser.add_argument("--registry", type=Path, default=REGISTRY)
    parser.add_argument(
        "--test-list",
        type=Path,
        default=None,
        help="file holding `cargo test --lib -- --list` output; runs cargo if absent",
    )
    args = parser.parse_args()

    findings: list[str] = []
    try:
        text = args.doc.read_text(encoding="utf-8")
    except OSError as error:
        print(f"M4 scenario map: unreadable ({error})", file=sys.stderr)
        return 1
    region = REGION.search(text)
    if region is None:
        print(
            f"M4 scenario map: the m4-map markers are missing from {args.doc}",
            file=sys.stderr,
        )
        return 1
    rows: list[str] = []
    for line in region.group(1).splitlines():
        if not line.startswith("|"):
            continue
        if set(line) <= set("| -:"):
            # A markdown separator; the line before it was the header, not a
            # scenario, so the row count stays the true scenario count.
            rows.pop()
            continue
        rows.append(line)
    tokens = [token for row in rows for token in TICKED.findall(row)]

    tests = {
        token if token.startswith("workspace_ops::") else IMPLICIT_PREFIX + token
        for token in tokens
        if TEST_PATH.match(token)
    }
    known = listed_tests(args.test_list)
    for test in sorted(tests - known):
        findings.append(f"map names a test that does not exist: {test}")

    registry = json.loads(args.registry.read_text(encoding="utf-8"))
    declared = {
        row["case_id"]
        for key in ("fixture_corpus", "valid_unlisted_corpus")
        for row in registry[key]
    }
    cited = {token for token in tokens if CASE_ID.match(token)}
    for case in sorted(cited - declared):
        findings.append(f"map cites a registry case that does not exist: {case}")
    for case in sorted(declared - cited):
        findings.append(f"registry case is unclaimed by the M4 map: {case}")

    if findings:
        print("M4 scenario map: failed", file=sys.stderr)
        for finding in findings:
            print(f"- {finding}", file=sys.stderr)
        return 1
    print(
        f"M4 scenario map: ok ({len(rows)} scenario rows, {len(tests)} named tests, "
        f"{len(declared)} registry rows all claimed)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

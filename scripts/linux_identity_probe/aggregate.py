#!/usr/bin/env python3
"""Validate and aggregate the two native Linux identity evidence rows."""

from __future__ import annotations

import argparse
import json
import pathlib

import provider


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", action="append", required=True, type=pathlib.Path)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument("--expected-core-commit", required=True)
    parser.add_argument("--expected-workflow-run", required=True)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    rows = [json.loads(path.read_text(encoding="utf-8")) for path in args.input]
    evidence = provider.aggregate_evidence(
        rows,
        expected_core_commit=args.expected_core_commit,
        expected_workflow_run=args.expected_workflow_run,
        expected_source_sha256=provider.probe_source_digest(pathlib.Path(__file__).parent),
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(provider.canonical_json(evidence))
    print(json.dumps(evidence, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()

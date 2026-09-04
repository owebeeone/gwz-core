#!/usr/bin/env python3
"""Prove the gwz-log schema growth leaves every pre-log wire shape unchanged."""

from __future__ import annotations

import hashlib
import json
import sys
from copy import deepcopy
from pathlib import Path
from typing import Any

from taut.ir.export import schema_json
from taut.ir.load import load_schema


# Moved deliberately on 2026-09-03 by DR-1 ship (1) W1
# (dev-docs/GwzM5-8DR1-WarnOrRefuse-Charter.md §3.7): the charter adds
# MergeRequest.filesystem_strict (slot 8), MergeResponse.crash_recovery
# (slot 11), MergeCrashRecovery/MergeCrashRecoveryGap and EventKind.diagnostic
# (slot 8). All additive; no pre-existing slot changed. This pin still guards
# gwz-log against reshaping older messages.
#   was: d0c205c8767f8d54d32ead2f676a05077d849f6a12278d9de52b3c132c3c9372
#
# Moved deliberately again on 2026-09-03 by M5d step (3)
# (dev-docs/GwzM5-8M5d-Charter.md §3/§10.2): the charter allocates exactly one
# more optional response field, MergeCrashRecovery.handles_ok (slot 4). No
# version bump, no record or catalog format change. MEASURED additive, not
# assumed: the projection was rendered on both trees and diffed -- the only
# delta is the one new `handles_ok` field object, and the previous pin below
# reproduced exactly on the pre-change tree.
#   was: 7a66e301c5c0147a12c59b2cddb6f2ebc1515ef4d65297ec53c3b312a3769697
PRE_LOG_WIRE_SHA256 = "71bf6b9223ba6d2b4d12049e425e567254ca79396d67922be737c86c6dd97a40"
LOG_METHODS = {"log", "log.output"}


def pre_log_projection(schema_ir: dict[str, Any]) -> dict[str, Any]:
    """Remove only S2.0's additive surface, retaining every older wire slot."""
    projected = deepcopy(schema_ir)
    projected["messages"] = [
        message for message in projected["messages"] if not message["name"].startswith("Log")
    ]
    projected["enums"] = [
        enum for enum in projected["enums"] if not enum["name"].startswith("Log")
    ]

    action_kind = next(enum for enum in projected["enums"] if enum["name"] == "ActionKind")
    log_slot = action_kind["members"].pop("log", None)
    if log_slot != 26:
        raise ValueError(f"ActionKind.log must use the next additive slot 26, got {log_slot}")

    service = next(service for service in projected["services"] if service["name"] == "GwzCore")
    service["methods"] = [
        method for method in service["methods"] if method["name"] not in LOG_METHODS
    ]
    return projected


def fingerprint(value: dict[str, Any]) -> str:
    encoded = json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def main() -> int:
    schema_path = Path(sys.argv[1] if len(sys.argv) > 1 else "protocol/gwz.taut.py")
    actual = fingerprint(pre_log_projection(schema_json(load_schema(schema_path))))
    if actual != PRE_LOG_WIRE_SHA256:
        print(
            "check_log_additive: pre-existing protocol wire projection changed\n"
            f"  expected: sha256:{PRE_LOG_WIRE_SHA256}\n"
            f"  actual:   sha256:{actual}",
            file=sys.stderr,
        )
        return 1
    print(f"check_log_additive: OK sha256:{actual}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

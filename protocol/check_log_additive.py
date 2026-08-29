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


PRE_LOG_WIRE_SHA256 = "d0c205c8767f8d54d32ead2f676a05077d849f6a12278d9de52b3c132c3c9372"
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

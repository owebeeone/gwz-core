#!/usr/bin/env python3
"""Tests for the message catalog generator (docs/generate_message_catalog.py).

Needs the taut tree beside gwz-core (the generator loads `protocol/gwz.taut.py` through it), so it is not in the
boundary job's unittest list, where taut is not checked out.
"""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import unittest
from pathlib import Path


CORE_ROOT = Path(__file__).resolve().parents[2]
GENERATOR = CORE_ROOT / "docs" / "generate_message_catalog.py"


def load_generator():
    spec = importlib.util.spec_from_file_location("generate_message_catalog", GENERATOR)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def optional_cells(catalog: str) -> dict[tuple[str, str], str]:
    """(message, field) -> the Optional column cell, read from the rendered tables."""
    cells: dict[tuple[str, str], str] = {}
    message = ""
    for line in catalog.splitlines():
        if line.startswith("### "):
            message = line[4:].strip()
        elif line.startswith("| ") and message and not line.startswith(("| Field |", "| Member |", "| ---")):
            columns = [c.strip() for c in line.strip("|").split(" | ")]
            if len(columns) == 6:
                cells[(message, columns[0])] = columns[3]
    return cells


class MessageCatalogTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        generator = load_generator()
        cls.cells = optional_cells(generator.render_catalog(generator.load_schema()))

    def test_missing_ok_fields_render_differently_from_optional_fields(self) -> None:
        # optional=True: the key is required and the value may be null (an absent key is MissingKey).
        self.assertEqual("yes", self.cells[("OperationPolicy", "max_connections_per_host")])
        # MISSING_OK: an absent key also reads as null.
        missing_ok = [
            ("OperationPolicy", "max_retries"),
            ("TransportObservation", "endpoint_id"),
            ("TransportObservation", "connection_id"),
            ("TransportObservation", "stream_id"),
            ("TransportObservation", "reused"),
        ]
        for key in missing_ok:
            self.assertEqual("yes (absent ok)", self.cells[key], key)
        self.assertEqual("no", self.cells[("ResponseMeta", "request_id")])

    def test_catalog_is_current(self) -> None:
        result = subprocess.run(
            [sys.executable, "-B", str(GENERATOR), "--check"], capture_output=True, text=True, check=False
        )
        self.assertEqual(0, result.returncode, result.stderr)


if __name__ == "__main__":
    unittest.main()

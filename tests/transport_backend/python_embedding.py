"""In-process Python codec bridge used by Rust integration tests.

The module imports gwz-py's own codec from the workspace's ``gwz-py/src``: since plan step 2.2 its generated
protocol carries the placement and attachment fields the embedded-message tests round-trip, so nothing is
patched or composed here, and no production Python module or generated file is modified.
"""

from __future__ import annotations

import importlib
import os
import sys
from functools import cache
from pathlib import Path
from typing import Any


def _workspace(manifest_dir: str | None) -> Path:
    explicit = os.environ.get("GWZ_TEST_WORKSPACE")
    root = Path(explicit or manifest_dir or os.environ.get("CARGO_MANIFEST_DIR", ".")).resolve()
    if root.name == "gwz-core":
        return root.parent
    if (root / "gwz-core").is_dir():
        return root
    prepared_tests = root / "tests" / "transport_consumer"
    if prepared_tests.is_dir():
        core = prepared_tests.resolve().parents[1]
        if core.name == "gwz-core":
            return core.parent
    raise RuntimeError(f"cannot locate gwz-dev workspace from {root}")


@cache
def _codec(manifest_dir: str | None):
    workspace = _workspace(manifest_dir)
    py_src = workspace / "gwz-py" / "src"
    taut_src = workspace / "taut" / "src"
    for path in (py_src, taut_src):
        if not path.exists():
            raise RuntimeError(f"codec input is missing: {path}")

    for path in (str(taut_src), str(py_src)):
        if path not in sys.path:
            sys.path.insert(0, path)

    codec = importlib.import_module("gwz.protocol.codec")
    origin = getattr(codec, "__file__", None)
    if origin is None or py_src not in Path(origin).resolve().parents:
        raise RuntimeError(f"gwz.protocol.codec came from outside gwz-py/src: {origin}")
    return codec


def roundtrip(message_name: str, payload: bytes, manifest_dir: str | None = None) -> bytes:
    """Decode and re-encode one message in the same interpreter."""

    codec = _codec(manifest_dir)
    value: Any = codec.decode_message(message_name, bytes(payload))
    return bytes(codec.encode_message(message_name, value))

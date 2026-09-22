"""In-process candidate Python codec bridge used by Rust integration tests.

The module loads the checked-in candidate Python dataclasses and composes the
candidate IR in memory.  It patches only the imported test copy of
``gwz.protocol.codec``; production Python modules and generated files are not
modified.
"""

from __future__ import annotations

import importlib
import importlib.util
import json
import os
import sys
from functools import cache
from pathlib import Path
from types import ModuleType
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


def _load_candidate_generated(path: Path) -> ModuleType:
    name = "gwz_test_candidate_generated"
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load candidate generated module: {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


@cache
def _candidate_codec(manifest_dir: str | None):
    workspace = _workspace(manifest_dir)
    core = workspace / "gwz-core"
    py_src = workspace / "gwz-py" / "src"
    taut_src = workspace / "taut" / "src"
    candidate_schema = core / "tests" / "transport_consumer" / "protocol" / "candidate.taut.py"
    candidate_generated = core / "tests" / "transport_consumer" / "candidate" / "candidate_generated.py"
    core_schema = core / "protocol" / "gwz.taut.py"
    owner_schema = workspace / "gwz-transport" / "protocol" / "transport.ir.json"
    for path in (py_src, taut_src, candidate_schema, candidate_generated, core_schema, owner_schema):
        if not path.exists():
            raise RuntimeError(f"candidate codec input is missing: {path}")

    for path in (str(taut_src), str(py_src)):
        if path not in sys.path:
            sys.path.insert(0, path)
    os.environ["GWZ_CORE_SCHEMA"] = str(core_schema)
    os.environ["GWZ_TRANSPORT_SCHEMA"] = str(owner_schema)

    from taut.ir.export import schema_json
    from taut.ir.load import load_schema

    schema_value = load_schema(candidate_schema)
    candidate_ir = schema_json(schema_value)
    candidate_module = _load_candidate_generated(candidate_generated)
    codec = importlib.import_module("gwz.protocol.codec")
    origin = getattr(codec, "__file__", None)
    if origin is None or py_src not in Path(origin).resolve().parents:
        raise RuntimeError(f"gwz.protocol.codec came from outside gwz-py/src: {origin}")
    codec.generated = candidate_module
    codec.schema = lambda: schema_value
    codec._ir_bytes = lambda: json.dumps(candidate_ir, sort_keys=True).encode()
    codec.generated_classes.cache_clear()
    return codec


def roundtrip(message_name: str, payload: bytes, manifest_dir: str | None = None) -> bytes:
    """Decode and re-encode one candidate message in the same interpreter."""

    codec = _candidate_codec(manifest_dir)
    value: Any = codec.decode_message(message_name, bytes(payload))
    return bytes(codec.encode_message(message_name, value))

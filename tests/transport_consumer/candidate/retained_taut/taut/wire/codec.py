"""IR-driven codec — native value <-> CBOR bytes, driven by the schema.

This is the mechanism the IR derives, realized as a runtime interpreter over the
IR data (not emitted source). A "native value" here is a plain dict keyed by
field name (enums as their member-name string); the dataclass binding is a thin
adapter layered on top. The wire is a projection of the *tagged* subset:

  - message  -> CBOR map { field.tag : encoded-value }, transient fields skipped
  - enum     -> its integer wire value
  - list     -> CBOR array
  - scalar   -> passthrough (int/str/bytes/bool); float fields coerce value->float
  - optional -> always emitted; None -> CBOR null (deterministic; no omission)
"""

from __future__ import annotations

from typing import Any

from ..ir.model import EnumRef, ListOf, MapOf, MsgRef, Scalar, Schema, TypeRef
from . import cbor

DecodeError = cbor.DecodeError
EncodeError = cbor.EncodeError


def encode(schema: Schema, message: str, value: dict[str, Any]) -> bytes:
    return cbor.dumps(encode_struct(schema, message, value))


def decode(schema: Schema, message: str, data: bytes) -> dict[str, Any]:
    return decode_struct(schema, message, cbor.loads(data), strict=True)


def encode_struct(schema: Schema, message: str, value: dict[str, Any]) -> dict[int, Any]:
    """Native dict -> CBOR-ready int-keyed structure (composable, no bytes yet)."""
    return _to_wire(schema, MsgRef(message), value)


def decode_struct(schema: Schema, message: str, struct: Any, *, strict: bool = False) -> dict[str, Any]:
    """CBOR-decoded int-keyed structure -> native dict."""
    return _from_wire(schema, MsgRef(message), struct, strict=strict)


def _to_wire(schema: Schema, tref: TypeRef, value: Any) -> Any:
    if isinstance(tref, Scalar):
        if tref.kind == "float":
            return float(value)
        if tref.kind == "int":
            if not isinstance(value, int) or isinstance(value, bool):
                raise TypeError("int field expects int")
            if value < cbor.INT_MIN or value > cbor.INT_MAX:
                raise EncodeError("IntOutOfSubset", value=value)
            return value
        if tref.kind == "str":
            if not isinstance(value, str):
                raise TypeError("str field expects str")
            return value
        if tref.kind == "bytes":
            if not isinstance(value, (bytes, bytearray)):
                raise TypeError("bytes field expects bytes")
            return bytes(value)
        if tref.kind == "bool":
            if not isinstance(value, bool):
                raise TypeError("bool field expects bool")
            return value
        raise TypeError(f"unknown scalar kind {tref.kind!r}")
    if isinstance(tref, EnumRef):
        return schema.enums[tref.name].members[value]   # member name -> int
    if isinstance(tref, ListOf):
        if not isinstance(value, (list, tuple)):
            raise TypeError("list field expects list")
        return [_to_wire(schema, tref.elem, v) for v in value]
    if isinstance(tref, MapOf):
        if not isinstance(value, dict):
            raise TypeError("map field expects dict")
        # key-sorted array of {1: key, 2: value} — deterministic, like a message list
        return [
            {1: _to_wire(schema, tref.key, k), 2: _to_wire(schema, tref.value, value[k])}
            for k in sorted(value)
        ]
    if isinstance(tref, MsgRef):
        if not isinstance(value, dict):
            raise TypeError("message field expects dict")
        msg = schema.messages[tref.name]
        out: dict[int, Any] = {}
        for f in msg.wire_fields():
            fv = value.get(f.name)
            out[f.tag] = None if fv is None else _to_wire(schema, f.type, fv)
        # forward-compat: re-emit fields this schema doesn't know (preserved raw)
        for tag, raw in value.get("__unknown__", {}).items():
            out[int(tag)] = raw
        return out
    raise TypeError(f"unknown type ref {tref!r}")


def _from_wire(schema: Schema, tref: TypeRef, cv: Any, *, strict: bool = False) -> Any:
    if isinstance(tref, Scalar):
        if tref.kind == "int":
            if not isinstance(cv, int) or isinstance(cv, bool):
                raise DecodeError("WrongType", expected="int")
            if cv < cbor.INT_MIN or cv > cbor.INT_MAX:
                raise DecodeError("IntOverflow", value=cv)
            return cv
        if tref.kind == "str":
            if not isinstance(cv, str):
                raise DecodeError("WrongType", expected="text")
            return cv
        if tref.kind == "bytes":
            if not isinstance(cv, bytes):
                raise DecodeError("WrongType", expected="bytes")
            return cv
        if tref.kind == "bool":
            if not isinstance(cv, bool):
                raise DecodeError("WrongType", expected="bool")
            return cv
        if tref.kind == "float":
            if not isinstance(cv, float):
                raise DecodeError("WrongType", expected="float")
            return cv
        return cv
    if isinstance(tref, EnumRef):
        if not isinstance(cv, int) or isinstance(cv, bool):
            raise DecodeError("WrongType", expected="int")
        members = schema.enums[tref.name].members
        reverse = {v: k for k, v in members.items()}
        if cv not in reverse:
            raise DecodeError("UnknownEnum", enum=tref.name, value=cv)
        return reverse[cv]                               # int -> member name
    if isinstance(tref, ListOf):
        if not isinstance(cv, list):
            raise DecodeError("WrongType", expected="array")
        return [_from_wire(schema, tref.elem, v, strict=strict) for v in cv]
    if isinstance(tref, MapOf):
        if not isinstance(cv, list):
            raise DecodeError("WrongType", expected="array")
        out: dict[Any, Any] = {}
        for e in cv:
            if not isinstance(e, dict):
                raise DecodeError("WrongType", expected="map")
            if 1 not in e:
                raise DecodeError("MissingKey", key=1)
            if 2 not in e:
                raise DecodeError("MissingKey", key=2)
            key = _from_wire(schema, tref.key, e[1], strict=strict)
            if key in out:
                raise DecodeError("DuplicateMapKey", key=key)
            out[key] = _from_wire(schema, tref.value, e[2], strict=strict)
        return out
    if isinstance(tref, MsgRef):
        if not isinstance(cv, dict):
            raise DecodeError("WrongType", expected="map")
        msg = schema.messages[tref.name]
        out: dict[str, Any] = {}
        known: set[int] = set()
        for f in msg.wire_fields():
            known.add(f.tag)
            if f.tag not in cv:
                if strict and not f.optional:
                    raise DecodeError("MissingKey", key=f.tag)
                out[f.name] = None
                continue
            raw = cv[f.tag]
            out[f.name] = None if raw is None and f.optional else _from_wire(
                schema, f.type, raw, strict=strict
            )
        # forward-compat: capture tags this schema doesn't know (raw, preserved)
        unknown = {tag: val for tag, val in cv.items() if tag not in known}
        if unknown:
            out["__unknown__"] = unknown
        return out
    raise TypeError(f"unknown type ref {tref!r}")

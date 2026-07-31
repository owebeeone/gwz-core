"""Deterministic parser/canonicalizer for the checked GWZ YAML subset.

The retained fixtures use mappings, scalar sequences, scalars, and literal
blocks. Unsupported YAML constructs fail closed instead of being flattened.
"""

from __future__ import annotations

import hashlib
import json
import re
from typing import Any


UUID_RE = re.compile(
    r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b"
)


class YamlSubsetError(ValueError):
    """Input is outside the deterministic retained-reader YAML subset."""


def _indent(line: str) -> int:
    if "\t" in line[: len(line) - len(line.lstrip())]:
        raise YamlSubsetError("tabs are not allowed in indentation")
    return len(line) - len(line.lstrip(" "))


def _scalar(value: str) -> str:
    value = value.strip()
    if not value:
        raise YamlSubsetError("empty scalar is ambiguous")
    if value.startswith(("&", "*", "!", ">")):
        raise YamlSubsetError(f"unsupported scalar form {value!r}")
    if value[0:1] in {"'", '"'}:
        if len(value) < 2 or value[-1] != value[0]:
            raise YamlSubsetError(f"unterminated quoted scalar {value!r}")
        return value[1:-1].replace("''", "'") if value[0] == "'" else value[1:-1]
    return value


class _Parser:
    def __init__(self, text: str):
        self.lines = text.replace("\r\n", "\n").replace("\r", "\n").splitlines()
        self.position = 0

    def parse(self) -> dict[str, Any]:
        if not self.lines:
            raise YamlSubsetError("YAML document is empty")
        result = self.mapping(0)
        if self.position != len(self.lines):
            raise YamlSubsetError(f"unexpected content at line {self.position + 1}")
        return result

    def mapping(self, indent: int) -> dict[str, Any]:
        result: dict[str, Any] = {}
        while self.position < len(self.lines):
            line = self.lines[self.position]
            if not line.strip():
                self.position += 1
                continue
            actual = _indent(line)
            if actual < indent or (actual == indent and line[indent:].startswith("- ")):
                break
            if actual != indent:
                raise YamlSubsetError(f"unexpected indentation at line {self.position + 1}")
            content = line[indent:]
            if ":" not in content:
                raise YamlSubsetError(f"mapping entry lacks colon at line {self.position + 1}")
            key, remainder = content.split(":", 1)
            if not key or key in result or key.strip() != key:
                raise YamlSubsetError(f"invalid or duplicate key {key!r} at line {self.position + 1}")
            self.position += 1
            remainder = remainder.lstrip()
            if remainder == "|":
                result[key] = self.literal(indent)
            elif remainder:
                result[key] = _scalar(remainder)
            elif self.position >= len(self.lines):
                result[key] = None
            else:
                next_line = self.lines[self.position]
                next_indent = _indent(next_line)
                if next_indent == indent and next_line[indent:].startswith("- "):
                    result[key] = self.sequence(indent)
                elif next_indent > indent:
                    result[key] = self.mapping(next_indent)
                else:
                    result[key] = None
        return result

    def sequence(self, indent: int) -> list[Any]:
        result: list[Any] = []
        while self.position < len(self.lines):
            line = self.lines[self.position]
            if _indent(line) != indent or not line[indent:].startswith("- "):
                break
            remainder = line[indent + 2 :]
            self.position += 1
            if not remainder:
                if self.position >= len(self.lines) or _indent(self.lines[self.position]) <= indent:
                    result.append(None)
                else:
                    result.append(self.mapping(_indent(self.lines[self.position])))
            elif ":" in remainder:
                key, value = remainder.split(":", 1)
                if not key or key.strip() != key:
                    raise YamlSubsetError(
                        f"invalid sequence mapping key {key!r} at line {self.position}"
                    )
                item: dict[str, Any] = {key: _scalar(value) if value.strip() else None}
                if self.position < len(self.lines):
                    next_indent = _indent(self.lines[self.position])
                    if next_indent > indent:
                        continuation = self.mapping(next_indent)
                        duplicate = set(item) & set(continuation)
                        if duplicate:
                            raise YamlSubsetError(f"duplicate sequence mapping keys {sorted(duplicate)}")
                        item.update(continuation)
                result.append(item)
            else:
                result.append(_scalar(remainder))
        return result

    def literal(self, parent_indent: int) -> str:
        if self.position >= len(self.lines):
            return ""
        first_indent = _indent(self.lines[self.position])
        if first_indent <= parent_indent:
            return ""
        result: list[str] = []
        while self.position < len(self.lines):
            line = self.lines[self.position]
            if line.strip() and _indent(line) < first_indent:
                break
            result.append(line[first_indent:] if len(line) >= first_indent else "")
            self.position += 1
        return "\n".join(result) + "\n"


def parse_yaml_subset(text: str) -> dict[str, Any]:
    return _Parser(text).parse()


def _normalize_dynamic(value: Any) -> Any:
    if isinstance(value, dict):
        return {
            UUID_RE.sub("<uuid>", key): _normalize_dynamic(item)
            for key, item in value.items()
        }
    if isinstance(value, list):
        return [_normalize_dynamic(item) for item in value]
    if isinstance(value, str):
        return UUID_RE.sub("<uuid>", value)
    return value


def canonical_yaml_payload(text: str, *, normalize_dynamic: bool = False) -> str:
    value: Any = parse_yaml_subset(text)
    if normalize_dynamic:
        value = _normalize_dynamic(value)
    return json.dumps(value, ensure_ascii=True, sort_keys=True, separators=(",", ":"))


def canonical_yaml_sha256(text: str, *, normalize_dynamic: bool = False) -> str:
    return hashlib.sha256(
        canonical_yaml_payload(text, normalize_dynamic=normalize_dynamic).encode()
    ).hexdigest()


def yaml_lookup(document: dict[str, Any], dotted_path: str) -> Any:
    value: Any = document
    for part in dotted_path.split("."):
        if not isinstance(value, dict) or part not in value:
            raise YamlSubsetError(f"missing YAML path {dotted_path!r}")
        value = value[part]
    return value

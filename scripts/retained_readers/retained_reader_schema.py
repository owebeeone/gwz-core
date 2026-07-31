"""Small fail-closed JSON Schema evaluator for checked retained-reader inputs.

Only the Draft 2020-12 keywords used by this directory are implemented.  An
unknown schema keyword is an authoring error, never something silently ignored.
"""

from __future__ import annotations

import json
import re
import urllib.parse
from pathlib import Path
from typing import Any, Mapping


class SchemaValidationError(ValueError):
    """A checked document or its checked schema is invalid."""


_ANNOTATIONS = {"$schema", "$id", "$defs", "title", "description"}
_KEYWORDS = _ANNOTATIONS | {
    "$ref", "type", "const", "enum", "properties", "required",
    "additionalProperties", "minProperties", "items", "minItems",
    "uniqueItems", "minLength", "pattern", "format", "minimum", "maximum",
    "oneOf", "allOf", "if", "then", "else",
}


def load_schema(path: Path) -> dict[str, Any]:
    try:
        schema = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise SchemaValidationError(f"cannot load schema {path}: {error}") from error
    if not isinstance(schema, dict):
        raise SchemaValidationError(f"schema {path} must be an object")
    return schema


def validate(instance: Any, schema: Mapping[str, Any]) -> None:
    _validate(instance, schema, schema, "$")


def _resolve(root: Mapping[str, Any], reference: object, path: str) -> Mapping[str, Any]:
    if not isinstance(reference, str) or not reference.startswith("#/"):
        raise SchemaValidationError(f"{path}: unsupported $ref {reference!r}")
    value: Any = root
    for token in reference[2:].split("/"):
        token = token.replace("~1", "/").replace("~0", "~")
        if not isinstance(value, dict) or token not in value:
            raise SchemaValidationError(f"{path}: unresolved $ref {reference!r}")
        value = value[token]
    if not isinstance(value, dict):
        raise SchemaValidationError(f"{path}: $ref {reference!r} is not a schema")
    return value


def _is_type(value: Any, expected: str) -> bool:
    return {
        "object": lambda: isinstance(value, dict),
        "array": lambda: isinstance(value, list),
        "string": lambda: isinstance(value, str),
        "integer": lambda: isinstance(value, int) and not isinstance(value, bool),
        "number": lambda: isinstance(value, (int, float)) and not isinstance(value, bool),
        "boolean": lambda: isinstance(value, bool),
        "null": lambda: value is None,
    }.get(expected, lambda: False)()


def _validate(instance: Any, schema: Mapping[str, Any], root: Mapping[str, Any], path: str) -> None:
    unknown = set(schema) - _KEYWORDS
    if unknown:
        raise SchemaValidationError(f"{path}: schema has unsupported keyword(s): {sorted(unknown)}")
    if "$ref" in schema:
        _validate(instance, _resolve(root, schema["$ref"], path), root, path)

    if "type" in schema:
        expected = schema["type"]
        choices = expected if isinstance(expected, list) else [expected]
        if not choices or not all(isinstance(item, str) for item in choices):
            raise SchemaValidationError(f"{path}: schema type must be text or a non-empty array")
        if not any(_is_type(instance, item) for item in choices):
            raise SchemaValidationError(f"{path}: expected type {expected!r}")
    if "const" in schema and instance != schema["const"]:
        raise SchemaValidationError(f"{path}: expected constant {schema['const']!r}")
    if "enum" in schema and instance not in schema["enum"]:
        raise SchemaValidationError(f"{path}: {instance!r} is not one of {schema['enum']!r}")

    if isinstance(instance, dict):
        required = schema.get("required", [])
        if not isinstance(required, list) or not all(isinstance(item, str) for item in required):
            raise SchemaValidationError(f"{path}: schema required must be an array of strings")
        missing = [item for item in required if item not in instance]
        if missing:
            raise SchemaValidationError(f"{path}: required property missing: {', '.join(missing)}")
        minimum = schema.get("minProperties")
        if minimum is not None and len(instance) < minimum:
            raise SchemaValidationError(f"{path}: requires at least {minimum} properties")
        properties = schema.get("properties", {})
        if not isinstance(properties, dict):
            raise SchemaValidationError(f"{path}: schema properties must be an object")
        additional = schema.get("additionalProperties", True)
        for key, value in instance.items():
            child = f"{path}.{key}"
            if key in properties:
                _validate(value, properties[key], root, child)
            elif additional is False:
                raise SchemaValidationError(f"{child}: additional property {key!r} is not allowed")
            elif isinstance(additional, dict):
                _validate(value, additional, root, child)

    if isinstance(instance, list):
        minimum = schema.get("minItems")
        if minimum is not None and len(instance) < minimum:
            raise SchemaValidationError(f"{path}: requires at least {minimum} items")
        if schema.get("uniqueItems"):
            encoded = [json.dumps(item, ensure_ascii=True, sort_keys=True) for item in instance]
            if len(encoded) != len(set(encoded)):
                raise SchemaValidationError(f"{path}: items must be unique")
        items = schema.get("items")
        if isinstance(items, dict):
            for index, item in enumerate(instance):
                _validate(item, items, root, f"{path}[{index}]")

    if isinstance(instance, str):
        minimum = schema.get("minLength")
        if minimum is not None and len(instance) < minimum:
            raise SchemaValidationError(f"{path}: requires at least {minimum} characters")
        pattern = schema.get("pattern")
        if pattern is not None and re.search(pattern, instance) is None:
            raise SchemaValidationError(f"{path}: does not match pattern {pattern!r}")
        if schema.get("format") == "uri":
            parsed = urllib.parse.urlparse(instance)
            if not parsed.scheme or not parsed.netloc:
                raise SchemaValidationError(f"{path}: is not an absolute URI")
        elif schema.get("format") not in {None, "uri"}:
            raise SchemaValidationError(f"{path}: unsupported schema format {schema['format']!r}")

    if isinstance(instance, (int, float)) and not isinstance(instance, bool):
        if "minimum" in schema and instance < schema["minimum"]:
            raise SchemaValidationError(f"{path}: is less than minimum {schema['minimum']}")
        if "maximum" in schema and instance > schema["maximum"]:
            raise SchemaValidationError(f"{path}: exceeds maximum {schema['maximum']}")

    if "oneOf" in schema:
        matches = 0
        failures: list[str] = []
        for alternative in schema["oneOf"]:
            try:
                _validate(instance, alternative, root, path)
                matches += 1
            except SchemaValidationError as error:
                failures.append(str(error))
        if matches != 1:
            detail = "; ".join(failures) if failures else f"matched {matches} alternatives"
            raise SchemaValidationError(f"{path}: oneOf failed ({detail})")
    for conjunct in schema.get("allOf", []):
        _validate(instance, conjunct, root, path)
    if "if" in schema:
        try:
            _validate(instance, schema["if"], root, path)
            branch = schema.get("then")
        except SchemaValidationError:
            branch = schema.get("else")
        if branch is not None:
            _validate(instance, branch, root, path)

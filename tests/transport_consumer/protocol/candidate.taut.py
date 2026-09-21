"""Candidate GWZ placement schema composed from the two owning schemas.

This module is intentionally only a composition layer.  The production GWZ
schema and the transport-owner schema remain the only authored wire schemas;
the candidate adds the frozen placement projection in this isolated consumer
harness until activation is approved.
"""

from __future__ import annotations

import json
import os
from dataclasses import replace
from pathlib import Path

from taut.ir.load import load_schema, schema_from_json
from taut.ir.model import (
    EnumDef,
    EnumRef,
    FieldDef,
    ListOf,
    MessageDef,
    MsgRef,
    Scalar,
    Schema,
)


def _required_path(name: str) -> Path:
    value = os.environ.get(name)
    if not value:
        raise RuntimeError(f"{name} must name an explicit input to candidate composition")
    path = Path(value).resolve()
    if not path.is_file():
        raise RuntimeError(f"{name} does not exist: {path}")
    return path


def _add_fields(message: MessageDef, additions: tuple[FieldDef, ...]) -> MessageDef:
    fields = list(message.fields)
    by_name = {field.name: field for field in fields}
    by_tag = {field.tag: field for field in fields}
    for field in additions:
        existing_name = by_name.get(field.name)
        existing_tag = by_tag.get(field.tag)
        if existing_name is not None or existing_tag is not None:
            if existing_name == field and existing_tag == field:
                continue
            raise ValueError(
                f"candidate field collision in {message.name}: "
                f"{field.name}/{field.tag}"
            )
        fields.append(field)
        by_name[field.name] = field
        by_tag[field.tag] = field
    return replace(message, fields=tuple(fields))


def compose(core_path: Path, owner_path: Path) -> Schema:
    core = load_schema(core_path)
    owner = schema_from_json(json.loads(owner_path.read_text()))
    overlaps = (set(core.enums) | set(core.messages)) & (
        set(owner.enums) | set(owner.messages)
    )
    if overlaps:
        raise ValueError(f"core/owner declaration collision: {sorted(overlaps)}")

    enums = dict(owner.enums)
    enums["TransportPlacement"] = EnumDef(
        "TransportPlacement", {"local": 1, "cli": 2}
    )
    enums.update(core.enums)
    messages = dict(owner.messages)
    messages.update(core.messages)

    optional = True
    missing_ok = True
    messages["TransportOptions"] = _add_fields(
        messages["TransportOptions"],
        (
            FieldDef(
                "placement", 4, EnumRef("TransportPlacement"), optional, False, None,
                missing_ok
            ),
            FieldDef("endpoint_path_base", 5, Scalar("str"), optional, False, None, missing_ok),
        ),
    )
    messages["RequestMeta"] = _add_fields(
        messages["RequestMeta"],
        (
            FieldDef("transport_message", 10, MsgRef("Envelope"), optional, False, None, missing_ok),
        ),
    )
    messages["ResponseMeta"] = _add_fields(
        messages["ResponseMeta"],
        (
            FieldDef("transport_message", 9, MsgRef("Envelope"), optional, False, None, missing_ok),
        ),
    )
    messages["TransportCapabilitiesResponse"] = _add_fields(
        messages["TransportCapabilitiesResponse"],
        (
            FieldDef("message_versions", 3, ListOf(Scalar("int")), optional, False, None, missing_ok),
            FieldDef("placements", 4, ListOf(EnumRef("TransportPlacement")), optional, False, None, missing_ok),
            FieldDef("schemes", 5, ListOf(EnumRef("Scheme")), optional, False, None, missing_ok),
            FieldDef("auth_policies", 6, ListOf(EnumRef("AuthPolicy")), optional, False, None, missing_ok),
            FieldDef("message_limits", 7, MsgRef("Limits"), optional, False, None, missing_ok),
        ),
    )
    messages["TransportObservation"] = _add_fields(
        messages["TransportObservation"],
        (
            FieldDef("endpoint_id", 9, Scalar("str"), optional, False, None, missing_ok),
            FieldDef("connection_id", 10, Scalar("str"), optional, False, None, missing_ok),
            FieldDef("stream_id", 11, Scalar("int"), optional, False, None, missing_ok),
            FieldDef("reused", 12, Scalar("bool"), optional, False, None, missing_ok),
        ),
    )
    return Schema(
        enums=enums,
        messages=messages,
        services=dict(core.services),
        extensions=core.extensions,
    )


CORE_PATH = _required_path("GWZ_CORE_SCHEMA")
OWNER_PATH = _required_path("GWZ_TRANSPORT_SCHEMA")
SCHEMA = compose(CORE_PATH, OWNER_PATH)

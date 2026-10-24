"""Compatibility fixtures across the retained old readers and the current ordinary schema.

Plan step 2.2 made the placement projection part of the ordinary schema, `protocol/gwz.taut.py`, which composes
the transport owner's export. "New" below is that schema; "old" is it with the members of S7.1 step C2.1 and
step 2.2 removed (the repository's "measured additive" idiom), beside the retained decoders that never had them.
"""

import importlib.util
import hashlib
import json
import os
import sys
from pathlib import Path

import pytest

TAUT_SOURCE = Path(__file__).resolve().parents[4] / "taut" / "src"
sys.path.insert(0, str(TAUT_SOURCE))

ROOT = Path(__file__).resolve().parents[1]
CORE_SCHEMA = ROOT.parents[1] / "protocol" / "gwz.taut.py"
OWNER_SCHEMA = ROOT.parents[2] / "gwz-transport" / "protocol" / "transport.ir.json"

os.environ["GWZ_TRANSPORT_SCHEMA"] = str(OWNER_SCHEMA)

from dataclasses import replace  # noqa: E402

from taut.ir.load import load_schema, schema_from_json  # noqa: E402
from taut.ir.model import MISSING_OK, Schema  # noqa: E402
from taut.wire import codec  # noqa: E402

spec = importlib.util.spec_from_file_location("candidate_admission", Path(__file__).with_name("admission.py"))
assert spec is not None and spec.loader is not None
admission = importlib.util.module_from_spec(spec)
spec.loader.exec_module(admission)

RETAINED_CODEC_PATH = ROOT / "candidate" / "retained_taut" / "taut" / "wire" / "codec.py"
retained_spec = importlib.util.spec_from_file_location("taut.wire.retained_codec", RETAINED_CODEC_PATH)
assert retained_spec is not None and retained_spec.loader is not None
retained_codec = importlib.util.module_from_spec(retained_spec)
retained_spec.loader.exec_module(retained_codec)

# S7.1 (1.1.0) step C2.1 moved `OperationPolicy.max_retries` and the four
# `TransportObservation` receipts into the ordinary schema, and plan step 2.2
# moved the placement and attachment members there. The old reader is the
# ordinary schema with exactly those members removed (the repository's "measured
# additive" idiom); the Rust old reader is a checked-in file already.
C2_ADDITIONS = {
    "OperationPolicy": {"max_retries": 9},
    "TransportObservation": {
        "endpoint_id": 9,
        "connection_id": 10,
        "stream_id": 11,
        "reused": 12,
    },
}


PLACEMENT_ADDITIONS = {
    "TransportOptions": {"placement": 4, "endpoint_path_base": 5},
    "RequestMeta": {"transport_message": 10},
    "ResponseMeta": {"transport_message": 9},
    "TransportCapabilitiesResponse": {
        "message_versions": 3,
        "placements": 4,
        "schemes": 5,
        "auth_policies": 6,
        "message_limits": 7,
    },
}


def _without_members(schema: Schema, additions: dict, enums: tuple[str, ...] = ()) -> Schema:
    messages = dict(schema.messages)
    for message_name, fields in additions.items():
        message = messages[message_name]
        kept = tuple(field for field in message.fields if field.name not in fields)
        assert len(kept) == len(message.fields) - len(fields), message_name
        messages[message_name] = replace(message, fields=kept)
    return Schema(
        enums={name: enum for name, enum in schema.enums.items() if name not in enums},
        messages=messages,
        services=dict(schema.services),
        extensions=schema.extensions,
    )


# The ordinary schema, which composes the transport owner's export.
NEW = load_schema(CORE_SCHEMA)
PRODUCTION = NEW
# The ordinary schema as it was before plan step 2.2, and as an older reader had it before C2.1 too.
BEFORE_PLACEMENT = _without_members(NEW, PLACEMENT_ADDITIONS, enums=("TransportPlacement",))
OLD = _without_members(BEFORE_PLACEMENT, C2_ADDITIONS)
# The transport owner's declarations, which the ordinary schema composes
# whole: they are the owner's schema, not GWZ's.
OWNER = schema_from_json(json.loads(OWNER_SCHEMA.read_text()))


def _old_options():
    return {"default_identity": None, "remote_identities": [], "url_scheme": None}


def _old_capabilities():
    return {"file_identity": True, "exact_agent_identity": True}


def _old_policy():
    return {
        "partial": None,
        "destructive": None,
        "sync": None,
        "unsupported_member": None,
        "remote": None,
        "concurrency": 4,
        "progress_min_interval_ms": None,
        "max_connections_per_host": None,
    }


def _old_observation():
    return {
        "repository_path": "repos/core",
        "remote": "origin",
        "operation": "fetch",
        "credential_method": "agent",
        "selection_source": "ambient",
        "credential_offered": True,
        "authenticated": True,
        "public_key_fingerprint": None,
    }


def _receipts():
    return {
        "endpoint_id": "endpoint-1",
        "connection_id": "connection-1",
        "stream_id": 7,
        "reused": True,
    }


def _old_request_meta():
    return {
        "request_id": "request-1",
        "schema_version": "gwz.protocol/v0",
        "workspace": None,
        "selection": None,
        "policy": None,
        "dry_run": None,
        "attribution": None,
        "transport": None,
        "invocation": None,
    }


def test_placement_members_have_exact_additive_tags_and_missing_ok_contract():
    expected = PLACEMENT_ADDITIONS
    assert NEW.enums["TransportPlacement"].members == {"local": 1, "cli": 2}
    # `expected` is the whole of step 2.2's addition: every field the ordinary
    # schema has and the schema before the step lacked, outside the owner's
    # declarations, and the one enum it adds. A field added, removed or renamed
    # without it fails here. `max_retries` and the four receipts are C2.1's and
    # are pinned by `test_production_carries_the_1_1_0_fields_as_missing_ok`.
    added = {
        (message_name, field.name)
        for message_name, message in NEW.messages.items()
        if message_name not in OWNER.messages
        for field in message.fields
        if message_name not in BEFORE_PLACEMENT.messages
        or field.name not in {old.name for old in BEFORE_PLACEMENT.messages[message_name].fields}
    }
    assert added == {
        (message_name, field_name)
        for message_name, fields in expected.items()
        for field_name in fields
    }
    assert set(NEW.enums) - set(BEFORE_PLACEMENT.enums) - set(OWNER.enums) == {"TransportPlacement"}
    for message_name, fields in expected.items():
        actual = {field.name: field for field in NEW.messages[message_name].fields}
        for field_name, tag in fields.items():
            field = actual[field_name]
            assert field.tag == tag
            assert field.optional == MISSING_OK


def test_production_carries_the_1_1_0_fields_as_missing_ok():
    # C2.1: the ordinary protocol names the five members, at the candidate's
    # tags and with the candidate's presence contract, and the old reader is
    # exactly production without them.
    for message_name, fields in C2_ADDITIONS.items():
        production = {field.name: field for field in PRODUCTION.messages[message_name].fields}
        candidate = {field.name: field for field in NEW.messages[message_name].fields}
        old = {field.name for field in OLD.messages[message_name].fields}
        for field_name, tag in fields.items():
            assert production[field_name].tag == tag
            assert production[field_name].optional == MISSING_OK
            assert production[field_name] == candidate[field_name]
            assert field_name not in old


def test_old_production_request_and_capability_snapshots_are_stable():
    # These are generated by the retained production schema, then checked as
    # bytes so the compatibility proof cannot accidentally use the candidate
    # schema under an old-reader label.
    assert codec.encode(OLD, "TransportOptions", _old_options()).hex() == "a301f6028003f6"
    assert (
        codec.encode(OLD, "RequestMeta", _old_request_meta()).hex()
        == "a90169726571756573742d31026f67777a2e70726f746f636f6c2f763003f604f605f606f607f608f609f6"
    )
    assert codec.encode(OLD, "TransportCapabilitiesResponse", _old_capabilities()).hex() == "a201f502f5"


def test_new_writer_old_schema_reader_ignores_new_tags():
    value = {
        **_old_options(),
        "placement": "cli",
        "endpoint_path_base": "/client/workspace",
    }
    decoded = codec.decode(OLD, "TransportOptions", codec.encode(NEW, "TransportOptions", value))
    assert decoded["default_identity"] is None
    assert decoded["remote_identities"] == []
    assert decoded["url_scheme"] is None
    assert set(decoded["__unknown__"]) == {4, 5}


def test_retained_old_python_decoder_reads_local_and_explicit_cli_additive_fixtures():
    for placement in ("local", "cli"):
        value = {
            **_old_options(),
            "placement": placement,
            "endpoint_path_base": "/client/workspace",
        }
        decoded = retained_codec.decode(
            OLD, "TransportOptions", codec.encode(NEW, "TransportOptions", value)
        )
        assert decoded["default_identity"] is None
        assert set(decoded["__unknown__"]) == {4, 5}


def test_retained_old_rust_reader_is_the_pinned_checked_in_baseline():
    # The Rust old reader is a checked-in pre-placement file, never regenerated; its digest is pinned here (it was
    # the candidate generator's pin until plan step 2.2 retired that generator).
    pin = json.loads((ROOT / "candidate" / "retained-reader.json").read_text())
    retained = ROOT / "candidate" / "retained_old_generated.rs"
    assert hashlib.sha256(retained.read_bytes()).hexdigest() == pin["retained-old-rust-sha256"]


def test_retained_python_decoder_provenance_is_pinned():
    metadata = json.loads((RETAINED_CODEC_PATH.parents[2] / "PROVENANCE.json").read_text())
    assert metadata["revision"] == "733e8a78897a90f017f4726e4331aed95e8cb977"
    assert hashlib.sha256(RETAINED_CODEC_PATH.read_bytes()).hexdigest() == metadata["sha256"]


def test_retained_old_writer_new_schema_reader_defaults_new_fields():
    decoded = codec.decode(NEW, "TransportOptions", codec.encode(OLD, "TransportOptions", _old_options()))
    assert decoded["placement"] is None
    assert decoded["endpoint_path_base"] is None


def test_candidate_missing_null_and_malformed_values_have_distinct_rules():
    old_wire = codec.encode_struct(OLD, "TransportOptions", _old_options())
    absent = codec.decode_struct(NEW, "TransportOptions", old_wire, strict=True)
    assert absent["placement"] is None
    assert absent["endpoint_path_base"] is None

    null_wire = {**old_wire, 4: None, 5: None}
    null = codec.decode_struct(NEW, "TransportOptions", null_wire, strict=True)
    assert null["placement"] is None
    assert null["endpoint_path_base"] is None

    malformed = {**old_wire, 4: "cli"}
    with pytest.raises(codec.DecodeError):
        codec.decode_struct(NEW, "TransportOptions", malformed, strict=True)


def test_max_retries_crosses_the_retained_python_reader_and_absent_or_null_is_none():
    # A candidate writer's `--max-retries` is an unknown tag to the retained
    # old Python reader, which reads the rest of the policy.
    wire = codec.encode(NEW, "OperationPolicy", {**_old_policy(), "max_retries": 0})
    retained = retained_codec.decode(OLD, "OperationPolicy", wire)
    assert retained["concurrency"] == 4
    assert set(retained["__unknown__"]) == {9}
    assert codec.decode(NEW, "OperationPolicy", wire)["max_retries"] == 0
    # An older writer leaves the field absent, and null reads the same: no
    # budget, so the transport's default of three retries applies.
    old_wire = codec.encode_struct(OLD, "OperationPolicy", _old_policy())
    absent = codec.decode_struct(NEW, "OperationPolicy", old_wire, strict=True)
    null = codec.decode_struct(NEW, "OperationPolicy", {**old_wire, 9: None}, strict=True)
    assert absent["max_retries"] is None
    assert null["max_retries"] is None
    assert absent["concurrency"] == null["concurrency"] == 4


def test_receipts_cross_the_retained_python_reader_and_absent_or_null_are_none():
    # A new writer's receipts are unknown tags 9 to 12 to the old reader, which
    # reads the rest of the observation.
    wire = codec.encode(
        PRODUCTION, "TransportObservation", {**_old_observation(), **_receipts()}
    )
    retained = retained_codec.decode(OLD, "TransportObservation", wire)
    assert retained["remote"] == "origin"
    assert set(retained["__unknown__"]) == {9, 10, 11, 12}
    read = codec.decode(PRODUCTION, "TransportObservation", wire)
    assert {name: read[name] for name in _receipts()} == _receipts()
    # An older writer leaves them absent, and null reads the same.
    old_wire = codec.encode_struct(OLD, "TransportObservation", _old_observation())
    absent = codec.decode_struct(PRODUCTION, "TransportObservation", old_wire, strict=True)
    null = codec.decode_struct(
        PRODUCTION,
        "TransportObservation",
        {**old_wire, 9: None, 10: None, 11: None, 12: None},
        strict=True,
    )
    for decoded in (absent, null):
        assert all(decoded[name] is None for name in _receipts())
        assert decoded["remote"] == "origin"
    malformed = {**old_wire, 11: "seven"}
    with pytest.raises(codec.DecodeError):
        codec.decode_struct(PRODUCTION, "TransportObservation", malformed, strict=True)


def test_max_retries_is_production_and_an_older_writer_without_it_decodes():
    wire = codec.encode(PRODUCTION, "OperationPolicy", {**_old_policy(), "max_retries": 0})
    assert codec.decode(PRODUCTION, "OperationPolicy", wire)["max_retries"] == 0
    retained = retained_codec.decode(OLD, "OperationPolicy", wire)
    assert set(retained["__unknown__"]) == {9}
    old_wire = codec.encode_struct(OLD, "OperationPolicy", _old_policy())
    absent = codec.decode_struct(PRODUCTION, "OperationPolicy", old_wire, strict=True)
    assert absent["max_retries"] is None
    assert absent["concurrency"] == 4


def test_default_local_and_capability_gating_refuse_old_or_incomplete_results():
    assert admission.placement_or_local(None) == "local"
    capable = {
        "message_versions": [2],
        "placements": ["cli"],
        "schemes": ["ssh"],
        "auth_policies": ["ssh_ambient"],
        "message_limits": {
            "encoded_frame": 4096,
            "data_payload": 1024,
            "metadata_bytes": 256,
            "nesting": 10,
            "collection_entries": 128,
            "decode_allocation": 65536,
            "queued_bytes": 131072,
            "queued_frames": 8,
            "receive_window": 65536,
            "control_reserve_bytes": 4096,
            "control_reserve_frames": 2,
        },
    }
    assert admission.supports_cli(capable, "ssh_ambient")
    for key in ("message_versions", "placements", "schemes", "auth_policies", "message_limits"):
        incomplete = dict(capable)
        incomplete[key] = None
        assert not admission.supports_cli(incomplete, "ssh_ambient")
    unsafe = dict(capable)
    unsafe["message_limits"] = dict(capable["message_limits"])
    unsafe["message_limits"]["queued_frames"] = 0
    assert not admission.supports_cli(unsafe, "ssh_ambient")
    assert not admission.supports_cli(capable, "gh")

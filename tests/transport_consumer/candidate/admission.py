"""Pure Python mirror of the candidate host capability admission predicate."""

from __future__ import annotations


def placement_or_local(placement):
    return "local" if placement is None else placement


def supports_cli(capabilities: dict, policy: str) -> bool:
    """Return true only when every required capability is present and supported."""
    limits = capabilities.get("message_limits")
    usable_limits = _usable_limits(limits)
    return (
        2 in (capabilities.get("message_versions") or [])
        and "cli" in (capabilities.get("placements") or [])
        and "ssh" in (capabilities.get("schemes") or [])
        and policy in ("ssh_ambient", "ssh_explicit")
        and policy in (capabilities.get("auth_policies") or [])
        and usable_limits
    )


def _usable_limits(limits) -> bool:
    """Mirror owner codec bounds plus the owner binding's live minimums."""
    if not isinstance(limits, dict):
        return False
    names = (
        "encoded_frame",
        "data_payload",
        "metadata_bytes",
        "nesting",
        "collection_entries",
        "decode_allocation",
        "queued_bytes",
        "queued_frames",
        "receive_window",
        "control_reserve_bytes",
        "control_reserve_frames",
    )
    if any(type(limits.get(name)) is not int for name in names):
        return False
    if any(limits[name] <= 0 for name in names):
        return False
    if (
        limits["encoded_frame"] > 131072
        or limits["data_payload"] > 65536
        or limits["metadata_bytes"] > 16384
        or limits["nesting"] > 16
        or limits["collection_entries"] > 256
        or limits["decode_allocation"] > 524288
        or limits["queued_bytes"] > 4 * 1024 * 1024
        or limits["queued_frames"] > 64
        or limits["receive_window"] > limits["queued_bytes"]
        or limits["control_reserve_bytes"] >= limits["queued_bytes"]
        or limits["control_reserve_frames"] >= limits["queued_frames"]
    ):
        return False
    return (
        limits["encoded_frame"] >= 4096
        and limits["metadata_bytes"] >= 256
        and limits["nesting"] >= 10
        and limits["collection_entries"] >= 128
        and limits["decode_allocation"] >= 65536
        and limits["control_reserve_bytes"] >= 4096
        and limits["control_reserve_frames"] >= 2
        and limits["queued_bytes"] - limits["control_reserve_bytes"]
        >= limits["encoded_frame"] + limits["decode_allocation"]
        and limits["queued_frames"] - limits["control_reserve_frames"] >= 2
        and limits["receive_window"]
        <= limits["queued_bytes"] - limits["control_reserve_bytes"]
    )

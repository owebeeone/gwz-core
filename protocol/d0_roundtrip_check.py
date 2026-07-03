#!/usr/bin/env python3
"""D0-DRAFT tooling — NOT part of the release/regen flow.

Round-trips the proposed `gwz diff` D0 messages through the vendored taut wire
codec to prove they encode/decode losslessly before D0 is frozen. This exercises
the acceptance cases called out in the D0 brief and the design doc's corpus
catalog (dev-docs/GwzDiffD0Protocol.md §10):

  - a minimal DiffRequest (C1)
  - a DiffOutputRecord whose `data` carries NUL-laden binary bytes (C15)
  - a scoped DiffParsedTarget with left/right snapshot ids (C7)
  - a DiffExcludedTarget (C12/C13)
  - a rename DiffFileEntry with both old_path and new_path + similarity (C8)
  - the opt-in manifest-entry echo option round-trip (ruling #3): the new
    DiffOptions.echo_manifest_entries field, plus a DiffOutputRecord that carries
    the echoed DiffFileEntry when the option is requested (C21)

Run:
  PYTHONPATH=/Users/owebeeone/limbo/gwz-dev/taut/src \
      python3 protocol/d0_roundtrip_check.py

This is a standalone check, not a pytest/cargo target; it is intentionally not
wired into CI or regen.py. Delete once the design is frozen and the generated
corpus (`tautc corpus`) covers these messages.
"""

from __future__ import annotations

import sys
from pathlib import Path

from taut.ir.load import load_schema
from taut.wire import codec

SCHEMA_PATH = Path(__file__).resolve().parent / "gwz.taut.py"


def roundtrip(schema, message: str, value: dict) -> dict:
    """Encode -> decode via the wire codec; assert the value survives intact."""
    wire = codec.encode(schema, message, value)
    back = codec.decode(schema, message, wire)
    return {"wire_len": len(wire), "decoded": back}


def check(name: str, ok: bool, detail: str = "") -> None:
    mark = "ok  " if ok else "FAIL"
    print(f"  [{mark}] {name}" + (f" — {detail}" if detail else ""))
    if not ok:
        check.failed = True  # type: ignore[attr-defined]


check.failed = False  # type: ignore[attr-defined]


def main() -> int:
    schema = load_schema(str(SCHEMA_PATH))
    print(f"loaded schema: {len(schema.messages)} messages, {len(schema.enums)} enums")

    meta = {
        "request_id": "req-1",
        "schema_version": "0",
        "workspace": None,
        "selection": None,
        "policy": None,
        "dry_run": None,
        "attribution": None,
    }

    # C1 — minimal DiffRequest: meta + empty operand/pathspec lists, options absent.
    req = {
        "meta": meta,
        "workspace_cwd": None,
        "operands": [],
        "explicit_pathspecs": [],
        "options": None,
        "cached": None,
        "merge_base": None,
    }
    r = roundtrip(schema, "DiffRequest", req)
    check(
        "C1 minimal DiffRequest",
        r["decoded"]["operands"] == [] and r["decoded"]["meta"]["request_id"] == "req-1",
        f"{r['wire_len']}B",
    )

    # cached=true / merge_base=true are first-class (not operand tunnels).
    req2 = dict(req, cached=True, merge_base=True, operands=["A...B"])
    r = roundtrip(schema, "DiffRequest", req2)
    d = r["decoded"]
    check(
        "cached/merge_base first-class + A...B in operands",
        d["cached"] is True and d["merge_base"] is True and d["operands"] == ["A...B"],
    )

    # C15 — DiffOutputRecord.data carries exact bytes INCLUDING NULs + binary hunks.
    nul_bytes = b"@@ -1 +1 @@\n\x00\x01\x02patch\x00tail\xff\x00"
    rec = {
        "kind": "patch_bytes",
        "scope": {"root": None, "member_id": "mem_app", "member_path": "repos/app",
                  "source_kind": "git"},
        "file_id": "f1",
        "entry": None,
        "data": nul_bytes,
        "stale": None,
        "diagnostic": None,
    }
    r = roundtrip(schema, "DiffOutputRecord", rec)
    check(
        "C15 DiffOutputRecord NUL-laden BYTES survive",
        r["decoded"]["data"] == nul_bytes,
        f"{len(nul_bytes)} bytes incl. {nul_bytes.count(0)} NULs",
    )

    # stale_file record.
    stale = {
        "kind": "stale_file",
        "scope": {"root": True, "member_id": None, "member_path": None, "source_kind": None},
        "file_id": "f2",
        "entry": None,
        "data": None,
        "stale": True,
        "diagnostic": "worktree changed during render",
    }
    r = roundtrip(schema, "DiffOutputRecord", stale)
    check("stale_file record", r["decoded"]["kind"] == "stale_file" and r["decoded"]["stale"] is True)

    # C7 — scoped DiffParsedTarget with left/right snapshot ids preserved.
    tgt = {
        "target_id": "t0",
        "scope": {"root": None, "member_id": "mem_app", "member_path": "repos/app",
                  "source_kind": "git"},
        "comparison": {"kind": "tree_vs_tree", "left": "+base", "right": "+tip",
                       "merge_base": None},
        "pathspecs": ["src/"],
        "left_oid": "aaaa1111",
        "right_oid": "bbbb2222",
        "merge_base_oid": None,
        "left_snapshot_id": "base",
        "right_snapshot_id": "tip",
    }
    r = roundtrip(schema, "DiffParsedTarget", tgt)
    d = r["decoded"]
    check(
        "C7 scoped DiffParsedTarget w/ snapshot ids",
        d["left_snapshot_id"] == "base" and d["right_snapshot_id"] == "tip"
        and d["left_oid"] == "aaaa1111" and d["comparison"]["kind"] == "tree_vs_tree",
    )

    # C12/C13 — DiffExcludedTarget (member absent from snapshot).
    excl = {
        "scope": {"root": None, "member_id": "mem_new", "member_path": "repos/new",
                  "source_kind": "git"},
        "reason": "snapshot_missing",
        "snapshot_id": "base",
        "message": "member added after snapshot capture",
    }
    r = roundtrip(schema, "DiffExcludedTarget", excl)
    check(
        "C12/C13 DiffExcludedTarget",
        r["decoded"]["reason"] == "snapshot_missing" and r["decoded"]["snapshot_id"] == "base",
    )

    # C8 — rename DiffFileEntry carries BOTH paths + similarity, stays a rename.
    rename = {
        "file_id": "f3",
        "scope": {"root": True, "member_id": None, "member_path": None, "source_kind": None},
        "status": "renamed",
        "old_path": "src/old_name.rs",
        "new_path": "src/new_name.rs",
        "old_mode": 0o100644,
        "new_mode": 0o100644,
        "similarity": 92,
        "insertions": 3,
        "deletions": 1,
        "is_binary": None,
    }
    r = roundtrip(schema, "DiffFileEntry", rename)
    d = r["decoded"]
    check(
        "C8 rename entry keeps both paths + similarity",
        d["status"] == "renamed" and d["old_path"] == "src/old_name.rs"
        and d["new_path"] == "src/new_name.rs" and d["similarity"] == 92,
    )

    # Ruling #3 — the new opt-in DiffOptions.echo_manifest_entries field. Absent
    # (default off) and true must both round-trip; the field must exist on the
    # message so an unaware peer that omits it still decodes.
    opts_default = {f.name: None for f in schema.messages["DiffOptions"].fields}
    r = roundtrip(schema, "DiffOptions", opts_default)
    check(
        "ruling #3 echo_manifest_entries present + default off",
        "echo_manifest_entries" in r["decoded"]
        and r["decoded"]["echo_manifest_entries"] is None,
    )
    opts_echo = dict(opts_default, echo_manifest_entries=True)
    r = roundtrip(schema, "DiffOptions", opts_echo)
    check(
        "ruling #3 echo_manifest_entries=true round-trips",
        r["decoded"]["echo_manifest_entries"] is True,
    )

    # C21 — when the echo option is requested, a DiffOutputRecord carries the full
    # manifest entry on `entry`, and it survives the round-trip intact.
    echo_rec = {
        "kind": "file_started",
        "scope": {"root": True, "member_id": None, "member_path": None,
                  "source_kind": None},
        "file_id": "f3",
        "entry": rename,
        "data": None,
        "stale": None,
        "diagnostic": None,
    }
    r = roundtrip(schema, "DiffOutputRecord", echo_rec)
    d = r["decoded"]
    check(
        "C21 DiffOutputRecord.entry echo round-trips",
        d["kind"] == "file_started" and d["entry"] is not None
        and d["entry"]["status"] == "renamed"
        and d["entry"]["new_path"] == "src/new_name.rs"
        and d["entry"]["similarity"] == 92,
    )

    print()
    if check.failed:  # type: ignore[attr-defined]
        print("RESULT: FAIL")
        return 1
    print("RESULT: all diff D0 round-trips passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())

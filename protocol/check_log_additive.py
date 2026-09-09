#!/usr/bin/env python3
# DR-5 startup timeout: removing the service method and two messages exactly
# reproduces prior projection 9f338f2287cf7127b760b5dfaf4e86a5f5152fb38234fb9c5ca94949db3e271d.
# DR-5 observation fields: removing the message, three enums and two optional
# slots exactly reproduces prior projection f45ebbb8cfa1ed81f29cf18c4e6df03314ee45d4584229d2a8daa1b9e16bdc73.
# DR-5 local configuration: removing method, three messages, op enum and action 29
# exactly reproduces prior projection ab44d75d4ef6bca60864c7150c44f381c318aaa642db143aad619951fa4ff44a.
# DR-5 capability query: removing only its service method and two messages
# exactly reproduces previous projection 09f98f645608b84b2eb9dbaede79f2b0d3750e8e6c337f2b254eca2b0da990ce.
# DR-5 (2026-09-07): measured additive RemoteSshIdentity, TransportOptions,
# and optional RequestMeta.transport slot 8. Removing exactly these additions
# reproduced prior projection 6fd2f8829a920d6e4264a102f995a28ccc5d3dc47b25c66eca98980ad5488ca7.
"""Prove the gwz-log schema growth leaves every pre-log wire shape unchanged."""

from __future__ import annotations

import hashlib
import json
import sys
from copy import deepcopy
from pathlib import Path
from typing import Any

from taut.ir.export import schema_json
from taut.ir.load import load_schema


# Moved deliberately on 2026-09-03 by DR-1 ship (1) W1
# (dev-docs/GwzM5-8DR1-WarnOrRefuse-Charter.md §3.7): the charter adds
# MergeRequest.filesystem_strict (slot 8), MergeResponse.crash_recovery
# (slot 11), MergeCrashRecovery/MergeCrashRecoveryGap and EventKind.diagnostic
# (slot 8). All additive; no pre-existing slot changed. This pin still guards
# gwz-log against reshaping older messages.
#   was: d0c205c8767f8d54d32ead2f676a05077d849f6a12278d9de52b3c132c3c9372
#
# Moved deliberately again on 2026-09-03 by M5d step (3)
# (dev-docs/GwzM5-8M5d-Charter.md §3/§10.2): the charter allocates exactly one
# more optional response field, MergeCrashRecovery.handles_ok (slot 4). No
# version bump, no record or catalog format change. MEASURED additive, not
# assumed: the projection was rendered on both trees and diffed -- the only
# delta is the one new `handles_ok` field object, and the previous pin below
# reproduced exactly on the pre-change tree.
#   was: 7a66e301c5c0147a12c59b2cddb6f2ebc1515ef4d65297ec53c3b312a3769697
#
# Moved deliberately again on 2026-09-05 by LCM1.0c (gwz-dev
# dev-docs/GwzLocalCloneDesign.md §7, GwzLocalClonePlan.md §3 "1.0c"), which
# allocates the local-clone surface: ActionKind.clone_local_workspace (27) and
# local_family (28), the LocalCloneMode and LocalFamilyOp enums, the
# CloneLocalWorkspaceRequest/Response and LocalFamilyRequest/Response
# messages, the two matching GwzCore service methods, and one optional
# request field, MergeRequest.local_source_name (slot 9). MEASURED additive,
# not assumed: the projection was rendered on both trees and diffed -- 242
# added lines, 0 removed lines (249 diff lines with the 7 hunk headers), and
# every added object is one of the items named above; the previous pin below
# reproduced exactly on the pre-allocation tree (gwz-core 87207c2).
#   was: 71bf6b9223ba6d2b4d12049e425e567254ca79396d67922be737c86c6dd97a40
#
# Moved deliberately again on 2026-09-05 by LCM1.0c follow-up 2 (gwz-dev
# dev-docs/GwzLocalCloneDesign.md revision 9 §7 and §11 items 11-13, the
# operator's rulings on the LCM1.0c checkpoint's §7 questions 1-3), which
# allocates: CloneLocalWorkspaceRequest.copy_source (optional, tag 6, the
# `--from` selector), LocalFamilyResponse.members (tag 2, the `gwz local
# list` payload) with its LocalFamilyMemberEntry message and the
# LocalMemberKind / LocalMemberState / LocalObservedState enums, and
# GwzErrorCode.unknown_local (62). MEASURED additive, not assumed: the
# projection was rendered on both trees and diffed -- 128 added lines, 0
# removed lines, 5 hunks, every added object one of the items named above
# (the enum members appear as map keys, `unknown_local` among them); the
# previous pin below reproduced exactly on the pre-allocation tree
# (gwz-core 0d7b53d).
#   was: 3c34bd741b32f366f63928211eec83c920b5b0ca0ed1d847447f4d3428c22031
#
# Moved deliberately again on 2026-09-06 by LCM1.0c follow-up 3 (the
# operator's cross-driver ruling 3 of 2026-09-06, gwz-dev
# dev-docs/GwzLocalClone-LCM1.0c-Checkpoint.md §12 and GwzLocalCloneDesign.md
# §7/§8.1), which allocates exactly one more optional response field,
# LocalFamilyResponse.root_path (tag 3): the family root's path, so a
# driver resolves each member's root-relative `path` against it instead of
# guessing. MEASURED additive, not assumed: the projection was rendered on
# both trees and diffed -- 11 added lines, 0 removed lines, 1 hunk, the
# added lines being the one `root_path` field object; the previous pin
# below reproduced exactly on the pre-allocation tree (gwz-core 7e962d2).
#   was: 26f0d16ffebdcdc26bbbe682a6347688781cd202694333dbb0c066d087fb6b4e
#
# Moved deliberately again on 2026-09-06 by LCM1.1 fix 1 (lane C, gwz-dev
# dev-docs/GwzLocalClone-LCM1.0c-Checkpoint.md §14; GwzLocalCloneDesign.md
# §4, §4.0, §4.1, §12), which allocates exactly four more `GwzErrorCode`
# members for the local-create outcomes LCM1.1 had folded into
# `unsupported_operation` and `io_error`: unsupported_source_layout (63),
# copy_failed (64), source_drift (65) and destination_incomplete (66). No
# message, field or slot changed. MEASURED additive, not assumed: the
# projection was rendered on both trees and diffed -- 4 added lines, 0
# removed lines, 3 hunks, the added lines being the four enum members as map
# keys; the previous pin below reproduced exactly on the pre-allocation tree
# (gwz-core 81fcaf2).
#   was: 2eca6469ed1281e77a95f1e419aa4065002aa94c77507a73ada6f6f9c8bb5503
#
# Moved deliberately again on 2026-09-06 by LCM1.2 (lane C, gwz-dev
# dev-docs/GwzLocalClone-LCM1.0c-Checkpoint.md §16; GwzLocalCloneDesign.md
# §6, §6.2, §12), which allocates exactly two more `GwzErrorCode` members
# for the family-merge import outcomes: pairing_mismatch (67) and
# import_incomplete (68). No message, field or slot changed. MEASURED
# additive, not assumed: the projection was rendered on both trees and
# diffed -- 2 added lines, 0 removed lines, 2 hunks, the added lines being
# the two enum members as map keys; the previous pin below reproduced
# exactly on the pre-allocation tree (gwz-core 63f1332).
#   was: 0a173de982aaa93225e26581d678b4722356afc967fb4543de531708900cf981
#
# Moved deliberately again on 2026-09-06 by LCM2.1/LCM2.2 (lane C, gwz-dev
# dev-docs/GwzLocalClone-LCM1.0c-Checkpoint.md §17; GwzLocalCloneDesign.md
# §5, §5.1, §5.2, §12), which allocates exactly three more `GwzErrorCode`
# members for the ordinary-disposal outcomes: unwaived_hazard (69),
# unknown_evidence (70) and disposal_incomplete (71). No message, field or
# slot changed. MEASURED additive, not assumed: the projection was rendered
# on both trees and diffed -- 3 added lines, 0 removed lines, 2 hunks, the
# added lines being the three enum members as map keys; the previous pin
# below reproduced exactly on the pre-allocation tree (gwz-core 6d1a28e).
#   was: ba55594fa54123b865e06eb4bedfbf1eba4c9f52467a468831f9b699df0763a2
# Debt recovery DR-3 (2026-09-07) adds only GwzCore.resolve_forall_targets,
# reusing LsRequest/LsResponse. Measured: removing that single method from the
# new IR reproduces e99ce51a85b439fb03bb43df5beb3a33156048b8212d3f2fc609ba2db163db32
# exactly. No message, enum, field or existing method changes.
PRE_LOG_WIRE_SHA256 = "8aa25038218daf2d085b62bb37fb4438afd06bb77628746dac80efe53a56e76c"
LOG_METHODS = {"log", "log.output"}


def pre_log_projection(schema_ir: dict[str, Any]) -> dict[str, Any]:
    """Remove only S2.0's additive surface, retaining every older wire slot."""
    projected = deepcopy(schema_ir)
    projected["messages"] = [
        message for message in projected["messages"] if not message["name"].startswith("Log")
    ]
    projected["enums"] = [
        enum for enum in projected["enums"] if not enum["name"].startswith("Log")
    ]

    action_kind = next(enum for enum in projected["enums"] if enum["name"] == "ActionKind")
    log_slot = action_kind["members"].pop("log", None)
    if log_slot != 26:
        raise ValueError(f"ActionKind.log must use the next additive slot 26, got {log_slot}")

    service = next(service for service in projected["services"] if service["name"] == "GwzCore")
    service["methods"] = [
        method for method in service["methods"] if method["name"] not in LOG_METHODS
    ]
    # 2026-09-10 private-member policy adds exactly these optional booleans.
    # Removing them must reproduce the unchanged historical projection hash.
    for name, tag in (("MemberSpec", 8), ("RepoSyncRequest", 2)):
        message = next(m for m in projected["messages"] if m["name"] == name)
        added = [f for f in message["fields"] if f["name"] == "private"]
        expected = {"name": "private", "tag": tag,
                    "type": {"k": "scalar", "scalar": "bool"},
                    "optional": True, "transient": False, "merge": None}
        if added != [expected]:
            raise ValueError(f"{name}.private must be the optional boolean at tag {tag}")
        message["fields"].remove(added[0])
    return projected


def fingerprint(value: dict[str, Any]) -> str:
    encoded = json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def main() -> int:
    schema_path = Path(sys.argv[1] if len(sys.argv) > 1 else "protocol/gwz.taut.py")
    actual = fingerprint(pre_log_projection(schema_json(load_schema(schema_path))))
    if actual != PRE_LOG_WIRE_SHA256:
        print(
            "check_log_additive: pre-existing protocol wire projection changed\n"
            f"  expected: sha256:{PRE_LOG_WIRE_SHA256}\n"
            f"  actual:   sha256:{actual}",
            file=sys.stderr,
        )
        return 1
    print(f"check_log_additive: OK sha256:{actual}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

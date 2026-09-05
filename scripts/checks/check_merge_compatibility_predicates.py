#!/usr/bin/env python3
"""Validate the closed I2 archive corpus and wire-reason registry.

M5d (2026-09-05), operator ruling "remove the fragile stuff". This checker used
to validate a v0->v1 migration registry -- `migration_whitelist`,
`fixture_corpus`, `valid_unlisted_corpus`, `migration_policy` and
`descriptor_schema` -- whose rows named Rust test SOURCE FILES by path and
asserted the file, its `fn` and its subcase string existed. `55cf479` deleted
the v0 merge engine and its test corpus, so every one of those paths dangled
and the push-to-main gate went red on `characterization_v0.rs`.

The whole class is gone, not re-pointed: the five adapter sections are deleted
and this checker no longer resolves any registry value to a file on disk. It
takes no `--core`, imports no source tree, and cannot break on a rename. What
remains indexes DATA and WIRE CODES -- things that need an index and that no
test-name coupling can supply:

  * `normalization` -- the closed canonicalization rule, normative definition
    corpus and enum registry the archive/projection vocabulary is stated in.
  * `archive_corpus` -- the ten Table B archived-v0 decode shapes of the O8
    archive-equivalence decision, recorded by CLAUSE. Users' pre-0.14 history
    must keep decoding, so the M5d charter retains it.
  * `rejection_reasons` -- despite its name this is the wire-code reason
    registry that `dev-docs/GwzM5-8I2ProtocolContract.md` §1 binds codes 48-61
    to ("Allowed exact reasons are the matching lists in
    `GwzM5-8I2CompatibilityPredicates.json`; they are not free-form
    diagnostics"). It is protocol surface, not v0 scaffolding.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any, Mapping


SCHEMA = "gwz.merge-i2-compatibility-predicates/v3"
TOP_KEYS = {
    "schema",
    "normalization",
    # R2-E Phase E5.2, 2026-08-28. The standalone archive corpus of the O8
    # archive-equivalence mechanism decision. Archive rows are cited by clause,
    # in the shape `GwzM5-8R4bG-Evidence.md` §12.9's disposition table uses, and
    # validated here so the per-scenario record exists where this checker looks.
    "archive_corpus",
    "rejection_reasons",
}
NORMALIZATION_KEYS = {"identity", "definitions", "enums"}
EXPECTED_IDENTITY = (
    "Canonical JSON of descriptor only: UTF-8, keys sorted recursively, no insignificant "
    "whitespace. Fixture address and classification are excluded. Selected non-root identities "
    "are alpha-normalized in selection order to p0, p1, ...; their paths normalize to "
    "selected_path and branch names to attached_live_branch only after duplicate/collision and "
    "exact record-to-live relations are validated. @root remains @root."
)
EXPECTED_ENUMS = {
    "location": {"open"},
    "mode": {"normal"},
    "operation_state": {"finalizing"},
    "target_kind": {"member"},
    "participant_state": {"fast_forwarded", "up_to_date"},
    "result_relation": {"changed_exact", "equals_before"},
    "presence_relation": {"absent", "present_digest_valid"},
    "root_checkout": {"unborn_attached"},
    "publication_presence": {"absent", "present"},
    "publication_step": {"absent", "validating_results", "preparing_candidate", "committing_evidence", "publishing_candidate", "complete"},
    "candidate_relation": {"absent", "complete_valid"},
    "composition_relation": {"absent", "complete_valid"},
    "hash_relation": {"empty", "canonical_valid"},
    "root_observation": {"baseline_unborn", "unrecorded_evidence", "recorded_evidence", "prefix_boundary"},
    "base_phase": {"pre_candidate", "candidate_persisted", "evidence_unrecorded", "evidence_recorded", "publishing_prefix", "no_publication_complete"},
    "acceptance": {"construct_operation_baseline", "recover_candidate"},
    "metadata_source": {"operation_baseline"},
    "next_action": {"validate_results", "create_or_adopt_evidence", "publish_candidate", "complete_no_publication"},
}
EXPECTED_DEFINITIONS = {
    "present_digest_valid": "Bytes are present and their SHA-256 equals the paired recorded digest.",
    "complete_valid_candidate": "Candidate bytes, marker/path identities, baseline bytes, all internal SHA-256 values, workspace/root identities, and accepted-workspace relations are complete and mutually exact.",
    "complete_valid_composition": "Composition commit and tree are both present; parent, tree, message, candidate files, root merge input, and recorded candidate hashes cross-check exactly.",
    "canonical_valid_hashes": "The nonempty path/hash list is sorted, unique, complete for the candidate files, and every digest matches bytes and composition tree.",
    "result_changed_exact": "resulting_commit is present, differs from before_commit, equals the recorded source/result semantics for the participant state, and all three object ids are canonical and available.",
    "result_equals_before": "resulting_commit is present and equals before_commit; source/result/state are the exact durable unchanged-success relation and all object ids are canonical and available.",
    "attached_live_branch": "The recorded target branch is an existing attached local branch and the live symbolic HEAD names that exact branch.",
    "participant_live_exact": "No native action is active; HEAD and target ref equal the normalized recorded result; index and worktree are clean.",
    "baseline_unborn": "The root is unborn on the recorded attached branch and the recorded metadata files plus their index entries equal the pre-publication baseline form; unrelated pre-existing root state is outside this descriptor.",
    "unrecorded_evidence": "The root is one exact evidence commit derived from the accepted base, but that commit/tree has not yet been persisted in the record.",
    "recorded_evidence": "The live root exactly equals the persisted composition commit/tree and pre-publication candidate artifacts remain at baseline.",
    "prefix_boundary": "The persisted composition checkout has exact candidate marker, lock, boundary, and index publication entries; no other change to those publication paths exists.",
    "no_reverse_owner": "No preservation or rollback owner, artifact, prefix, ref, stash, bundle, or reverse mutation exists.",
}
# --- the standalone archive corpus (R2-E Phase E5.2, 2026-08-28) ----------
#
# The two-tier mechanism of the O8 archive-equivalence decision. Tier 1 is
# byte-preservation by digest for v0-origin archives -- "precisely what
# 'byte-preserving archival' asserts". Tier 2 is projection equivalence by
# canonical-JSON digest for archives an operation finished under v1, where
# byte equality is unavailable by construction. Both statuses are per row and
# per tier, so a row can never report the O8 archive clause met on a tier it
# has not executed.
#
# M5d (2026-09-05): a tier no longer carries `test`/`subcase`. Those two fields
# existed only to address a Rust source file, and the removed assertion class
# resolved them to disk. `status` plus `carrier` keeps the property that made
# the pair load-bearing -- an unexecuted tier must name the lane that owes it,
# and an executed one must name no carrier, so a row can never look discharged
# and owed at once. TIER_KEYS being exact is what now holds the class shut: a
# row that reintroduces a `test` field is rejected as an unregistered field.
ARCHIVE_KEYS = {"shape", "fixture", "disposition", "clause", "tier1", "tier2"}
TIER_KEYS = {"status", "carrier"}
ARCHIVE_DISPOSITIONS = {"byte-preserved-v0-origin", "pending-fixture"}
TIER_STATUSES = {"executed", "owed", "pending-fixture"}
# `GwzM5-8R4bG-Evidence.md` §12.4 "Table B -- the 10 archive shapes", in its
# own order. Closed: a shape the table does not name cannot be filed here, and
# a shape it names cannot be dropped.
ARCHIVE_SHAPES = [
    "AC-CANDIDATE",
    "AC-NOPUB-BORN",
    "AC-NOPUB-UNBORN",
    "AA-PREACCEPTANCE",
    "AA-CANDIDATE-COMPLETE",
    "AA-CANDIDATE-PARTIAL",
    "AP-PRESERVED",
    "AL-OPTIONAL-MISSING",
    "AL-UNKNOWN",
    "AR-C",
]
# The E0 §6.4 disposition, named: these two and only these two are
# DISPOSITIONED-PROJECTION-ONLY, UNFIXTURED, carried to R2-F. They are also
# two of C-2's four unfixtured scenarios; the other two (`B-NOT-STARTED`,
# `B-PREPARING-EMPTY`) are progress shapes and are not R2-E's at all.
ARCHIVE_PENDING_FIXTURE_SHAPES = {"AC-NOPUB-UNBORN", "AP-PRESERVED"}
EXPECTED_REASONS = {
    "UnexpectedAcceptanceEvidence": ["accepted workspace is present before complete participant validation", "publication evidence exists without accepted workspace"],
    "AcceptanceInputDrift": ["a selected participant result no longer matches its durable result", "the accepted metadata base cannot be verified from its recorded source", "the live root no longer matches the accepted pre-evidence checkout"],
    "CandidateIntegrityMismatch": ["candidate bytes or digest do not match accepted workspace", "candidate metadata base does not match accepted metadata base", "candidate cross-field identity or hash validation failed"],
    "AmbiguousEvidenceCommit": ["live root is neither the accepted base nor one exact unrecorded evidence commit", "more than one evidence-commit interpretation is valid"],
    "RecordedEvidenceDrift": ["recorded composition commit, tree, parent, message, files, or hashes changed"],
    "PublicationPrefixMismatch": ["filesystem or index does not match one legal recorded publication prefix"],
    "PublishedCandidateMismatch": ["published marker, lock, boundary, index, or evidence does not match the candidate"],
    "PreservationEvidenceMismatch": ["preservation evidence has no unique owner or action step", "preservation ref, stash, root prefix, bundle, or branch result is not exact"],
    "RollbackEvidenceMismatch": ["rollback evidence has no unique owner or action step", "participant, evidence, or selected-root rollback result is not exact"],
    "UnexpectedPublicationEvidence": ["no-publication completion contains candidate, composition, hash, or published-prefix evidence"],
    "TerminalEvidenceMismatch": ["completed record is not published or no-publication-complete"],
    "RecoveryEvidenceMismatch": ["recovery evidence does not match any legal origin", "recovery evidence matches more than one legal origin"],
    "TerminalRollbackMismatch": ["aborted record retains an incomplete or contradictory rollback action"],
    "ArchivedRecordUnreadable": ["archive envelope or terminal state is contradictory", "archive preservation ref is outside the canonical merge-owned namespace", "archive contains duplicate or colliding preservation owners"],
}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def exact_keys(value: Any, expected: set[str], path: str) -> Mapping[str, Any]:
    require(isinstance(value, dict), f"{path} must be an object")
    require(set(value) == expected, f"{path} fields must be exactly {sorted(expected)}")
    return value


def text(value: Any, path: str) -> str:
    require(isinstance(value, str) and value, f"{path} must be nonempty text")
    return value


def string_list(value: Any, path: str, *, nonempty: bool = True) -> list[str]:
    require(isinstance(value, list), f"{path} must be an array")
    require(not nonempty or value, f"{path} must be nonempty")
    require(all(isinstance(item, str) and item for item in value), f"{path} must contain text")
    require(len(value) == len(set(value)), f"{path} contains duplicates")
    return value


def load_json_exact(text_value: str) -> Any:
    def exact_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in pairs:
            require(key not in result, f"duplicate JSON key {key!r}")
            result[key] = value
        return result

    return json.loads(text_value, object_pairs_hook=exact_object)


def validate_archive_tier(tier: Any, path: str, *, expected_status: str | None = None) -> str:
    row = exact_keys(tier, TIER_KEYS, path)
    status = text(row["status"], f"{path}.status")
    require(status in TIER_STATUSES, f"{path}.status is not a registered tier status")
    require(
        expected_status is None or status == expected_status,
        f"{path}.status must be {expected_status!r} for this row's disposition",
    )
    if status == "executed":
        # An executed tier carries no carrier: nothing is owed elsewhere.
        require(row["carrier"] is None, f"{path}.carrier must be absent on an executed tier")
        return status
    # An unexecuted tier names the lane that owes it, so a row can never read
    # as discharged and owed at the same time.
    text(row["carrier"], f"{path}.carrier")
    return status


def validate_archive_corpus(corpus: Any) -> None:
    require(isinstance(corpus, list), "archive_corpus must be an array")
    shapes = [
        text(exact_keys(row, ARCHIVE_KEYS, f"archive_corpus[{index}]")["shape"], "shape")
        for index, row in enumerate(corpus)
    ]
    require(
        shapes == ARCHIVE_SHAPES,
        "archive_corpus must be exactly the ten Table B archive shapes, in table order",
    )
    executed_tier1 = 0
    pending: set[str] = set()
    for index, raw in enumerate(corpus):
        path = f"archive_corpus[{index}]"
        row = exact_keys(raw, ARCHIVE_KEYS, path)
        shape = row["shape"]
        text(row["fixture"], f"{path}.fixture")
        # The clause is the whole point of an archive row: it is recorded by
        # clause, not by registry membership. Content-anchored per the R2-E
        # citing rule, so a bare line number cannot stand alone.
        clause = text(row["clause"], f"{path}.clause")
        require(
            "GwzM5-8I2CompatibilityContract.md" in clause and "§" in clause and '"' in clause,
            f"{path}.clause must cite the frozen contract content-anchored: § plus a quoted anchor",
        )
        disposition = text(row["disposition"], f"{path}.disposition")
        require(
            disposition in ARCHIVE_DISPOSITIONS,
            f"{path}.disposition is not a registered archive disposition",
        )
        if disposition == "pending-fixture":
            pending.add(shape)
            require(row["fixture"] == "none", f"{path}.fixture must be 'none' when unfixtured")
            validate_archive_tier(row["tier1"], f"{path}.tier1", expected_status="pending-fixture")
            validate_archive_tier(row["tier2"], f"{path}.tier2", expected_status="pending-fixture")
            continue
        # A fixtured v0-origin row owes its tier-1 byte-digest proof now.
        validate_archive_tier(row["tier1"], f"{path}.tier1", expected_status="executed")
        executed_tier1 += 1
        validate_archive_tier(row["tier2"], f"{path}.tier2")
    # The E0.2b §8 [P2-2] denominators, machine-enforced: 8 archive-corpus rows
    # plus 2 PENDING-FIXTURE, and the pending pair is exactly the §6.4 pair.
    require(
        executed_tier1 == 8,
        f"archive_corpus must carry exactly 8 tier-1-executed rows, found {executed_tier1}",
    )
    require(
        pending == ARCHIVE_PENDING_FIXTURE_SHAPES,
        f"archive_corpus PENDING-FIXTURE rows must be exactly {sorted(ARCHIVE_PENDING_FIXTURE_SHAPES)}",
    )


def validate(document: Any) -> None:
    root = exact_keys(document, TOP_KEYS, "registry")
    require(root["schema"] == SCHEMA, f"registry schema must be {SCHEMA!r}")

    normalization = exact_keys(root["normalization"], NORMALIZATION_KEYS, "normalization")
    require(
        normalization["identity"] == EXPECTED_IDENTITY,
        "normalization.identity must equal the closed canonicalization rule",
    )
    definitions = normalization["definitions"]
    require(isinstance(definitions, dict) and definitions, "normalization.definitions must be nonempty")
    for name, definition in definitions.items():
        text(name, "normalization.definitions key")
        text(definition, f"normalization.definitions.{name}")
    require(
        definitions == EXPECTED_DEFINITIONS,
        "normalization.definitions must equal the closed normative definition corpus",
    )
    raw_enums = normalization["enums"]
    require(isinstance(raw_enums, dict) and raw_enums, "normalization.enums must be nonempty")
    enums = {name: set(string_list(values, f"normalization.enums.{name}")) for name, values in raw_enums.items()}
    require(enums == EXPECTED_ENUMS, "normalization.enums must equal the closed I2 enum registry")

    validate_archive_corpus(root["archive_corpus"])

    reasons = root["rejection_reasons"]
    require(isinstance(reasons, dict) and reasons, "rejection_reasons must be nonempty")
    for code, values in reasons.items():
        text(code, "rejection_reasons key")
        string_list(values, f"rejection_reasons.{code}")
    require(reasons == EXPECTED_REASONS, "rejection_reasons must equal the closed protocol reason corpus")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("registry", type=Path)
    args = parser.parse_args()
    document = load_json_exact(args.registry.read_text(encoding="utf-8"))
    validate(document)
    print(
        "validated the closed normalization corpus, "
        f"{len(document['archive_corpus'])} archive shapes, and "
        f"{len(document['rejection_reasons'])} rejection reasons"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

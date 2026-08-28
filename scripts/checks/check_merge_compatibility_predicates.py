#!/usr/bin/env python3
"""Validate the closed I2 v0-to-v1 migration whitelist and fixture corpus."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path
from typing import Any, Mapping


SCHEMA = "gwz.merge-i2-compatibility-predicates/v2"
DIGEST_RE = re.compile(r"[0-9a-f]{64}")
TOP_KEYS = {
    "schema",
    "normalization",
    "migration_policy",
    "descriptor_schema",
    "migration_whitelist",
    "fixture_corpus",
    "valid_unlisted_corpus",
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
POLICY_KEYS = {
    "match",
    "valid_unlisted",
    "recovery_required",
    "terminal",
    "no_match",
}
SCHEMA_KEYS = {
    "top_level",
    "operation",
    "selection",
    "participant",
    "pending",
    "baseline",
    "publication",
    "observation",
    "participant_observation",
}
DESCRIPTOR_KEYS = {
    "location",
    "mode",
    "operation",
    "selection",
    "participants",
    "baseline",
    "publication",
    "observation",
}
OPERATION_KEYS = {"state", "drift"}
SELECTION_KEYS = {"ordered_ids", "root_selected"}
PARTICIPANT_KEYS = {
    "id",
    "path",
    "target_kind",
    "target_branch",
    "state",
    "result",
    "pending",
    "conflict",
    "error",
    "preservation",
    "drift",
}
PENDING_KEYS = {"kind", "expected", "commit_spec"}
BASELINE_KEYS = {
    "lock_yaml",
    "manifest_yaml",
    "lock_commit_hash",
    "manifest_commit_hash",
    "root_checkout",
    "root_commit_hash",
}
PUBLICATION_KEYS = {
    "presence",
    "step",
    "candidate",
    "composition",
    "hashes",
    "root_merge",
    "evidence_rolled_back",
    "root_preservation",
    "preservation_prefix",
}
OBSERVATION_KEYS = {"participants", "root", "preservation", "rollback"}
PARTICIPANT_OBSERVATION_KEYS = {
    "id",
    "action",
    "head",
    "target_ref",
    "index",
    "worktree",
}
CLASSIFICATION_KEYS = {"base_phase", "acceptance", "metadata_source", "next_action"}
RULE_KEYS = {"id", "descriptor_sha256", "descriptor", "classification"}
FIXTURE_KEYS = {"case_id", "test", "subcase", "rule"}
UNLISTED_KEYS = {"case_id", "test", "subcase", "operation_state", "reason"}
# Every non-`finalizing` open state a valid-unlisted corpus row may declare.
# R2-E Phase E5.1, 2026-08-28: the three pre-acceptance states join the five
# the corpus already carried, for `GwzM5-8R4bG-Evidence.md` §12.9(d)'s ten
# unbound progress rows. This EXTENDS the corpus vocabulary and does not weaken
# it: the load-bearing closure the corpus rests on is `assert_ne!(record.state,
# OperationState::Finalizing)` read against "every whitelist rule is
# open+finalizing" (`compatibility_v0.rs`), and every state listed here is
# non-`finalizing`, so a row declaring one still cannot match a rule. §12.9(c)'s
# ruling that widening the corpus "to admit these rows would weaken the
# registry, not extend it" is about `finalizing` shapes specifically, and
# `finalizing` is deliberately still absent below.
VALID_UNLISTED_STATES = {
    "aborted",
    "awaiting_resolution",
    "completed",
    "executing",
    "halted",
    "preserving",
    "recovery_required",
    "rolling_back",
}
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
EXPECTED_POLICY = {
    "match": "After complete structural validation, normalize record plus live observation and require byte-for-byte canonical descriptor equality with exactly one whitelist rule.",
    "valid_unlisted": "A structurally valid v0 descriptor not listed here is not corrupt and is never staged for migration. Open read-only status remains byte-exact and projects source/version with acceptance and recovery absent. Mutating commands remain on the existing v0 lifecycle and may write v0 only when that released path's existing preflight authorizes the mutation. Archived v0 uses only the separately frozen archive decoder and legacy projection.",
    "recovery_required": "No v0 recovery_required, preserving, or rolling_back descriptor is migration-eligible in A1. Origin/owner ambiguity is therefore outside the matcher and cannot be manufactured away by whitelist membership.",
    "terminal": "Completed and aborted v0 records remain v0 and use the existing byte-preserving archive path.",
    "no_match": "Zero matches means valid-unlisted v0, not a compatibility error. Multiple matches is a registry defect and fails the build/checker.",
}
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


def canonical(value: Any) -> str:
    return json.dumps(value, ensure_ascii=True, sort_keys=True, separators=(",", ":"))


def load_json_exact(text_value: str) -> Any:
    def exact_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in pairs:
            require(key not in result, f"duplicate JSON key {key!r}")
            result[key] = value
        return result

    return json.loads(text_value, object_pairs_hook=exact_object)


def descriptor_digest(value: Any) -> str:
    return hashlib.sha256(canonical(value).encode()).hexdigest()


def fixture_source(core: Path, fixture: str) -> tuple[Path, str]:
    parts = fixture.split("::")
    require(
        len(parts) >= 4 and parts[0] == "workspace_ops",
        f"fixture {fixture!r} is not an exact workspace_ops test path",
    )
    function = parts[-1]
    require(
        re.fullmatch(r"[a-z][a-z0-9_]*", function) is not None,
        f"fixture {fixture!r} has an invalid test name",
    )
    source = core / "src" / "workspace_ops" / Path(*parts[1:-1]).with_suffix(".rs")
    return source, function


def validate_fixture(core: Path, row: Mapping[str, Any], path: str) -> None:
    fixture = text(row["test"], f"{path}.test")
    subcase = text(row["subcase"], f"{path}.subcase")
    source, function = fixture_source(core, fixture)
    require(source.is_file(), f"{path}.test source does not exist: {source}")
    source_text = source.read_text(encoding="utf-8")
    require(
        re.search(rf"(?m)^fn\s+{re.escape(function)}\s*\(", source_text) is not None,
        f"{path}.test function does not exist",
    )
    require(
        f'"{subcase}"' in source_text,
        f"{path}.subcase {subcase!r} is not exported by its Rust test",
    )


def enum(enums: Mapping[str, set[str]], name: str, value: Any, path: str) -> str:
    require(value in enums[name], f"{path} is not a registered {name}")
    return value


def expected_classification(descriptor: Mapping[str, Any]) -> Mapping[str, str]:
    publication = descriptor["publication"]
    result = descriptor["participants"][0]["result"]
    shape = (
        publication["presence"],
        publication["step"],
        publication["candidate"],
        publication["composition"],
        publication["hashes"],
        descriptor["observation"]["root"],
        result,
    )
    table = {
        ("absent", "absent", "absent", "absent", "empty", "baseline_unborn", "changed_exact"):
            ("pre_candidate", "construct_operation_baseline", "validate_results"),
        ("present", "validating_results", "absent", "absent", "empty", "baseline_unborn", "changed_exact"):
            ("pre_candidate", "construct_operation_baseline", "validate_results"),
        ("present", "preparing_candidate", "complete_valid", "absent", "empty", "baseline_unborn", "changed_exact"):
            ("candidate_persisted", "recover_candidate", "create_or_adopt_evidence"),
        ("present", "committing_evidence", "complete_valid", "absent", "empty", "unrecorded_evidence", "changed_exact"):
            ("evidence_unrecorded", "recover_candidate", "create_or_adopt_evidence"),
        ("present", "committing_evidence", "complete_valid", "complete_valid", "canonical_valid", "recorded_evidence", "changed_exact"):
            ("evidence_recorded", "recover_candidate", "publish_candidate"),
        ("present", "publishing_candidate", "complete_valid", "complete_valid", "canonical_valid", "prefix_boundary", "changed_exact"):
            ("publishing_prefix", "recover_candidate", "publish_candidate"),
        ("present", "complete", "absent", "absent", "empty", "baseline_unborn", "equals_before"):
            ("no_publication_complete", "construct_operation_baseline", "complete_no_publication"),
    }
    require(shape in table, f"descriptor publication/observation shape is not migration-eligible: {shape!r}")
    base, acceptance, action = table[shape]
    return {
        "base_phase": base,
        "acceptance": acceptance,
        "metadata_source": "operation_baseline",
        "next_action": action,
    }


def validate_descriptor(
    descriptor: Any, enums: Mapping[str, set[str]], path: str
) -> Mapping[str, Any]:
    row = exact_keys(descriptor, DESCRIPTOR_KEYS, path)
    enum(enums, "location", row["location"], f"{path}.location")
    enum(enums, "mode", row["mode"], f"{path}.mode")
    operation = exact_keys(row["operation"], OPERATION_KEYS, f"{path}.operation")
    enum(enums, "operation_state", operation["state"], f"{path}.operation.state")
    require(operation["drift"] == [], f"{path}.operation.drift must be empty")

    selection = exact_keys(row["selection"], SELECTION_KEYS, f"{path}.selection")
    ids = string_list(selection["ordered_ids"], f"{path}.selection.ordered_ids")
    require(ids == ["p0"], f"{path} identities must be the one-member alpha-normalized whitelist shape")
    require(selection["root_selected"] is False, f"{path}.selection.root_selected must be false in the A1 whitelist")

    participants = row["participants"]
    require(isinstance(participants, list) and len(participants) == len(ids), f"{path}.participants must cover selection exactly")
    for index, raw in enumerate(participants):
        participant = exact_keys(raw, PARTICIPANT_KEYS, f"{path}.participants[{index}]")
        require(participant["id"] == ids[index], f"{path}.participants are not in selection order")
        require(participant["path"] == "selected_path", f"{path}.participants[{index}].path is not alpha-normalized")
        enum(enums, "target_kind", participant["target_kind"], f"{path}.participants[{index}].target_kind")
        require(participant["target_branch"] == "attached_live_branch", f"{path}.participants[{index}].target_branch is invalid")
        enum(enums, "participant_state", participant["state"], f"{path}.participants[{index}].state")
        enum(enums, "result_relation", participant["result"], f"{path}.participants[{index}].result")
        require(exact_keys(participant["pending"], PENDING_KEYS, f"{path}.participants[{index}].pending") == {"kind": "absent", "expected": "absent", "commit_spec": "absent"}, f"{path}.participants[{index}].pending must be absent")
        for key in ("conflict", "error", "preservation"):
            require(participant[key] == "absent", f"{path}.participants[{index}].{key} must be absent")
        require(participant["drift"] == [], f"{path}.participants[{index}].drift must be empty")

    baseline = exact_keys(row["baseline"], BASELINE_KEYS, f"{path}.baseline")
    for key in ("lock_yaml", "manifest_yaml", "lock_commit_hash", "manifest_commit_hash", "root_commit_hash"):
        enum(enums, "presence_relation", baseline[key], f"{path}.baseline.{key}")
    require(baseline["lock_yaml"] == baseline["manifest_yaml"] == "present_digest_valid", f"{path}.baseline bytes must be present and valid")
    require(baseline["lock_commit_hash"] == baseline["manifest_commit_hash"] == baseline["root_commit_hash"] == "absent", f"{path}.baseline unborn commit fields must be absent")
    enum(enums, "root_checkout", baseline["root_checkout"], f"{path}.baseline.root_checkout")

    publication = exact_keys(row["publication"], PUBLICATION_KEYS, f"{path}.publication")
    enum(enums, "publication_presence", publication["presence"], f"{path}.publication.presence")
    enum(enums, "publication_step", publication["step"], f"{path}.publication.step")
    enum(enums, "candidate_relation", publication["candidate"], f"{path}.publication.candidate")
    enum(enums, "composition_relation", publication["composition"], f"{path}.publication.composition")
    enum(enums, "hash_relation", publication["hashes"], f"{path}.publication.hashes")
    require(publication["root_merge"] == "absent", f"{path}.publication.root_merge must be absent")
    require(publication["evidence_rolled_back"] is False, f"{path}.publication.evidence_rolled_back must be false")
    require(publication["root_preservation"] == publication["preservation_prefix"] == "absent", f"{path}.publication reverse evidence must be absent")

    observation = exact_keys(row["observation"], OBSERVATION_KEYS, f"{path}.observation")
    observations = observation["participants"]
    require(isinstance(observations, list) and len(observations) == len(ids), f"{path}.observation.participants must cover selection exactly")
    for index, raw in enumerate(observations):
        item = exact_keys(raw, PARTICIPANT_OBSERVATION_KEYS, f"{path}.observation.participants[{index}]")
        require(item == {"id": ids[index], "action": "none", "head": "equals_result", "target_ref": "equals_result", "index": "clean", "worktree": "clean"}, f"{path}.observation.participants[{index}] is not exact")
    enum(enums, "root_observation", observation["root"], f"{path}.observation.root")
    require(observation["preservation"] == observation["rollback"] == "none", f"{path}.observation reverse owners must be none")
    return row


def validate(document: Any, core: Path) -> None:
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

    policy = exact_keys(root["migration_policy"], POLICY_KEYS, "migration_policy")
    for name, value in policy.items():
        text(value, f"migration_policy.{name}")
    require(policy == EXPECTED_POLICY, "migration_policy must equal the closed policy corpus")
    schema = exact_keys(root["descriptor_schema"], SCHEMA_KEYS, "descriptor_schema")
    expected_schema = {
        "top_level": DESCRIPTOR_KEYS,
        "operation": OPERATION_KEYS,
        "selection": SELECTION_KEYS,
        "participant": PARTICIPANT_KEYS,
        "pending": PENDING_KEYS,
        "baseline": BASELINE_KEYS,
        "publication": PUBLICATION_KEYS,
        "observation": OBSERVATION_KEYS,
        "participant_observation": PARTICIPANT_OBSERVATION_KEYS,
    }
    for name, fields in schema.items():
        require(set(string_list(fields, f"descriptor_schema.{name}")) == expected_schema[name], f"descriptor_schema.{name} is not exact")

    rules = root["migration_whitelist"]
    require(isinstance(rules, list) and rules, "migration_whitelist must be nonempty")
    rule_ids: set[str] = set()
    identities: set[str] = set()
    for index, raw in enumerate(rules):
        path = f"migration_whitelist[{index}]"
        rule = exact_keys(raw, RULE_KEYS, path)
        rule_id = text(rule["id"], f"{path}.id")
        require(rule_id not in rule_ids, f"duplicate rule id {rule_id!r}")
        rule_ids.add(rule_id)
        descriptor = validate_descriptor(rule["descriptor"], enums, f"{path}.descriptor")
        digest = text(rule["descriptor_sha256"], f"{path}.descriptor_sha256")
        require(DIGEST_RE.fullmatch(digest) is not None and digest == descriptor_digest(descriptor), f"{path}.descriptor_sha256 does not match canonical descriptor")
        identity = canonical(descriptor)
        require(identity not in identities, f"duplicate semantic descriptor at {path}")
        identities.add(identity)
        classification = exact_keys(rule["classification"], CLASSIFICATION_KEYS, f"{path}.classification")
        for key in CLASSIFICATION_KEYS:
            enum(enums, key, classification[key], f"{path}.classification.{key}")
        require(classification == expected_classification(descriptor), f"{path}.classification does not match the executable shape rule")

    corpus = root["fixture_corpus"]
    require(isinstance(corpus, list) and corpus, "fixture_corpus must be nonempty")
    case_ids: set[str] = set()
    fixture_bindings: set[tuple[str, str]] = set()
    coverage: dict[str, int] = {rule_id: 0 for rule_id in rule_ids}
    for index, raw in enumerate(corpus):
        path = f"fixture_corpus[{index}]"
        row = exact_keys(raw, FIXTURE_KEYS, path)
        case_id = text(row["case_id"], f"{path}.case_id")
        require(case_id not in case_ids, f"duplicate fixture case id {case_id!r}")
        case_ids.add(case_id)
        validate_fixture(core, row, path)
        binding = (row["test"], row["subcase"])
        require(binding not in fixture_bindings, f"duplicate fixture test/subcase {binding!r}")
        fixture_bindings.add(binding)
        require(row["rule"] in rule_ids, f"{path}.rule is unknown")
        coverage[row["rule"]] += 1
    require(all(count == 1 for count in coverage.values()), f"fixture coverage must bind every whitelist rule exactly once: {coverage}")

    unlisted = root["valid_unlisted_corpus"]
    require(isinstance(unlisted, list) and unlisted, "valid_unlisted_corpus must be nonempty")
    for index, raw in enumerate(unlisted):
        path = f"valid_unlisted_corpus[{index}]"
        row = exact_keys(raw, UNLISTED_KEYS, path)
        case_id = text(row["case_id"], f"{path}.case_id")
        require(case_id not in case_ids, f"duplicate corpus case id {case_id!r}")
        case_ids.add(case_id)
        validate_fixture(core, row, path)
        require(
            row["operation_state"] in VALID_UNLISTED_STATES,
            f"{path}.operation_state is not an exact valid-unlisted state",
        )
        text(row["reason"], f"{path}.reason")

    reasons = root["rejection_reasons"]
    require(isinstance(reasons, dict) and reasons, "rejection_reasons must be nonempty")
    for code, values in reasons.items():
        text(code, "rejection_reasons key")
        string_list(values, f"rejection_reasons.{code}")
    require(reasons == EXPECTED_REASONS, "rejection_reasons must equal the closed protocol reason corpus")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("registry", type=Path)
    parser.add_argument("--core", type=Path, required=True)
    args = parser.parse_args()
    document = load_json_exact(args.registry.read_text(encoding="utf-8"))
    validate(document, args.core.resolve())
    print(
        f"validated {len(document['migration_whitelist'])} migration rules and "
        f"{len(document['fixture_corpus'])} runtime bindings"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

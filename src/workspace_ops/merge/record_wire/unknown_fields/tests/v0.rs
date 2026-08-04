use super::super::UnknownFieldManifest;
use super::{raw, unknown_value};

#[test]
fn v0_manifest_records_common_unknowns_before_migration() {
    let value = raw(r#"
schema: gwz.merge-operation/v0
record_schema_version: 0
future_record: record
baseline: {future_baseline: baseline}
participants:
  mem_a:
    future_participant: participant
    conflict_snapshot:
      - {path: a, sha256: digest, future_conflict: conflict}
    pending_action:
      kind: true_merge
      target_branch: main
      before_commit: before
      source_commit: source
      commit_message: message
      future_action: action
    preservation:
      - {backup_ref: backup, backup_commit: before, stash_id: null, stash_object_id: null, future_preservation: preservation}
    drift:
      - {kind: branch_changed, message: diagnostic, future_drift: drift}
publication:
  future_publication: publication
  candidate_hashes:
    - {path: lock, sha256: digest, future_hash: hash}
  candidate:
    future_candidate: candidate
operation_drift:
  - {kind: record_unreadable, message: diagnostic, future_operation_drift: operation-drift}
"#);
    let manifest = UnknownFieldManifest::extract_v0(&value).unwrap();
    for field in [
        "future_record",
        "future_baseline",
        "future_participant",
        "future_conflict",
        "future_action",
        "future_preservation",
        "future_drift",
        "future_publication",
        "future_hash",
        "future_candidate",
        "future_operation_drift",
    ] {
        assert!(unknown_value(&manifest, field).is_some(), "missing {field}");
    }
    assert_eq!(manifest.map_v0_to_v1().unwrap(), manifest);
}

#[test]
fn every_v1_top_level_collision_makes_v0_migration_ineligible() {
    for field in [
        "accepted_workspace",
        "recovery_context",
        "pending_rollback",
        "pending_preservation",
    ] {
        let value = raw(&format!("{field}: {{future: retained}}\n"));
        let manifest = UnknownFieldManifest::extract_v0(&value).unwrap();
        let error = manifest.map_v0_to_v1().unwrap_err();
        assert!(error.detail.contains(field));
        assert!(error.detail.contains("collides"));
    }
}

#[test]
fn v0_sequence_unknowns_use_the_same_identity_aware_overlay() {
    let source = raw(r#"
publication:
  candidate_hashes:
    - {path: a, sha256: one, future: from-a}
    - {path: b, sha256: two, future: from-b}
"#);
    let mut v1 = raw(r#"
publication:
  candidate_hashes:
    - {path: b, sha256: two}
    - {path: a, sha256: one}
"#);
    let migrated = UnknownFieldManifest::extract_v0(&source)
        .unwrap()
        .map_v0_to_v1()
        .unwrap();
    migrated.apply_surviving(&mut v1).unwrap();
    let rows = v1["publication"]["candidate_hashes"].as_sequence().unwrap();
    assert_eq!(rows[0]["future"], "from-b");
    assert_eq!(rows[1]["future"], "from-a");
}

use serde_yaml::Value;

use super::{ContainerSegment, UnknownFieldManifest};

mod nulls;
mod overlay_regressions;
mod v0;

fn raw(yaml: &str) -> Value {
    serde_yaml::from_str(yaml).unwrap()
}

fn unknown_value<'a>(manifest: &'a UnknownFieldManifest, field: &str) -> Option<&'a Value> {
    manifest
        .entries()
        .iter()
        .find_map(|(locator, value)| (locator.field == field).then_some(value))
}

#[test]
fn extracts_unknowns_from_common_and_v1_containers_without_claiming_known_nulls() {
    let value = raw(r#"
schema: gwz.merge-operation/v1
record_schema_version: 1
future_record: retained
publication: null
baseline:
  lock_sha256: lock
  manifest_sha256: manifest
  future_baseline: {nested: true}
participants:
  mem_a:
    path: a
    target_kind: member
    target_branch: main
    before_commit: before
    source_commit: source
    commit_message: message
    state: planned
    error: null
    future_participant: 7
accepted_workspace:
  operation_baseline_lock_sha256: lock
  future_acceptance: yes
  metadata_base:
    source:
      kind: operation_baseline
      future_source: kept
    manifest_exact_yaml: manifest
    manifest_sha256: digest
    lock_exact_yaml: lock
    lock_sha256: digest
  lock:
    exact_yaml: lock
    sha256: digest
  member_audit:
    mem_a:
      kind: absent
      future_audit: kept
  root:
    base:
      kind: born_detached
      commit: head
    publication_branch: null
    baseline_artifact_hashes:
      lock_worktree_sha256: lock
      manifest_worktree_sha256: manifest
      lock_commit_sha256: null
      manifest_commit_sha256: null
      future_hash: kept
"#);

    let manifest = UnknownFieldManifest::extract_v1(&value).unwrap();
    for field in [
        "future_record",
        "future_baseline",
        "future_participant",
        "future_acceptance",
        "future_source",
        "future_audit",
        "future_hash",
    ] {
        assert!(unknown_value(&manifest, field).is_some(), "missing {field}");
    }
    assert!(unknown_value(&manifest, "publication").is_none());
    assert!(unknown_value(&manifest, "error").is_none());
}

#[test]
fn conflict_extensions_follow_identity_when_rows_reorder() {
    let source = raw(r#"
participants:
  mem_a:
    conflict_snapshot:
      - path: a.txt
        sha256: aaa
        future: from-a
      - path: b.txt
        sha256: bbb
        future: from-b
"#);
    let mut next = raw(r#"
participants:
  mem_a:
    conflict_snapshot:
      - path: b.txt
        sha256: bbb
      - path: a.txt
        sha256: aaa
"#);

    let source = UnknownFieldManifest::extract_v1(&source).unwrap();
    let result = source.apply_surviving(&mut next).unwrap();
    assert_eq!(result.entries().len(), 2);
    let rows = next["participants"]["mem_a"]["conflict_snapshot"]
        .as_sequence()
        .unwrap();
    assert_eq!(rows[0]["future"], "from-b");
    assert_eq!(rows[1]["future"], "from-a");
}

#[test]
fn diagnostic_message_changes_do_not_retire_error_or_drift_extensions() {
    let source = raw(r#"
participants:
  mem_a:
    error:
      code: GitCommandFailed
      message: old
      detail: stable
      future_error: retained
    drift:
      - kind: head_advanced
        message: old
        expected_branch: main
        live_branch: main
        expected_head: before
        live_head: after
        expected_merge_head: null
        live_merge_head: null
        future_drift: retained
"#);
    let mut next = raw(r#"
participants:
  mem_a:
    error:
      code: GitCommandFailed
      message: rewritten
      detail: stable
    drift:
      - kind: head_advanced
        message: rewritten
        expected_branch: main
        live_branch: main
        expected_head: before
        live_head: after
        expected_merge_head: null
        live_merge_head: null
"#);

    UnknownFieldManifest::extract_v1(&source)
        .unwrap()
        .apply_surviving(&mut next)
        .unwrap();
    assert_eq!(
        next["participants"]["mem_a"]["error"]["future_error"],
        "retained"
    );
    assert_eq!(
        next["participants"]["mem_a"]["drift"][0]["future_drift"],
        "retained"
    );
}

#[test]
fn replacement_error_identity_retires_old_unknowns() {
    let source = raw(r#"
participants:
  mem_a:
    error:
      code: GitCommandFailed
      message: old
      detail: first
      future_error: retire
"#);
    let mut next = raw(r#"
participants:
  mem_a:
    error:
      code: GitCommandFailed
      message: new
      detail: second
"#);
    let result = UnknownFieldManifest::extract_v1(&source)
        .unwrap()
        .apply_surviving(&mut next)
        .unwrap();
    assert!(result.entries().is_empty());
    assert!(next["participants"]["mem_a"]["error"]["future_error"].is_null());
}

#[test]
fn participant_target_change_cannot_rebind_error_unknowns() {
    let source = raw(r#"
participants:
  mem_a:
    path: a
    target_kind: member
    error: {code: GitCommandFailed, message: old, detail: stable, future: retire}
"#);
    let mut next = raw(r#"
participants:
  mem_a:
    path: .
    target_kind: root
    error: {code: GitCommandFailed, message: new, detail: stable}
"#);
    let result = UnknownFieldManifest::extract_v1(&source)
        .unwrap()
        .apply_surviving(&mut next)
        .unwrap();
    assert!(result.entries().is_empty());
}

#[test]
fn pending_preservation_progress_keeps_unknowns_but_a_new_action_does_not() {
    let source = raw(r#"
pending_preservation:
  kind: stash
  owner: {kind: participant, member_id: mem_a}
  phase: create_stash
  stash_id: null
  stash_object_id: null
  message: preserve
  head_commit: before
  preimage_sha256: image
  root_publication_prefix: null
  future_action: retained
"#);
    let manifest = UnknownFieldManifest::extract_v1(&source).unwrap();
    let mut progressed = raw(r#"
pending_preservation:
  kind: stash
  owner: {kind: participant, member_id: mem_a}
  phase: write_bundle
  stash_id: stash@{0}
  stash_object_id: {algorithm: sha1, digest_hex: abc}
  message: preserve
  head_commit: before
  preimage_sha256: image
  root_publication_prefix: null
"#);
    manifest.apply_surviving(&mut progressed).unwrap();
    assert_eq!(
        progressed["pending_preservation"]["future_action"],
        "retained"
    );

    let mut replacement = progressed.clone();
    replacement["pending_preservation"]["head_commit"] = Value::String("different".into());
    replacement["pending_preservation"]
        .as_mapping_mut()
        .unwrap()
        .remove("future_action");
    let result = manifest.apply_surviving(&mut replacement).unwrap();
    assert!(result.entries().is_empty());
}

#[test]
fn duplicate_unique_sequence_identities_are_rejected() {
    for value in [
        raw(r#"
participants:
  mem_a:
    conflict_snapshot:
      - {path: same, sha256: same, future: one}
      - {path: same, sha256: same, future: two}
"#),
        raw(r#"
publication:
  candidate_hashes:
    - {path: same, sha256: one, future: one}
    - {path: same, sha256: two, future: two}
"#),
        raw(r#"
operation_drift:
  - {kind: record_unreadable, message: one, future: one}
  - {kind: record_unreadable, message: two, future: two}
"#),
    ] {
        let error = UnknownFieldManifest::extract_v1(&value).unwrap_err();
        assert!(error.detail.contains("duplicated"));
    }
}

#[test]
fn participant_drift_occurrence_is_part_of_the_semantic_path() {
    let value = raw(r#"
participants:
  mem_a:
    drift:
      - {kind: branch_changed, message: one, future: first}
      - {kind: branch_changed, message: two, future: second}
"#);
    let manifest = UnknownFieldManifest::extract_v1(&value).unwrap();
    let occurrences = manifest
        .entries()
        .keys()
        .map(|locator| {
            locator
                .container
                .iter()
                .find_map(|segment| match segment {
                    ContainerSegment::Identity(identity)
                        if identity.kind == "participant_drift" =>
                    {
                        Some(identity.occurrence)
                    }
                    _ => None,
                })
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(occurrences, vec![0, 1]);
}

#[test]
fn manifest_covers_every_frozen_surviving_container_family() {
    let value = raw(r#"
future_record: record
baseline:
  future_baseline: baseline
participants:
  mem_a:
    future_participant: participant
    conflict_snapshot:
      - {path: conflict, sha256: digest, future_conflict: conflict}
    error:
      code: GitCommandFailed
      message: diagnostic
      detail: stable
      future_error: error
    pending_action:
      kind: true_merge
      target_branch: main
      before_commit: before
      source_commit: source
      commit_message: message
      expected_result: commit
      future_action: action
      commit_spec:
        tree_oid: tree
        future_spec: spec
        author:
          name: author
          email: author@example.com
          time_seconds: 1
          timezone_offset_minutes: 0
          future_author: author
        committer:
          name: committer
          email: committer@example.com
          time_seconds: 2
          timezone_offset_minutes: 0
          future_committer: committer
    preservation:
      - backup_ref: refs/gwz/backup
        backup_commit: before
        stash_id: null
        stash_object_id: null
        future_participant_preservation: preservation
    drift:
      - kind: branch_changed
        message: diagnostic
        future_participant_drift: drift
publication:
  step: candidate_prepared
  future_publication: publication
  candidate_hashes:
    - {path: gwz.lock.yaml, sha256: digest, future_candidate_hash: hash}
  candidate:
    marker_id: marker
    root_branch: main
    actor_id: actor
    baseline_lock_yaml: baseline
    lock_yaml: lock
    marker_yaml: marker
    baseline_boundary_text: before
    boundary_text: after
    baseline_boundary_sha256: digest
    marker_sha256: digest
    boundary_sha256: digest
    future_candidate: candidate
  root_preservation:
    - backup_ref: refs/gwz/root
      backup_commit: root
      stash_id: null
      stash_object_id: null
      future_root_preservation: root-preservation
operation_drift:
  - kind: baseline_lock_changed
    message: diagnostic
    future_operation_drift: operation-drift
accepted_workspace:
  operation_baseline_lock_sha256: digest
  future_accepted: accepted
  metadata_base:
    source: {kind: operation_baseline, future_metadata_source: source}
    manifest_exact_yaml: manifest
    manifest_sha256: digest
    lock_exact_yaml: lock
    lock_sha256: digest
    future_metadata: metadata
  lock: {exact_yaml: lock, sha256: digest, future_lock: lock}
  member_audit:
    mem_a:
      kind: selected
      future_member_audit: audit
      integration: {branch: main, before_commit: before, resulting_commit: after, future_integration: integration}
      final_checkout: {branch: main, commit: after, future_checkout: checkout}
      lock_member: {path: a, source_id: source, source_kind: git, future_lock_member: lock-member}
  root:
    future_accepted_root: root
    base: {kind: born_attached, commit: root, symbolic_branch: main, future_root_base: root-base}
    publication_branch: main
    baseline_artifact_hashes: {lock_worktree_sha256: lock, manifest_worktree_sha256: manifest, future_root_hashes: hashes}
recovery_context: {origin_state: executing, future_recovery: recovery}
pending_rollback:
  kind: publication_evidence
  next_step: boundary
  future_rollback: rollback
pending_preservation:
  kind: stash
  owner: {kind: participant, member_id: mem_a, future_owner: owner}
  phase: create_stash
  stash_id: null
  stash_object_id: {algorithm: sha1, digest_hex: abc, future_object_id: object-id}
  message: preserve
  head_commit: before
  preimage_sha256: digest
  root_publication_prefix: baseline
  future_pending_preservation: pending-preservation
"#);
    let manifest = UnknownFieldManifest::extract_v1(&value).unwrap();
    for field in [
        "future_record",
        "future_baseline",
        "future_participant",
        "future_conflict",
        "future_error",
        "future_action",
        "future_spec",
        "future_author",
        "future_committer",
        "future_participant_preservation",
        "future_participant_drift",
        "future_publication",
        "future_candidate_hash",
        "future_candidate",
        "future_root_preservation",
        "future_operation_drift",
        "future_accepted",
        "future_metadata_source",
        "future_metadata",
        "future_lock",
        "future_member_audit",
        "future_integration",
        "future_checkout",
        "future_lock_member",
        "future_accepted_root",
        "future_root_base",
        "future_root_hashes",
        "future_recovery",
        "future_rollback",
        "future_owner",
        "future_object_id",
        "future_pending_preservation",
    ] {
        assert!(unknown_value(&manifest, field).is_some(), "missing {field}");
    }
    assert_eq!(manifest.entries().len(), 32);
}

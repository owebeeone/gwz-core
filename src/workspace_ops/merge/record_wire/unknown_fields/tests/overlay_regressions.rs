use super::super::UnknownFieldManifest;
use super::raw;

#[test]
fn preattached_unknowns_on_replacement_identities_are_rejected() {
    let cases = [
        (
            r#"participants: {mem_a: {error: {code: GitCommandFailed, message: old, detail: first, future: retained}}}"#,
            r#"participants: {mem_a: {error: {code: GitCommandFailed, message: new, detail: second, future: retained}}}"#,
        ),
        (
            r#"participants: {mem_a: {pending_action: {kind: true_merge, target_branch: main, before_commit: first, source_commit: source, commit_message: message, future: retained}}}"#,
            r#"participants: {mem_a: {pending_action: {kind: true_merge, target_branch: main, before_commit: second, source_commit: source, commit_message: message, future: retained}}}"#,
        ),
        (
            r#"participants: {mem_a: {drift: [{kind: head_advanced, message: old, expected_head: first, future: retained}]}}"#,
            r#"participants: {mem_a: {drift: [{kind: head_advanced, message: new, expected_head: second, future: retained}]}}"#,
        ),
        (
            r#"publication: {candidate_hashes: [{path: first, sha256: one, future: retained}]}"#,
            r#"publication: {candidate_hashes: [{path: second, sha256: two, future: retained}]}"#,
        ),
    ];
    for (source, replacement) in cases {
        let source = UnknownFieldManifest::extract_v1(&raw(source)).unwrap();
        let mut replacement = raw(replacement);
        let error = source.apply_surviving(&mut replacement).unwrap_err();
        assert!(error.detail.contains("unauthorized unknown field"));
    }
}

#[test]
fn a_different_value_on_a_surviving_locator_is_rejected() {
    let source = raw(r#"
participants: {mem_a: {conflict_snapshot: [{path: same, sha256: same, future: old}]}}
"#);
    let mut next = raw(r#"
participants: {mem_a: {conflict_snapshot: [{path: same, sha256: same, future: new}]}}
"#);
    let error = UnknownFieldManifest::extract_v1(&source)
        .unwrap()
        .apply_surviving(&mut next)
        .unwrap_err();
    assert!(error.detail.contains("unauthorized unknown field"));
}

#[test]
fn candidate_and_operation_drift_extensions_follow_reordered_identities() {
    let source = raw(r#"
publication:
  candidate_hashes:
    - {path: a, sha256: one, future: hash-a}
    - {path: b, sha256: two, future: hash-b}
operation_drift:
  - {kind: baseline_lock_changed, message: old-a, future: drift-a}
  - {kind: baseline_manifest_changed, message: old-b, future: drift-b}
"#);
    let mut next = raw(r#"
publication:
  candidate_hashes:
    - {path: b, sha256: two}
    - {path: a, sha256: one}
operation_drift:
  - {kind: baseline_manifest_changed, message: new-b}
  - {kind: baseline_lock_changed, message: new-a}
"#);
    UnknownFieldManifest::extract_v1(&source)
        .unwrap()
        .apply_surviving(&mut next)
        .unwrap();
    assert_eq!(
        next["publication"]["candidate_hashes"][0]["future"],
        "hash-b"
    );
    assert_eq!(
        next["publication"]["candidate_hashes"][1]["future"],
        "hash-a"
    );
    assert_eq!(next["operation_drift"][0]["future"], "drift-b");
    assert_eq!(next["operation_drift"][1]["future"], "drift-a");
}

#[test]
fn pending_rollback_progress_survives_and_recovery_origin_replacement_retires() {
    let source = raw(r#"
pending_rollback: {kind: publication_evidence, next_step: boundary, future_rollback: retained}
recovery_context: {origin_state: rolling_back, future_recovery: retire}
"#);
    let mut next = raw(r#"
pending_rollback: {kind: publication_evidence, next_step: lock}
recovery_context: {origin_state: preserving}
"#);
    let result = UnknownFieldManifest::extract_v1(&source)
        .unwrap()
        .apply_surviving(&mut next)
        .unwrap();
    assert_eq!(next["pending_rollback"]["future_rollback"], "retained");
    assert!(next["recovery_context"]["future_recovery"].is_null());
    assert_eq!(result.entries().len(), 1);
}

#[test]
fn identical_drift_occurrence_controls_retirement_after_removal() {
    let source = raw(r#"
participants:
  mem_a:
    drift:
      - {kind: branch_changed, message: first, future: first}
      - {kind: branch_changed, message: second, future: second}
"#);
    let mut next = raw(r#"
participants:
  mem_a:
    drift:
      - {kind: branch_changed, message: remaining}
"#);
    let result = UnknownFieldManifest::extract_v1(&source)
        .unwrap()
        .apply_surviving(&mut next)
        .unwrap();
    assert_eq!(next["participants"]["mem_a"]["drift"][0]["future"], "first");
    assert_eq!(result.entries().len(), 1);
}

#[test]
fn participant_drift_extensions_follow_distinct_identities_when_reordered() {
    let source = raw(r#"
participants:
  mem_a:
    drift:
      - {kind: head_advanced, message: first, expected_head: before, live_head: after, future: head}
      - {kind: branch_changed, message: second, expected_branch: main, live_branch: topic, future: branch}
"#);
    let mut next = raw(r#"
participants:
  mem_a:
    drift:
      - {kind: branch_changed, message: rewritten, expected_branch: main, live_branch: topic}
      - {kind: head_advanced, message: rewritten, expected_head: before, live_head: after}
"#);
    UnknownFieldManifest::extract_v1(&source)
        .unwrap()
        .apply_surviving(&mut next)
        .unwrap();
    assert_eq!(
        next["participants"]["mem_a"]["drift"][0]["future"],
        "branch"
    );
    assert_eq!(next["participants"]["mem_a"]["drift"][1]["future"], "head");
}

#[test]
fn pending_action_survives_only_while_exact_intent_survives() {
    let source = raw(r#"
participants:
  mem_a:
    pending_action:
      kind: true_merge
      target_branch: main
      before_commit: before
      source_commit: source
      commit_message: message
      expected_result: expected_conflict
      future: retained
"#);
    let manifest = UnknownFieldManifest::extract_v1(&source).unwrap();
    let mut same = raw(r#"
participants:
  mem_a:
    pending_action:
      kind: true_merge
      target_branch: main
      before_commit: before
      source_commit: source
      commit_message: message
      expected_result: expected_conflict
"#);
    manifest.apply_surviving(&mut same).unwrap();
    assert_eq!(
        same["participants"]["mem_a"]["pending_action"]["future"],
        "retained"
    );

    let mut completed = raw("participants: {mem_a: {state: conflicted}}\n");
    let result = manifest.apply_surviving(&mut completed).unwrap();
    assert!(result.entries().is_empty());
}

#[test]
fn recovery_and_preservation_owner_lifetimes_are_identity_bound() {
    let source = raw(r#"
recovery_context: {origin_state: rolling_back, future_recovery: retained}
participants:
  mem_a:
    preservation:
      - {backup_ref: backup, backup_commit: before, stash_id: null, stash_object_id: null, future_preservation: retained}
"#);
    let manifest = UnknownFieldManifest::extract_v1(&source).unwrap();
    let mut same = raw(r#"
recovery_context: {origin_state: rolling_back}
participants:
  mem_a:
    preservation:
      - {backup_ref: backup, backup_commit: before, stash_id: stash, stash_object_id: object}
"#);
    manifest.apply_surviving(&mut same).unwrap();
    assert_eq!(same["recovery_context"]["future_recovery"], "retained");
    assert_eq!(
        same["participants"]["mem_a"]["preservation"][0]["future_preservation"],
        "retained"
    );

    let mut retired = raw(r#"
publication:
  root_preservation:
    - {backup_ref: root, backup_commit: before, stash_id: null, stash_object_id: null}
"#);
    let result = manifest.apply_surviving(&mut retired).unwrap();
    assert!(result.entries().is_empty());
}

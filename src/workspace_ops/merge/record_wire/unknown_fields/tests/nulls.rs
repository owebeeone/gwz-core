use super::super::UnknownFieldManifest;
use super::raw;

#[test]
fn nested_known_nulls_do_not_enter_the_manifest() {
    let value = raw(r#"
participants:
  mem_a:
    resulting_commit: null
    expected_merge_head: null
    error: null
    pending_action:
      kind: true_merge
      target_branch: main
      before_commit: before
      source_commit: source
      commit_message: message
      expected_result: null
      commit_spec: null
accepted_workspace:
  member_audit:
    mem_a:
      kind: unselected_present
      lock_member:
        path: a
        source_id: source
        source_kind: git
        commit: null
        branch: null
        detached: null
        upstream: null
        dirty: null
        materialized: null
pending_preservation:
  kind: stash
  owner: {kind: participant, member_id: mem_a}
  phase: create_stash
  stash_id: null
  stash_object_id: null
  message: preserve
  head_commit: before
  preimage_sha256: digest
  root_publication_handoff: null
"#);
    assert!(
        UnknownFieldManifest::extract_v1(&value)
            .unwrap()
            .entries()
            .is_empty()
    );
}

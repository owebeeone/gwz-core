use serde_yaml::Value;

use super::{UnknownFieldManifest, raw, unknown_value};

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
  root_publication_handoff: null
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
  root_publication_handoff: null
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
fn typed_preservation_handoff_is_known_and_cannot_be_resurrected_by_overlay() {
    for body in [
        "kind: no_candidate",
        "kind: evidence_pending",
        "kind: candidate\n  prefix: boundary\n  index: staged",
    ] {
        let source = raw(&format!(
            "preservation_publication_handoff:\n  {body}\n  future_handoff: retained\n"
        ));
        let manifest = UnknownFieldManifest::extract_v1(&source).unwrap();
        assert!(
            unknown_value(&manifest, "preservation_publication_handoff").is_none(),
            "typed field leaked into unknown manifest for {body}"
        );
        assert_eq!(
            unknown_value(&manifest, "future_handoff"),
            Some(&Value::String("retained".into()))
        );

        let mut retired = raw("preservation_publication_handoff: null\n");
        let surviving = manifest.apply_surviving(&mut retired).unwrap();
        assert!(surviving.entries().is_empty());
        assert!(retired["preservation_publication_handoff"].is_null());
    }
}

#[test]
fn root_parent_action_identity_keeps_unknowns_only_across_exact_progress() {
    for (name, body, parent, next, identity_key) in [
        (
            "stash",
            "kind: stash\n  owner: {kind: publication_root}\n  stash_id: null\n  stash_object_id: null\n  message: preserve\n  head_commit: before\n  preimage_sha256: image",
            "normalize_parent",
            "normalize_marker",
            "head_commit",
        ),
        (
            "reset",
            "kind: reset_attached_ref\n  owner: {kind: publication_root}\n  branch: main\n  expected_commit: before\n  restore_commit: restore",
            "prepare_parent",
            "prepare_marker",
            "branch",
        ),
    ] {
        let source = raw(&format!(
            r#"
pending_preservation:
  {body}
  phase: {parent}
  root_publication_handoff: {{prefix: boundary, index: pre, future_pair: retained}}
  future_action: retained
"#
        ));
        let manifest = UnknownFieldManifest::extract_v1(&source).unwrap();
        let mut progressed = source.clone();
        progressed["pending_preservation"]["phase"] = Value::String(next.into());
        remove_unknowns(&mut progressed);
        manifest.apply_surviving(&mut progressed).unwrap();
        assert_eq!(
            progressed["pending_preservation"]["future_action"], "retained",
            "{name}"
        );
        assert_eq!(
            progressed["pending_preservation"]["root_publication_handoff"]["future_pair"],
            "retained",
            "{name}"
        );

        for changed_identity in ["handoff", "action"] {
            let mut changed = source.clone();
            if changed_identity == "handoff" {
                changed["pending_preservation"]["root_publication_handoff"]["index"] =
                    Value::String("staged".into());
            } else {
                changed["pending_preservation"][identity_key] = Value::String("different".into());
            }
            remove_unknowns(&mut changed);
            assert!(
                manifest
                    .apply_surviving(&mut changed)
                    .unwrap()
                    .entries()
                    .is_empty(),
                "{name} {changed_identity}"
            );
            assert!(
                changed["pending_preservation"]
                    .get("future_action")
                    .is_none()
            );
            assert!(
                changed["pending_preservation"]["root_publication_handoff"]
                    .get("future_pair")
                    .is_none()
            );
        }
    }
}

fn remove_unknowns(value: &mut Value) {
    value["pending_preservation"]
        .as_mapping_mut()
        .unwrap()
        .remove("future_action");
    value["pending_preservation"]["root_publication_handoff"]
        .as_mapping_mut()
        .unwrap()
        .remove("future_pair");
}

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

/// A record body carrying `noop_commit`/`reset_commit` inside the single
/// `mem_a` preservation evidence row, plus an unknown descendant beside them.
fn marker_record(markers: &str) -> Value {
    raw(&format!(
        r#"
participants:
  mem_a:
    preservation:
      - backup_ref: refs/gwz/merge/merge_1/mem_a/head
        backup_commit: dddddddddddddddddddddddddddddddddddddddd
        stash_id: null
        stash_object_id: null
{markers}
        future_row: retained
"#
    ))
}

const BOTH_MARKERS: &str = "        noop_commit: dddddddddddddddddddddddddddddddddddddddd\n        reset_commit: cccccccccccccccccccccccccccccccccccccccc";

#[test]
fn v1_known_key_set_adopts_the_markers_so_they_never_reach_a_v1_manifest() {
    // §2.3 / §8.3: the v1 evidence-row known-key set gains the two names —
    // without which the first marker write fails the overlay's
    // unauthorized-unknown-field check.
    let manifest = UnknownFieldManifest::extract_v1(&marker_record(BOTH_MARKERS)).unwrap();
    for name in ["noop_commit", "reset_commit"] {
        assert!(
            unknown_value(&manifest, name).is_none(),
            "typed v1 marker '{name}' leaked into the v1 unknown manifest"
        );
    }
    // The genuine unknown beside them still survives by stable-owner identity.
    assert_eq!(
        unknown_value(&manifest, "future_row"),
        Some(&Value::String("retained".into()))
    );
}

#[test]
fn v0_known_key_set_does_not_adopt_the_markers_so_they_surface_in_the_v0_manifest() {
    // §2.3: the evidence-row known-key set forks by version. In a v0 record
    // the two names DO surface in the v0 unknown manifest, and that manifest
    // membership is the collision trigger.
    let manifest = UnknownFieldManifest::extract_v0(&marker_record(BOTH_MARKERS)).unwrap();
    assert_eq!(
        unknown_value(&manifest, "noop_commit"),
        Some(&Value::String("d".repeat(40)))
    );
    assert_eq!(
        unknown_value(&manifest, "reset_commit"),
        Some(&Value::String("c".repeat(40)))
    );
}

#[test]
fn a_marker_inside_a_v0_evidence_row_makes_migration_ineligible() {
    // §2.3: presence of either name inside a v0 record's preservation evidence
    // row makes migration ineligible; the value is never adopted, overwritten,
    // or moved — the same doctrine as the five top-level v1 names.
    for markers in [
        "        noop_commit: dddddddddddddddddddddddddddddddddddddddd",
        "        reset_commit: cccccccccccccccccccccccccccccccccccccccc",
        BOTH_MARKERS,
    ] {
        let manifest = UnknownFieldManifest::extract_v0(&marker_record(markers)).unwrap();
        assert!(
            manifest.map_v0_to_v1().is_err(),
            "v0 in-row marker did not trigger migration ineligibility: {markers}"
        );
    }
}

#[test]
fn a_v0_record_without_markers_still_migrates() {
    // The collision leg must not fire on the legitimate pre-amendment shape.
    let manifest = UnknownFieldManifest::extract_v0(&marker_record("")).unwrap();
    let mapped = manifest.map_v0_to_v1().unwrap();
    assert_eq!(
        mapped.entries().len(),
        1,
        "the unknown descendant beside the row must still survive migration"
    );
}

#[test]
fn unknown_descendants_beside_the_markers_survive_by_stable_owner_identity() {
    // §2.3: sequence identity remains the stable owner, so an unknown beside
    // the markers survives a rewrite that fills them in.
    let manifest = UnknownFieldManifest::extract_v1(&marker_record("")).unwrap();
    let mut filled = marker_record(BOTH_MARKERS);
    filled["participants"]["mem_a"]["preservation"][0]
        .as_mapping_mut()
        .unwrap()
        .remove("future_row");
    manifest.apply_surviving(&mut filled).unwrap();
    assert_eq!(
        filled["participants"]["mem_a"]["preservation"][0]["future_row"], "retained",
        "unknown descendant lost across a marker write"
    );
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

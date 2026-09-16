//! Round-trip, golden-byte and schema/validation tests for the records.

use crate::model::ErrorCode;

use super::*;

#[test]
fn manifest_round_trips_and_matches_golden_yaml() {
    let manifest = sample_manifest();

    assert_eq!(manifest.to_yaml().unwrap(), MANIFEST_GOLDEN);
    assert_eq!(
        ManifestArtifact::from_yaml(MANIFEST_GOLDEN).unwrap(),
        manifest
    );
}

#[test]
fn lock_and_snapshot_round_trip_and_match_golden_yaml() {
    assert_eq!(sample_lock().to_yaml().unwrap(), LOCK_GOLDEN);
    assert_eq!(LockArtifact::from_yaml(LOCK_GOLDEN).unwrap(), sample_lock());

    assert_eq!(sample_snapshot().to_yaml().unwrap(), SNAPSHOT_GOLDEN);
    assert_eq!(
        SnapshotArtifact::from_yaml(SNAPSHOT_GOLDEN).unwrap(),
        sample_snapshot()
    );
}

#[test]
fn marker_round_trips() {
    let marker = sample_marker();
    let yaml = marker.to_yaml().unwrap();

    assert_eq!(MarkerArtifact::from_yaml(&yaml).unwrap(), marker);
}

#[test]
fn marker_merge_targets_must_match_outer_marker_targets() {
    let mut marker = sample_marker();
    marker.merge = Some(MarkerMergeArtifact {
        merge_id: "merge_1".to_owned(),
        operation_id: "op_1".to_owned(),
        source_ref: "feature/x".to_owned(),
        selected_targets: vec!["mem_01".to_owned()],
        participants: [(
            "mem_01".to_owned(),
            MarkerMergeParticipantArtifact {
                target_kind: MarkerMergeTargetKind::Member,
                target_branch: "main".to_owned(),
                before_commit: "before".to_owned(),
                source_commit: "source".to_owned(),
                resulting_commit: "result".to_owned(),
            },
        )]
        .into(),
        root_merge_commit: None,
    });

    assert_eq!(
        marker.validate().unwrap_err().code,
        ErrorCode::InvalidRequest
    );
}

#[test]
fn unsupported_major_schema_versions_fail_with_typed_error() {
    let manifest = MANIFEST_GOLDEN.replace("gwz.workspace/v0", "gwz.workspace/v1");
    let lock = LOCK_GOLDEN.replacen("gwz.lock/v0", "gwz.lock/v1", 1);
    let snapshot = SNAPSHOT_GOLDEN.replace("gwz.snapshot/v0", "gwz.snapshot/v1");
    let marker = sample_marker()
        .to_yaml()
        .unwrap()
        .replace("gwz.marker/v0", "gwz.marker/v1");

    assert_eq!(
        ManifestArtifact::from_yaml(&manifest).unwrap_err().code,
        ErrorCode::SchemaUnsupported
    );
    assert_eq!(
        LockArtifact::from_yaml(&lock).unwrap_err().code,
        ErrorCode::SchemaUnsupported
    );
    assert_eq!(
        SnapshotArtifact::from_yaml(&snapshot).unwrap_err().code,
        ErrorCode::SchemaUnsupported
    );
    assert_eq!(
        MarkerArtifact::from_yaml(&marker).unwrap_err().code,
        ErrorCode::SchemaUnsupported
    );
}

#[test]
fn manifest_reader_rejects_duplicate_remote_names() {
    let yaml = MANIFEST_GOLDEN.replace(
        "  - name: origin\n    url: git@example.invalid:example.git\n    fetch: true\n    push: true\n",
        "  - name: origin\n    url: git@example.invalid:example.git\n    fetch: true\n    push: true\n  - name: origin\n    url: git@example.invalid:example-2.git\n    fetch: true\n    push: false\n",
    );

    assert_eq!(
        ManifestArtifact::from_yaml(&yaml).unwrap_err().code,
        ErrorCode::InvalidRequest
    );
}

#[test]
fn manifest_rejects_duplicate_member_ids_across_active_and_inactive_rows() {
    let mut manifest = sample_manifest();
    let mut historical = manifest.members[0].clone();
    historical.path = "repos/historical".to_owned();
    historical.active = false;
    manifest.members.push(historical);

    let error = manifest.validate().unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(error.message.contains("duplicate member id 'mem_01'"));
}

#[test]
fn manifest_allows_shared_source_ids_and_inactive_path_overlap() {
    let mut manifest = sample_manifest();
    let mut replacement = manifest.members[0].clone();
    replacement.id = "mem_02".to_owned();
    replacement.source_id = manifest.members[0].source_id.clone();
    manifest.members[0].active = false;
    manifest.members.push(replacement);

    manifest.validate().unwrap();
}

#[test]
fn manifest_rejects_overlap_between_active_rows_only() {
    let mut manifest = sample_manifest();
    let mut nested = manifest.members[0].clone();
    nested.id = "mem_02".to_owned();
    nested.path = "repos/example/tools".to_owned();
    nested.source_id = "src_02".to_owned();
    manifest.members.push(nested);

    assert_eq!(
        manifest.validate().unwrap_err().code,
        ErrorCode::PathCollision
    );

    manifest.members[1].active = false;
    manifest.validate().unwrap();
}

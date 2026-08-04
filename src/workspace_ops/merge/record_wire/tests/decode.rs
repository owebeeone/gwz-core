use super::super::decode::{RecordDecodeError, decode_production_v0, decode_v1_for_r3_tests};
use super::super::decode_archived_for_r3_tests;
use super::super::header::HeaderClassificationError;
use super::super::raw_yaml::StrictYamlErrorKind;
use sha2::{Digest, Sha256};

fn record(envelope: &str) -> String {
    let manifest = "schema: gwz.workspace/v0\nworkspace:\n  id: ws_default\nmembers:\n- id: mem_a\n  path: members/a\n  type: git\n  source_id: src_a\n  active: true\n  remotes: []\n";
    let lock = "schema: gwz.lock/v0\nworkspace_id: ws_default\nmanifest_schema: gwz.workspace/v0\nmembers: {}\n";
    format!(
        "{envelope}\nwriter_version: 0.10.3\nworkspace_id: ws_default\nmerge_id: merge_1\noperation_id: op_1\nstate: executing\nsource_ref: feature/x\ncreated_at: now\nbaseline:\n  lock_sha256: '{}'\n  manifest_sha256: '{}'\n  lock_yaml: |\n{}  manifest_yaml: |\n{}  root_head: {}\n  root_branch: main\nselected_targets: [mem_a]\nparticipants:\n  mem_a:\n    path: members/a\n    target_kind: member\n    target_branch: main\n    before_commit: {}\n    source_commit: {}\n    commit_message: \"merge topic\\n\\nGWZ-Merge-ID: merge_1\\nGWZ-Operation-ID: op_1\"\n    state: planned\nfuture_record: retained\n",
        digest(lock),
        digest(manifest),
        indent(lock),
        indent(manifest),
        "a".repeat(40),
        "a".repeat(40),
        "b".repeat(40),
    )
}

fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn indent(text: &str) -> String {
    text.lines().map(|line| format!("    {line}\n")).collect()
}

#[test]
fn production_decoder_uses_the_strict_tree_for_v0_body_decode() {
    let decoded = decode_production_v0(
        record("schema: gwz.merge-operation/v0\nrecord_schema_version: 0").as_bytes(),
    )
    .unwrap();
    assert_eq!(decoded.record().merge_id, "merge_1");
    assert!(
        decoded
            .unknown_fields()
            .entries()
            .keys()
            .any(|locator| locator.field == "future_record")
    );
    assert_eq!(
        decoded
            .raw()
            .as_mapping()
            .unwrap()
            .get("future_record")
            .and_then(serde_yaml::Value::as_str),
        Some("retained")
    );
}

#[test]
fn production_decoder_rejects_v1_before_body_decode() {
    let error = decode_production_v0(
        b"schema: gwz.merge-operation/v1\nrecord_schema_version: 1\nbody: invalid\n",
    )
    .unwrap_err();
    assert!(matches!(
        error,
        RecordDecodeError::Header(HeaderClassificationError::Unsupported {
            required_wave: Some(crate::MergeRecordRequiredWave::A1),
            ..
        })
    ));
}

#[test]
fn exact_v0_header_precedes_typed_body_failure() {
    let error = decode_production_v0(
        b"schema: gwz.merge-operation/v0\nrecord_schema_version: 0\nbody: invalid\n",
    )
    .unwrap_err();
    assert!(matches!(error, RecordDecodeError::Body { .. }));
}

#[test]
fn test_only_v1_decoder_uses_the_same_strict_tree_for_the_complete_body() {
    let decoded = decode_v1_for_r3_tests(
        record("schema: gwz.merge-operation/v1\nrecord_schema_version: 1").as_bytes(),
    )
    .unwrap();
    assert_eq!(decoded.header.schema, "gwz.merge-operation/v1");
    assert_eq!(decoded.canonical.common().merge_id(), "merge_1");
    assert_eq!(
        decoded.canonical.installed_kind(),
        super::super::super::model::v1::CanonicalInstalledKind::V1
    );
    assert_eq!(
        decoded
            .raw
            .as_mapping()
            .unwrap()
            .get("future_record")
            .and_then(serde_yaml::Value::as_str),
        Some("retained")
    );
    assert!(
        decoded
            .unknown_fields
            .entries()
            .keys()
            .any(|locator| locator.field == "future_record")
    );
}

#[test]
fn test_only_v1_decoder_rejects_duplicates_inside_new_v1_containers() {
    let input = format!(
        "{}accepted_workspace:\n  operation_baseline_lock_sha256: first\n  operation_baseline_lock_sha256: second\n",
        record("schema: gwz.merge-operation/v1\nrecord_schema_version: 1")
    );
    let error = decode_v1_for_r3_tests(input.as_bytes()).unwrap_err();
    assert!(matches!(
        error,
        RecordDecodeError::Raw(error) if error.kind == StrictYamlErrorKind::DuplicateKey
    ));
}

#[test]
fn test_only_v1_decoder_preserves_typed_post_body_validation_errors() {
    let input = format!(
        "{}publication:\n  step: preparing_candidate\n  composition_commit: '{}'\n",
        record("schema: gwz.merge-operation/v1\nrecord_schema_version: 1"),
        "a".repeat(40)
    );
    let error = decode_v1_for_r3_tests(input.as_bytes()).unwrap_err();
    assert!(matches!(
        error,
        RecordDecodeError::Validation { header, error }
            if header.schema == "gwz.merge-operation/v1"
                && error.code == crate::model::ErrorCode::UnexpectedAcceptanceEvidence
    ));
}

#[test]
fn test_only_archive_decoder_rejects_allocated_future_before_body_decode() {
    let error = decode_archived_for_r3_tests(
        b"schema: gwz.merge-operation/v2\nrecord_schema_version: 2\nbody: invalid\n",
        "merge_future",
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        crate::model::ErrorCode::UnsupportedRecordVersion
    );
}

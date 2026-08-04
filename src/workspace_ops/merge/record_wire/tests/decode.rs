use super::super::decode::{RecordDecodeError, decode_production_v0, decode_v1_for_r3_tests};
use super::super::header::HeaderClassificationError;
use super::super::raw_yaml::StrictYamlErrorKind;

fn record(envelope: &str) -> String {
    format!(
        "{envelope}\nwriter_version: 0.10.3\nworkspace_id: ws_default\nmerge_id: merge_1\noperation_id: op_1\nstate: executing\nsource_ref: feature/x\ncreated_at: now\nbaseline:\n  lock_sha256: lock\n  manifest_sha256: manifest\nselected_targets: []\nparticipants: {{}}\nfuture_record: retained\n"
    )
}

#[test]
fn production_decoder_uses_the_strict_tree_for_v0_body_decode() {
    let decoded = decode_production_v0(
        record("schema: gwz.merge-operation/v0\nrecord_schema_version: 0").as_bytes(),
    )
    .unwrap();
    assert_eq!(decoded.record.merge_id, "merge_1");
    assert_eq!(
        decoded
            .raw
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
    assert_eq!(decoded.record.merge_id, "merge_1");
    assert!(decoded.record.accepted_workspace.is_none());
    assert_eq!(
        decoded
            .raw
            .as_mapping()
            .unwrap()
            .get("future_record")
            .and_then(serde_yaml::Value::as_str),
        Some("retained")
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

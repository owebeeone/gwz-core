use crate::MergeRecordRequiredWave;

use super::super::header::{
    HeaderClassificationError, HeaderMalformedReason, InstalledMergeRecordVersions,
    MergeRecordDispatch, MergeRecordHeader, classify_merge_record_header, read_merge_record_header,
};
use super::super::raw_yaml::parse_strict_yaml;

fn document(schema: &str, version: &str) -> super::super::raw_yaml::StrictYamlDocument {
    parse_strict_yaml(format!("schema: {schema}\nrecord_schema_version: {version}\n").as_bytes())
        .unwrap()
}

fn header(schema: &str, version: u32) -> MergeRecordHeader {
    MergeRecordHeader {
        schema: schema.to_owned(),
        record_schema_version: version,
    }
}

#[test]
fn installed_dispatch_contains_only_v0_and_v1() {
    let installed = InstalledMergeRecordVersions::PRODUCTION;
    assert_eq!(
        classify_merge_record_header(&header("gwz.merge-operation/v0", 0), installed).unwrap(),
        MergeRecordDispatch::V0
    );
    assert_eq!(
        classify_merge_record_header(&header("gwz.merge-operation/v1", 1), installed).unwrap(),
        MergeRecordDispatch::V1
    );
}

/// T-2, inverted at A1. Pre-A1 the production installed set refused
/// `gwz.merge-operation/v1` with `required_wave: A1`; A1 installs v1 and the
/// refusal survives only for v2-v4. The v0-only set — the v0 record store's
/// own decoder — keeps the pre-A1 answer, which is what lets the dispatch
/// route a v1 record to the v1 lifecycle instead of the v0 model.
#[test]
fn production_installs_v0_and_v1_and_reports_every_later_required_wave() {
    let rows = [
        ("gwz.merge-operation/v2", 2, MergeRecordRequiredWave::A2),
        ("gwz.merge-operation/v3", 3, MergeRecordRequiredWave::A3),
        ("gwz.merge-operation/v4", 4, MergeRecordRequiredWave::A4),
    ];
    assert_eq!(
        classify_merge_record_header(
            &header("gwz.merge-operation/v0", 0),
            InstalledMergeRecordVersions::PRODUCTION,
        )
        .unwrap(),
        MergeRecordDispatch::V0
    );
    assert_eq!(
        classify_merge_record_header(
            &header("gwz.merge-operation/v1", 1),
            InstalledMergeRecordVersions::PRODUCTION,
        )
        .unwrap(),
        MergeRecordDispatch::V1
    );
    for (schema, version, required_wave) in rows {
        assert_eq!(
            classify_merge_record_header(
                &header(schema, version),
                InstalledMergeRecordVersions::PRODUCTION,
            )
            .unwrap_err(),
            HeaderClassificationError::Unsupported {
                header: header(schema, version),
                required_wave: Some(required_wave),
            }
        );
    }
    assert_eq!(
        classify_merge_record_header(
            &header("gwz.merge-operation/v1", 1),
            InstalledMergeRecordVersions::V0_ONLY,
        )
        .unwrap_err(),
        HeaderClassificationError::Unsupported {
            header: header("gwz.merge-operation/v1", 1),
            required_wave: Some(MergeRecordRequiredWave::A1),
        }
    );
}

#[test]
fn every_recognized_schema_with_a_wrong_number_is_malformed() {
    for allocated in 0..=4 {
        let schema = format!("gwz.merge-operation/v{allocated}");
        for actual in [0, 1, 2, 3, 4, u32::MAX] {
            if actual == allocated {
                continue;
            }
            assert_eq!(
                classify_merge_record_header(
                    &header(&schema, actual),
                    InstalledMergeRecordVersions::PRODUCTION,
                )
                .unwrap_err(),
                HeaderClassificationError::Malformed(
                    HeaderMalformedReason::RecognizedSchemaVersionMismatch {
                        schema: schema.clone(),
                        allocated,
                        actual,
                    }
                )
            );
        }
    }
}

#[test]
fn unknown_schema_with_any_valid_u32_has_no_claimed_wave() {
    for version in [0, 1, 2, 3, 4, u32::MAX] {
        let value = header("vendor.future/merge", version);
        assert_eq!(
            classify_merge_record_header(&value, InstalledMergeRecordVersions::PRODUCTION,)
                .unwrap_err(),
            HeaderClassificationError::Unsupported {
                header: value,
                required_wave: None,
            }
        );
    }
}

#[test]
fn malformed_header_types_and_ranges_are_disjoint() {
    let cases = [
        (
            "record_schema_version: 0\n",
            HeaderMalformedReason::MissingSchema,
        ),
        (
            "schema: 1\nrecord_schema_version: 0\n",
            HeaderMalformedReason::SchemaNotString,
        ),
        ("schema: known\n", HeaderMalformedReason::MissingVersion),
        (
            "schema: known\nrecord_schema_version: '1'\n",
            HeaderMalformedReason::VersionNotInteger,
        ),
        (
            "schema: known\nrecord_schema_version: 1.0\n",
            HeaderMalformedReason::VersionNotInteger,
        ),
        (
            "schema: known\nrecord_schema_version: -1\n",
            HeaderMalformedReason::VersionNegative,
        ),
        (
            "schema: known\nrecord_schema_version: 4294967296\n",
            HeaderMalformedReason::VersionOutOfRange,
        ),
    ];
    for (yaml, expected) in cases {
        let document = parse_strict_yaml(yaml.as_bytes()).unwrap();
        assert_eq!(read_merge_record_header(&document).unwrap_err(), expected);
    }
}

#[test]
fn header_classification_precedes_body_decode() {
    let wrong = parse_strict_yaml(
        b"schema: gwz.merge-operation/v0\nrecord_schema_version: 1\nbaseline: invalid\n",
    )
    .unwrap();
    let header = read_merge_record_header(&wrong).unwrap();
    assert!(matches!(
        classify_merge_record_header(&header, InstalledMergeRecordVersions::PRODUCTION,),
        Err(HeaderClassificationError::Malformed(
            HeaderMalformedReason::RecognizedSchemaVersionMismatch { .. }
        ))
    ));

    let unsupported = read_merge_record_header(&document("gwz.merge-operation/v2", "2")).unwrap();
    assert!(matches!(
        classify_merge_record_header(&unsupported, InstalledMergeRecordVersions::PRODUCTION,),
        Err(HeaderClassificationError::Unsupported {
            required_wave: Some(MergeRecordRequiredWave::A2),
            ..
        })
    ));
}

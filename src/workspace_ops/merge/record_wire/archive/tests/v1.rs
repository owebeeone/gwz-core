use super::super::super::super::model::archive_projection::*;
use super::super::decode_archived_for_r3_tests;
use super::fixtures::{MERGE_ID, Shape, v1_bytes, v1_record};
use crate::model::ErrorCode;

#[test]
fn v1_completed_and_aborted_acceptance_project_losslessly() {
    for (shape, outcome) in [
        (
            Shape::CompletedCandidate,
            ArchivedTerminalOutcome::Completed,
        ),
        (
            Shape::AbortedCompleteCandidate,
            ArchivedTerminalOutcome::Aborted,
        ),
    ] {
        let record = v1_record(shape);
        let expected = record.accepted_workspace.as_ref().unwrap();
        let decoded = decode_archived_for_r3_tests(&v1_bytes(&record), MERGE_ID).unwrap();
        assert_eq!(decoded.projection.terminal_outcome, outcome);
        let ArchivedAcceptanceProjection::SupportedPersisted {
            workspace: InstalledAcceptedWorkspaceProjection::V1(projected),
        } = decoded.projection.acceptance
        else {
            panic!("expected supported persisted acceptance")
        };
        assert_eq!(projected.lock_yaml, expected.lock.exact_yaml);
        assert_eq!(projected.lock_sha256, expected.lock.sha256);
        assert_eq!(projected.members[0].member_id, "mem_a");
    }
}

#[test]
fn v1_aborted_before_acceptance_is_not_accepted_not_unreadable() {
    let record = v1_record(Shape::AbortedPreAcceptance);
    let decoded = decode_archived_for_r3_tests(&v1_bytes(&record), MERGE_ID).unwrap();
    assert_eq!(
        decoded.projection.terminal_outcome,
        ArchivedTerminalOutcome::Aborted
    );
    assert!(matches!(
        decoded.projection.acceptance,
        ArchivedAcceptanceProjection::NotAccepted
    ));
}

#[test]
fn v1_missing_or_contradictory_terminal_acceptance_is_unreadable() {
    let mut missing = v1_record(Shape::CompletedCandidate);
    missing.accepted_workspace = None;
    let error = decode_archived_for_r3_tests(&v1_bytes(&missing), MERGE_ID).unwrap_err();
    assert_eq!(error.code, ErrorCode::ArchivedRecordUnreadable);

    let mut contradictory = v1_record(Shape::CompletedCandidate);
    contradictory
        .accepted_workspace
        .as_mut()
        .unwrap()
        .lock
        .sha256 = "0".repeat(64);
    let error = decode_archived_for_r3_tests(&v1_bytes(&contradictory), MERGE_ID).unwrap_err();
    assert_eq!(error.code, ErrorCode::ArchivedRecordUnreadable);

    let mut marker_drift = v1_record(Shape::CompletedCandidate);
    let publication = marker_drift.publication.as_mut().unwrap();
    let candidate = publication.candidate.as_mut().unwrap();
    let mut marker = crate::artifact::MarkerArtifact::from_yaml(&candidate.marker_yaml).unwrap();
    marker.merge = None;
    candidate.marker_yaml = marker.to_yaml().unwrap();
    candidate.marker_sha256 = super::fixtures::digest(&candidate.marker_yaml);
    publication.candidate_hashes[1].sha256 = candidate.marker_sha256.clone();
    let error = decode_archived_for_r3_tests(&v1_bytes(&marker_drift), MERGE_ID).unwrap_err();
    assert_eq!(error.code, ErrorCode::ArchivedRecordUnreadable);

    let mut marker_time_drift = v1_record(Shape::CompletedCandidate);
    let publication = marker_time_drift.publication.as_mut().unwrap();
    let candidate = publication.candidate.as_mut().unwrap();
    let mut marker = crate::artifact::MarkerArtifact::from_yaml(&candidate.marker_yaml).unwrap();
    marker.created_at = "2026-08-04T01:00:00Z".to_owned();
    candidate.marker_yaml = marker.to_yaml().unwrap();
    candidate.marker_sha256 = super::fixtures::digest(&candidate.marker_yaml);
    publication.candidate_hashes[1].sha256 = candidate.marker_sha256.clone();
    let error = decode_archived_for_r3_tests(&v1_bytes(&marker_time_drift), MERGE_ID).unwrap_err();
    assert_eq!(error.code, ErrorCode::ArchivedRecordUnreadable);
}

#[test]
fn archive_header_and_filename_identity_fail_closed_before_projection() {
    for version in [2, 3, 4] {
        let bytes = format!(
            "schema: gwz.merge-operation/v{version}\nrecord_schema_version: {version}\nbody: invalid\n"
        );
        let error = decode_archived_for_r3_tests(bytes.as_bytes(), MERGE_ID).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedRecordVersion);
    }
    let unknown = decode_archived_for_r3_tests(
        b"schema: example.future/v8\nrecord_schema_version: 8\nbody: invalid\n",
        MERGE_ID,
    )
    .unwrap_err();
    assert_eq!(unknown.code, ErrorCode::UnsupportedRecordVersion);

    let record = v1_record(Shape::CompletedCandidate);
    let mismatch = decode_archived_for_r3_tests(&v1_bytes(&record), "merge_other").unwrap_err();
    assert_eq!(mismatch.code, ErrorCode::ArchivedRecordUnreadable);
}

#[test]
fn production_archive_decoder_rejects_v1_without_entering_the_v1_body() {
    let record = v1_record(Shape::CompletedCandidate);
    let error = super::super::decode_archived_v0(&v1_bytes(&record), MERGE_ID).unwrap_err();

    assert_eq!(error.code, ErrorCode::UnsupportedRecordVersion);
    assert_eq!(
        error
            .record_context
            .as_ref()
            .and_then(|context| context.required_wave),
        Some(crate::MergeRecordRequiredWave::A1)
    );
}

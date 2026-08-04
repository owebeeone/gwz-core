use super::super::super::{OperationState, ParticipantState, PublicationProgress, PublicationStep};
use super::super::{
    CanonicalInstalledKind, CanonicalMergeRecord, ParticipantRollbackKindV1,
    PendingRollbackActionV1, RecordVersion,
};
use super::tests::{record, sha};
use super::{validate_v1_publication, validate_v1_record};
use crate::model::ErrorCode;

#[test]
fn opaque_validation_is_the_only_canonical_v1_adapter_input() {
    let source = record();
    let canonical = CanonicalMergeRecord::from(validate_v1_record(source.clone()).unwrap());
    assert_eq!(canonical.source_version(), RecordVersion::V1);
    assert_eq!(canonical.common().merge_id(), source.merge_id);
    assert_eq!(canonical.common().participants(), &source.participants);
    assert_eq!(canonical.installed_kind(), CanonicalInstalledKind::V1);
    assert!(canonical.v1_state().unwrap().is_empty());

    // V0 is the only other installed canonical version; no V2-V4 variant is
    // compiled by the I2 model.
    let _ = RecordVersion::V0;
    let _ = CanonicalInstalledKind::V0;
}

#[test]
fn terminal_and_no_publication_shapes_fail_closed() {
    let mut case = record();
    case.state = OperationState::Completed;
    assert_eq!(
        validate_v1_publication(&case).unwrap_err().code,
        ErrorCode::TerminalEvidenceMismatch
    );

    case.publication = Some(PublicationProgress {
        step: PublicationStep::Complete,
        candidate_lock_sha256: Some(sha('1')),
        candidate_marker_path: None,
        root_merge_commit: None,
        composition_commit: None,
        composition_tree: None,
        candidate_hashes: Vec::new(),
        candidate: None,
        evidence_rolled_back: false,
        root_preservation: Vec::new(),
        preservation_prefix: None,
    });
    assert_eq!(
        validate_v1_publication(&case).unwrap_err().code,
        ErrorCode::UnexpectedPublicationEvidence
    );

    let mut aborted = record();
    aborted.state = OperationState::Aborted;
    aborted.pending_rollback = Some(PendingRollbackActionV1::Participant {
        member_id: "mem_a".to_owned(),
        action: ParticipantRollbackKindV1::AbortConflict,
        terminal_state: ParticipantState::Aborted,
    });
    assert_eq!(
        validate_v1_record(aborted).unwrap_err().code,
        ErrorCode::TerminalRollbackMismatch
    );
}

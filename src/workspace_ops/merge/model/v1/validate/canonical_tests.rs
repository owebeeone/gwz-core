use super::super::super::{OperationState, ParticipantState, PublicationProgress, PublicationStep};
use super::super::{
    CanonicalInstalledKind, CanonicalMergeRecord, ParticipantRollbackKindV1,
    PendingRollbackActionV1, PreservationPublicationHandoffV1, PublicationIndexFormV1,
    PublicationPrefixV1, RecordVersion,
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
fn canonical_v1_state_distinguishes_every_durable_handoff_variant() {
    let mut no_candidate = record();
    no_candidate.state = OperationState::Preserving;
    no_candidate.preservation_publication_handoff =
        Some(PreservationPublicationHandoffV1::NoCandidate);

    let mut evidence_pending = super::acceptance_tests::selected_acceptance_record_for_tests();
    evidence_pending.state = OperationState::Preserving;
    evidence_pending.publication =
        Some(super::acceptance_tests::valid_candidate_publication_for_tests(&evidence_pending));
    evidence_pending.publication.as_mut().unwrap().step =
        super::super::super::PublicationStep::CommittingEvidence;
    evidence_pending.preservation_publication_handoff =
        Some(PreservationPublicationHandoffV1::EvidencePending);

    let mut candidate = super::acceptance_tests::selected_acceptance_record_for_tests();
    candidate.state = OperationState::Preserving;
    candidate.publication =
        Some(super::acceptance_tests::valid_candidate_publication_for_tests(&candidate));
    candidate.preservation_publication_handoff =
        Some(PreservationPublicationHandoffV1::Candidate {
            prefix: PublicationPrefixV1::Baseline,
            index: PublicationIndexFormV1::Pre,
        });

    let canonical = [no_candidate, evidence_pending, candidate]
        .map(|record| CanonicalMergeRecord::from(validate_v1_record(record).unwrap()));
    assert!(
        canonical
            .iter()
            .all(|record| !record.v1_state().unwrap().is_empty())
    );
    assert_ne!(canonical[0], canonical[1]);
    assert_ne!(canonical[1], canonical[2]);
    assert_ne!(canonical[0], canonical[2]);
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

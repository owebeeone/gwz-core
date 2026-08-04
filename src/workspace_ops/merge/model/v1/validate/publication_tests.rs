use sha2::{Digest, Sha256};

use super::super::super::{
    OperationState, ParticipantState, PublicationCandidateHash, PublicationStep,
};
use super::tests::{oid, record};
use super::validate_v1_publication;
use crate::model::ErrorCode;

#[test]
fn candidate_phase_rejects_partial_composition_evidence() {
    let mut case = super::acceptance_tests::selected_acceptance_record_for_tests();
    let publication = super::acceptance_tests::valid_candidate_publication_for_tests(&case);
    case.publication = Some(publication);
    case.publication.as_mut().unwrap().step = PublicationStep::PublishingCandidate;
    case.publication.as_mut().unwrap().composition_commit = Some(oid('e'));
    assert_eq!(
        validate_v1_publication(&case).unwrap_err().code,
        ErrorCode::RecordedEvidenceDrift
    );
}

#[test]
fn no_publication_complete_requires_an_unchanged_accepted_result() {
    let mut case = record();
    case.state = OperationState::Finalizing;
    let participant = case.participants.get_mut("mem_a").unwrap();
    participant.state = ParticipantState::Merged;
    participant.resulting_commit = Some(oid('d'));
    case.publication = Some(super::acceptance_tests::empty_publication_for_tests(
        PublicationStep::Complete,
    ));
    assert_eq!(
        validate_v1_publication(&case).unwrap_err().code,
        ErrorCode::UnexpectedPublicationEvidence
    );
}

#[test]
fn aborted_candidate_requires_exactly_rolled_back_complete_evidence() {
    let mut case = super::acceptance_tests::selected_acceptance_record_for_tests();
    case.state = OperationState::Aborted;
    case.participants.get_mut("mem_a").unwrap().state = ParticipantState::RolledBack;
    case.publication = Some(super::acceptance_tests::valid_candidate_publication_for_tests(&case));
    let publication = case.publication.as_mut().unwrap();
    publication.step = PublicationStep::CommittingEvidence;
    publication.composition_commit = Some(oid('e'));
    publication.composition_tree = Some(oid('f'));
    let candidate = publication.candidate.as_ref().unwrap();
    let mut hashes = vec![
        PublicationCandidateHash {
            path: crate::artifact::LOCK_PATH.to_owned(),
            sha256: digest(&candidate.lock_yaml),
        },
        PublicationCandidateHash {
            path: publication.candidate_marker_path.clone().unwrap(),
            sha256: digest(&candidate.marker_yaml),
        },
    ];
    hashes.sort_by(|left, right| left.path.cmp(&right.path));
    publication.candidate_hashes = hashes;

    assert_eq!(
        validate_v1_publication(&case).unwrap_err().code,
        ErrorCode::TerminalRollbackMismatch
    );
    case.publication.as_mut().unwrap().evidence_rolled_back = true;
    validate_v1_publication(&case).unwrap();
}

fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

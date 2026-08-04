use super::super::acceptance::{
    CandidatePublicationObservation, CandidatePublicationPrefix, FinalizationNextAction,
    classify_candidate_publication, finalization_next_action, publication_prefix_allowed,
};
use super::super::{OperationState, PublicationStep};
use super::fixtures::{progress, record};

#[test]
fn candidate_prefix_reconciliation_and_step_legality_are_exhaustive() {
    let mut record = record();
    record.publication = Some(progress(PublicationStep::PublishingCandidate, true));
    let candidate = record
        .publication
        .as_ref()
        .unwrap()
        .candidate
        .as_ref()
        .unwrap();
    let baseline_lock = super::super::publication::sha256(candidate.baseline_lock_yaml.as_bytes());
    let candidate_lock = super::super::publication::sha256(candidate.lock_yaml.as_bytes());
    let rows = [
        (
            CandidatePublicationObservation::new(
                Some(baseline_lock.clone()),
                None,
                Some(candidate.baseline_boundary_sha256.clone()),
            ),
            Some(CandidatePublicationPrefix::Baseline),
        ),
        (
            CandidatePublicationObservation::new(
                Some(baseline_lock.clone()),
                Some(candidate.marker_sha256.clone()),
                Some(candidate.baseline_boundary_sha256.clone()),
            ),
            Some(CandidatePublicationPrefix::Marker),
        ),
        (
            CandidatePublicationObservation::new(
                Some(candidate_lock.clone()),
                Some(candidate.marker_sha256.clone()),
                Some(candidate.baseline_boundary_sha256.clone()),
            ),
            Some(CandidatePublicationPrefix::Lock),
        ),
        (
            CandidatePublicationObservation::new(
                Some(candidate_lock),
                Some(candidate.marker_sha256.clone()),
                Some(candidate.boundary_sha256.clone()),
            ),
            Some(CandidatePublicationPrefix::Boundary),
        ),
    ];
    for (observation, expected) in rows {
        assert_eq!(
            classify_candidate_publication(&record, &observation).unwrap(),
            expected
        );
    }
    for step in [
        PublicationStep::NotStarted,
        PublicationStep::ValidatingResults,
        PublicationStep::PreparingCandidate,
        PublicationStep::CommittingEvidence,
    ] {
        record.publication.as_mut().unwrap().step = step;
        assert!(publication_prefix_allowed(&record, CandidatePublicationPrefix::Baseline).unwrap());
        assert!(!publication_prefix_allowed(&record, CandidatePublicationPrefix::Marker).unwrap());
    }
    record.publication.as_mut().unwrap().step = PublicationStep::PublishingCandidate;
    for prefix in [
        CandidatePublicationPrefix::Baseline,
        CandidatePublicationPrefix::Marker,
        CandidatePublicationPrefix::Lock,
        CandidatePublicationPrefix::Boundary,
    ] {
        assert!(publication_prefix_allowed(&record, prefix).unwrap());
    }
}

#[test]
fn finalization_next_action_matches_every_durable_v0_window() {
    let mut record = record();
    assert_eq!(
        finalization_next_action(&record).unwrap(),
        FinalizationNextAction::ValidateResults
    );
    for (step, candidate, composition, expected) in [
        (
            PublicationStep::ValidatingResults,
            false,
            false,
            FinalizationNextAction::ValidateResults,
        ),
        (
            PublicationStep::PreparingCandidate,
            false,
            false,
            FinalizationNextAction::PrepareCandidate,
        ),
        (
            PublicationStep::PreparingCandidate,
            true,
            false,
            FinalizationNextAction::CreateOrAdoptEvidence,
        ),
        (
            PublicationStep::CommittingEvidence,
            true,
            false,
            FinalizationNextAction::CreateOrAdoptEvidence,
        ),
        (
            PublicationStep::CommittingEvidence,
            true,
            true,
            FinalizationNextAction::PublishCandidate,
        ),
        (
            PublicationStep::PublishingCandidate,
            true,
            true,
            FinalizationNextAction::PublishCandidate,
        ),
        (
            PublicationStep::VerifyingPublication,
            true,
            true,
            FinalizationNextAction::VerifyPublication,
        ),
    ] {
        let mut publication = progress(step, candidate);
        if composition {
            publication.composition_commit = Some("composition".to_owned());
            publication.composition_tree = Some("tree".to_owned());
        }
        record.publication = Some(publication);
        assert_eq!(
            finalization_next_action(&record).unwrap(),
            expected,
            "{step:?}"
        );
    }
    record.publication = Some(progress(PublicationStep::Complete, false));
    assert_eq!(
        finalization_next_action(&record).unwrap(),
        FinalizationNextAction::CompleteNoPublication
    );
    record.state = OperationState::Completed;
    assert_eq!(
        finalization_next_action(&record).unwrap(),
        FinalizationNextAction::ArchiveCompleted
    );
}

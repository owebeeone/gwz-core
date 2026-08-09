use super::super::authority::{
    BoundExactObservation, CompletedObservation, ExactObservationFact, ExecutionDiagnostic,
    NotStartedObservation, PhysicalActionKind, PublicationObservation, PublicationPhysicalAction,
    ResolvedV1Action, V1LifecycleRequest, V1NextAction, VerifiedCandidatePublicationCompletion,
    VerifiedPublicationAction, next_action, resolve_observation,
};
use super::super::checked::StoredV1Record;
use super::fixtures::evidence_rollback_record;
use crate::workspace_ops::merge::{OperationState, PublicationStep};
use crate::workspace_ops::tests::TempDir;

#[test]
fn completed_publication_rejects_a_different_same_owner_attempt() {
    let root = TempDir::new("merge-v1-publication-attempt-exactness");
    let current = StoredV1Record::for_test(&root.path, publishing_record(&root)).unwrap();
    let attempt = publication_attempt(&current, PublicationPhysicalAction::WriteMarker);
    let V1NextAction::Observe(request) =
        next_action(&current, V1LifecycleRequest::Continue).unwrap()
    else {
        panic!("publication owner was not reobserved")
    };
    let proof = VerifiedCandidatePublicationCompletion::for_test(
        &current,
        "@publication",
        "candidate_publication",
        "completed",
        (),
    )
    .unwrap();
    let observation = BoundExactObservation::for_test(
        &current,
        &request,
        ExactObservationFact::Completed(CompletedObservation::Publication(
            PublicationObservation::CandidatePublished(proof),
        )),
    )
    .unwrap();
    assert!(
        resolve_observation(
            &current,
            V1LifecycleRequest::Continue,
            request,
            observation,
            Some(attempt),
        )
        .is_err()
    );
}

#[test]
fn same_publication_action_after_attempt_is_rejected_as_no_progress() {
    let root = TempDir::new("merge-v1-publication-same-action-no-progress");
    let current = StoredV1Record::for_test(&root.path, publishing_record(&root)).unwrap();
    let attempt = publication_attempt(&current, PublicationPhysicalAction::WriteMarker);
    let V1NextAction::Observe(request) =
        next_action(&current, V1LifecycleRequest::Continue).unwrap()
    else {
        panic!("publication owner was not reobserved")
    };
    let proof = VerifiedPublicationAction::for_test(
        &current,
        "@publication",
        "publication_action",
        "write_marker",
        PublicationPhysicalAction::WriteMarker,
    )
    .unwrap();
    let observation = BoundExactObservation::for_test(
        &current,
        &request,
        ExactObservationFact::NotStarted(NotStartedObservation::Publication(proof)),
    )
    .unwrap();

    assert!(matches!(
        resolve_observation(
            &current,
            V1LifecycleRequest::Continue,
            request,
            observation,
            Some(attempt),
        )
        .unwrap(),
        ResolvedV1Action::Reject(_)
    ));
}

#[test]
fn exact_stage_attempt_can_record_candidate_publication_completion() {
    let root = TempDir::new("merge-v1-publication-stage-completion");
    let current = StoredV1Record::for_test(&root.path, publishing_record(&root)).unwrap();
    let attempt = publication_attempt(&current, PublicationPhysicalAction::StageIndex);
    let V1NextAction::Observe(request) =
        next_action(&current, V1LifecycleRequest::Continue).unwrap()
    else {
        panic!("publication owner was not reobserved")
    };
    let proof = VerifiedCandidatePublicationCompletion::for_test(
        &current,
        "@publication",
        "candidate_publication",
        "completed",
        (),
    )
    .unwrap();
    let observation = BoundExactObservation::for_test(
        &current,
        &request,
        ExactObservationFact::Completed(CompletedObservation::Publication(
            PublicationObservation::CandidatePublished(proof),
        )),
    )
    .unwrap();

    assert!(matches!(
        resolve_observation(
            &current,
            V1LifecycleRequest::Continue,
            request,
            observation,
            Some(attempt),
        )
        .unwrap(),
        ResolvedV1Action::Apply(_)
    ));
}

#[test]
fn exact_publication_prefix_progress_authorizes_the_next_same_owner_action() {
    let root = TempDir::new("merge-v1-publication-prefix-progress");
    let current = StoredV1Record::for_test(&root.path, publishing_record(&root)).unwrap();
    let attempt = publication_attempt(&current, PublicationPhysicalAction::WriteMarker);
    let V1NextAction::Observe(request) =
        next_action(&current, V1LifecycleRequest::Continue).unwrap()
    else {
        panic!("publication owner was not reobserved")
    };
    let proof = VerifiedPublicationAction::for_test(
        &current,
        "@publication",
        "publication_action",
        "write_lock",
        PublicationPhysicalAction::WriteLock,
    )
    .unwrap();
    let observation = BoundExactObservation::for_test(
        &current,
        &request,
        ExactObservationFact::NotStarted(NotStartedObservation::Publication(proof)),
    )
    .unwrap();

    let ResolvedV1Action::Execute(action) = resolve_observation(
        &current,
        V1LifecycleRequest::Continue,
        request,
        observation,
        Some(attempt),
    )
    .unwrap() else {
        panic!("exact prefix progress did not authorize the next publication action")
    };
    assert_eq!(
        action.kind(),
        &PhysicalActionKind::Publication(PublicationPhysicalAction::WriteLock)
    );
}

fn publication_attempt(
    current: &StoredV1Record,
    action: PublicationPhysicalAction,
) -> super::super::authority::BoundExecutionAttempt {
    let V1NextAction::Observe(request) =
        next_action(current, V1LifecycleRequest::Continue).unwrap()
    else {
        panic!("publication owner was not observed")
    };
    let proof = VerifiedPublicationAction::for_test(
        current,
        "@publication",
        "publication_action",
        action_phase(action),
        action,
    )
    .unwrap();
    let observation = BoundExactObservation::for_test(
        current,
        &request,
        ExactObservationFact::NotStarted(NotStartedObservation::Publication(proof)),
    )
    .unwrap();
    let ResolvedV1Action::Execute(action) = resolve_observation(
        current,
        V1LifecycleRequest::Continue,
        request,
        observation,
        None,
    )
    .unwrap() else {
        panic!("publication action was not authorized")
    };
    action
        .record_attempt(current, ExecutionDiagnostic::Success)
        .unwrap()
}

fn action_phase(action: PublicationPhysicalAction) -> &'static str {
    match action {
        PublicationPhysicalAction::EvidenceCommit => "evidence_commit",
        PublicationPhysicalAction::WriteMarker => "write_marker",
        PublicationPhysicalAction::WriteLock => "write_lock",
        PublicationPhysicalAction::WriteBoundary => "write_boundary",
        PublicationPhysicalAction::StageIndex => "stage_index",
    }
}

fn publishing_record(root: &TempDir) -> super::super::super::model::v1::MergeOperationRecordV1 {
    let mut model = evidence_rollback_record(root);
    model.state = OperationState::Finalizing;
    model.publication.as_mut().unwrap().step = PublicationStep::PublishingCandidate;
    model
}

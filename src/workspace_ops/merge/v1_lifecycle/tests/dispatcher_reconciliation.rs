use sha2::{Digest, Sha256};

use super::super::authority::{
    BoundAmbiguityEvidence, BoundExactObservation, CompletedObservation, EntryFact,
    ExactObservationFact, ExecutionDiagnostic, ParticipantActionPayload, ParticipantFailurePayload,
    ParticipantObservation, PhysicalActionKind, PreparedFailureHaltBatch, PreparedRollbackEntry,
    PreservationCursorPosition, PreservationObservation, PublicationObservation,
    PublicationPhysicalAction, ResolvedV1Action, V1LifecycleRequest, V1NextAction,
    VerifiedEvidenceResult, VerifiedParticipantOutcome, VerifiedPreservationExhausted,
    VerifiedPublicationHandoff, next_action, resolve_observation,
};
use super::super::checked::{StoredV1Record, V1MutationLease};
use super::super::transition::prepare;
use super::fixtures::{
    evidence_payload, evidence_rollback_record, preserving_record, up_to_date_action,
};
use crate::artifact::ManifestArtifact;
use crate::model::ErrorCode;
use crate::workspace_ops::merge::model::v1::{
    MergeOperationRecordV1, RecoveryOriginStateV1, test_record as record,
};
use crate::workspace_ops::merge::{MergeRecordError, OperationState, ParticipantState};
use crate::workspace_ops::tests::TempDir;

#[test]
fn halted_completion_resumes_only_after_the_final_halt_cause_is_removed() {
    let root = TempDir::new("merge-v1-halted-outcome-aggregate");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let mut model = record();
    add_second_participant(&mut model);
    model.state = OperationState::Halted;
    let first = model.participants.get_mut("mem_a").unwrap();
    first.state = ParticipantState::Failed;
    first.error = Some(git_error("first"));
    first.pending_action = Some(up_to_date_action());
    let second = model.participants.get_mut("mem_b").unwrap();
    second.state = ParticipantState::Failed;
    second.error = Some(git_error("second"));
    let current = StoredV1Record::for_test(&root.path, model).unwrap();
    let V1NextAction::Observe(request) =
        next_action(&current, V1LifecycleRequest::Continue).unwrap()
    else {
        panic!("pending halted owner was not observed")
    };
    let mut row = current.record().participants["mem_a"].clone();
    row.state = ParticipantState::UpToDate;
    row.resulting_commit = Some(row.before_commit.clone());
    row.error = None;
    row.pending_action = None;
    let proof = VerifiedParticipantOutcome::for_test(
        &current,
        "mem_a",
        "participant_outcome",
        "completed",
        ParticipantActionPayload {
            member_id: "mem_a".into(),
            row,
        },
    )
    .unwrap();
    let observation = BoundExactObservation::for_test(
        &current,
        &request,
        ExactObservationFact::Completed(CompletedObservation::Participant(
            ParticipantObservation::Outcome(Box::new(proof), EntryFact::None),
        )),
    )
    .unwrap();
    let ResolvedV1Action::Apply(transition) = resolve_observation(
        &current,
        V1LifecycleRequest::Continue,
        request,
        observation,
        None,
    )
    .unwrap() else {
        panic!("halted aggregate outcome was not resolved")
    };
    let rewrite = prepare(&lease, &current, transition).unwrap();
    assert_eq!(rewrite.next().state, OperationState::Halted);
    assert_eq!(
        rewrite.next().participants["mem_a"].state,
        ParticipantState::UpToDate
    );
    assert_eq!(
        rewrite.next().participants["mem_b"].state,
        ParticipantState::Failed
    );
}

#[test]
fn preparation_failure_must_match_the_requested_participant() {
    let root = TempDir::new("merge-v1-preparation-failure-owner");
    let mut model = record();
    add_second_participant(&mut model);
    let current = StoredV1Record::for_test(&root.path, model).unwrap();
    let V1NextAction::Observe(request) =
        next_action(&current, V1LifecycleRequest::Continue).unwrap()
    else {
        panic!("first participant preparation was not requested")
    };
    let mut row = current.record().participants["mem_b"].clone();
    row.state = ParticipantState::Failed;
    row.error = Some(git_error("wrong owner"));
    let batch = PreparedFailureHaltBatch::for_test(
        &current,
        "mem_b",
        "preparation_failure",
        "verified",
        ParticipantFailurePayload {
            member_id: "mem_b".into(),
            row,
            later_unattempted: Vec::new(),
        },
    )
    .unwrap();
    let observation = BoundExactObservation::for_test(
        &current,
        &request,
        ExactObservationFact::Completed(CompletedObservation::Participant(
            ParticipantObservation::PreparationFailed(Box::new(batch)),
        )),
    )
    .unwrap();
    assert!(matches!(
        resolve_observation(
            &current,
            V1LifecycleRequest::Continue,
            request,
            observation,
            None,
        )
        .unwrap(),
        ResolvedV1Action::Reject(_)
    ));
}

#[test]
fn abort_and_preserve_record_live_evidence_before_their_entry() {
    let root = TempDir::new("merge-v1-finalizing-handoff-evidence");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    for lifecycle in [V1LifecycleRequest::Abort, V1LifecycleRequest::Preserve] {
        let current = StoredV1Record::for_test(&root.path, publication_record(&root)).unwrap();
        let V1NextAction::Observe(request) = next_action(&current, lifecycle).unwrap() else {
            panic!("finalizing entry did not request a handoff observation")
        };
        let proof = VerifiedEvidenceResult::for_test(
            &current,
            "@publication",
            "record_evidence",
            "completed",
            evidence_payload(&current),
        )
        .unwrap();
        let observation = BoundExactObservation::for_test(
            &current,
            &request,
            ExactObservationFact::Completed(CompletedObservation::Publication(
                PublicationObservation::EvidenceResult(Box::new(proof)),
            )),
        )
        .unwrap();
        let ResolvedV1Action::Apply(transition) =
            resolve_observation(&current, lifecycle, request, observation, None).unwrap()
        else {
            panic!("live evidence was not ordered before the requested entry")
        };
        let rewrite = prepare(&lease, &current, transition).unwrap();
        assert_eq!(rewrite.next().state, OperationState::Finalizing);
        assert!(
            rewrite
                .next()
                .publication
                .as_ref()
                .unwrap()
                .composition_commit
                .is_some()
        );
    }
}

#[test]
fn completed_preservation_enters_rollback_for_every_mutating_resume_request() {
    let root = TempDir::new("merge-v1-preservation-exhausted-request-matrix");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    for lifecycle in [
        V1LifecycleRequest::ResumeStart,
        V1LifecycleRequest::Continue,
        V1LifecycleRequest::Abort,
        V1LifecycleRequest::Preserve,
        V1LifecycleRequest::Archive,
    ] {
        let current = StoredV1Record::for_test(&root.path, preserving_record()).unwrap();
        let V1NextAction::Observe(request) = next_action(&current, lifecycle).unwrap() else {
            panic!("preserving cursor was not resumed")
        };
        let exhausted = VerifiedPreservationExhausted::for_test(
            &current,
            "@operation",
            "preservation_exhausted",
            "verified",
            (),
        )
        .unwrap();
        let handoff = VerifiedPublicationHandoff::for_test(
            &current,
            "@publication",
            "handoff",
            "verified",
            (),
        )
        .unwrap();
        let entry = PreparedRollbackEntry::from_preserving_for_test(
            &current,
            current.record(),
            handoff,
            exhausted,
        )
        .unwrap();
        let observation = BoundExactObservation::for_test(
            &current,
            &request,
            ExactObservationFact::Completed(CompletedObservation::Preservation(
                PreservationObservation::Exhausted(Box::new(entry)),
            )),
        )
        .unwrap();
        let ResolvedV1Action::Apply(transition) =
            resolve_observation(&current, lifecycle, request, observation, None).unwrap()
        else {
            panic!("completed preservation did not enter rollback")
        };
        assert_eq!(
            prepare(&lease, &current, transition).unwrap().next().state,
            OperationState::RollingBack
        );
    }
}

#[test]
fn ambiguous_observations_override_both_executor_diagnostics_for_every_action_owner() {
    use super::dispatcher_attempt_matrix as attempts;

    let root = TempDir::new("merge-v1-ambiguous-after-diagnostic-matrix");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let mut resolution = attempts::participant_record();
    let row = resolution.participants.get_mut("mem_a").unwrap();
    row.state = ParticipantState::Conflicted;
    row.expected_merge_head = Some(row.source_commit.clone());
    row.pending_action = Some(super::dispatcher::resolve_action());
    let cases = [
        (
            "participant",
            attempts::participant_record(),
            PhysicalActionKind::Participant {
                member_id: "mem_a".into(),
                action: Box::new(up_to_date_action()),
            },
            RecoveryOriginStateV1::Executing,
        ),
        (
            "resolution",
            resolution,
            PhysicalActionKind::Participant {
                member_id: "mem_a".into(),
                action: Box::new(super::dispatcher::resolve_action()),
            },
            RecoveryOriginStateV1::Executing,
        ),
        (
            "publication",
            attempts::publication_record(&root),
            PhysicalActionKind::Publication(PublicationPhysicalAction::EvidenceCommit),
            RecoveryOriginStateV1::Finalizing,
        ),
        (
            "preservation",
            attempts::preservation_record(),
            PhysicalActionKind::Preservation(super::fixtures::backup_action()),
            RecoveryOriginStateV1::Preserving,
        ),
        (
            "rollback",
            attempts::rollback_record(),
            PhysicalActionKind::Rollback(attempts::rollback_action()),
            RecoveryOriginStateV1::RollingBack,
        ),
    ];

    for (name, model, action, origin) in cases {
        for failed in [false, true] {
            let current = StoredV1Record::for_test(&root.path, model.clone()).unwrap();
            let diagnostic = if failed {
                attempts::failed_diagnostic()
            } else {
                ExecutionDiagnostic::Success
            };
            let attempt = attempts::execution_attempt(&current, action.clone(), diagnostic);
            let V1NextAction::Observe(request) =
                next_action(&current, V1LifecycleRequest::Continue).unwrap()
            else {
                panic!("{name} owner was not reobserved")
            };
            let ambiguity = BoundAmbiguityEvidence::for_test(
                &current,
                "@operation",
                "enter_recovery",
                "ambiguous",
                origin,
            )
            .unwrap();
            let fact = if origin == RecoveryOriginStateV1::Preserving {
                ExactObservationFact::PreservationAmbiguous(
                    ambiguity,
                    super::fixtures::preservation_prefix(
                        &current,
                        PreservationCursorPosition::BackupRef,
                    ),
                )
            } else {
                ExactObservationFact::Ambiguous(ambiguity)
            };
            let observation = BoundExactObservation::for_test(&current, &request, fact).unwrap();
            let ResolvedV1Action::Apply(transition) = resolve_observation(
                &current,
                V1LifecycleRequest::Continue,
                request,
                observation,
                Some(attempt),
            )
            .unwrap() else {
                panic!("{name} ambiguity did not take authority")
            };
            assert_eq!(
                prepare(&lease, &current, transition).unwrap().next().state,
                OperationState::RecoveryRequired,
                "{name} failed={failed}"
            );
        }
    }
}

fn publication_record(root: &TempDir) -> MergeOperationRecordV1 {
    let mut model = evidence_rollback_record(root);
    model.state = OperationState::Finalizing;
    let progress = model.publication.as_mut().unwrap();
    progress.composition_commit = None;
    progress.composition_tree = None;
    progress.root_merge_commit = None;
    progress.candidate_hashes.clear();
    model
}

fn add_second_participant(model: &mut MergeOperationRecordV1) {
    let mut manifest =
        ManifestArtifact::from_yaml(model.baseline.manifest_yaml.as_deref().unwrap()).unwrap();
    let mut member = manifest.members[0].clone();
    member.id = "mem_b".into();
    member.path = "members/b".into();
    member.source_id = "src_b".into();
    manifest.members.push(member);
    let manifest = manifest.to_yaml().unwrap();
    model.baseline.manifest_sha256 = format!("{:x}", Sha256::digest(manifest.as_bytes()));
    model.baseline.manifest_yaml = Some(manifest);
    let mut second = model.participants["mem_a"].clone();
    second.path = "members/b".into();
    model.participants.insert("mem_b".into(), second);
    model.selected_targets.push("mem_b".into());
}

fn git_error(message: &str) -> MergeRecordError {
    MergeRecordError {
        code: ErrorCode::GitCommandFailed,
        message: message.into(),
        detail: None,
    }
}

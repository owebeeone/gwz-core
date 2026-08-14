use super::super::authority::{
    BoundAmbiguityEvidence, BoundExactObservation, BoundExecutionAttempt, CompletedObservation,
    EntryFact, ExactObservationFact, ExecutionDiagnostic, NotStartedObservation,
    ParticipantActionPayload, ParticipantObservation, PhysicalActionKind,
    PreservationCursorPosition, PreservationObservation, PublicationObservation,
    PublicationPhysicalAction, ResolvedV1Action, RollbackObservation, V1LifecycleRequest,
    V1NextAction, VerifiedBackupRef, VerifiedEvidenceResult, VerifiedParticipantNotStarted,
    VerifiedParticipantOutcome, VerifiedParticipantRollback, VerifiedPublicationAction,
    VerifiedPublicationHandoff, next_action, resolve_observation,
};
use super::super::checked::{StoredV1Record, V1MutationLease};
use super::super::transition::ReverseEntryKind;
use super::super::transition::prepare;
use super::fixtures::{
    backup_action, evidence_payload, evidence_rollback_record, preservation_evidence,
    preservation_payload, preservation_prefix, preserving_record, up_to_date_action,
};
use crate::model::ErrorCode;
use crate::workspace_ops::merge::model::v1::{
    MergeOperationRecordV1, ParticipantRollbackKindV1, PendingPreservationActionV1,
    PendingRollbackActionV1, RecoveryOriginStateV1, test_record as record,
};
use crate::workspace_ops::merge::{OperationState, ParticipantState};
use crate::workspace_ops::tests::TempDir;

#[test]
fn exact_owner_attempt_classification_matrix_is_closed() {
    let root = TempDir::new_git("merge-v1-exact-owner-attempt-matrix");
    let cases = [
        (
            "participant",
            participant_record(),
            PhysicalActionKind::Participant {
                member_id: "mem_a".into(),
                action: Box::new(up_to_date_action()),
            },
            RecoveryOriginStateV1::Executing,
            true,
        ),
        (
            "publication",
            publication_record(&root),
            PhysicalActionKind::Publication(PublicationPhysicalAction::EvidenceCommit),
            RecoveryOriginStateV1::Finalizing,
            false,
        ),
        (
            "preservation",
            preservation_record(),
            PhysicalActionKind::Preservation(backup_action()),
            RecoveryOriginStateV1::Preserving,
            false,
        ),
        (
            "rollback",
            rollback_record(),
            PhysicalActionKind::Rollback(rollback_action()),
            RecoveryOriginStateV1::RollingBack,
            false,
        ),
    ];

    for (name, model, expected, origin, participant) in cases {
        let current = StoredV1Record::for_test(&root.path, model).unwrap();
        for candidate in physical_candidates() {
            let result = authorize(&current, candidate.clone());
            if candidate == expected {
                assert!(
                    matches!(result.unwrap(), ResolvedV1Action::Execute(_)),
                    "{name}"
                );
            } else {
                assert!(result.is_err(), "{name} accepted {candidate:?}");
                let attempt =
                    execution_attempt(&current, expected.clone(), ExecutionDiagnostic::Success);
                assert!(
                    resolve_fresh(
                        &current,
                        not_started_fact(&current, candidate.clone()),
                        Some(attempt),
                    )
                    .is_err(),
                    "{name} replayed its attempt against {candidate:?}"
                );
            }
        }

        let success = execution_attempt(&current, expected.clone(), ExecutionDiagnostic::Success);
        assert!(matches!(
            resolve_fresh(
                &current,
                not_started_fact(&current, expected.clone()),
                Some(success)
            )
            .unwrap(),
            ResolvedV1Action::Reject(_)
        ));

        let failed = execution_attempt(&current, expected.clone(), failed_diagnostic());
        let after_failure = resolve_fresh(
            &current,
            not_started_fact(&current, expected.clone()),
            Some(failed),
        )
        .unwrap();
        assert_eq!(
            matches!(after_failure, ResolvedV1Action::Apply(_)),
            participant,
            "{name}"
        );

        let ambiguity = BoundAmbiguityEvidence::for_test(
            &current,
            "@operation",
            "enter_recovery",
            "ambiguous",
            origin,
        )
        .unwrap();
        assert!(matches!(
            resolve_fresh(&current, ambiguity_fact(&current, ambiguity), None).unwrap(),
            ResolvedV1Action::Apply(_)
        ));
    }
}

fn ambiguity_fact(current: &StoredV1Record, proof: BoundAmbiguityEvidence) -> ExactObservationFact {
    if current.record().pending_preservation.is_some() {
        ExactObservationFact::PreservationAmbiguous(
            proof,
            preservation_prefix(current, PreservationCursorPosition::BackupRef),
        )
    } else {
        ExactObservationFact::Ambiguous(proof)
    }
}

#[test]
fn completed_observations_override_late_executor_diagnostics_for_every_owner() {
    let root = TempDir::new_git("merge-v1-completed-after-diagnostic-matrix");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();

    let current = StoredV1Record::for_test(&root.path, participant_record()).unwrap();
    let action = PhysicalActionKind::Participant {
        member_id: "mem_a".into(),
        action: Box::new(up_to_date_action()),
    };
    let attempt = execution_attempt(&current, action, failed_diagnostic());
    let mut row = current.record().participants["mem_a"].clone();
    row.pending_action = None;
    row.state = ParticipantState::UpToDate;
    row.resulting_commit = Some(row.before_commit.clone());
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
    apply_resolved(
        &lease,
        &current,
        CompletedObservation::Participant(ParticipantObservation::Outcome(
            Box::new(proof),
            EntryFact::None,
        )),
        attempt,
    );

    let current = StoredV1Record::for_test(&root.path, publication_record(&root)).unwrap();
    let action = PhysicalActionKind::Publication(PublicationPhysicalAction::EvidenceCommit);
    let attempt = execution_attempt(&current, action, failed_diagnostic());
    let proof = VerifiedEvidenceResult::for_test(
        &current,
        "@publication",
        "record_evidence",
        "completed",
        evidence_payload(&current),
    )
    .unwrap();
    apply_resolved(
        &lease,
        &current,
        CompletedObservation::Publication(PublicationObservation::EvidenceResult(Box::new(proof))),
        attempt,
    );

    let current = StoredV1Record::for_test(&root.path, preservation_record()).unwrap();
    let action = PhysicalActionKind::Preservation(backup_action());
    let attempt = execution_attempt(&current, action, failed_diagnostic());
    let position = PreservationCursorPosition::BackupRef;
    let proof = VerifiedBackupRef::for_test(
        &current,
        "mem_a",
        "finish_backup_ref",
        "completed",
        preservation_payload(position, None, Some(preservation_evidence(false))),
        preservation_prefix(&current, position),
    )
    .unwrap();
    apply_resolved(
        &lease,
        &current,
        CompletedObservation::Preservation(PreservationObservation::BackupDone(Box::new(proof))),
        attempt,
    );

    let current = StoredV1Record::for_test(&root.path, rollback_record()).unwrap();
    let action = PhysicalActionKind::Rollback(rollback_action());
    let attempt = execution_attempt(&current, action, failed_diagnostic());
    let mut row = current.record().participants["mem_a"].clone();
    row.state = ParticipantState::RolledBack;
    let proof = VerifiedParticipantRollback::for_test(
        &current,
        "mem_a",
        "finish_participant_rollback",
        "completed",
        ParticipantActionPayload {
            member_id: "mem_a".into(),
            row,
        },
    )
    .unwrap();
    apply_resolved(
        &lease,
        &current,
        CompletedObservation::Rollback(RollbackObservation::ParticipantDone(Box::new(proof))),
        attempt,
    );
}

#[test]
fn abort_and_preserve_abandon_only_their_bound_not_started_owner() {
    let root = TempDir::new_git("merge-v1-abandon-request-matrix");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    for request in [V1LifecycleRequest::Abort, V1LifecycleRequest::Preserve] {
        let current = StoredV1Record::for_test(&root.path, participant_record()).unwrap();
        let ordinary = not_started_fact(
            &current,
            PhysicalActionKind::Participant {
                member_id: "mem_a".into(),
                action: Box::new(up_to_date_action()),
            },
        );
        assert!(matches!(
            resolve_fresh_for_request(&current, request, ordinary, None).unwrap(),
            ResolvedV1Action::Reject(_)
        ));
        let mut anticipated = current.record().clone();
        anticipated
            .participants
            .get_mut("mem_a")
            .unwrap()
            .pending_action = None;
        let proof = VerifiedParticipantNotStarted::for_test(
            &current,
            "mem_a",
            "participant_action",
            "not_started",
            "mem_a".into(),
        )
        .unwrap();
        let kind = match request {
            V1LifecycleRequest::Abort => ReverseEntryKind::DirectRollback,
            V1LifecycleRequest::Preserve => ReverseEntryKind::Preservation,
            _ => unreachable!(),
        };
        let handoff =
            VerifiedPublicationHandoff::for_entry_test(&current, kind, &anticipated).unwrap();
        let entry = match request {
            V1LifecycleRequest::Abort => EntryFact::Rollback(Box::new(
                super::super::authority::PreparedRollbackEntry::direct_for_test(
                    &current,
                    &anticipated,
                    handoff,
                )
                .unwrap(),
            )),
            V1LifecycleRequest::Preserve => EntryFact::Preservation(Box::new(
                super::super::authority::PreparedPreservationEntry::for_test(
                    &current,
                    &anticipated,
                    handoff,
                )
                .unwrap(),
            )),
            _ => unreachable!(),
        };
        let V1NextAction::Observe(observation_request) = next_action(&current, request).unwrap()
        else {
            panic!("persisted participant owner was not reconciled")
        };
        let observation = BoundExactObservation::for_test(
            &current,
            &observation_request,
            ExactObservationFact::Abandon(Box::new(proof), entry),
        )
        .unwrap();
        let ResolvedV1Action::Apply(transition) =
            resolve_observation(&current, request, observation_request, observation, None).unwrap()
        else {
            panic!("request-specific abandonment was not resolved")
        };
        let rewrite = prepare(&lease, &current, transition).unwrap();
        let expected = if request == V1LifecycleRequest::Abort {
            OperationState::RollingBack
        } else {
            OperationState::Preserving
        };
        assert_eq!(rewrite.next().state, expected);
        assert!(
            rewrite.next().participants["mem_a"]
                .pending_action
                .is_none()
        );
    }
}

fn authorize(
    current: &StoredV1Record,
    action: PhysicalActionKind,
) -> crate::model::ModelResult<ResolvedV1Action> {
    resolve_fresh(current, not_started_fact(current, action), None)
}

fn not_started_fact(current: &StoredV1Record, action: PhysicalActionKind) -> ExactObservationFact {
    let observation = match action {
        PhysicalActionKind::Participant { member_id, action } => {
            NotStartedObservation::Participant { member_id, action }
        }
        PhysicalActionKind::Publication(action) => {
            let phase = match action {
                PublicationPhysicalAction::EvidenceCommit => "evidence_commit",
                PublicationPhysicalAction::WriteMarker => "write_marker",
                PublicationPhysicalAction::WriteLock => "write_lock",
                PublicationPhysicalAction::WriteBoundary => "write_boundary",
                PublicationPhysicalAction::StageIndex => "stage_index",
            };
            NotStartedObservation::Publication(
                VerifiedPublicationAction::for_test(
                    current,
                    "@publication",
                    "publication_action",
                    phase,
                    action,
                )
                .unwrap(),
            )
        }
        PhysicalActionKind::Preservation(action) => NotStartedObservation::Preservation {
            prefix: preservation_prefix(current, PreservationCursorPosition::BackupRef),
            action,
        },
        PhysicalActionKind::Rollback(action) => NotStartedObservation::Rollback(action),
        PhysicalActionKind::Archive => NotStartedObservation::Archive,
    };
    ExactObservationFact::NotStarted(observation)
}

fn resolve_fresh(
    current: &StoredV1Record,
    fact: ExactObservationFact,
    attempt: Option<BoundExecutionAttempt>,
) -> crate::model::ModelResult<ResolvedV1Action> {
    let request_kind = V1LifecycleRequest::Continue;
    let V1NextAction::Observe(request) = next_action(current, request_kind)? else {
        panic!("owner did not request observation")
    };
    let observation = BoundExactObservation::for_test(current, &request, fact)?;
    resolve_observation(current, request_kind, request, observation, attempt)
}

fn resolve_fresh_for_request(
    current: &StoredV1Record,
    request_kind: V1LifecycleRequest,
    fact: ExactObservationFact,
    attempt: Option<BoundExecutionAttempt>,
) -> crate::model::ModelResult<ResolvedV1Action> {
    let V1NextAction::Observe(request) = next_action(current, request_kind)? else {
        panic!("owner did not request observation")
    };
    let observation = BoundExactObservation::for_test(current, &request, fact)?;
    resolve_observation(current, request_kind, request, observation, attempt)
}

pub(super) fn execution_attempt(
    current: &StoredV1Record,
    action: PhysicalActionKind,
    diagnostic: ExecutionDiagnostic,
) -> BoundExecutionAttempt {
    let ResolvedV1Action::Execute(action) = authorize(current, action).unwrap() else {
        panic!("exact physical action was not authorized")
    };
    action.record_attempt(current, diagnostic).unwrap()
}

fn apply_resolved(
    lease: &V1MutationLease,
    current: &StoredV1Record,
    completed: CompletedObservation,
    attempt: BoundExecutionAttempt,
) {
    let ResolvedV1Action::Apply(transition) = resolve_fresh(
        current,
        ExactObservationFact::Completed(completed),
        Some(attempt),
    )
    .unwrap() else {
        panic!("completed observation did not override the late diagnostic")
    };
    prepare(lease, current, transition).unwrap();
}

fn physical_candidates() -> Vec<PhysicalActionKind> {
    let mut wrong_participant = up_to_date_action();
    wrong_participant.source_commit = "f".repeat(40);
    vec![
        PhysicalActionKind::Participant {
            member_id: "mem_a".into(),
            action: Box::new(up_to_date_action()),
        },
        PhysicalActionKind::Participant {
            member_id: "wrong".into(),
            action: Box::new(up_to_date_action()),
        },
        PhysicalActionKind::Participant {
            member_id: "mem_a".into(),
            action: Box::new(wrong_participant),
        },
        PhysicalActionKind::Publication(PublicationPhysicalAction::EvidenceCommit),
        PhysicalActionKind::Publication(PublicationPhysicalAction::WriteMarker),
        PhysicalActionKind::Preservation(backup_action()),
        PhysicalActionKind::Preservation(PendingPreservationActionV1::BackupRef {
            owner: super::fixtures::preservation_owner(),
            name: "refs/gwz/merge/wrong/head".into(),
            target_commit: "a".repeat(40),
        }),
        PhysicalActionKind::Rollback(rollback_action()),
        PhysicalActionKind::Rollback(PendingRollbackActionV1::PublicationEvidence {
            next_step:
                crate::workspace_ops::merge::model::v1::EvidenceRollbackStepV1::EvidenceCommit,
        }),
        PhysicalActionKind::Archive,
    ]
}

pub(super) fn participant_record() -> MergeOperationRecordV1 {
    let mut model = record();
    model.participants.get_mut("mem_a").unwrap().pending_action = Some(up_to_date_action());
    model
}

pub(super) fn publication_record(root: &TempDir) -> MergeOperationRecordV1 {
    let mut model = evidence_rollback_record(root);
    model.state = OperationState::Finalizing;
    let progress = model.publication.as_mut().unwrap();
    progress.composition_commit = None;
    progress.composition_tree = None;
    progress.root_merge_commit = None;
    progress.candidate_hashes.clear();
    model
}

pub(super) fn preservation_record() -> MergeOperationRecordV1 {
    let mut model = preserving_record();
    model.pending_preservation = Some(backup_action());
    model
}

pub(super) fn rollback_record() -> MergeOperationRecordV1 {
    let mut model = record();
    model.state = OperationState::RollingBack;
    let row = model.participants.get_mut("mem_a").unwrap();
    row.state = ParticipantState::FastForwarded;
    row.resulting_commit = Some("d".repeat(40));
    model.pending_rollback = Some(rollback_action());
    model
}

pub(super) fn rollback_action() -> PendingRollbackActionV1 {
    PendingRollbackActionV1::Participant {
        member_id: "mem_a".into(),
        action: ParticipantRollbackKindV1::ResetIntegrated,
        terminal_state: ParticipantState::RolledBack,
    }
}

pub(super) fn failed_diagnostic() -> ExecutionDiagnostic {
    ExecutionDiagnostic::Failed {
        code: ErrorCode::GitCommandFailed,
        message: "late executor diagnostic".into(),
        detail: Some("diagnostic is not outcome authority".into()),
    }
}

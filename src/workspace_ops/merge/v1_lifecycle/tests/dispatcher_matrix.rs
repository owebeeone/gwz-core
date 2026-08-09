use super::super::authority::{
    BoundAmbiguityEvidence, BoundExactObservation, CompletedObservation, ExactObservationFact,
    NotStartedObservation, ResolvedV1Action, V1LifecycleRequest, V1NextAction,
    VerifiedRecoveryOrigin, next_action, resolve_observation,
};
use super::super::checked::{StoredV1Record, V1MutationLease};
use super::super::transition::prepare;
use super::fixtures::{
    accepted_workspace, align_baseline_lock, backup_action, preserving_record, up_to_date_action,
};
use crate::model::ErrorCode;
use crate::workspace_ops::merge::model::v1::{
    MergeOperationRecordV1, ParticipantRollbackKindV1, PendingRollbackActionV1, RecoveryContextV1,
    RecoveryOriginStateV1, test_record as record,
};
use crate::workspace_ops::merge::{
    MergeRecordError, OperationState, ParticipantState, PublicationProgress, PublicationStep,
};
use crate::workspace_ops::tests::TempDir;

#[test]
fn request_state_dispatch_matrix_is_closed_and_precedence_ordered() {
    use V1LifecycleRequest as R;
    let root = TempDir::new("merge-v1-dispatch-request-state-matrix");
    let requests = [
        R::ResumeStart,
        R::Continue,
        R::Abort,
        R::Preserve,
        R::Status,
        R::Archive,
    ];
    let states = [
        OperationState::Executing,
        OperationState::AwaitingResolution,
        OperationState::Halted,
        OperationState::Finalizing,
        OperationState::Preserving,
        OperationState::RollingBack,
        OperationState::Completed,
        OperationState::Aborted,
        OperationState::RecoveryRequired,
    ];
    for state in states {
        for request in requests {
            let current = StoredV1Record::for_test(&root.path, record_for_state(state)).unwrap();
            assert_eq!(
                action_class(next_action(&current, request).unwrap()),
                expected_action_class(state, request),
                "state={state:?} request={request:?}"
            );
        }
    }
}

#[test]
fn request_binding_and_between_action_ambiguity_reject_replay() {
    use V1LifecycleRequest as R;
    let root = TempDir::new("merge-v1-dispatch-request-binding");
    let mut pending = record();
    pending
        .participants
        .get_mut("mem_a")
        .unwrap()
        .pending_action = Some(up_to_date_action());
    for replay in [R::ResumeStart, R::Abort, R::Preserve, R::Status, R::Archive] {
        let current = StoredV1Record::for_test(&root.path, pending.clone()).unwrap();
        let V1NextAction::Observe(request) = next_action(&current, R::Continue).unwrap() else {
            panic!("pending participant was not observed")
        };
        let observation = BoundExactObservation::for_test(
            &current,
            &request,
            ExactObservationFact::NotStarted(NotStartedObservation::Participant {
                member_id: "mem_a".into(),
                action: Box::new(up_to_date_action()),
            }),
        )
        .unwrap();
        assert!(resolve_observation(&current, replay, request, observation, None).is_err());
    }
    for (mut model, origin) in [
        (preserving_record(), RecoveryOriginStateV1::Preserving),
        (
            record_for_state(OperationState::RollingBack),
            RecoveryOriginStateV1::RollingBack,
        ),
    ] {
        model.pending_preservation = None;
        model.pending_rollback = None;
        let current = StoredV1Record::for_test(&root.path, model).unwrap();
        let V1NextAction::Observe(request) = next_action(&current, R::Abort).unwrap() else {
            panic!("between-action state was not observed")
        };
        let proof = BoundAmbiguityEvidence::for_test(
            &current,
            "@operation",
            "enter_recovery",
            "ambiguous",
            origin,
        )
        .unwrap();
        let observation = BoundExactObservation::for_test(
            &current,
            &request,
            ExactObservationFact::Ambiguous(proof),
        )
        .unwrap();
        assert!(matches!(
            resolve_observation(&current, R::Abort, request, observation, None).unwrap(),
            ResolvedV1Action::Reject(_)
        ));
    }
}

#[test]
fn recovery_resolver_restores_each_literal_recorded_origin() {
    let root = TempDir::new("merge-v1-recovery-origin-matrix");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let cases = [
        (RecoveryOriginStateV1::Executing, OperationState::Executing),
        (
            RecoveryOriginStateV1::AwaitingResolution,
            OperationState::AwaitingResolution,
        ),
        (RecoveryOriginStateV1::Halted, OperationState::Halted),
        (
            RecoveryOriginStateV1::Finalizing,
            OperationState::Finalizing,
        ),
        (
            RecoveryOriginStateV1::Preserving,
            OperationState::Preserving,
        ),
        (
            RecoveryOriginStateV1::RollingBack,
            OperationState::RollingBack,
        ),
    ];
    for (origin, expected) in cases {
        let current = StoredV1Record::for_test(&root.path, record_for_origin(origin)).unwrap();
        for (wrong_origin, _) in cases
            .into_iter()
            .filter(|(candidate, _)| *candidate != origin)
        {
            let V1NextAction::Observe(request) =
                next_action(&current, V1LifecycleRequest::Continue).unwrap()
            else {
                panic!("recovery origin was not observed")
            };
            let proof = VerifiedRecoveryOrigin::for_test(
                &current,
                "@operation",
                "resume_recovery",
                "verified",
                wrong_origin,
            )
            .unwrap();
            let observation = BoundExactObservation::for_test(
                &current,
                &request,
                ExactObservationFact::Completed(CompletedObservation::Recovery(proof)),
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
                panic!("wrong recovery proof did not reach the checked reducer")
            };
            assert!(
                prepare(&lease, &current, transition).is_err(),
                "recorded={origin:?} accepted={wrong_origin:?}"
            );
        }
        let V1NextAction::Observe(request) =
            next_action(&current, V1LifecycleRequest::Continue).unwrap()
        else {
            panic!("recovery origin was not observed")
        };
        let proof = VerifiedRecoveryOrigin::for_test(
            &current,
            "@operation",
            "resume_recovery",
            "verified",
            origin,
        )
        .unwrap();
        let observation = BoundExactObservation::for_test(
            &current,
            &request,
            ExactObservationFact::Completed(CompletedObservation::Recovery(proof)),
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
            panic!("recovery proof did not map to resume")
        };
        assert_eq!(
            prepare(&lease, &current, transition).unwrap().next().state,
            expected
        );
    }
}

fn record_for_origin(origin: RecoveryOriginStateV1) -> MergeOperationRecordV1 {
    let state = match origin {
        RecoveryOriginStateV1::Executing => OperationState::Executing,
        RecoveryOriginStateV1::AwaitingResolution => OperationState::AwaitingResolution,
        RecoveryOriginStateV1::Halted => OperationState::Halted,
        RecoveryOriginStateV1::Finalizing => OperationState::Finalizing,
        RecoveryOriginStateV1::Preserving => OperationState::Preserving,
        RecoveryOriginStateV1::RollingBack => OperationState::RollingBack,
    };
    let mut model = record_for_state(state);
    if origin == RecoveryOriginStateV1::Preserving {
        model.pending_preservation = Some(backup_action());
    } else if origin == RecoveryOriginStateV1::RollingBack {
        let row = model.participants.get_mut("mem_a").unwrap();
        row.state = ParticipantState::FastForwarded;
        row.resulting_commit = Some("d".repeat(40));
        model.pending_rollback = Some(PendingRollbackActionV1::Participant {
            member_id: "mem_a".into(),
            action: ParticipantRollbackKindV1::ResetIntegrated,
            terminal_state: ParticipantState::RolledBack,
        });
    }
    model.state = OperationState::RecoveryRequired;
    model.recovery_context = Some(RecoveryContextV1 {
        origin_state: origin,
    });
    model
}

fn record_for_state(state: OperationState) -> MergeOperationRecordV1 {
    let mut model = if state == OperationState::Preserving {
        preserving_record()
    } else {
        record()
    };
    model.state = state;
    let row = model.participants.get_mut("mem_a").unwrap();
    match state {
        OperationState::AwaitingResolution => {
            row.state = ParticipantState::Conflicted;
            row.expected_merge_head = Some(row.source_commit.clone());
        }
        OperationState::Halted => {
            row.state = ParticipantState::Failed;
            row.error = Some(git_error("halted"));
        }
        OperationState::Finalizing | OperationState::Completed => {
            row.state = ParticipantState::UpToDate;
            row.resulting_commit = Some(row.before_commit.clone());
            align_baseline_lock(&mut model);
            if state == OperationState::Completed {
                model.state = OperationState::Finalizing;
                let seed =
                    StoredV1Record::for_test(std::path::Path::new("."), model.clone()).unwrap();
                model.accepted_workspace = Some(accepted_workspace(&seed));
                model.publication = Some(empty_publication());
                model.state = OperationState::Completed;
            }
        }
        OperationState::Aborted => row.state = ParticipantState::Aborted,
        OperationState::RecoveryRequired => {
            model.recovery_context = Some(RecoveryContextV1 {
                origin_state: RecoveryOriginStateV1::Executing,
            });
        }
        _ => {}
    }
    model
}

fn empty_publication() -> PublicationProgress {
    PublicationProgress {
        step: PublicationStep::Complete,
        candidate_lock_sha256: None,
        candidate_marker_path: None,
        root_merge_commit: None,
        composition_commit: None,
        composition_tree: None,
        candidate_hashes: Vec::new(),
        candidate: None,
        evidence_rolled_back: false,
        root_preservation: Vec::new(),
        preservation_prefix: None,
    }
}

fn action_class(action: V1NextAction) -> &'static str {
    match action {
        V1NextAction::Observe(_) => "observe",
        V1NextAction::Apply(_) => "apply",
        V1NextAction::Respond(_) => "respond",
        V1NextAction::Reject(_) => "reject",
    }
}

fn expected_action_class(state: OperationState, request: V1LifecycleRequest) -> &'static str {
    use OperationState as S;
    use V1LifecycleRequest as R;
    if request == R::Status || matches!(state, S::Completed | S::Aborted) {
        return if request == R::Archive && state != S::RecoveryRequired {
            "observe"
        } else {
            "respond"
        };
    }
    match state {
        S::RecoveryRequired | S::Preserving | S::RollingBack => "observe",
        S::Executing => match request {
            R::ResumeStart | R::Continue | R::Abort | R::Preserve => "observe",
            R::Archive => "reject",
            R::Status => unreachable!(),
        },
        S::AwaitingResolution => match request {
            R::ResumeStart => "respond",
            R::Continue | R::Abort | R::Preserve => "observe",
            R::Archive => "reject",
            R::Status => unreachable!(),
        },
        S::Halted => match request {
            R::ResumeStart => "respond",
            R::Continue => "apply",
            R::Abort | R::Preserve => "observe",
            R::Archive => "reject",
            R::Status => unreachable!(),
        },
        S::Finalizing => match request {
            R::Archive => "reject",
            _ => "observe",
        },
        S::Completed | S::Aborted => unreachable!(),
    }
}

fn git_error(message: &str) -> MergeRecordError {
    MergeRecordError {
        code: ErrorCode::GitCommandFailed,
        message: message.into(),
        detail: None,
    }
}

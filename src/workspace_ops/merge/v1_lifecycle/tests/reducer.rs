use super::super::authority::*;
use super::super::checked::{StoredV1Record, V1MutationLease};
use super::super::transition::*;
use super::fixtures::{accepted_workspace, align_baseline_lock, up_to_date_action};
use crate::model::ErrorCode;
use crate::workspace_ops::merge::model::v1::test_record as record;
use crate::workspace_ops::merge::{MergeRecordError, OperationState, ParticipantState};
use crate::workspace_ops::tests::TempDir;

#[test]
pub(super) fn participant_prepare_and_outcome_are_checked_reducers() {
    let root = TempDir::new("merge-v1-reducer-participant");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let current = StoredV1Record::for_test(&root.path, record()).unwrap();
    let mut prepared_row = current.record().participants["mem_a"].clone();
    prepared_row.pending_action = Some(up_to_date_action());
    let intent = PreparedParticipantAction::for_test(
        &current,
        "mem_a",
        "prepare_participant",
        "prepared",
        ParticipantActionPayload {
            member_id: "mem_a".into(),
            row: prepared_row.clone(),
        },
    )
    .unwrap();
    let prepared = prepare(
        &lease,
        &current,
        V1Transition::Participant(Box::new(ParticipantTransition::Prepare(Box::new(intent)))),
    )
    .unwrap();
    assert_eq!(prepared.base_digest(), current.source_digest());
    assert!(prepared.effect().retired().unwrap().is_empty());
    assert_eq!(
        prepared.next().participants["mem_a"].pending_action,
        prepared_row.pending_action
    );

    let pending = StoredV1Record::for_test(&root.path, prepared.next().clone()).unwrap();
    let mut outcome_row = pending.record().participants["mem_a"].clone();
    outcome_row.pending_action = None;
    outcome_row.state = ParticipantState::UpToDate;
    outcome_row.resulting_commit = Some(outcome_row.before_commit.clone());
    let proof = VerifiedParticipantOutcome::for_test(
        &pending,
        "mem_a",
        "participant_outcome",
        "completed",
        ParticipantActionPayload {
            member_id: "mem_a".into(),
            row: outcome_row,
        },
    )
    .unwrap();
    let outcome = prepare(
        &lease,
        &pending,
        V1Transition::Participant(Box::new(ParticipantTransition::RecordOutcome(Box::new(
            proof,
        )))),
    )
    .unwrap();
    assert_eq!(
        outcome.next().participants["mem_a"].state,
        ParticipantState::UpToDate
    );
    assert!(
        outcome.next().participants["mem_a"]
            .pending_action
            .is_none()
    );
}

#[test]
fn reducer_rejects_wrong_authority_and_wrong_predecessor_before_rewrite() {
    let first = TempDir::new("merge-v1-reducer-authority-first");
    let second = TempDir::new("merge-v1-reducer-authority-second");
    let current = StoredV1Record::for_test(&first.path, record()).unwrap();
    let wrong_lease = V1MutationLease::acquire_for_test(&second.path).unwrap();
    let mut row = current.record().participants["mem_a"].clone();
    row.pending_action = Some(up_to_date_action());
    let intent = PreparedParticipantAction::for_test(
        &current,
        "mem_a",
        "prepare_participant",
        "prepared",
        ParticipantActionPayload {
            member_id: "mem_a".into(),
            row,
        },
    )
    .unwrap();
    assert!(
        prepare(
            &wrong_lease,
            &current,
            V1Transition::Participant(Box::new(ParticipantTransition::Prepare(Box::new(intent)))),
        )
        .is_err()
    );
}

#[test]
pub(super) fn operation_reducers_cover_every_direct_state_edge() {
    let root = TempDir::new("merge-v1-operation-reducers");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();

    let mut halted = record();
    halted.state = OperationState::Halted;
    fail(halted.participants.get_mut("mem_a").unwrap(), "halted");
    let halted = stored(&root, halted);
    assert_eq!(
        operation(&root, &lease, &halted, OperationTransition::BeginExecution)
            .record()
            .state,
        OperationState::Executing
    );

    let mut failed = record();
    fail(failed.participants.get_mut("mem_a").unwrap(), "failed");
    let failed = stored(&root, failed);
    assert_eq!(
        operation(&root, &lease, &failed, OperationTransition::Halt)
            .record()
            .state,
        OperationState::Halted
    );

    let mut conflicted = record();
    let row = conflicted.participants.get_mut("mem_a").unwrap();
    row.state = ParticipantState::Conflicted;
    row.expected_merge_head = Some(row.source_commit.clone());
    let conflicted = stored(&root, conflicted);
    assert_eq!(
        operation(
            &root,
            &lease,
            &conflicted,
            OperationTransition::AwaitResolution
        )
        .record()
        .state,
        OperationState::AwaitingResolution
    );

    let successful = successful(&root);
    let proof = VerifiedParticipants::for_test(
        &successful,
        "@operation",
        "enter_finalizing",
        "executing",
        (),
    )
    .unwrap();
    assert_eq!(proof.value(), &());
    let finalizing = operation(
        &root,
        &lease,
        &successful,
        OperationTransition::EnterFinalizing(proof),
    );
    assert_eq!(finalizing.record().state, OperationState::Finalizing);

    let base = stored(&root, record());
    let preservation = PreparedPreservationEntry::for_test(
        &base,
        base.record(),
        handoff(&base, ReverseEntryKind::Preservation, base.record()),
    )
    .unwrap();
    let preserving = operation(
        &root,
        &lease,
        &base,
        OperationTransition::BeginPreservation(Box::new(preservation)),
    );
    assert_eq!(preserving.record().state, OperationState::Preserving);
    assert_eq!(
        preserving.record().preservation_publication_handoff,
        Some(crate::workspace_ops::merge::model::v1::PreservationPublicationHandoffV1::NoCandidate)
    );
    let exhausted = VerifiedPreservationExhausted::for_test(
        &preserving,
        "@operation",
        "preservation_exhausted",
        "verified",
        (),
    )
    .unwrap();
    let preserving_handoff = handoff(
        &preserving,
        ReverseEntryKind::ExhaustedRollback,
        preserving.record(),
    );
    let rollback_from_preserving = PreparedRollbackEntry::from_preserving_for_test(
        &preserving,
        preserving.record(),
        preserving_handoff,
        exhausted,
    )
    .unwrap();
    let rolled_back = operation(
        &root,
        &lease,
        &preserving,
        OperationTransition::BeginRollback(Box::new(rollback_from_preserving)),
    );
    assert_eq!(rolled_back.record().state, OperationState::RollingBack);
    assert_eq!(
        rolled_back.record().preservation_publication_handoff,
        preserving.record().preservation_publication_handoff
    );
    let rollback = PreparedRollbackEntry::direct_for_test(
        &base,
        base.record(),
        handoff(&base, ReverseEntryKind::DirectRollback, base.record()),
    )
    .unwrap();
    assert_eq!(
        operation(
            &root,
            &lease,
            &base,
            OperationTransition::BeginRollback(Box::new(rollback)),
        )
        .record()
        .state,
        OperationState::RollingBack
    );

    let mut complete = finalizing.record().clone();
    align_baseline_lock(&mut complete);
    let complete = stored(&root, complete);
    let accepted = PreparedAcceptedWorkspace::for_test(
        &complete,
        "@operation",
        "freeze_acceptance",
        "prepared",
        accepted_workspace(&complete),
    )
    .unwrap();
    let complete = apply(
        &root,
        &lease,
        &complete,
        V1Transition::Acceptance(Box::new(AcceptanceTransition::Freeze(Box::new(accepted)))),
    );
    let none = BoundPublicationDecision::for_test(
        &complete,
        "@publication",
        "classify_publication",
        "none",
        false,
    )
    .unwrap();
    let complete = apply(
        &root,
        &lease,
        &complete,
        V1Transition::Publication(Box::new(PublicationTransition::ClassifyNone(none))),
    );
    let proof = VerifiedPublicationCompletion::for_test(
        &complete,
        "@operation",
        "publication_complete",
        "verified",
        (),
    )
    .unwrap();
    assert_eq!(proof.value(), &());
    assert_eq!(
        operation(
            &root,
            &lease,
            &complete,
            OperationTransition::CompleteOperation(proof),
        )
        .record()
        .state,
        OperationState::Completed
    );

    let mut aborting = record();
    aborting.state = OperationState::RollingBack;
    aborting.participants.get_mut("mem_a").unwrap().state = ParticipantState::Aborted;
    let aborting = stored(&root, aborting);
    let exhausted = rollback_exhausted(&aborting).unwrap();
    let _ = exhausted.value();
    assert_eq!(
        operation(
            &root,
            &lease,
            &aborting,
            OperationTransition::AbortOperation(exhausted),
        )
        .record()
        .state,
        OperationState::Aborted
    );
}

#[test]
pub(super) fn participant_compounds_preserve_write_ahead_ownership() {
    let root = TempDir::new("merge-v1-participant-compounds");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    for destination in [
        OperationState::Executing,
        OperationState::RollingBack,
        OperationState::Preserving,
    ] {
        let current = halted_pending(&root);
        let proof = outcome(&current);
        let transition = match destination {
            OperationState::Executing => {
                ParticipantTransition::RecordHaltedOutcomeAndResumeExecution(Box::new(proof))
            }
            OperationState::RollingBack => {
                let anticipated = anticipated_outcome(&current);
                let entry = PreparedRollbackEntry::direct_for_test(
                    &current,
                    &anticipated,
                    handoff(&current, ReverseEntryKind::DirectRollback, &anticipated),
                )
                .unwrap();
                ParticipantTransition::RecordHaltedOutcomeAndBeginRollback(
                    Box::new(proof),
                    Box::new(entry),
                )
            }
            OperationState::Preserving => {
                let anticipated = anticipated_outcome(&current);
                let entry = PreparedPreservationEntry::for_test(
                    &current,
                    &anticipated,
                    handoff(&current, ReverseEntryKind::Preservation, &anticipated),
                )
                .unwrap();
                ParticipantTransition::RecordHaltedOutcomeAndBeginPreservation(
                    Box::new(proof),
                    Box::new(entry),
                )
            }
            _ => unreachable!(),
        };
        let next = apply(
            &root,
            &lease,
            &current,
            V1Transition::Participant(Box::new(transition)),
        );
        assert_eq!(next.record().state, destination);
        assert!(next.record().participants["mem_a"].pending_action.is_none());
        assert_eq!(
            next.record().preservation_publication_handoff.is_some(),
            destination == OperationState::Preserving
        );
    }

    for destination in [OperationState::RollingBack, OperationState::Preserving] {
        let mut model = record();
        model.participants.get_mut("mem_a").unwrap().pending_action = Some(up_to_date_action());
        let current = stored(&root, model);
        let proof = VerifiedParticipantNotStarted::for_test(
            &current,
            "mem_a",
            "participant_action",
            "not_started",
            "mem_a".into(),
        )
        .unwrap();
        let mut anticipated = current.record().clone();
        anticipated
            .participants
            .get_mut("mem_a")
            .unwrap()
            .pending_action = None;
        let transition = if destination == OperationState::RollingBack {
            ParticipantTransition::AbandonNotStartedAndBeginRollback(
                Box::new(proof),
                Box::new(
                    PreparedRollbackEntry::direct_for_test(
                        &current,
                        &anticipated,
                        handoff(&current, ReverseEntryKind::DirectRollback, &anticipated),
                    )
                    .unwrap(),
                ),
            )
        } else {
            ParticipantTransition::AbandonNotStartedAndBeginPreservation(
                Box::new(proof),
                Box::new(
                    PreparedPreservationEntry::for_test(
                        &current,
                        &anticipated,
                        handoff(&current, ReverseEntryKind::Preservation, &anticipated),
                    )
                    .unwrap(),
                ),
            )
        };
        let next = apply(
            &root,
            &lease,
            &current,
            V1Transition::Participant(Box::new(transition)),
        );
        assert_eq!(next.record().state, destination);
        assert_eq!(
            next.record().preservation_publication_handoff.is_some(),
            destination == OperationState::Preserving
        );
    }
}

fn apply(
    root: &TempDir,
    lease: &V1MutationLease,
    current: &StoredV1Record,
    transition: V1Transition,
) -> StoredV1Record {
    let rewrite = prepare(lease, current, transition).unwrap();
    stored(root, rewrite.next().clone())
}

fn operation(
    root: &TempDir,
    lease: &V1MutationLease,
    current: &StoredV1Record,
    transition: OperationTransition,
) -> StoredV1Record {
    apply(
        root,
        lease,
        current,
        V1Transition::Operation(Box::new(transition)),
    )
}

fn stored(
    root: &TempDir,
    model: crate::workspace_ops::merge::model::v1::MergeOperationRecordV1,
) -> StoredV1Record {
    StoredV1Record::for_test(&root.path, model).unwrap()
}

fn successful(root: &TempDir) -> StoredV1Record {
    let mut model = record();
    let row = model.participants.get_mut("mem_a").unwrap();
    row.state = ParticipantState::UpToDate;
    row.resulting_commit = Some(row.before_commit.clone());
    stored(root, model)
}

fn fail(row: &mut crate::workspace_ops::merge::MergeParticipantRecord, message: &str) {
    row.state = ParticipantState::Failed;
    row.error = Some(MergeRecordError {
        code: ErrorCode::GitCommandFailed,
        message: message.into(),
        detail: None,
    });
}

fn handoff(
    current: &StoredV1Record,
    kind: ReverseEntryKind,
    anticipated: &crate::workspace_ops::merge::model::v1::MergeOperationRecordV1,
) -> VerifiedPublicationHandoff {
    let proof = VerifiedPublicationHandoff::for_entry_test(current, kind, anticipated).unwrap();
    assert_eq!(proof.value().kind, kind);
    proof
}

fn halted_pending(root: &TempDir) -> StoredV1Record {
    let mut model = record();
    model.state = OperationState::Halted;
    let row = model.participants.get_mut("mem_a").unwrap();
    fail(row, "retry");
    row.pending_action = Some(up_to_date_action());
    stored(root, model)
}

fn anticipated_outcome(
    current: &StoredV1Record,
) -> crate::workspace_ops::merge::model::v1::MergeOperationRecordV1 {
    let mut anticipated = current.record().clone();
    anticipated
        .participants
        .insert("mem_a".into(), outcome_row(current));
    anticipated
}

fn outcome(current: &StoredV1Record) -> VerifiedParticipantOutcome {
    VerifiedParticipantOutcome::for_test(
        current,
        "mem_a",
        "participant_outcome",
        "completed",
        ParticipantActionPayload {
            member_id: "mem_a".into(),
            row: outcome_row(current),
        },
    )
    .unwrap()
}

fn outcome_row(current: &StoredV1Record) -> crate::workspace_ops::merge::MergeParticipantRecord {
    let mut row = current.record().participants["mem_a"].clone();
    row.state = ParticipantState::UpToDate;
    row.resulting_commit = Some(row.before_commit.clone());
    row.error = None;
    row.pending_action = None;
    row
}

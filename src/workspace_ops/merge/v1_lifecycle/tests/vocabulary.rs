use super::super::authority::*;
use super::super::checked::{StoredV1Record, V1MutationLease};
use super::super::transition::*;
use crate::model::ErrorCode;
use crate::workspace_ops::merge::model::v1::test_record as record;
use crate::workspace_ops::merge::{MergeRecordError, OperationState, ParticipantState};
use crate::workspace_ops::tests::TempDir;

#[test]
pub(super) fn preparation_failure_and_no_mutation_abort_reduce_exactly_once() {
    let root = TempDir::new_git("merge-v1-stop-transition-vocabulary");
    let lease = V1MutationLease::acquire_for_test(&root.path).unwrap();
    let current = StoredV1Record::for_test(&root.path, record()).unwrap();
    let mut row = current.record().participants["mem_a"].clone();
    row.state = ParticipantState::Failed;
    row.error = Some(MergeRecordError {
        code: ErrorCode::GitCommandFailed,
        message: "preflight failed".into(),
        detail: None,
    });
    let batch = PreparedFailureHaltBatch::for_test(
        &current,
        "mem_a",
        "preparation_failure",
        "verified",
        ParticipantFailurePayload {
            member_id: "mem_a".into(),
            row,
            later_unattempted: Vec::new(),
        },
    )
    .unwrap();
    let rewrite = prepare(
        &lease,
        &current,
        V1Transition::Participant(Box::new(
            ParticipantTransition::RecordPreparationFailureAndHalt(Box::new(batch)),
        )),
    )
    .unwrap();
    assert_eq!(rewrite.next().state, OperationState::Halted);

    let mut rolling_back = record();
    rolling_back.state = OperationState::RollingBack;
    let current = StoredV1Record::for_test(&root.path, rolling_back).unwrap();
    let proof = no_mutation_abort(&current).unwrap();
    let rewrite = prepare(
        &lease,
        &current,
        V1Transition::Participant(Box::new(ParticipantTransition::RecordNoMutationAbort(
            Box::new(proof),
        ))),
    )
    .unwrap();
    assert_eq!(
        rewrite.next().participants["mem_a"].state,
        ParticipantState::Aborted
    );
}

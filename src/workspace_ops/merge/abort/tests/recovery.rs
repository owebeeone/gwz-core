use super::super::evidence::{
    V1EvidenceRollbackObservation as E, classify_v1_evidence_shape_for_test,
};
use super::*;
use crate::workspace_ops::merge::model::v1::EvidenceRollbackStepV1;

#[test]
fn rollback_applied_before_record_failure_is_recognized_on_resume() {
    let (root, store) = fixture(&[("docs", ParticipantState::Conflicted)]);
    let runtime = Runtime::default();
    store.fail_write_at.set(Some(2));
    let error = run(&runtime, &root, &store).unwrap_err();
    assert_eq!(error.code, ErrorCode::IoError);
    store.fail_write_at.set(None);
    run(&runtime, &root, &store).unwrap();
    assert_eq!(&*runtime.calls.borrow(), &["abort:docs"]);
    assert_eq!(runtime.mutations.get(), 1);
}

#[test]
fn terminal_archive_failure_is_retryable_without_reobserving_repositories() {
    let (root, store) = fixture(&[("docs", ParticipantState::Conflicted)]);
    let runtime = Runtime::default();
    store.fail_archive_at.set(Some(1));

    assert_eq!(
        run(&runtime, &root, &store).unwrap_err().code,
        ErrorCode::IoError
    );
    assert_eq!(
        store.record.borrow().as_ref().unwrap().state,
        OperationState::Aborted
    );
    let calls = runtime.calls.borrow().clone();
    let snapshots = runtime.snapshots.get();

    store.fail_archive_at.set(None);
    let sink = CollectingSink::default();
    let response = run_with_sink(&runtime, &root, &store, None, &sink).unwrap();
    assert!(!response.open);
    assert_eq!(&*runtime.calls.borrow(), &calls);
    assert_eq!(runtime.snapshots.get(), snapshots);
    assert!(store.record.borrow().is_none());
    assert_eq!(
        sink.0
            .lock()
            .unwrap()
            .last()
            .and_then(|event| event.artifact_path.as_deref()),
        Some(".gwz/merge/done/merge_1.yaml")
    );
}

#[test]
fn retry_by_id_succeeds_when_archive_moved_before_reporting_failure() {
    let (root, store) = fixture(&[("docs", ParticipantState::Conflicted)]);
    let runtime = Runtime::default();
    store.fail_archive_at.set(Some(1));
    store.move_before_archive_failure.set(true);

    assert_eq!(
        run(&runtime, &root, &store).unwrap_err().code,
        ErrorCode::IoError
    );
    assert!(store.record.borrow().is_none());
    assert!(store.archived.borrow().is_some());

    store.fail_archive_at.set(None);
    let response = run_with_id(&runtime, &root, &store, Some("merge_1")).unwrap();
    assert_eq!(response.state, crate::MergeOperationState::Aborted);
    assert!(!response.open);
}

const ALL_EVIDENCE_SHAPES: [&str; 16] = [
    "BBBB", "BBBC", "BBCB", "BBCC", "BCBB", "BCBC", "BCCB", "BCCC", "CBBB", "CBBC", "CBCB", "CBCC",
    "CCBB", "CCBC", "CCCB", "CCCC",
];

#[test]
fn evidence_commit_accepts_exactly_the_nine_frozen_shapes() {
    const ALLOWED: [&str; 9] = [
        "BBBB", "BBCB", "BCCB", "CBCB", "CCCB", "BBCC", "BCCC", "CBCC", "CCCC",
    ];
    for shape in ALL_EVIDENCE_SHAPES {
        for (head_before, head_after, expected) in
            [(true, false, E::Before), (false, true, E::After)]
        {
            assert_eq!(
                classify_v1_evidence_shape_for_test(
                    EvidenceRollbackStepV1::EvidenceCommit,
                    head_before,
                    head_after,
                    shape,
                ),
                if ALLOWED.contains(&shape) {
                    expected
                } else {
                    E::Ambiguous
                },
                "{expected:?} {shape}",
            );
        }
    }
}

#[test]
fn every_later_step_accepts_only_its_frozen_before_and_after_rows() {
    let rows = [
        (
            EvidenceRollbackStepV1::Boundary,
            &["CBCB", "CCCB", "CBCC", "CCCC"][..],
            &["BBBB", "BBCB", "BCCB", "BBCC", "BCCC"][..],
        ),
        (
            EvidenceRollbackStepV1::Lock,
            &["BCCB", "BCCC"][..],
            &["BBBB", "BBCB", "BBCC"][..],
        ),
        (
            EvidenceRollbackStepV1::Marker,
            &["BBCB", "BBCC"][..],
            &["BBBB", "BBBC"][..],
        ),
        (EvidenceRollbackStepV1::Index, &["BBBC"][..], &["BBBB"][..]),
        (EvidenceRollbackStepV1::Complete, &[][..], &["BBBB"][..]),
    ];
    for (step, before, after) in rows {
        for shape in ALL_EVIDENCE_SHAPES {
            let expected = if before.contains(&shape) {
                E::Before
            } else if after.contains(&shape) {
                E::After
            } else {
                E::Ambiguous
            };
            assert_eq!(
                classify_v1_evidence_shape_for_test(step, false, true, shape),
                expected,
                "{step:?} {shape}",
            );
        }
    }
}

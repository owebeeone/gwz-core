use super::*;

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

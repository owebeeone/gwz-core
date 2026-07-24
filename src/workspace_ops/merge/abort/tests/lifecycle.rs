use super::*;

#[test]
fn mixed_three_member_abort_unwinds_only_mutated_rows() {
    let (root, store) = fixture(&[
        ("app", ParticipantState::UpToDate),
        ("lib", ParticipantState::Merged),
        ("docs", ParticipantState::Conflicted),
    ]);
    let runtime = Runtime::default();
    let response = run(&runtime, &root, &store).unwrap();
    assert_eq!(&*runtime.calls.borrow(), &["abort:docs", "reset:lib"]);
    assert_eq!(response.participant_counts.aborted, 2);
    assert_eq!(response.participant_counts.rolled_back, 1);
}

#[test]
fn foreign_state_in_earlier_app_rejects_before_later_docs_rollback() {
    let (root, store) = fixture(&[
        ("app", ParticipantState::Merged),
        ("docs", ParticipantState::Conflicted),
    ]);
    let runtime = Runtime {
        blocked: Some("app"),
        ..Runtime::default()
    };
    let error = run(&runtime, &root, &store).unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert!(runtime.calls.borrow().is_empty());
    assert_eq!(store.writes.get(), 0);
}

#[test]
fn externally_restored_conflict_is_persisted_without_a_second_git_abort() {
    let (root, store) = fixture(&[
        ("lib", ParticipantState::Merged),
        ("docs", ParticipantState::Conflicted),
    ]);
    let runtime = Runtime::default();
    runtime.applied.borrow_mut().insert("docs".to_owned());

    let response = run(&runtime, &root, &store).unwrap();

    assert_eq!(&*runtime.calls.borrow(), &["reset:lib"]);
    assert_eq!(response.participant_counts.aborted, 1);
    assert_eq!(response.participant_counts.rolled_back, 1);
}

#[test]
fn recovery_required_can_enter_guarded_rollback() {
    let (root, store) = fixture(&[("lib", ParticipantState::Merged)]);
    store.record.borrow_mut().as_mut().unwrap().state = OperationState::RecoveryRequired;

    let response = run(&Runtime::default(), &root, &store).unwrap();

    assert_eq!(response.state, crate::MergeOperationState::Aborted);
    assert!(!response.open);
}

#[test]
fn durable_rollback_row_ignores_later_worktree_changes() {
    let (root, store) = fixture(&[
        ("app", ParticipantState::RolledBack),
        ("docs", ParticipantState::Conflicted),
    ]);
    store.record.borrow_mut().as_mut().unwrap().state = OperationState::RollingBack;
    let runtime = Runtime {
        dirty_durable: Some("app"),
        ..Runtime::default()
    };

    run(&runtime, &root, &store).unwrap();

    assert_eq!(&*runtime.calls.borrow(), &["abort:docs"]);
}

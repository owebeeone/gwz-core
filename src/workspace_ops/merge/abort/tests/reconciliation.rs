use super::*;

#[test]
fn completed_pending_action_is_adopted_then_rolled_back() {
    let (root, store) = fixture(&[("app", ParticipantState::Planned)]);
    set_pending(&store, "app", PendingMergeActionKind::FastForward);
    let runtime = Runtime::default();
    reconcile(
        &runtime,
        "app",
        PendingActionReconciliation::Completed {
            resulting_commit: "app-source".to_owned(),
        },
    );

    run(&runtime, &root, &store).unwrap();

    assert_eq!(&*runtime.calls.borrow(), &["reset:app"]);
    let archived = store.archived.borrow();
    let app = &archived.as_ref().unwrap().participants["app"];
    assert_eq!(app.state, ParticipantState::RolledBack);
    assert!(app.pending_action.is_none());
}

#[test]
fn expected_pending_conflict_is_adopted_then_aborted() {
    let (root, store) = fixture(&[("docs", ParticipantState::Planned)]);
    set_pending(&store, "docs", PendingMergeActionKind::TrueMerge);
    let runtime = Runtime::default();
    reconcile(
        &runtime,
        "docs",
        PendingActionReconciliation::ExpectedConflict {
            conflict_paths: vec!["conflicted.txt".to_owned()],
        },
    );

    run(&runtime, &root, &store).unwrap();

    assert_eq!(&*runtime.calls.borrow(), &["abort:docs"]);
    let archived = store.archived.borrow();
    let docs = &archived.as_ref().unwrap().participants["docs"];
    assert_eq!(docs.state, ParticipantState::Aborted);
    assert!(docs.pending_action.is_none());
}

#[test]
fn ambiguous_pending_action_blocks_before_record_or_git_mutation() {
    let (root, store) = fixture(&[("app", ParticipantState::Planned)]);
    set_pending(&store, "app", PendingMergeActionKind::TrueMerge);
    let runtime = Runtime::default();
    reconcile(
        &runtime,
        "app",
        PendingActionReconciliation::Ambiguous {
            reason: "unexpected live commit".to_owned(),
            drift: Vec::new(),
        },
    );

    let error = run(&runtime, &root, &store).unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert_eq!(error.member_id.as_deref(), Some("app"));
    assert_eq!(store.writes.get(), 0);
    assert!(runtime.calls.borrow().is_empty());
}

#[test]
fn pending_resolution_not_started_still_requires_abort_eligible_index() {
    let (root, store) = fixture(&[("docs", ParticipantState::Conflicted)]);
    set_pending(&store, "docs", PendingMergeActionKind::ResolveConflict);
    let runtime = Runtime {
        blocked: Some("docs"),
        ..Runtime::default()
    };
    reconcile(&runtime, "docs", PendingActionReconciliation::NotStarted);

    let error = run(&runtime, &root, &store).unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert_eq!(store.writes.get(), 0);
    assert!(runtime.calls.borrow().is_empty());
}

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
fn root_abort_restores_exact_recorded_metadata_after_checkout_conversion() {
    let (root, store) = fixture(&[("root", ParticipantState::Conflicted)]);
    let manifest = "schema: gwz.workspace/v0\nworkspace:\n  id: ws_test\nmembers: []\n";
    let lock = "schema: gwz.lock/v0\nworkspace_id: ws_test\nmanifest_schema: gwz.workspace/v0\nmembers: {}\n";
    {
        let mut record_ref = store.record.borrow_mut();
        let record = record_ref.as_mut().unwrap();
        let mut participant = record.participants.remove("root").unwrap();
        participant.target_kind = super::super::super::MergeTargetKind::Root;
        participant.path = ".".to_owned();
        record.selected_targets = vec!["@root".to_owned()];
        record.participants.insert("@root".to_owned(), participant);
        record.baseline.manifest_sha256 = format!("{:x}", Sha256::digest(manifest.as_bytes()));
        record.baseline.lock_sha256 = format!("{:x}", Sha256::digest(lock.as_bytes()));
        record.baseline.manifest_yaml = Some(manifest.to_owned());
        record.baseline.lock_yaml = Some(lock.to_owned());
    }
    fs::write(
        root.path().join(WORKSPACE_MANIFEST),
        manifest.replace('\n', "\r\n"),
    )
    .unwrap();
    fs::write(
        root.path().join(artifact::LOCK_PATH),
        lock.replace('\n', "\r\n"),
    )
    .unwrap();

    let response = run(&Runtime::default(), &root, &store).unwrap();

    assert_eq!(response.state, crate::MergeOperationState::Aborted);
    assert_eq!(
        fs::read(root.path().join(WORKSPACE_MANIFEST)).unwrap(),
        manifest.as_bytes()
    );
    assert_eq!(
        fs::read(root.path().join(artifact::LOCK_PATH)).unwrap(),
        lock.as_bytes()
    );
}

#[test]
fn abort_does_not_restore_root_metadata_when_root_is_not_selected() {
    let (root, store) = fixture(&[("member", ParticipantState::Conflicted)]);
    let original_manifest = fs::read(root.path().join(WORKSPACE_MANIFEST)).unwrap();
    let original_lock = fs::read(root.path().join(artifact::LOCK_PATH)).unwrap();
    {
        let mut record_ref = store.record.borrow_mut();
        let record = record_ref.as_mut().unwrap();
        let participant = record.participants.get("member").unwrap().clone();
        record.participants.insert(
            "@root".to_owned(),
            super::super::super::MergeParticipantRecord {
                target_kind: super::super::super::MergeTargetKind::Root,
                path: ".".to_owned(),
                ..participant
            },
        );
        record.baseline.manifest_yaml = Some(String::from_utf8(original_manifest.clone()).unwrap());
        record.baseline.lock_yaml = Some(String::from_utf8(original_lock.clone()).unwrap());
    }
    let changed_manifest = b"unselected root manifest must remain untouched\n";
    let changed_lock = b"unselected root lock must remain untouched\n";
    fs::write(root.path().join(WORKSPACE_MANIFEST), changed_manifest).unwrap();
    fs::write(root.path().join(artifact::LOCK_PATH), changed_lock).unwrap();

    let error = run(&Runtime::default(), &root, &store).unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert_eq!(
        fs::read(root.path().join(WORKSPACE_MANIFEST)).unwrap(),
        changed_manifest
    );
    assert_eq!(
        fs::read(root.path().join(artifact::LOCK_PATH)).unwrap(),
        changed_lock
    );
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

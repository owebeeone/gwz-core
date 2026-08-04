use super::*;

#[test]
fn plain_abort_rejects_interrupted_preservation_before_rollback() {
    let temp = TempDir::new("merge-preserve-plain-abort-gate");
    let backend = crate::git::Git2Backend::new();
    let fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let mut start = request(false);
    start.message = Some("Preserve this merge\r\n".to_owned());
    let started = handle_merge(
        &backend,
        temp.path(),
        start,
        "op_preserve_plain_abort_start",
    )
    .unwrap();
    let merge_id = started.merge_id.clone().unwrap();
    let lib = temp.path().join("lib");
    let result = merge_repo(&started, "mem_lib")
        .resulting_commit
        .clone()
        .unwrap();
    let extra = commit_file(
        &lib,
        "plain-abort-gate.txt",
        "preserve me\n",
        "plain abort preservation gate",
        &[git2::Oid::from_str(&result).unwrap()],
    )
    .unwrap();
    fs::write(lib.join("plain-abort-untracked.txt"), "stash me\n").unwrap();
    let store = FailingPreservationStore {
        // Transition and backup evidence persist; native stash creation then
        // succeeds immediately before this third record write fails.
        fail_at_write: 3,
        writes: Cell::new(0),
        fired: Cell::new(false),
    };
    let mut preserve = recovery_request(crate::MergeOp::Abort, Some(merge_id.clone()));
    preserve.preserve = Some(true);

    invoke_preservation_store(
        &backend,
        &store,
        temp.path(),
        preserve.clone(),
        "op_preserve_plain_abort_fault",
    )
    .unwrap_err();
    let stash_object = backend
        .stash_list(&lib)
        .unwrap()
        .into_iter()
        .find(|entry| entry.message.contains(&format!("gwz:stash_{merge_id}:")))
        .unwrap()
        .object_id;
    let plain_abort = recovery_request(crate::MergeOp::Abort, Some(merge_id.clone()));

    let error = invoke_preservation_store(
        &backend,
        &store,
        temp.path(),
        plain_abort,
        "op_preserve_plain_abort_rejected",
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert!(error.message.contains("--abort --preserve"));
    let open = FileMergeStore.discover_open(temp.path()).unwrap().unwrap();
    assert_eq!(open.state, OperationState::Preserving);
    assert_eq!(
        backend.head(&lib).unwrap().commit.as_deref(),
        Some(extra.as_str())
    );
    assert_eq!(
        backend.repository_state(&temp.path().join("docs")).unwrap(),
        crate::git::GitRepositoryState::Merge
    );

    let aborted = invoke_preservation_store(
        &backend,
        &store,
        temp.path(),
        preserve,
        "op_preserve_plain_abort_retry",
    )
    .unwrap();
    assert_eq!(aborted.state, crate::MergeOperationState::Aborted);
    assert_eq!(
        backend.head(&lib).unwrap().commit.as_deref(),
        Some(fixture.lib_before.as_str())
    );
    assert!(aborted.preservation.unwrap().iter().any(|evidence| {
        evidence.target_id == "mem_lib"
            && evidence.stash_object_id.as_deref() == Some(stash_object.as_str())
    }));
    let archived = FileMergeStore
        .load_archived(temp.path(), &merge_id)
        .unwrap();
    let expected = format!(
        "Preserve this merge\n\nGWZ-Merge-ID: {merge_id}\nGWZ-Operation-ID: op_preserve_plain_abort_start"
    );
    assert!(
        archived
            .participants
            .values()
            .all(|participant| participant.commit_message == expected)
    );
}

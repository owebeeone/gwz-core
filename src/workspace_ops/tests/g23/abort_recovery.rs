use super::*;

#[test]
fn mixed_merge_abort_restores_exact_baseline_and_archives_operation() {
    let temp = TempDir::new("merge-mixed-abort");
    let backend = crate::git::Git2Backend::new();
    let fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let lock_before = fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap();
    let manifest_before = fs::read(temp.path().join(crate::workspace::WORKSPACE_MANIFEST)).unwrap();
    let started = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();
    let merge_id = started.merge_id.clone().unwrap();

    let aborted = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Abort, Some(merge_id.clone())),
        "op_abort",
    )
    .unwrap();

    assert_eq!(aborted.state, crate::MergeOperationState::Aborted);
    assert!(!aborted.open);
    for (path, expected) in [
        ("app", fixture.app_before),
        ("lib", fixture.lib_before),
        ("docs", fixture.docs_before),
    ] {
        assert_eq!(
            backend.head(&temp.path().join(path)).unwrap().commit,
            Some(expected)
        );
        assert!(
            backend
                .merge_state(&temp.path().join(path))
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(
        fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
        lock_before
    );
    assert_eq!(
        fs::read(temp.path().join(crate::workspace::WORKSPACE_MANIFEST)).unwrap(),
        manifest_before
    );
    assert!(
        !temp
            .path()
            .join(format!(".gwz/merge/{merge_id}.yaml"))
            .exists()
    );
    assert!(
        temp.path()
            .join(format!(".gwz/merge/done/{merge_id}.yaml"))
            .is_file()
    );
    let status = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Status, None),
        "op_status",
    )
    .unwrap();
    assert_eq!(status.state, crate::MergeOperationState::Idle);
    assert!(!status.open);
}

#[test]
fn post_merge_commit_rejects_abort_before_conflicted_member_changes() {
    let temp = TempDir::new("merge-mixed-abort-drift");
    let backend = crate::git::Git2Backend::new();
    let fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let started = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();
    let lib = temp.path().join("lib");
    let lib_result = backend.head(&lib).unwrap().commit.unwrap();
    let post_merge = commit_file(
        &lib,
        "post-merge.txt",
        "later work\n",
        "later work",
        &[git2::Oid::from_str(&lib_result).unwrap()],
    )
    .unwrap();
    let docs = temp.path().join("docs");
    let docs_state = backend.merge_state(&docs).unwrap().unwrap();

    let error = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Abort, started.merge_id),
        "op_abort",
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert_eq!(error.member_id.as_deref(), Some("mem_lib"));
    assert_eq!(backend.head(&lib).unwrap().commit, Some(post_merge));
    assert_eq!(
        backend.head(&docs).unwrap().commit,
        Some(fixture.docs_before)
    );
    assert_eq!(backend.merge_state(&docs).unwrap(), Some(docs_state));
}

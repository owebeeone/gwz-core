use super::*;

#[test]
fn open_awaiting_resolution_blocks_dry_run_and_real_starts_from_an_explicit_root() {
    let temp = TempDir::new("merge-start-gate-awaiting");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let started = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();
    assert_eq!(
        started.state,
        crate::MergeOperationState::AwaitingResolution
    );

    assert_open_merge_blocks_all_starts_without_mutation(
        temp.path(),
        &backend,
        started.merge_id.as_deref().unwrap(),
    );
}

#[test]
fn open_finalizing_blocks_dry_run_and_real_starts_from_an_explicit_root() {
    let temp = TempDir::new("merge-start-gate-finalizing");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-gate-finalizing");
    feature_commit(
        &backend,
        &temp.path().join("remote"),
        "README.md",
        "source\n",
    );
    let started = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();
    assert_eq!(started.state, crate::MergeOperationState::Completed);
    let merge_id = started.merge_id.clone().unwrap();
    reopen_archived_record(temp.path(), &merge_id, OperationState::Finalizing);

    assert_open_merge_blocks_all_starts_without_mutation(temp.path(), &backend, &merge_id);
}

#[test]
fn open_halted_blocks_dry_run_and_real_starts_from_an_explicit_root() {
    let temp = TempDir::new("merge-start-gate-halted");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let started = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();
    assert_eq!(
        started.state,
        crate::MergeOperationState::AwaitingResolution
    );
    let merge_id = force_open_merge_state(temp.path(), OperationState::Halted);

    assert_open_merge_blocks_all_starts_without_mutation(temp.path(), &backend, &merge_id);
}

#[test]
fn open_recovery_required_blocks_dry_run_and_real_starts_from_an_explicit_root() {
    let temp = TempDir::new("merge-start-gate-recovery-required");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let started = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();
    assert_eq!(
        started.state,
        crate::MergeOperationState::AwaitingResolution
    );
    let merge_id = force_open_merge_state(temp.path(), OperationState::RecoveryRequired);

    assert_open_merge_blocks_all_starts_without_mutation(temp.path(), &backend, &merge_id);
}

#[test]
fn direct_core_mutator_cannot_bypass_open_merge_gate() {
    let temp = TempDir::new("merge-direct-core-gate");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let started = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();
    assert!(started.open);

    let error = handle_branch(
        &backend,
        temp.path(),
        crate::BranchRequest {
            meta: request_meta(),
            op: crate::BranchOp::Create,
            name: Some("blocked-during-merge".to_owned()),
            start_ref: Some("HEAD".to_owned()),
            switch_after_create: None,
        },
        "op_direct_branch",
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::OpenOperation);
    assert!(error.message.contains(started.merge_id.as_deref().unwrap()));
    assert!(
        !backend
            .branch_list(&temp.path().join("app"))
            .unwrap()
            .iter()
            .any(|branch| branch.name == "blocked-during-merge")
    );
}

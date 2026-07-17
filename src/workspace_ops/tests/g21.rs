use std::fs;

use crate::artifact::read_lock;
use crate::git::GitBackend;
use crate::model::ErrorCode;

use super::*;

fn branch_request(op: crate::BranchOp, name: Option<&str>) -> crate::BranchRequest {
    crate::BranchRequest {
        meta: request_meta(),
        op,
        name: name.map(str::to_owned),
        start_ref: None,
        switch_after_create: None,
    }
}

fn create_request(name: &str) -> crate::BranchRequest {
    crate::BranchRequest {
        start_ref: Some("HEAD".to_owned()),
        ..branch_request(crate::BranchOp::Create, Some(name))
    }
}

fn merge_request(source_ref: &str) -> crate::BranchRequest {
    crate::BranchRequest {
        start_ref: Some(source_ref.to_owned()),
        ..branch_request(crate::BranchOp::Merge, None)
    }
}

#[test]
fn branch_list_reports_current_and_local_branches() {
    let temp = TempDir::new("branch-list-op");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "branch-list-source");
    let member = temp.path().join("remote");
    backend
        .branch_create(&member, "feature/list", "HEAD")
        .unwrap();

    let response = handle_branch(
        &backend,
        temp.path(),
        branch_request(crate::BranchOp::List, None),
        "op_branch_list",
    )
    .unwrap();

    let repos = response.repos.unwrap();
    assert!(
        repos
            .iter()
            .any(|repo| repo.branch.as_deref() == Some("main")
                && repo.current_branch.as_deref() == Some("main"))
    );
    assert!(
        repos
            .iter()
            .any(|repo| repo.branch.as_deref() == Some("feature/list"))
    );
}

#[test]
fn branch_create_is_idempotent_and_dry_run_reports_planned_create() {
    let temp = TempDir::new("branch-create-op");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "branch-create-source");
    let member = temp.path().join("remote");

    let mut dry_run = create_request("feature/create");
    dry_run.meta.dry_run = Some(true);
    let planned = handle_branch(&backend, temp.path(), dry_run, "op_branch_dry").unwrap();
    assert_eq!(
        planned.response.meta.aggregate_status,
        crate::AggregateStatus::Accepted
    );
    assert_eq!(
        planned.repos.unwrap().single().result,
        crate::BranchActionResult::Created
    );
    assert!(
        backend
            .read_ref(&member, "refs/heads/feature/create")
            .unwrap()
            .is_none()
    );

    let created = handle_branch(
        &backend,
        temp.path(),
        create_request("feature/create"),
        "op_branch_create",
    )
    .unwrap();
    assert_eq!(
        created.repos.unwrap().single().result,
        crate::BranchActionResult::Created
    );

    let exists = handle_branch(
        &backend,
        temp.path(),
        create_request("feature/create"),
        "op_branch_exists",
    )
    .unwrap();
    assert_eq!(
        exists.repos.unwrap().single().result,
        crate::BranchActionResult::Exists
    );
}

#[test]
fn branch_create_rejects_existing_branch_at_different_commit_before_mutation() {
    let temp = TempDir::new("branch-create-diverged");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "branch-diverged-source");
    let member = temp.path().join("remote");
    let old_head = backend.head(&member).unwrap().commit.unwrap();
    backend
        .branch_create(&member, "feature/diverged", "HEAD")
        .unwrap();
    let parent = git2::Oid::from_str(&old_head).unwrap();
    commit_file(&member, "README.md", "advanced\n", "advance", &[parent]).unwrap();

    let error = handle_branch(
        &backend,
        temp.path(),
        create_request("feature/diverged"),
        "op_branch_diverged",
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::DivergedMember);
    assert_eq!(
        backend
            .read_ref(&member, "refs/heads/feature/diverged")
            .unwrap(),
        Some(old_head)
    );
}

#[test]
fn branch_delete_rejects_current_branch_but_allows_dirty_non_current_branch() {
    let temp = TempDir::new("branch-delete-op");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "branch-delete-source");
    let member = temp.path().join("remote");
    backend
        .branch_create(&member, "feature/delete", "HEAD")
        .unwrap();
    fs::write(member.join("README.md"), "dirty\n").unwrap();

    let deleted = handle_branch(
        &backend,
        temp.path(),
        branch_request(crate::BranchOp::Delete, Some("feature/delete")),
        "op_branch_delete",
    )
    .unwrap();
    assert_eq!(
        deleted.repos.unwrap().single().result,
        crate::BranchActionResult::Deleted
    );
    assert!(
        backend
            .read_ref(&member, "refs/heads/feature/delete")
            .unwrap()
            .is_none()
    );

    let current = handle_branch(
        &backend,
        temp.path(),
        branch_request(crate::BranchOp::Delete, Some("main")),
        "op_branch_delete_current",
    )
    .unwrap_err();
    assert_eq!(current.code, ErrorCode::InvalidRequest);
}

#[test]
fn branch_create_with_switch_rewrites_lock_from_observed_state() {
    let temp = TempDir::new("branch-create-switch");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "branch-switch-source");
    let member = temp.path().join("remote");

    let mut request = create_request("feature/switch");
    request.switch_after_create = Some(true);
    let response = handle_branch(&backend, temp.path(), request, "op_branch_switch").unwrap();

    assert_eq!(
        response.repos.unwrap().single().result,
        crate::BranchActionResult::Switched
    );
    let head = backend.head(&member).unwrap();
    assert_eq!(head.branch.as_deref(), Some("feature/switch"));
    let lock = read_lock(temp.path()).unwrap();
    let state = &lock.members["mem_remote"];
    assert_eq!(state.branch.as_deref(), Some("feature/switch"));
    assert_eq!(state.commit, head.commit);
}

#[test]
fn branch_merge_protocol_is_deprecated_before_workspace_resolution() {
    let temp = TempDir::new("branch-merge-deprecated");
    let backend = crate::git::Git2Backend::new();
    let error = handle_branch(
        &backend,
        temp.path(),
        merge_request("feature/source"),
        "op_branch_merge_deprecated",
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::DeprecatedOperation);
    assert!(error.message.contains("first-class merge"));
}

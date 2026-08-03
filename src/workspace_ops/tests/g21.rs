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
fn branch_create_with_switch_preserves_dirty_state_and_records_it_in_lock() {
    let temp = TempDir::new("branch-create-switch");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "branch-switch-source");
    let member = temp.path().join("remote");
    fs::write(member.join("README.md"), "staged\n").unwrap();
    let repo = git2::Repository::open(&member).unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("README.md")).unwrap();
    index.write().unwrap();
    fs::write(member.join("README.md"), "unstaged\n").unwrap();
    fs::write(member.join("untracked.txt"), "untracked\n").unwrap();
    let index_before = fs::read(member.join(".git/index")).unwrap();
    let status_before = backend.status(&member).unwrap();

    let mut request = create_request("feature/switch");
    request.switch_after_create = Some(true);
    let response = handle_branch(&backend, temp.path(), request, "op_branch_switch").unwrap();

    let member_response = response.response.members.single();
    assert_eq!(member_response.member_id, "mem_remote");
    assert_eq!(
        member_response.state.as_ref().and_then(|state| state.dirty),
        Some(true)
    );
    assert_eq!(
        response.repos.unwrap().single().result,
        crate::BranchActionResult::Switched
    );
    let head = backend.head(&member).unwrap();
    assert_eq!(head.branch.as_deref(), Some("feature/switch"));
    assert_eq!(fs::read(member.join(".git/index")).unwrap(), index_before);
    assert_eq!(
        fs::read_to_string(member.join("README.md")).unwrap(),
        "unstaged\n"
    );
    assert_eq!(
        fs::read_to_string(member.join("untracked.txt")).unwrap(),
        "untracked\n"
    );
    assert_eq!(backend.status(&member).unwrap(), status_before);
    let lock = read_lock(temp.path()).unwrap();
    let state = &lock.members["mem_remote"];
    assert_eq!(state.branch.as_deref(), Some("feature/switch"));
    assert_eq!(state.commit, head.commit);
    assert_eq!(state.dirty, Some(true));
}

#[test]
fn branch_create_switches_to_existing_same_head_branch_with_dirty_state() {
    let temp = TempDir::new("branch-existing-switch-dirty");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "branch-existing-source");
    let member = temp.path().join("remote");
    backend
        .branch_create(&member, "feature/existing", "HEAD")
        .unwrap();
    fs::write(member.join("untracked.txt"), "keep\n").unwrap();

    let mut request = create_request("feature/existing");
    request.switch_after_create = Some(true);
    let response = handle_branch(&backend, temp.path(), request, "op_existing_switch").unwrap();

    assert_eq!(
        response.repos.unwrap().single().result,
        crate::BranchActionResult::Switched
    );
    assert_eq!(
        backend.head(&member).unwrap().branch.as_deref(),
        Some("feature/existing")
    );
    assert_eq!(
        fs::read_to_string(member.join("untracked.txt")).unwrap(),
        "keep\n"
    );
    assert_eq!(
        read_lock(temp.path()).unwrap().members["mem_remote"].dirty,
        Some(true)
    );
}

#[test]
fn branch_create_switch_dry_run_accepts_dirty_same_head_without_mutation() {
    let temp = TempDir::new("branch-switch-dirty-dry-run");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "branch-dry-source");
    let member = temp.path().join("remote");
    fs::write(member.join("README.md"), "dirty\n").unwrap();
    let head_before = backend.head(&member).unwrap();
    let status_before = backend.status(&member).unwrap();

    let mut request = create_request("feature/dry");
    request.switch_after_create = Some(true);
    request.meta.dry_run = Some(true);
    let response = handle_branch(&backend, temp.path(), request, "op_dirty_dry").unwrap();

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Accepted
    );
    assert_eq!(
        response.repos.unwrap().single().result,
        crate::BranchActionResult::Switched
    );
    assert_eq!(backend.head(&member).unwrap(), head_before);
    assert_eq!(backend.status(&member).unwrap(), status_before);
    assert!(
        backend
            .read_ref(&member, "refs/heads/feature/dry")
            .unwrap()
            .is_none()
    );
}

#[test]
fn branch_create_switch_rejects_dirty_different_start_before_creating_branch() {
    let temp = TempDir::new("branch-switch-dirty-different-start");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "branch-different-source");
    let member = temp.path().join("remote");
    let old_head = backend.head(&member).unwrap().commit.unwrap();
    let parent = git2::Oid::from_str(&old_head).unwrap();
    let current_head = commit_file(&member, "README.md", "advanced\n", "advance", &[parent])
        .unwrap()
        .to_string();
    fs::write(member.join("README.md"), "pending\n").unwrap();

    let mut request = create_request("feature/old-start");
    request.start_ref = Some(old_head);
    request.switch_after_create = Some(true);
    request.meta.policy = Some(crate::OperationPolicy {
        destructive: Some(crate::DestructiveBehavior::Allow),
        ..Default::default()
    });
    let error = handle_branch(&backend, temp.path(), request, "op_dirty_different").unwrap_err();

    assert_eq!(error.code, ErrorCode::DirtyMember);
    assert_eq!(error.member_id.as_deref(), Some("mem_remote"));
    assert_eq!(error.member_path.as_deref(), Some("remote"));
    assert_eq!(
        backend.head(&member).unwrap().commit.as_deref(),
        Some(current_head.as_str())
    );
    assert_eq!(
        backend.head(&member).unwrap().branch.as_deref(),
        Some("main")
    );
    assert!(
        backend
            .read_ref(&member, "refs/heads/feature/old-start")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        fs::read_to_string(member.join("README.md")).unwrap(),
        "pending\n"
    );
}

#[test]
fn branch_create_switch_rejects_clean_looking_git_operation_before_any_member_mutation() {
    let temp = TempDir::new("branch-switch-native-operation");
    let backend = crate::git::Git2Backend::new();
    let (_app_fixture, _lib_fixture) = super::g19::init_two_member_workspace(temp.path(), &backend);
    let app = temp.path().join("app");
    let lib = temp.path().join("lib");
    let lib_head = backend.head(&lib).unwrap().commit.unwrap();
    fs::write(lib.join(".git/MERGE_HEAD"), format!("{lib_head}\n")).unwrap();
    assert!(!backend.status(&lib).unwrap().is_dirty);
    assert_eq!(
        backend.repository_state(&lib).unwrap(),
        crate::git::GitRepositoryState::Merge
    );

    let mut request = create_request("feature/native-operation");
    request.switch_after_create = Some(true);
    let error = handle_branch(&backend, temp.path(), request, "op_native_operation").unwrap_err();

    assert_eq!(error.code, ErrorCode::GitCommandFailed);
    assert_eq!(error.member_id.as_deref(), Some("mem_lib"));
    assert_eq!(error.member_path.as_deref(), Some("lib"));
    for member in [&app, &lib] {
        assert_eq!(
            backend.head(member).unwrap().branch.as_deref(),
            Some("main")
        );
        assert!(
            backend
                .read_ref(member, "refs/heads/feature/native-operation")
                .unwrap()
                .is_none()
        );
    }
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

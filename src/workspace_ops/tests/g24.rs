use std::fs;

use crate::git::{Git2Backend, GitBackend};
use crate::model::ErrorCode;

use super::*;

fn conflicting_pull_fixture(
    temp: &TempDir,
    backend: &Git2Backend,
    name: &str,
) -> (RemoteFixture, String, Vec<u8>, Vec<u8>, Vec<u8>) {
    handle_create_workspace(create_workspace_request(temp.path()), "op_create").unwrap();
    let fixture = RemoteFixture::new(name);
    let base = fixture.commit_and_push("README.md", "base\n", "base", backend);
    let member = temp.path().join("repos/app");
    backend.clone_repo(fixture.remote_url(), &member).unwrap();
    let local = commit_file(
        &member,
        "README.md",
        "local\n",
        "local",
        &[git2::Oid::from_str(&base).unwrap()],
    )
    .unwrap();
    fixture.commit_and_push("README.md", "remote\n", "remote", backend);
    write_pull_fixture(
        temp.path(),
        vec![("mem_app", "repos/app", fixture.remote_url(), &local)],
    );
    (
        fixture,
        local,
        fs::read(member.join(".git/index")).unwrap(),
        fs::read(member.join("README.md")).unwrap(),
        fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
    )
}

fn merge_pull_request(partial: bool) -> crate::PullHeadRequest {
    let mut request = pull_head_request_with_sync(crate::SyncBehavior::Merge);
    request.meta.selection = Some(crate::Selection {
        targets: vec!["mem_app".to_owned()],
        ..Default::default()
    });
    if partial {
        request
            .meta
            .policy
            .get_or_insert_with(Default::default)
            .partial = Some(crate::PartialBehavior::Partial);
    }
    request
}

#[test]
fn merge_pull_predicts_conflict_and_rejects_before_local_mutation() {
    let temp = TempDir::new("pull-predict-conflict");
    let backend = Git2Backend::new();
    let (_fixture, local, index, worktree, lock) =
        conflicting_pull_fixture(&temp, &backend, "pull-predict-conflict-source");
    let member = temp.path().join("repos/app");

    let error =
        handle_pull_head(&backend, temp.path(), merge_pull_request(false), "op_pull").unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeValidationFailed);
    assert_eq!(error.member_id.as_deref(), Some("mem_app"));
    assert_eq!(error.member_path.as_deref(), Some("repos/app"));
    assert!(error.message.contains("README.md"));
    assert_eq!(
        backend.head(&member).unwrap().commit.as_deref(),
        Some(local.as_str())
    );
    assert_eq!(fs::read(member.join(".git/index")).unwrap(), index);
    assert_eq!(fs::read(member.join("README.md")).unwrap(), worktree);
    assert_eq!(
        fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
        lock
    );
    assert!(backend.merge_state(&member).unwrap().is_none());
}

#[test]
fn partial_merge_pull_skips_predicted_conflict_without_native_merge_state() {
    let temp = TempDir::new("pull-predict-conflict-partial");
    let backend = Git2Backend::new();
    let (_fixture, local, index, worktree, lock) =
        conflicting_pull_fixture(&temp, &backend, "pull-predict-partial-source");
    let member = temp.path().join("repos/app");

    let response =
        handle_pull_head(&backend, temp.path(), merge_pull_request(true), "op_pull").unwrap();

    let row = response.response.members.single();
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Partial
    );
    assert_eq!(row.status, crate::MemberStatus::Skipped);
    assert!(
        row.planned
            .as_ref()
            .unwrap()
            .message
            .as_deref()
            .unwrap()
            .contains("README.md")
    );
    assert_eq!(
        backend.head(&member).unwrap().commit.as_deref(),
        Some(local.as_str())
    );
    assert_eq!(fs::read(member.join(".git/index")).unwrap(), index);
    assert_eq!(fs::read(member.join("README.md")).unwrap(), worktree);
    assert_eq!(
        fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
        lock
    );
    assert!(backend.merge_state(&member).unwrap().is_none());
}

#[test]
fn default_merge_pull_does_not_fast_forward_root_before_member_prediction_rejects() {
    let temp = TempDir::new("pull-root-before-member");
    let backend = Git2Backend::new();
    let member_remote = init_one_member_workspace(temp.path(), &backend, "pull-root-member-source");
    let member = temp.path().join("remote");
    let member_base = backend.head(&member).unwrap().commit.unwrap();
    let local_member = commit_file(
        &member,
        "README.md",
        "local\n",
        "local",
        &[git2::Oid::from_str(&member_base).unwrap()],
    )
    .unwrap();
    member_remote.commit_and_push("README.md", "remote\n", "remote", &backend);
    write_pull_fixture(
        temp.path(),
        vec![(
            "mem_remote",
            "remote",
            member_remote.remote_url(),
            &local_member,
        )],
    );

    backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
    let root_before = commit_file(temp.path(), "root.txt", "root\n", "root baseline", &[]).unwrap();
    let root_remote = TempDir::new("pull-root-remote");
    let bare = root_remote.path().join("remote.git");
    git2::Repository::init_bare(&bare).unwrap();
    backend
        .add_remote(temp.path(), "origin", bare.to_str().unwrap())
        .unwrap();
    backend
        .push(temp.path(), "origin", "refs/heads/main:refs/heads/main")
        .unwrap();
    let peer = root_remote.path().join("peer");
    backend.clone_repo(bare.to_str().unwrap(), &peer).unwrap();
    let peer_parent = backend.head(&peer).unwrap().commit.unwrap();
    let root_remote_commit = commit_file(
        &peer,
        "remote-root.txt",
        "remote\n",
        "remote root",
        &[git2::Oid::from_str(&peer_parent).unwrap()],
    )
    .unwrap();
    backend
        .push(&peer, "origin", "refs/heads/main:refs/heads/main")
        .unwrap();
    let root_index = fs::read(temp.path().join(".git/index")).unwrap();
    let lock = fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap();

    let error = handle_pull_head(
        &backend,
        temp.path(),
        pull_head_request_with_sync(crate::SyncBehavior::Merge),
        "op_pull",
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeValidationFailed);
    assert_eq!(error.member_id.as_deref(), Some("mem_remote"));
    assert_eq!(
        backend.head(temp.path()).unwrap().commit.as_deref(),
        Some(root_before.as_str())
    );
    assert_ne!(root_before, root_remote_commit);
    assert_eq!(
        fs::read(temp.path().join(".git/index")).unwrap(),
        root_index
    );
    assert_eq!(
        fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
        lock
    );
    assert!(!temp.path().join("remote-root.txt").exists());
    assert!(backend.merge_state(temp.path()).unwrap().is_none());
}

#[test]
fn default_merge_pull_applies_a_planned_root_fast_forward_after_member_preflight() {
    let temp = TempDir::new("pull-root-fast-forward");
    let backend = Git2Backend::new();
    let _member_remote =
        init_one_member_workspace(temp.path(), &backend, "pull-root-ff-member-source");

    backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
    let root_before = commit_file(temp.path(), "root.txt", "root\n", "root baseline", &[]).unwrap();
    let root_remote = TempDir::new("pull-root-ff-remote");
    let bare = root_remote.path().join("remote.git");
    git2::Repository::init_bare(&bare).unwrap();
    backend
        .add_remote(temp.path(), "origin", bare.to_str().unwrap())
        .unwrap();
    backend
        .push(temp.path(), "origin", "refs/heads/main:refs/heads/main")
        .unwrap();
    let peer = root_remote.path().join("peer");
    backend.clone_repo(bare.to_str().unwrap(), &peer).unwrap();
    let remote_commit = commit_file(
        &peer,
        "remote-root.txt",
        "remote\n",
        "remote root",
        &[git2::Oid::from_str(&root_before).unwrap()],
    )
    .unwrap();
    backend
        .push(&peer, "origin", "refs/heads/main:refs/heads/main")
        .unwrap();

    let response = handle_pull_head(
        &backend,
        temp.path(),
        pull_head_request_with_sync(crate::SyncBehavior::Merge),
        "op_pull",
    )
    .unwrap();

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert_eq!(
        backend.head(temp.path()).unwrap().commit.as_deref(),
        Some(remote_commit.as_str())
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("remote-root.txt")).unwrap(),
        "remote\n"
    );
    assert!(backend.merge_state(temp.path()).unwrap().is_none());
}

#[test]
fn default_merge_pull_rejects_a_predicted_root_conflict_without_local_mutation() {
    let temp = TempDir::new("pull-root-conflict");
    let backend = Git2Backend::new();
    let _member_remote =
        init_one_member_workspace(temp.path(), &backend, "pull-root-conflict-member-source");
    backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
    let root_before = commit_file(temp.path(), "root.txt", "base\n", "root baseline", &[]).unwrap();

    let root_remote = TempDir::new("pull-root-conflict-remote");
    let bare = root_remote.path().join("remote.git");
    git2::Repository::init_bare(&bare).unwrap();
    backend
        .add_remote(temp.path(), "origin", bare.to_str().unwrap())
        .unwrap();
    backend
        .push(temp.path(), "origin", "refs/heads/main:refs/heads/main")
        .unwrap();
    let peer = root_remote.path().join("peer");
    backend.clone_repo(bare.to_str().unwrap(), &peer).unwrap();
    commit_file(
        &peer,
        "root.txt",
        "remote\n",
        "remote root",
        &[git2::Oid::from_str(&root_before).unwrap()],
    )
    .unwrap();
    backend
        .push(&peer, "origin", "refs/heads/main:refs/heads/main")
        .unwrap();
    let local_commit = commit_file(
        temp.path(),
        "root.txt",
        "local\n",
        "local root",
        &[git2::Oid::from_str(&root_before).unwrap()],
    )
    .unwrap();
    let index = fs::read(temp.path().join(".git/index")).unwrap();
    let worktree = fs::read(temp.path().join("root.txt")).unwrap();
    let lock = fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap();

    let error = handle_pull_head(
        &backend,
        temp.path(),
        pull_head_request_with_sync(crate::SyncBehavior::Merge),
        "op_pull",
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeValidationFailed);
    assert_eq!(error.member_id.as_deref(), Some("@root"));
    assert_eq!(error.member_path.as_deref(), Some("."));
    assert!(error.message.contains("root.txt"));
    assert_eq!(
        backend.head(temp.path()).unwrap().commit.as_deref(),
        Some(local_commit.as_str())
    );
    assert_eq!(fs::read(temp.path().join(".git/index")).unwrap(), index);
    assert_eq!(fs::read(temp.path().join("root.txt")).unwrap(), worktree);
    assert_eq!(
        fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
        lock
    );
    assert!(backend.merge_state(temp.path()).unwrap().is_none());
}

#[test]
fn default_merge_pull_applies_a_clean_checked_root_merge_after_member_preflight() {
    let temp = TempDir::new("pull-root-clean-merge");
    let backend = Git2Backend::new();
    let _member_remote =
        init_one_member_workspace(temp.path(), &backend, "pull-root-merge-member-source");
    backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
    let root_before = commit_file(temp.path(), "root.txt", "base\n", "root baseline", &[]).unwrap();

    let root_remote = TempDir::new("pull-root-clean-merge-remote");
    let bare = root_remote.path().join("remote.git");
    git2::Repository::init_bare(&bare).unwrap();
    backend
        .add_remote(temp.path(), "origin", bare.to_str().unwrap())
        .unwrap();
    backend
        .push(temp.path(), "origin", "refs/heads/main:refs/heads/main")
        .unwrap();
    let peer = root_remote.path().join("peer");
    backend.clone_repo(bare.to_str().unwrap(), &peer).unwrap();
    let remote_commit = commit_file(
        &peer,
        "remote-root.txt",
        "remote\n",
        "remote root",
        &[git2::Oid::from_str(&root_before).unwrap()],
    )
    .unwrap();
    backend
        .push(&peer, "origin", "refs/heads/main:refs/heads/main")
        .unwrap();
    let local_commit = commit_file(
        temp.path(),
        "local-root.txt",
        "local\n",
        "local root",
        &[git2::Oid::from_str(&root_before).unwrap()],
    )
    .unwrap();

    let response = handle_pull_head(
        &backend,
        temp.path(),
        pull_head_request_with_sync(crate::SyncBehavior::Merge),
        "op_pull",
    )
    .unwrap();

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    let merge_commit = backend.head(temp.path()).unwrap().commit.unwrap();
    assert!(
        backend
            .commit_matches_merge(
                temp.path(),
                &merge_commit,
                &local_commit,
                &remote_commit,
                "Merge refs/remotes/origin/main into main",
            )
            .unwrap()
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("local-root.txt")).unwrap(),
        "local\n"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("remote-root.txt")).unwrap(),
        "remote\n"
    );
    assert!(backend.merge_state(temp.path()).unwrap().is_none());
}

use std::fs;

use crate::artifact::read_lock;
use crate::git::GitBackend;
use crate::model::ErrorCode;

use super::*;

fn request(dry_run: bool) -> crate::MergeRequest {
    let mut meta = request_meta();
    meta.dry_run = dry_run.then_some(true);
    crate::MergeRequest {
        meta,
        op: crate::MergeOp::Start,
        source_ref: Some("feature/source".to_owned()),
        ..Default::default()
    }
}

fn feature_commit(
    backend: &crate::git::Git2Backend,
    repo: &std::path::Path,
    file: &str,
    content: &str,
) -> (String, String) {
    let base = backend.head(repo).unwrap().commit.unwrap();
    backend
        .branch_create(repo, "feature/source", "HEAD")
        .unwrap();
    backend.switch_branch(repo, "feature/source").unwrap();
    let source = commit_file(
        repo,
        file,
        content,
        "source",
        &[git2::Oid::from_str(&base).unwrap()],
    )
    .unwrap();
    backend.switch_branch(repo, "main").unwrap();
    (base, source)
}

fn init_two_member_workspace(
    root: &std::path::Path,
    backend: &crate::git::Git2Backend,
) -> (RemoteFixture, RemoteFixture) {
    let app = RemoteFixture::new("merge-start-app");
    let lib = RemoteFixture::new("merge-start-lib");
    app.commit_and_push("README.md", "base\n", "initial", backend);
    lib.commit_and_push("README.md", "base\n", "initial", backend);
    handle_init_from_sources(
        backend,
        root,
        crate::InitFromSourcesRequest {
            meta: request_meta(),
            workspace_root: root.to_string_lossy().into_owned(),
            sources: vec![
                crate::SourceUrl {
                    url: app.remote_url().to_owned(),
                    path: Some("app".to_owned()),
                    remote_name: None,
                    branch: None,
                },
                crate::SourceUrl {
                    url: lib.remote_url().to_owned(),
                    path: Some("lib".to_owned()),
                    remote_name: None,
                    branch: None,
                },
            ],
            target: None,
            workspace_id: Some("ws_ops".to_owned()),
        },
        "op_init",
        &CollectingSink::default(),
    )
    .unwrap();
    (app, lib)
}

#[test]
fn first_class_merge_fast_forwards_and_advances_the_m0_lock() {
    let temp = TempDir::new("merge-start-ff");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-start-ff-source");
    let member = temp.path().join("remote");
    let (_, source) = feature_commit(&backend, &member, "README.md", "source\n");

    let response = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();

    assert_eq!(response.response.meta.action, crate::ActionKind::Merge);
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Ok
    );
    assert_eq!(
        response.repos[0].state,
        crate::MergeParticipantState::FastForwarded
    );
    assert_eq!(response.repos[0].source_ref, "feature/source");
    assert_eq!(
        backend.head(&member).unwrap().commit.as_deref(),
        Some(source.as_str())
    );
    assert_eq!(
        read_lock(temp.path()).unwrap().members["mem_remote"]
            .commit
            .as_deref(),
        Some(source.as_str())
    );
}

#[test]
fn first_class_merge_dry_run_does_not_change_head_lock_or_merge_state() {
    let temp = TempDir::new("merge-start-dry");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-start-dry-source");
    let member = temp.path().join("remote");
    let (base, _) = feature_commit(&backend, &member, "README.md", "source\n");
    let lock_before = fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap();

    let response = handle_merge(&backend, temp.path(), request(true), "op_merge_dry").unwrap();

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Accepted
    );
    assert_eq!(
        response.repos[0].state,
        crate::MergeParticipantState::Planned
    );
    assert_eq!(
        backend.head(&member).unwrap().commit.as_deref(),
        Some(base.as_str())
    );
    assert_eq!(
        fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
        lock_before
    );
    assert!(backend.merge_state(&member).unwrap().is_none());
}

#[test]
fn preflight_checks_every_member_before_mutating_an_earlier_member() {
    let temp = TempDir::new("merge-start-preflight");
    let backend = crate::git::Git2Backend::new();
    let (_app_fixture, _lib_fixture) = init_two_member_workspace(temp.path(), &backend);
    let app = temp.path().join("app");
    let lib = temp.path().join("lib");
    let (app_base, _) = feature_commit(&backend, &app, "README.md", "source\n");
    feature_commit(&backend, &lib, "README.md", "source\n");
    fs::write(lib.join("README.md"), "dirty\n").unwrap();

    let error = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap_err();

    assert_eq!(error.code, ErrorCode::DirtyMember);
    assert_eq!(
        backend.head(&app).unwrap().commit.as_deref(),
        Some(app_base.as_str())
    );
    assert!(backend.merge_state(&app).unwrap().is_none());
}

#[test]
fn conflict_continues_to_later_member_and_only_clean_outcome_advances_lock() {
    let temp = TempDir::new("merge-start-conflict-batch");
    let backend = crate::git::Git2Backend::new();
    let (_app_fixture, _lib_fixture) = init_two_member_workspace(temp.path(), &backend);
    let app = temp.path().join("app");
    let lib = temp.path().join("lib");
    let (app_base, _) = feature_commit(&backend, &app, "README.md", "source\n");
    let app_local = commit_file(
        &app,
        "README.md",
        "local\n",
        "local",
        &[git2::Oid::from_str(&app_base).unwrap()],
    )
    .unwrap();
    let (_, lib_source) = feature_commit(&backend, &lib, "README.md", "source\n");

    let response = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Conflicted
    );
    assert_eq!(response.participant_counts.conflicted, 1);
    assert_eq!(response.participant_counts.fast_forwarded, 1);
    assert_eq!(
        response.repos[0].state,
        crate::MergeParticipantState::Conflicted
    );
    assert_eq!(
        response.repos[1].state,
        crate::MergeParticipantState::FastForwarded
    );
    assert_eq!(
        backend.head(&app).unwrap().commit.as_deref(),
        Some(app_local.as_str())
    );
    let merge_state = backend.merge_state(&app).unwrap().unwrap();
    assert_eq!(merge_state.conflict_paths, ["README.md"]);
    assert_eq!(
        backend.head(&lib).unwrap().commit.as_deref(),
        Some(lib_source.as_str())
    );
    let lock = read_lock(temp.path()).unwrap();
    assert_eq!(
        lock.members["mem_app"].commit.as_deref(),
        Some(app_base.as_str())
    );
    assert_eq!(
        lock.members["mem_lib"].commit.as_deref(),
        Some(lib_source.as_str())
    );
}

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;

use crate::artifact::read_lock;
use crate::git::GitBackend;
use crate::model::ErrorCode;
use sha2::{Digest, Sha256};

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

struct MixedMergeFixture {
    _remotes: [RemoteFixture; 3],
    app_before: String,
    lib_before: String,
    docs_before: String,
    docs_source: String,
}

fn init_mixed_merge_workspace(
    root: &std::path::Path,
    backend: &crate::git::Git2Backend,
) -> MixedMergeFixture {
    let app = RemoteFixture::new("merge-mixed-app");
    let lib = RemoteFixture::new("merge-mixed-lib");
    let docs = RemoteFixture::new("merge-mixed-docs");
    for fixture in [&app, &lib, &docs] {
        fixture.commit_and_push("README.md", "base\n", "initial", backend);
    }
    handle_init_from_sources(
        backend,
        root,
        crate::InitFromSourcesRequest {
            meta: request_meta(),
            workspace_root: root.to_string_lossy().into_owned(),
            sources: [(&app, "app"), (&lib, "lib"), (&docs, "docs")]
                .into_iter()
                .map(|(fixture, path)| crate::SourceUrl {
                    url: fixture.remote_url().to_owned(),
                    path: Some(path.to_owned()),
                    remote_name: None,
                    branch: None,
                })
                .collect(),
            target: None,
            workspace_id: Some("ws_ops".to_owned()),
        },
        "op_init",
        &CollectingSink::default(),
    )
    .unwrap();

    let app_path = root.join("app");
    let lib_path = root.join("lib");
    let docs_path = root.join("docs");
    let app_before = backend.head(&app_path).unwrap().commit.unwrap();
    backend
        .branch_create(&app_path, "feature/source", "HEAD")
        .unwrap();

    let (lib_base, _) = feature_commit(backend, &lib_path, "source.txt", "source\n");
    let lib_before = commit_file(
        &lib_path,
        "local.txt",
        "local\n",
        "local",
        &[git2::Oid::from_str(&lib_base).unwrap()],
    )
    .unwrap();
    let (docs_base, docs_source) = feature_commit(backend, &docs_path, "README.md", "source\n");
    let docs_before = commit_file(
        &docs_path,
        "README.md",
        "local\n",
        "local",
        &[git2::Oid::from_str(&docs_base).unwrap()],
    )
    .unwrap();
    MixedMergeFixture {
        _remotes: [app, lib, docs],
        app_before,
        lib_before,
        docs_before,
        docs_source,
    }
}

fn recovery_request(op: crate::MergeOp, merge_id: Option<String>) -> crate::MergeRequest {
    crate::MergeRequest {
        meta: request_meta(),
        op,
        merge_id,
        ..Default::default()
    }
}

fn merge_repo<'a>(
    response: &'a crate::MergeResponse,
    target_id: &str,
) -> &'a crate::MergeRepoSummary {
    response
        .repos
        .iter()
        .find(|repo| repo.target_id == target_id)
        .unwrap()
}

#[test]
fn first_class_merge_fast_forwards_into_durable_finalizing_with_baseline_lock() {
    let temp = TempDir::new("merge-start-ff");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-start-ff-source");
    let member = temp.path().join("remote");
    let (base, source) = feature_commit(&backend, &member, "README.md", "source\n");

    let response = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();

    assert_eq!(response.response.meta.action, crate::ActionKind::Merge);
    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Accepted
    );
    assert_eq!(
        response.repos[0].state,
        crate::MergeParticipantState::FastForwarded
    );
    assert_eq!(response.repos[0].source_ref, "feature/source");
    assert_eq!(response.state, crate::MergeOperationState::Finalizing);
    assert!(response.open);
    assert_eq!(response.merge_id.as_deref(), Some("merge_op_merge_0001"));
    assert_eq!(
        backend.head(&member).unwrap().commit.as_deref(),
        Some(source.as_str())
    );
    assert!(
        temp.path()
            .join(".gwz/merge/merge_op_merge_0001.yaml")
            .is_file()
    );
    assert_eq!(
        read_lock(temp.path()).unwrap().members["mem_remote"]
            .commit
            .as_deref(),
        Some(base.as_str())
    );
}

#[test]
fn first_class_true_merge_uses_request_git_identities_and_planned_message() {
    let temp = TempDir::new("merge-start-identity");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-identity-source");
    let member = temp.path().join("remote");
    let (base, _) = feature_commit(&backend, &member, "source.txt", "source\n");
    commit_file(
        &member,
        "local.txt",
        "local\n",
        "local",
        &[git2::Oid::from_str(&base).unwrap()],
    )
    .unwrap();
    let mut request = request(false);
    request.meta.attribution = Some(crate::OperationAttribution {
        actor: None,
        git_author: Some(crate::GitObjectIdentity {
            name: "Merge Author".to_owned(),
            email: "author@example.invalid".to_owned(),
            time_ms: Some(1_700_000_000_000),
            timezone_offset_minutes: Some(600),
        }),
        git_committer: Some(crate::GitObjectIdentity {
            name: "Merge Committer".to_owned(),
            email: "committer@example.invalid".to_owned(),
            time_ms: Some(1_700_000_100_000),
            timezone_offset_minutes: Some(-300),
        }),
        credential_ref: None,
    });

    let response = handle_merge(&backend, temp.path(), request, "op_merge").unwrap();
    let oid = git2::Oid::from_str(response.repos[0].resulting_commit.as_deref().unwrap()).unwrap();
    let repo = git2::Repository::open(&member).unwrap();
    let commit = repo.find_commit(oid).unwrap();

    assert_eq!(
        response.repos[0].state,
        crate::MergeParticipantState::Merged
    );
    assert_eq!(
        commit.message(),
        Ok(
            "Merge 'feature/source' into 'main'\n\nGWZ-Merge-ID: merge_op_merge_0001\nGWZ-Operation-ID: op_merge"
        )
    );
    assert_eq!(commit.author().name(), Ok("Merge Author"));
    assert_eq!(commit.author().when().offset_minutes(), 600);
    assert_eq!(commit.committer().name(), Ok("Merge Committer"));
    assert_eq!(commit.committer().when().offset_minutes(), -300);
}

#[test]
fn invalid_identity_rejects_mixed_batch_before_fast_forward_mutation() {
    let temp = TempDir::new("merge-start-invalid-identity");
    let backend = crate::git::Git2Backend::new();
    let (_app_fixture, _lib_fixture) = init_two_member_workspace(temp.path(), &backend);
    let app = temp.path().join("app");
    let lib = temp.path().join("lib");
    let (app_before, _) = feature_commit(&backend, &app, "source.txt", "source\n");
    let (lib_base, _) = feature_commit(&backend, &lib, "source.txt", "source\n");
    let lib_before = commit_file(
        &lib,
        "local.txt",
        "local\n",
        "local",
        &[git2::Oid::from_str(&lib_base).unwrap()],
    )
    .unwrap();
    let lock_before = fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap();
    let mut request = request(false);
    request.meta.attribution = Some(crate::OperationAttribution {
        actor: None,
        git_author: Some(crate::GitObjectIdentity {
            name: "Invalid <Author>".to_owned(),
            email: "author@example.invalid".to_owned(),
            time_ms: None,
            timezone_offset_minutes: None,
        }),
        git_committer: None,
        credential_ref: None,
    });

    let error = handle_merge(&backend, temp.path(), request, "op_merge").unwrap_err();

    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(error.message.contains("git_identity.name"));
    assert_eq!(
        backend.head(&app).unwrap().commit.as_deref(),
        Some(app_before.as_str())
    );
    assert_eq!(
        backend.head(&lib).unwrap().commit.as_deref(),
        Some(lib_before.as_str())
    );
    assert_eq!(
        fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
        lock_before
    );
    assert!(backend.merge_state(&app).unwrap().is_none());
    assert!(backend.merge_state(&lib).unwrap().is_none());
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
fn first_class_merge_rejects_unrelated_history_without_mutation() {
    let temp = TempDir::new("merge-start-unrelated");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-unrelated-source");
    let member = temp.path().join("remote");
    create_orphan_ref(&member, "refs/heads/feature/source", "unrelated source\n");
    let head = backend.head(&member).unwrap();
    let target_ref = backend.read_ref(&member, "refs/heads/main").unwrap();
    let index = fs::read(member.join(".git/index")).unwrap();
    let worktree = fs::read(member.join("README.md")).unwrap();
    let status = backend.status(&member).unwrap();
    let native_state = backend.merge_state(&member).unwrap();
    let lock = fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap();

    let error = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap_err();

    assert_eq!(error.code, ErrorCode::GitCommandFailed);
    assert!(error.message.contains("do not share a merge base"));
    assert_eq!(backend.head(&member).unwrap(), head);
    assert_eq!(
        backend.read_ref(&member, "refs/heads/main").unwrap(),
        target_ref
    );
    assert_eq!(fs::read(member.join(".git/index")).unwrap(), index);
    assert_eq!(fs::read(member.join("README.md")).unwrap(), worktree);
    assert_eq!(backend.status(&member).unwrap(), status);
    assert_eq!(backend.merge_state(&member).unwrap(), native_state);
    assert_eq!(
        fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
        lock
    );
    assert!(!member.join(".git/MERGE_HEAD").exists());
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
fn conflict_continues_to_later_member_and_status_recovers_with_baseline_lock() {
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
    let (lib_base, lib_source) = feature_commit(&backend, &lib, "README.md", "source\n");

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
        Some(lib_base.as_str())
    );

    let merge_id = response.merge_id.clone();
    let mut status_request = request(false);
    status_request.op = crate::MergeOp::Status;
    status_request.source_ref = None;
    let status = handle_merge(
        &crate::git::Git2Backend::new(),
        temp.path(),
        status_request.clone(),
        "op_status",
    )
    .unwrap();
    assert_eq!(status.merge_id, merge_id);
    assert_eq!(status.state, crate::MergeOperationState::AwaitingResolution);
    assert!(status.open);
    assert_eq!(status.repos[0].conflict_paths, ["README.md"]);
    assert_eq!(
        status.repos[1].live_commit.as_deref(),
        Some(lib_source.as_str())
    );

    let manifest_path = temp.path().join(crate::workspace::WORKSPACE_MANIFEST);
    fs::OpenOptions::new()
        .append(true)
        .open(&manifest_path)
        .unwrap()
        .write_all(b"\n")
        .unwrap();
    let drifted = handle_merge(&backend, temp.path(), status_request, "op_status_drift").unwrap();
    assert_eq!(
        drifted.operation_drift[0].kind,
        crate::MergeOperationDriftKind::BaselineManifestChanged
    );
}

#[test]
fn mixed_merge_continue_resolves_conflict_and_preserves_prior_result() {
    let temp = TempDir::new("merge-mixed-continue");
    let backend = crate::git::Git2Backend::new();
    let fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let lock_before = fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap();

    let started = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();
    assert_eq!(
        merge_repo(&started, "mem_app").state,
        crate::MergeParticipantState::UpToDate
    );
    assert_eq!(
        merge_repo(&started, "mem_lib").state,
        crate::MergeParticipantState::Merged
    );
    assert_eq!(
        merge_repo(&started, "mem_docs").state,
        crate::MergeParticipantState::Conflicted
    );
    let lib_result = backend
        .head(&temp.path().join("lib"))
        .unwrap()
        .commit
        .unwrap();

    let docs = temp.path().join("docs");
    fs::write(docs.join("README.md"), "resolved\n").unwrap();
    backend
        .stage_paths_allowing_other_conflicts(&docs, &["README.md"])
        .unwrap();
    let continued = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Resume, started.merge_id.clone()),
        "op_continue",
    )
    .unwrap();

    assert_eq!(continued.state, crate::MergeOperationState::Finalizing);
    assert!(continued.open);
    assert_eq!(
        merge_repo(&continued, "mem_docs").state,
        crate::MergeParticipantState::Continued
    );
    assert_eq!(
        backend.head(&temp.path().join("lib")).unwrap().commit,
        Some(lib_result)
    );
    let docs_result = git2::Oid::from_str(
        merge_repo(&continued, "mem_docs")
            .resulting_commit
            .as_deref()
            .unwrap(),
    )
    .unwrap();
    let repo = git2::Repository::open(&docs).unwrap();
    let commit = repo.find_commit(docs_result).unwrap();
    assert_eq!(
        commit.parent_id(0).unwrap().to_string(),
        fixture.docs_before
    );
    assert_eq!(
        commit.parent_id(1).unwrap().to_string(),
        fixture.docs_source
    );
    assert_eq!(
        fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
        lock_before
    );
}

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
fn crash_reload_continue_foreign_rejection_and_external_restore_converge_on_abort() {
    let temp = TempDir::new("merge-adversarial-lifecycle");
    let backend = crate::git::Git2Backend::new();
    let fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let started = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();
    let merge_id = started.merge_id.clone().unwrap();
    assert_eq!(
        merge_repo(&started, "mem_lib").state,
        crate::MergeParticipantState::Merged
    );
    assert_eq!(
        merge_repo(&started, "mem_docs").state,
        crate::MergeParticipantState::Conflicted
    );

    // A new backend instance models a fresh process reloading only durable
    // operation state before the conflict is resolved.
    let reloaded = crate::git::Git2Backend::new();
    let status = handle_merge(
        &reloaded,
        temp.path(),
        recovery_request(crate::MergeOp::Status, None),
        "op_status_reload",
    )
    .unwrap();
    assert_eq!(
        merge_repo(&status, "mem_docs").state,
        crate::MergeParticipantState::Conflicted
    );

    let docs = temp.path().join("docs");
    fs::write(docs.join("README.md"), "resolved after reload\n").unwrap();
    reloaded
        .stage_paths_allowing_other_conflicts(&docs, &["README.md"])
        .unwrap();
    let continued = handle_merge(
        &reloaded,
        temp.path(),
        recovery_request(crate::MergeOp::Resume, Some(merge_id.clone())),
        "op_continue_reload",
    )
    .unwrap();
    assert_eq!(continued.state, crate::MergeOperationState::Finalizing);
    let lib = temp.path().join("lib");
    let lib_result = merge_repo(&continued, "mem_lib")
        .resulting_commit
        .clone()
        .unwrap();
    let docs_result = merge_repo(&continued, "mem_docs")
        .resulting_commit
        .clone()
        .unwrap();

    // Poison a participant that abort would have to roll back. Whole-operation
    // preflight must reject before changing the later docs participant.
    let lib_repo = git2::Repository::open(&lib).unwrap();
    let cherry_pick_head = lib_repo.path().join("CHERRY_PICK_HEAD");
    fs::write(&cherry_pick_head, format!("{lib_result}\n")).unwrap();
    let record_path = temp.path().join(format!(".gwz/merge/{merge_id}.yaml"));
    let record_before_rejection = fs::read(&record_path).unwrap();
    let error = handle_merge(
        &crate::git::Git2Backend::new(),
        temp.path(),
        recovery_request(crate::MergeOp::Abort, Some(merge_id.clone())),
        "op_abort_foreign",
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert_eq!(error.member_id.as_deref(), Some("mem_lib"));
    assert_eq!(
        reloaded.head(&lib).unwrap().commit.as_deref(),
        Some(lib_result.as_str())
    );
    assert_eq!(
        reloaded.head(&docs).unwrap().commit.as_deref(),
        Some(docs_result.as_str())
    );
    assert_eq!(fs::read(&record_path).unwrap(), record_before_rejection);
    fs::remove_file(cherry_pick_head).unwrap();

    // Simulate an exact external restoration after the interrupted process.
    // Coordinated abort must recognize it as a no-op and roll back only the
    // participant that remains changed.
    reloaded
        .set_branch_target_checked(&docs, "main", &docs_result, &fixture.docs_before)
        .unwrap();
    let aborted = handle_merge(
        &crate::git::Git2Backend::new(),
        temp.path(),
        recovery_request(crate::MergeOp::Abort, Some(merge_id.clone())),
        "op_abort_reloaded",
    )
    .unwrap();
    assert_eq!(aborted.state, crate::MergeOperationState::Aborted);
    assert!(!aborted.open);
    assert_eq!(
        reloaded.head(&lib).unwrap().commit,
        Some(fixture.lib_before)
    );
    assert_eq!(
        reloaded.head(&docs).unwrap().commit,
        Some(fixture.docs_before)
    );
    assert!(
        temp.path()
            .join(format!(".gwz/merge/done/{merge_id}.yaml"))
            .is_file()
    );
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

    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert_eq!(error.member_id.as_deref(), Some("mem_lib"));
    assert_eq!(backend.head(&lib).unwrap().commit, Some(post_merge));
    assert_eq!(
        backend.head(&docs).unwrap().commit,
        Some(fixture.docs_before)
    );
    assert_eq!(backend.merge_state(&docs).unwrap(), Some(docs_state));
}

#[test]
fn failed_and_unattempted_rows_retry_only_after_whole_operation_preflight() {
    use crate::workspace_ops::merge::{
        FileMergeStore, MERGE_RECORD_SCHEMA, MERGE_RECORD_SCHEMA_VERSION, MergeBaseline,
        MergeOperationRecord, MergeParticipantRecord, MergeStore, MergeTargetKind, OperationState,
        ParticipantState,
    };

    let temp = TempDir::new("merge-retry-recorded-rows");
    let backend = crate::git::Git2Backend::new();
    let (_app_fixture, _lib_fixture) = init_two_member_workspace(temp.path(), &backend);
    let app = temp.path().join("app");
    let lib = temp.path().join("lib");
    let (app_before, app_source) = feature_commit(&backend, &app, "source.txt", "app\n");
    let (lib_before, lib_source) = feature_commit(&backend, &lib, "source.txt", "lib\n");
    let participant = |path: &str, before: String, source: String, state| MergeParticipantRecord {
        path: path.to_owned(),
        target_kind: MergeTargetKind::Member,
        target_branch: "main".to_owned(),
        before_commit: before,
        source_commit: source,
        commit_message: format!("Retry recorded merge for {path}"),
        state,
        resulting_commit: None,
        expected_merge_head: None,
        conflict_paths: Vec::new(),
        error: None,
        pending_action: None,
        preservation: Vec::new(),
        drift: Vec::new(),
        extensions: BTreeMap::new(),
    };
    let digest = |path| format!("{:x}", Sha256::digest(fs::read(path).unwrap()));
    let merge_id = "merge_retry_rows".to_owned();
    let record = MergeOperationRecord {
        schema: MERGE_RECORD_SCHEMA.to_owned(),
        record_schema_version: MERGE_RECORD_SCHEMA_VERSION,
        writer_version: crate::VERSION.to_owned(),
        workspace_id: "ws_ops".to_owned(),
        merge_id: merge_id.clone(),
        operation_id: "op_start".to_owned(),
        state: OperationState::Halted,
        source_ref: "feature/source".to_owned(),
        created_at: "now".to_owned(),
        baseline: MergeBaseline {
            lock_sha256: digest(temp.path().join(crate::artifact::LOCK_PATH)),
            manifest_sha256: digest(temp.path().join(crate::workspace::WORKSPACE_MANIFEST)),
            root_head: None,
            extensions: BTreeMap::new(),
        },
        selected_targets: vec!["mem_app".to_owned(), "mem_lib".to_owned()],
        participants: BTreeMap::from([
            (
                "mem_app".to_owned(),
                participant(
                    "app",
                    app_before.clone(),
                    app_source.clone(),
                    ParticipantState::Failed,
                ),
            ),
            (
                "mem_lib".to_owned(),
                participant(
                    "lib",
                    lib_before.clone(),
                    lib_source.clone(),
                    ParticipantState::Unattempted,
                ),
            ),
        ]),
        publication: None,
        operation_drift: Vec::new(),
        extensions: BTreeMap::new(),
    };
    FileMergeStore.write_open(temp.path(), &record).unwrap();

    fs::write(lib.join("untracked.txt"), "blocks whole preflight\n").unwrap();
    let error = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Resume, Some(merge_id.clone())),
        "op_continue_blocked",
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert_eq!(error.member_id.as_deref(), Some("mem_lib"));
    assert_eq!(backend.head(&app).unwrap().commit, Some(app_before));

    fs::remove_file(lib.join("untracked.txt")).unwrap();
    let response = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Resume, Some(merge_id)),
        "op_continue_retry",
    )
    .unwrap();
    assert_eq!(response.state, crate::MergeOperationState::Finalizing);
    assert_eq!(
        merge_repo(&response, "mem_app").state,
        crate::MergeParticipantState::FastForwarded
    );
    assert_eq!(
        merge_repo(&response, "mem_lib").state,
        crate::MergeParticipantState::FastForwarded
    );
    assert_eq!(backend.head(&app).unwrap().commit, Some(app_source));
    assert_eq!(backend.head(&lib).unwrap().commit, Some(lib_source));
}

#[test]
fn unrelated_staged_conflict_work_blocks_every_resolution_commit() {
    let temp = TempDir::new("merge-conflict-index-preflight");
    let backend = crate::git::Git2Backend::new();
    let (_app_fixture, _lib_fixture) = init_two_member_workspace(temp.path(), &backend);
    let make_conflict = |repo: &std::path::Path| {
        let initial = backend.head(repo).unwrap().commit.unwrap();
        let stable = commit_file(
            repo,
            "stable.txt",
            "stable\n",
            "stable",
            &[git2::Oid::from_str(&initial).unwrap()],
        )
        .unwrap();
        let (base, _) = feature_commit(&backend, repo, "README.md", "source\n");
        assert_eq!(base, stable);
        commit_file(
            repo,
            "README.md",
            "local\n",
            "local",
            &[git2::Oid::from_str(&base).unwrap()],
        )
        .unwrap()
    };
    let app = temp.path().join("app");
    let lib = temp.path().join("lib");
    let app_before = make_conflict(&app);
    make_conflict(&lib);
    let started = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();
    assert_eq!(started.participant_counts.conflicted, 2);

    for repo in [&app, &lib] {
        fs::write(repo.join("README.md"), "resolved\n").unwrap();
        backend
            .stage_paths_allowing_other_conflicts(repo, &["README.md"])
            .unwrap();
    }
    fs::write(lib.join("stable.txt"), "unrelated staged work\n").unwrap();
    backend
        .stage_paths_allowing_other_conflicts(&lib, &["stable.txt"])
        .unwrap();

    let error = handle_merge(
        &backend,
        temp.path(),
        recovery_request(crate::MergeOp::Resume, started.merge_id),
        "op_continue",
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeDrift);
    assert_eq!(error.member_id.as_deref(), Some("mem_lib"));
    assert_eq!(backend.head(&app).unwrap().commit, Some(app_before));
    assert!(backend.merge_state(&app).unwrap().is_some());
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

#[test]
fn conditional_stage_allows_only_recorded_conflicted_participants() {
    let temp = TempDir::new("merge-stage-gate");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let started = handle_merge(&backend, temp.path(), request(false), "op_merge").unwrap();
    assert_eq!(
        started.state,
        crate::MergeOperationState::AwaitingResolution
    );

    let stage = |pathspec: &str, operation_id: &str| {
        handle_stage(
            &backend,
            temp.path(),
            crate::StageRequest {
                meta: request_meta(),
                cwd: temp.path().to_string_lossy().into_owned(),
                pathspecs: vec![pathspec.to_owned()],
                all: None,
            },
            operation_id,
        )
    };

    fs::write(temp.path().join("docs/README.md"), "resolved\n").unwrap();
    stage("docs/README.md", "op_stage_conflict").unwrap();
    assert_eq!(
        backend
            .status(&temp.path().join("docs"))
            .unwrap()
            .unresolved,
        0
    );
    let lib_staged = backend.status(&temp.path().join("lib")).unwrap().staged;
    let app_staged = backend.status(&temp.path().join("app")).unwrap().staged;
    let root_staged = backend.status(temp.path()).unwrap().staged;

    for (pathspec, operation_id) in [
        ("lib/new.txt", "op_stage_merged"),
        ("app/new.txt", "op_stage_unaffected"),
        ("root-new.txt", "op_stage_root"),
    ] {
        fs::write(temp.path().join(pathspec), "must remain unstaged\n").unwrap();
        let error = stage(pathspec, operation_id).unwrap_err();
        assert_eq!(error.code, ErrorCode::OpenOperation, "{pathspec}");
    }
    assert_eq!(
        backend.status(&temp.path().join("lib")).unwrap().staged,
        lib_staged
    );
    assert_eq!(
        backend.status(&temp.path().join("app")).unwrap().staged,
        app_staged
    );
    assert_eq!(backend.status(temp.path()).unwrap().staged, root_staged);
}

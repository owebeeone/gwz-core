use super::*;

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

    assert_eq!(continued.state, crate::MergeOperationState::Completed);
    assert!(!continued.open);
    assert_eq!(
        merge_repo(&continued, "mem_docs").state,
        crate::MergeParticipantState::Continued
    );
    assert_eq!(
        backend.head(&temp.path().join("lib")).unwrap().commit,
        Some(lib_result.clone())
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
    assert_ne!(
        fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
        lock_before
    );
    let published = read_lock(temp.path()).unwrap();
    assert_eq!(
        published.members["mem_lib"].commit.as_deref(),
        Some(lib_result.as_str())
    );
    assert_eq!(
        published.members["mem_docs"].commit.as_deref(),
        merge_repo(&continued, "mem_docs")
            .resulting_commit
            .as_deref()
    );
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
            root_branch: None,
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
    assert_eq!(response.state, crate::MergeOperationState::Completed);
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

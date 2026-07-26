use super::*;

#[test]
fn first_class_merge_fast_forwards_and_publishes_durable_composition() {
    let temp = TempDir::new("merge-start-ff");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-start-ff-source");
    let member = temp.path().join("remote");
    let (base, source) = feature_commit(&backend, &member, "README.md", "source\n");

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
    assert_eq!(response.state, crate::MergeOperationState::Completed);
    assert!(!response.open);
    assert_eq!(
        response.publication_step,
        Some(crate::MergePublicationStep::Complete)
    );
    assert_eq!(response.merge_id.as_deref(), Some("merge_op_merge_0001"));
    assert_eq!(
        backend.head(&member).unwrap().commit.as_deref(),
        Some(source.as_str())
    );
    assert!(
        temp.path()
            .join(".gwz/merge/done/merge_op_merge_0001.yaml")
            .is_file()
    );
    assert_eq!(
        read_lock(temp.path()).unwrap().members["mem_remote"]
            .commit
            .as_deref(),
        Some(source.as_str())
    );
    assert_ne!(base, source);
    let markers = crate::artifact::list_markers(temp.path()).unwrap();
    assert_eq!(markers.len(), 1);
    assert_eq!(
        markers[0].merge.as_ref().unwrap().participants["mem_remote"].resulting_commit,
        source
    );
}

#[test]
fn all_up_to_date_merge_completes_without_root_evidence() {
    let temp = TempDir::new("merge-all-up-to-date");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-no-op-source");
    let member = temp.path().join("remote");
    backend
        .branch_create(&member, "feature/source", "HEAD")
        .unwrap();
    let root_before = backend.head(temp.path()).unwrap();

    let response = handle_merge(&backend, temp.path(), request(false), "op_noop").unwrap();

    assert_eq!(response.state, crate::MergeOperationState::Completed);
    assert_eq!(
        response.repos[0].state,
        crate::MergeParticipantState::UpToDate
    );
    assert_eq!(backend.head(temp.path()).unwrap(), root_before);
    assert!(
        crate::artifact::list_markers(temp.path())
            .unwrap()
            .is_empty()
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
fn dry_run_predicts_conflicts_without_changing_git_or_gwz_state() {
    let temp = TempDir::new("merge-dry-conflict-prediction");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-dry-conflict-source");
    let member = temp.path().join("remote");
    let (base, _) = feature_commit(&backend, &member, "README.md", "source\n");
    let before = commit_file(
        &member,
        "README.md",
        "target\n",
        "target",
        &[git2::Oid::from_str(&base).unwrap()],
    )
    .unwrap();
    let root_head = backend.head(temp.path()).unwrap();
    let member_head = backend.head(&member).unwrap();
    let member_index = fs::read(member.join(".git/index")).unwrap();
    let member_file = fs::read(member.join("README.md")).unwrap();
    let lock = fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap();

    let response =
        handle_merge(&backend, temp.path(), request(true), "op_predict_conflict").unwrap();

    assert_eq!(
        response.repos[0].predicted,
        Some(crate::MergeAnalysisKind::TrueMerge)
    );
    assert_eq!(response.repos[0].prediction_complete, Some(true));
    assert_eq!(response.repos[0].conflict_paths, ["README.md"]);
    assert_eq!(backend.head(&member).unwrap(), member_head);
    assert_eq!(member_head.commit.as_deref(), Some(before.as_str()));
    assert_eq!(backend.head(temp.path()).unwrap(), root_head);
    assert_eq!(fs::read(member.join(".git/index")).unwrap(), member_index);
    assert_eq!(fs::read(member.join("README.md")).unwrap(), member_file);
    assert_eq!(
        fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
        lock
    );
    assert!(backend.merge_state(&member).unwrap().is_none());
    assert!(!temp.path().join(".gwz/merge/open").exists());
}

#[test]
fn ff_only_rejects_mixed_batch_before_an_earlier_fast_forward() {
    let temp = TempDir::new("merge-ff-only-atomic");
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
    let lock = fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap();
    let mut value = request(false);
    value.mode = Some(crate::MergeMode::FfOnly);

    let error = handle_merge(&backend, temp.path(), value, "op_ff_only").unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeValidationFailed);
    assert_eq!(error.member_id.as_deref(), Some("mem_lib"));
    assert_eq!(error.member_path.as_deref(), Some("lib"));
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
        lock
    );
    assert!(!temp.path().join(".gwz/merge/open").exists());
}

#[test]
fn ff_only_fast_forward_persists_its_durable_mode() {
    let temp = TempDir::new("merge-ff-only-success");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-ff-only-source");
    let member = temp.path().join("remote");
    let (_, source) = feature_commit(&backend, &member, "source.txt", "source\n");
    let mut value = request(false);
    value.mode = Some(crate::MergeMode::FfOnly);

    let response = handle_merge(&backend, temp.path(), value, "op_ff_only").unwrap();

    assert_eq!(
        response.repos[0].state,
        crate::MergeParticipantState::FastForwarded
    );
    assert_eq!(
        backend.head(&member).unwrap().commit.as_deref(),
        Some(source.as_str())
    );
    let record = fs::read_to_string(
        temp.path()
            .join(".gwz/merge/done/merge_op_ff_only_0001.yaml"),
    )
    .unwrap();
    assert!(record.contains("\nmode: ff_only\n"));
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

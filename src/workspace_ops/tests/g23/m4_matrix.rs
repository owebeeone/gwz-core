use super::*;

fn mixed_request(dry_run: bool, ff_only: bool) -> crate::MergeRequest {
    let mut request = request(dry_run);
    request.meta.selection = Some(crate::Selection {
        targets: vec!["mem_remote".to_owned(), "@root".to_owned()],
        ..Default::default()
    });
    if ff_only {
        request.mode = Some(crate::MergeMode::FfOnly);
    }
    request
}

fn commit_root_baseline(root: &Path, backend: &crate::git::Git2Backend) -> String {
    backend.stage_paths(root, &["gwz.conf"]).unwrap();
    commit_file(root, "root.txt", "baseline\n", "root baseline", &[]).unwrap()
}

fn seed_root_true_merge(root: &Path, backend: &crate::git::Git2Backend, conflict: bool) -> String {
    let baseline = commit_root_baseline(root, backend);
    backend
        .branch_create(root, "feature/source", "HEAD")
        .unwrap();
    backend.switch_branch(root, "feature/source").unwrap();
    let source_path = if conflict {
        "root.txt"
    } else {
        "root-source.txt"
    };
    commit_file(
        root,
        source_path,
        "source\n",
        "root source",
        &[git2::Oid::from_str(&baseline).unwrap()],
    )
    .unwrap();
    backend.switch_branch(root, "main").unwrap();
    commit_file(
        root,
        "root.txt",
        "target\n",
        "root target",
        &[git2::Oid::from_str(&baseline).unwrap()],
    )
    .unwrap()
}

#[test]
fn mixed_member_root_dry_run_reports_clean_and_conflicted_predictions_without_mutation() {
    let temp = TempDir::new("merge-mixed-predictions");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-mixed-predict-source");
    let member = temp.path().join("remote");
    let (member_base, _) = feature_commit(&backend, &member, "source.txt", "source\n");
    let member_target = commit_file(
        &member,
        "local.txt",
        "local\n",
        "local",
        &[git2::Oid::from_str(&member_base).unwrap()],
    )
    .unwrap();
    let root_target = seed_root_true_merge(temp.path(), &backend, true);

    let member_index = fs::read(member.join(".git/index")).unwrap();
    let root_index = fs::read(temp.path().join(".git/index")).unwrap();
    let member_file = fs::read(member.join("local.txt")).unwrap();
    let root_file = fs::read(temp.path().join("root.txt")).unwrap();
    let lock = fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap();

    let response = handle_merge(
        &backend,
        temp.path(),
        mixed_request(true, false),
        "op_mixed_predict",
    )
    .unwrap();

    let member_row = merge_repo(&response, "mem_remote");
    assert_eq!(
        member_row.predicted,
        Some(crate::MergeAnalysisKind::TrueMerge)
    );
    assert_eq!(member_row.prediction_complete, Some(true));
    assert!(member_row.conflict_paths.is_empty());
    let root_row = merge_repo(&response, "@root");
    assert_eq!(
        root_row.predicted,
        Some(crate::MergeAnalysisKind::TrueMerge)
    );
    assert_eq!(root_row.prediction_complete, Some(true));
    assert_eq!(root_row.conflict_paths, ["root.txt"]);

    assert_eq!(
        backend.head(&member).unwrap().commit.as_deref(),
        Some(member_target.as_str())
    );
    assert_eq!(
        backend.head(temp.path()).unwrap().commit.as_deref(),
        Some(root_target.as_str())
    );
    assert_eq!(fs::read(member.join(".git/index")).unwrap(), member_index);
    assert_eq!(
        fs::read(temp.path().join(".git/index")).unwrap(),
        root_index
    );
    assert_eq!(fs::read(member.join("local.txt")).unwrap(), member_file);
    assert_eq!(fs::read(temp.path().join("root.txt")).unwrap(), root_file);
    assert_eq!(
        fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
        lock
    );
    assert!(FileMergeStore.discover_open(temp.path()).unwrap().is_none());
}

#[test]
fn mixed_ff_only_rejects_later_root_true_merge_before_member_fast_forward() {
    let temp = TempDir::new("merge-mixed-ff-only-reject");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-mixed-ff-reject-source");
    let member = temp.path().join("remote");
    let (member_before, _) = feature_commit(&backend, &member, "source.txt", "source\n");
    let root_before = seed_root_true_merge(temp.path(), &backend, false);
    let lock = fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap();

    let error = handle_merge(
        &backend,
        temp.path(),
        mixed_request(false, true),
        "op_mixed_ff_reject",
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeValidationFailed);
    assert_eq!(error.member_id.as_deref(), Some("@root"));
    assert_eq!(error.member_path.as_deref(), Some("."));
    assert_eq!(
        backend.head(&member).unwrap().commit.as_deref(),
        Some(member_before.as_str())
    );
    assert_eq!(
        backend.head(temp.path()).unwrap().commit.as_deref(),
        Some(root_before.as_str())
    );
    assert_eq!(
        fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
        lock
    );
    assert!(FileMergeStore.discover_open(temp.path()).unwrap().is_none());
}

#[test]
fn mixed_ff_only_accepts_member_fast_forward_and_up_to_date_root() {
    let temp = TempDir::new("merge-mixed-ff-only-success");
    let backend = crate::git::Git2Backend::new();
    let _fixture =
        init_one_member_workspace(temp.path(), &backend, "merge-mixed-ff-success-source");
    let member = temp.path().join("remote");
    let (_, member_source) = feature_commit(&backend, &member, "source.txt", "source\n");
    let root_before = commit_root_baseline(temp.path(), &backend);
    backend
        .branch_create(temp.path(), "feature/source", "HEAD")
        .unwrap();

    let response = handle_merge(
        &backend,
        temp.path(),
        mixed_request(false, true),
        "op_mixed_ff_success",
    )
    .unwrap();

    assert_eq!(response.state, crate::MergeOperationState::Completed);
    assert_eq!(
        merge_repo(&response, "mem_remote").state,
        crate::MergeParticipantState::FastForwarded
    );
    let root = merge_repo(&response, "@root");
    assert_eq!(root.state, crate::MergeParticipantState::UpToDate);
    assert_eq!(root.resulting_commit.as_deref(), Some(root_before.as_str()));
    assert_eq!(
        backend.head(&member).unwrap().commit.as_deref(),
        Some(member_source.as_str())
    );
}

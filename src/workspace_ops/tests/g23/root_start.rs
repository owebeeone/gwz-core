use super::*;

fn root_request(dry_run: bool) -> crate::MergeRequest {
    let mut request = request(dry_run);
    request.meta.selection = Some(crate::Selection {
        targets: vec!["@root".to_owned()],
        ..Default::default()
    });
    request
}

fn mixed_request() -> crate::MergeRequest {
    let mut request = request(false);
    request.meta.selection = Some(crate::Selection {
        targets: vec!["mem_remote".to_owned(), "@root".to_owned()],
        ..Default::default()
    });
    request
}

fn init_root_feature(root: &Path, backend: &crate::git::Git2Backend) -> (String, String) {
    backend.stage_paths(root, &["gwz.conf"]).unwrap();
    let baseline = commit_file(root, "root.txt", "baseline\n", "root baseline", &[]).unwrap();
    let (reported_baseline, source) = feature_commit(backend, root, "root-source.txt", "source\n");
    assert_eq!(reported_baseline, baseline);
    (baseline, source)
}

fn init_root_true_merge(root: &Path, backend: &crate::git::Git2Backend) -> String {
    backend.stage_paths(root, &["gwz.conf"]).unwrap();
    let baseline = commit_file(root, "root.txt", "baseline\n", "root baseline", &[]).unwrap();
    backend
        .branch_create(root, "feature/source", "HEAD")
        .unwrap();
    backend.switch_branch(root, "feature/source").unwrap();
    commit_file(
        root,
        "root.txt",
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

fn init_root_metadata_feature(
    root: &Path,
    backend: &crate::git::Git2Backend,
) -> (String, String, String) {
    backend.stage_paths(root, &["gwz.conf"]).unwrap();
    let baseline = commit_file(root, "root.txt", "baseline\n", "root baseline", &[]).unwrap();
    backend
        .branch_create(root, "feature/source", "HEAD")
        .unwrap();
    backend.switch_branch(root, "feature/source").unwrap();

    let mut lock = crate::artifact::read_lock(root).unwrap();
    lock.members.get_mut("mem_remote").unwrap().branch = Some("feature/root-metadata".to_owned());
    let source_lock = lock.to_yaml().unwrap();
    fs::write(root.join(crate::artifact::LOCK_PATH), &source_lock).unwrap();
    backend
        .stage_paths(root, &[crate::artifact::LOCK_PATH])
        .unwrap();
    let source_manifest = fs::read_to_string(root.join(crate::workspace::WORKSPACE_MANIFEST))
        .unwrap()
        .replacen(
            "schema: gwz.workspace/v0",
            "schema: gwz.workspace/v0 # root source",
            1,
        );
    let source = commit_file(
        root,
        crate::workspace::WORKSPACE_MANIFEST,
        &source_manifest,
        "root metadata source",
        &[git2::Oid::from_str(&baseline).unwrap()],
    )
    .unwrap();
    backend.switch_branch(root, "main").unwrap();
    (baseline, source, source_lock)
}

#[test]
fn explicit_root_dry_run_is_visible_and_does_not_mutate() {
    let temp = TempDir::new("merge-root-dry-run");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-root-dry-source");
    let (baseline, source) = init_root_feature(temp.path(), &backend);

    let response = handle_merge(&backend, temp.path(), root_request(true), "op_root_dry").unwrap();

    assert_eq!(
        response.response.meta.aggregate_status,
        crate::AggregateStatus::Accepted
    );
    assert_eq!(response.repos.len(), 1);
    assert_eq!(response.repos[0].target_id, "@root");
    assert_eq!(response.repos[0].target_kind, crate::TargetKind::Root);
    assert_eq!(response.repos[0].path, ".");
    assert_eq!(
        response.repos[0].state,
        crate::MergeParticipantState::Planned
    );
    assert_eq!(response.repos[0].before_commit, baseline);
    assert_eq!(response.repos[0].source_commit, source);
    assert_eq!(
        backend.head(temp.path()).unwrap().commit.as_deref(),
        Some(baseline.as_str())
    );
    assert!(FileMergeStore.discover_open(temp.path()).unwrap().is_none());
}

#[test]
fn explicit_root_dry_run_predicts_conflict_without_mutation() {
    let temp = TempDir::new("merge-root-dry-conflict");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-root-conflict-source");
    let target = init_root_true_merge(temp.path(), &backend);
    let index = fs::read(temp.path().join(".git/index")).unwrap();
    let worktree = fs::read(temp.path().join("root.txt")).unwrap();
    let lock = fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap();

    let response =
        handle_merge(&backend, temp.path(), root_request(true), "op_root_predict").unwrap();

    let root = merge_repo(&response, "@root");
    assert_eq!(root.predicted, Some(crate::MergeAnalysisKind::TrueMerge));
    assert_eq!(root.prediction_complete, Some(true));
    assert_eq!(root.conflict_paths, ["root.txt"]);
    assert_eq!(
        backend.head(temp.path()).unwrap().commit.as_deref(),
        Some(target.as_str())
    );
    assert_eq!(fs::read(temp.path().join(".git/index")).unwrap(), index);
    assert_eq!(fs::read(temp.path().join("root.txt")).unwrap(), worktree);
    assert_eq!(
        fs::read(temp.path().join(crate::artifact::LOCK_PATH)).unwrap(),
        lock
    );
    assert!(FileMergeStore.discover_open(temp.path()).unwrap().is_none());
}

#[test]
fn explicit_root_ff_only_rejects_true_merge_before_mutation() {
    let temp = TempDir::new("merge-root-ff-only-reject");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-root-ff-only-source");
    let target = init_root_true_merge(temp.path(), &backend);
    let index = fs::read(temp.path().join(".git/index")).unwrap();
    let worktree = fs::read(temp.path().join("root.txt")).unwrap();
    let mut request = root_request(false);
    request.mode = Some(crate::MergeMode::FfOnly);

    let error = handle_merge(&backend, temp.path(), request, "op_root_ff_only").unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeValidationFailed);
    assert_eq!(error.member_id.as_deref(), Some("@root"));
    assert_eq!(error.member_path.as_deref(), Some("."));
    assert_eq!(
        backend.head(temp.path()).unwrap().commit.as_deref(),
        Some(target.as_str())
    );
    assert_eq!(fs::read(temp.path().join(".git/index")).unwrap(), index);
    assert_eq!(fs::read(temp.path().join("root.txt")).unwrap(), worktree);
    assert!(FileMergeStore.discover_open(temp.path()).unwrap().is_none());
}

#[test]
fn explicit_root_fast_forward_finalizes_on_top_of_root_result() {
    let temp = TempDir::new("merge-root-fast-forward");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-root-ff-source");
    let (baseline, source) = init_root_feature(temp.path(), &backend);

    let response = handle_merge(&backend, temp.path(), root_request(false), "op_root_ff").unwrap();

    assert_eq!(response.state, crate::MergeOperationState::Completed);
    assert_eq!(response.repos.len(), 1);
    assert_eq!(response.repos[0].target_kind, crate::TargetKind::Root);
    assert_eq!(
        response.repos[0].state,
        crate::MergeParticipantState::FastForwarded
    );
    assert_eq!(response.repos[0].before_commit, baseline);
    assert_eq!(
        response.repos[0].resulting_commit.as_deref(),
        Some(source.as_str())
    );
    let evidence = backend.head(temp.path()).unwrap().commit.unwrap();
    assert_ne!(evidence, source);
    let record = FileMergeStore
        .load(temp.path(), response.merge_id.as_deref().unwrap())
        .unwrap();
    let publication = record.publication.unwrap();
    assert_eq!(
        publication.root_merge_commit.as_deref(),
        Some(source.as_str())
    );
    assert_eq!(
        publication.composition_commit.as_deref(),
        Some(evidence.as_str())
    );
}

#[test]
fn explicit_root_up_to_date_is_a_no_op_without_evidence() {
    let temp = TempDir::new("merge-root-up-to-date");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-root-noop-source");
    backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
    let baseline =
        commit_file(temp.path(), "root.txt", "baseline\n", "root baseline", &[]).unwrap();
    backend
        .branch_create(temp.path(), "feature/source", "HEAD")
        .unwrap();

    let response =
        handle_merge(&backend, temp.path(), root_request(false), "op_root_noop").unwrap();

    assert_eq!(response.state, crate::MergeOperationState::Completed);
    assert_eq!(
        merge_repo(&response, "@root").state,
        crate::MergeParticipantState::UpToDate
    );
    assert_eq!(
        backend.head(temp.path()).unwrap().commit.as_deref(),
        Some(baseline.as_str())
    );
    assert!(
        crate::artifact::list_markers(temp.path())
            .unwrap()
            .is_empty()
    );
    let record = FileMergeStore
        .load(temp.path(), response.merge_id.as_deref().unwrap())
        .unwrap();
    assert!(
        record
            .publication
            .as_ref()
            .unwrap()
            .composition_commit
            .is_none()
    );
}

#[test]
fn explicit_root_clean_merge_records_merge_result_before_evidence() {
    let temp = TempDir::new("merge-root-clean-merge");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-root-clean-source");
    backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
    let base = commit_file(temp.path(), "root.txt", "baseline\n", "root baseline", &[]).unwrap();
    let (_, source) = feature_commit(&backend, temp.path(), "root-source.txt", "source\n");
    let before = commit_file(
        temp.path(),
        "root-local.txt",
        "local\n",
        "local",
        &[git2::Oid::from_str(&base).unwrap()],
    )
    .unwrap();

    let response = handle_merge(
        &backend,
        temp.path(),
        root_request(false),
        "op_root_true_merge",
    )
    .unwrap();

    assert_eq!(response.state, crate::MergeOperationState::Completed);
    assert_eq!(
        merge_repo(&response, "@root").state,
        crate::MergeParticipantState::Merged
    );
    let root_result = merge_repo(&response, "@root")
        .resulting_commit
        .as_deref()
        .unwrap();
    let repository = git2::Repository::open(temp.path()).unwrap();
    let merge_commit = repository
        .find_commit(git2::Oid::from_str(root_result).unwrap())
        .unwrap();
    assert_eq!(merge_commit.parent_count(), 2);
    assert_eq!(merge_commit.parent_id(0).unwrap().to_string(), before);
    assert_eq!(merge_commit.parent_id(1).unwrap().to_string(), source);
    assert_ne!(
        backend.head(temp.path()).unwrap().commit.as_deref(),
        Some(root_result)
    );
}

#[test]
fn root_metadata_merge_uses_the_root_result_as_publication_baseline() {
    let temp = TempDir::new("merge-root-metadata-fast-forward");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-root-meta-source");
    let (_baseline, source, source_lock) = init_root_metadata_feature(temp.path(), &backend);

    let response = handle_merge(
        &backend,
        temp.path(),
        root_request(false),
        "op_root_meta_ff",
    )
    .unwrap();

    assert_eq!(response.state, crate::MergeOperationState::Completed);
    assert_eq!(
        merge_repo(&response, "@root").resulting_commit.as_deref(),
        Some(source.as_str())
    );
    let record = FileMergeStore
        .load(temp.path(), response.merge_id.as_deref().unwrap())
        .unwrap();
    let publication = record.publication.unwrap();
    assert_eq!(
        publication.candidate.as_ref().unwrap().baseline_lock_yaml,
        source_lock
    );
}

#[test]
fn member_conflict_does_not_prevent_root_last_execution() {
    let temp = TempDir::new("merge-root-after-member-conflict");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-root-conflict-source");
    let member = temp.path().join("remote");
    let (member_base, _) = feature_commit(&backend, &member, "README.md", "source\n");
    commit_file(
        &member,
        "README.md",
        "local\n",
        "local",
        &[git2::Oid::from_str(&member_base).unwrap()],
    )
    .unwrap();
    let (_, root_source) = init_root_feature(temp.path(), &backend);

    let response = handle_merge(&backend, temp.path(), mixed_request(), "op_root_mixed").unwrap();

    assert_eq!(
        response.state,
        crate::MergeOperationState::AwaitingResolution
    );
    assert_eq!(
        merge_repo(&response, "mem_remote").state,
        crate::MergeParticipantState::Conflicted
    );
    assert_eq!(
        merge_repo(&response, "@root").state,
        crate::MergeParticipantState::FastForwarded
    );
    assert_eq!(
        backend.head(temp.path()).unwrap().commit.as_deref(),
        Some(root_source.as_str())
    );
    let record = FileMergeStore.discover_open(temp.path()).unwrap().unwrap();
    assert_eq!(record.selected_targets, ["mem_remote", "@root"]);
}

#[test]
fn unborn_explicit_root_rejects_before_member_mutation() {
    let temp = TempDir::new("merge-unborn-explicit-root");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-root-unborn-source");
    let member = temp.path().join("remote");
    let (member_before, _) = feature_commit(&backend, &member, "source.txt", "source\n");

    let error = handle_merge(&backend, temp.path(), mixed_request(), "op_root_unborn").unwrap_err();

    assert_eq!(error.code, ErrorCode::BranchUnbornHead);
    assert_eq!(error.member_id.as_deref(), Some("@root"));
    assert_eq!(error.member_path.as_deref(), Some("."));
    assert_eq!(
        backend.head(&member).unwrap().commit.as_deref(),
        Some(member_before.as_str())
    );
    assert!(FileMergeStore.discover_open(temp.path()).unwrap().is_none());
}

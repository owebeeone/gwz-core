use super::*;

#[test]
fn finalization_emits_verified_composition_artifacts_in_order() {
    let temp = TempDir::new("merge-finalize-artifact-events");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-artifact-events");
    feature_commit(
        &backend,
        &temp.path().join("remote"),
        "README.md",
        "source\n",
    );
    let sink = CollectingSink::default();
    let response = crate::workspace_ops::handle_merge_with_events(
        &backend,
        temp.path(),
        request(false),
        "op_artifact_events",
        &sink,
    )
    .unwrap();
    let record = archived_record(temp.path(), response.merge_id.as_deref().unwrap());
    let record = record.view();
    let publication = record.publication().unwrap();
    let expected = [
        format!(
            "git:@root/{}",
            publication.composition_commit.as_deref().unwrap()
        ),
        publication.candidate_marker_path.clone().unwrap(),
        crate::artifact::LOCK_PATH.to_owned(),
        ".git/info/exclude".to_owned(),
    ];
    let artifacts = sink
        .take()
        .into_iter()
        .filter(|event| event.kind == crate::EventKind::ArtifactWritten)
        .filter_map(|event| event.artifact_path)
        .filter(|path| expected.contains(path))
        .collect::<Vec<_>>();
    assert_eq!(artifacts, expected);
}

#[test]
fn finalization_preserves_unrelated_root_staged_dirty_and_untracked_work() {
    let temp = TempDir::new("merge-root-local-work");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-root-work-source");
    let member = temp.path().join("remote");
    feature_commit(&backend, &member, "README.md", "source\n");

    commit_file(
        temp.path(),
        "root-note.txt",
        "accepted\n",
        "root baseline",
        &[],
    )
    .unwrap();
    fs::write(temp.path().join("root-note.txt"), "dirty\n").unwrap();
    fs::write(temp.path().join("staged-local.txt"), "staged\n").unwrap();
    backend
        .stage_paths(temp.path(), &["staged-local.txt"])
        .unwrap();
    fs::write(temp.path().join("untracked-local.txt"), "untracked\n").unwrap();
    let repository = git2::Repository::open(temp.path()).unwrap();
    let staged_before = repository
        .index()
        .unwrap()
        .get_path(Path::new("staged-local.txt"), 0)
        .unwrap()
        .id;

    let response = handle_merge(&backend, temp.path(), request(false), "op_local_work").unwrap();

    assert_eq!(response.state, crate::MergeOperationState::Completed);
    assert_eq!(
        fs::read_to_string(temp.path().join("root-note.txt")).unwrap(),
        "dirty\n"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("untracked-local.txt")).unwrap(),
        "untracked\n"
    );
    let repository = git2::Repository::open(temp.path()).unwrap();
    let staged_after = repository
        .index()
        .unwrap()
        .get_path(Path::new("staged-local.txt"), 0)
        .unwrap()
        .id;
    assert_eq!(staged_after, staged_before);
    let head_tree = repository.head().unwrap().peel_to_tree().unwrap();
    assert!(head_tree.get_path(Path::new("staged-local.txt")).is_err());
}

use super::*;

#[test]
fn finalization_faults_report_status_and_resume_without_duplicate_evidence() {
    for fault in [
        FinalizationFault::AfterEnteringFinalizing,
        FinalizationFault::BeforeCandidateCreation,
        FinalizationFault::AfterCandidatePersistence,
        FinalizationFault::AfterEvidenceCommit,
        FinalizationFault::AfterEvidencePersistence,
        FinalizationFault::AfterLockPublication,
        FinalizationFault::BeforeArchive,
    ] {
        let temp = TempDir::new(&format!("merge-finalize-{fault:?}"));
        let backend = crate::git::Git2Backend::new();
        let _fixture =
            init_one_member_workspace(temp.path(), &backend, &format!("merge-fault-{fault:?}"));
        let member = temp.path().join("remote");
        feature_commit(&backend, &member, "README.md", "source\n");
        let baseline_root = backend.head(temp.path()).unwrap().commit;
        let store = FaultingMergeStore::new(fault);

        let failure = invoke_with_store(&backend, &store, temp.path(), request(false), "op_fault")
            .unwrap_err();
        assert_eq!(failure.code, ErrorCode::MergeRecoveryRequired, "{fault:?}");
        assert!(store.fired.get(), "{fault:?}");

        let before_status = store.discover_open(temp.path()).unwrap().unwrap();
        let head_before_status = backend.head(temp.path()).unwrap();
        let status = invoke_with_store(
            &backend,
            &store,
            temp.path(),
            recovery_request(crate::MergeOp::Status, None),
            "op_status",
        )
        .unwrap();
        assert!(matches!(
            status.state,
            crate::MergeOperationState::Finalizing | crate::MergeOperationState::Completed
        ));
        assert_eq!(
            store.discover_open(temp.path()).unwrap().unwrap(),
            before_status,
            "status changed the durable record at {fault:?}"
        );
        assert_eq!(
            backend.head(temp.path()).unwrap(),
            head_before_status,
            "status changed root HEAD at {fault:?}"
        );

        let merge_id = before_status.merge_id.clone();
        let completed = invoke_with_store(
            &backend,
            &store,
            temp.path(),
            recovery_request(crate::MergeOp::Resume, Some(merge_id.clone())),
            "op_resume",
        )
        .unwrap();
        assert_eq!(completed.state, crate::MergeOperationState::Completed);
        assert!(!completed.open);
        assert!(store.discover_open(temp.path()).unwrap().is_none());

        let archived = store.load(temp.path(), &merge_id).unwrap();
        let composition = archived
            .publication
            .as_ref()
            .and_then(|publication| publication.composition_commit.as_deref())
            .unwrap();
        assert_eq!(
            backend.head(temp.path()).unwrap().commit.as_deref(),
            Some(composition)
        );
        let repository = git2::Repository::open(temp.path()).unwrap();
        let commit = repository
            .find_commit(git2::Oid::from_str(composition).unwrap())
            .unwrap();
        match baseline_root {
            Some(baseline) => {
                assert_eq!(commit.parent_count(), 1);
                assert_eq!(commit.parent_id(0).unwrap().to_string(), baseline);
            }
            None => assert_eq!(commit.parent_count(), 0),
        }
    }
}

#[test]
fn repaired_candidate_prefix_resumes_to_one_evidence_commit() {
    let temp = TempDir::new("merge-finalize-repaired-prefix");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(temp.path(), &backend, "merge-repaired-prefix");
    feature_commit(
        &backend,
        &temp.path().join("remote"),
        "README.md",
        "source\n",
    );
    let store = FaultingMergeStore::new(FinalizationFault::AfterLockPublication);
    invoke_with_store(
        &backend,
        &store,
        temp.path(),
        request(false),
        "op_repaired_prefix",
    )
    .unwrap_err();
    let record = store.discover_open(temp.path()).unwrap().unwrap();
    let publication = record.publication.as_ref().unwrap();
    let candidate = publication.candidate.as_ref().unwrap();
    let evidence = publication.composition_commit.clone().unwrap();
    let marker_path = crate::artifact::marker_path(temp.path(), &candidate.marker_id);
    fs::remove_file(&marker_path).unwrap();

    let blocked = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        recovery_request(crate::MergeOp::Resume, Some(record.merge_id.clone())),
        "op_prefix_blocked",
    )
    .unwrap();
    assert!(
        blocked.operation_drift.iter().any(|drift| {
            drift.kind == crate::MergeOperationDriftKind::RootCandidateStateChanged
        })
    );
    fs::write(&marker_path, &candidate.marker_yaml).unwrap();

    let completed = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        recovery_request(crate::MergeOp::Resume, Some(record.merge_id.clone())),
        "op_prefix_repaired",
    )
    .unwrap();
    assert_eq!(completed.state, crate::MergeOperationState::Completed);
    assert_eq!(
        backend.head(temp.path()).unwrap().commit.as_deref(),
        Some(evidence.as_str())
    );
}

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
    let record = FileMergeStore
        .load(temp.path(), response.merge_id.as_deref().unwrap())
        .unwrap();
    let publication = record.publication.unwrap();
    let expected = [
        format!(
            "git:@root/{}",
            publication.composition_commit.as_deref().unwrap()
        ),
        publication.candidate_marker_path.unwrap(),
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

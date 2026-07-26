use super::*;
use crate::workspace_ops::merge::{
    ParticipantState, PendingMergeAction, PendingMergeActionKind, PendingMergeExpectedResult,
};

#[test]
fn preserve_abort_rejects_edited_or_staged_conflict_resolution_before_artifacts() {
    for staged in [false, true] {
        let temp = TempDir::new(if staged {
            "merge-preserve-staged-conflict"
        } else {
            "merge-preserve-edited-conflict"
        });
        let backend = crate::git::Git2Backend::new();
        let _fixture = init_mixed_merge_workspace(temp.path(), &backend);
        let started = handle_merge(
            &backend,
            temp.path(),
            request(false),
            "op_preserve_conflict",
        )
        .unwrap();
        let docs = temp.path().join("docs");
        fs::write(
            docs.join("README.md"),
            if staged {
                "fully resolved and staged\n"
            } else {
                "partially edited but unresolved\n"
            },
        )
        .unwrap();
        if staged {
            backend
                .stage_paths_allowing_other_conflicts(&docs, &["README.md"])
                .unwrap();
        }
        let merge_id = started.merge_id.clone().unwrap();
        let mut abort = recovery_request(crate::MergeOp::Abort, Some(merge_id.clone()));
        abort.preserve = Some(true);

        let error =
            handle_merge(&backend, temp.path(), abort, "op_preserve_conflict_abort").unwrap_err();

        assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
        assert_eq!(error.member_id.as_deref(), Some("mem_docs"));
        assert_eq!(
            backend.repository_state(&docs).unwrap(),
            crate::git::GitRepositoryState::Merge
        );
        assert!(
            FileMergeStore
                .discover_open(temp.path())
                .unwrap()
                .unwrap()
                .participants
                .values()
                .all(|participant| participant.preservation.is_empty())
        );
        assert!(
            backend
                .read_ref(
                    &temp.path().join("lib"),
                    &format!("refs/gwz/merge/{merge_id}/mem_lib/head"),
                )
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn preserve_abort_accepts_exact_interrupted_executing_state() {
    let temp = TempDir::new("merge-preserve-executing");
    let backend = crate::git::Git2Backend::new();
    let fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let started = handle_merge(
        &backend,
        temp.path(),
        request(false),
        "op_preserve_executing",
    )
    .unwrap();
    let merge_id = started.merge_id.clone().unwrap();
    let lib = temp.path().join("lib");
    let result = merge_repo(&started, "mem_lib")
        .resulting_commit
        .clone()
        .unwrap();
    let extra = commit_file(
        &lib,
        "after-interruption.txt",
        "preserve\n",
        "post-interruption work",
        &[git2::Oid::from_str(&result).unwrap()],
    )
    .unwrap();
    let mut record = FileMergeStore.discover_open(temp.path()).unwrap().unwrap();
    record.state = OperationState::Executing;
    FileMergeStore.write_open(temp.path(), &record).unwrap();
    let mut abort = recovery_request(crate::MergeOp::Abort, Some(merge_id));
    abort.preserve = Some(true);

    let response =
        handle_merge(&backend, temp.path(), abort, "op_preserve_executing_abort").unwrap();

    assert_eq!(response.state, crate::MergeOperationState::Aborted);
    assert_eq!(
        backend.head(&lib).unwrap().commit.as_deref(),
        Some(fixture.lib_before.as_str())
    );
    assert_eq!(
        response
            .preservation
            .unwrap()
            .into_iter()
            .find(|evidence| evidence.target_id == "mem_lib")
            .unwrap()
            .backup_commit
            .as_deref(),
        Some(extra.as_str())
    );
}

#[test]
fn member_only_merge_preserves_post_composition_root_work() {
    let temp = TempDir::new("merge-preserve-member-only-root");
    let backend = crate::git::Git2Backend::new();
    let _fixture =
        init_one_member_workspace(temp.path(), &backend, "merge-preserve-member-only-root");
    backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
    commit_file(temp.path(), "root.txt", "baseline\n", "root baseline", &[]).unwrap();
    feature_commit(
        &backend,
        &temp.path().join("remote"),
        "README.md",
        "source\n",
    );
    let root_before = backend.head(temp.path()).unwrap().commit;
    let store = FaultingMergeStore::new(FinalizationFault::AfterEvidencePersistence);
    invoke_with_store(
        &backend,
        &store,
        temp.path(),
        request(false),
        "op_preserve_member_only_root",
    )
    .unwrap_err();
    let record = store.discover_open(temp.path()).unwrap().unwrap();
    assert!(
        !record
            .selected_targets
            .iter()
            .any(|target| target == "@root")
    );
    let composition = record
        .publication
        .as_ref()
        .and_then(|publication| publication.composition_commit.as_ref())
        .unwrap()
        .clone();
    let extra = commit_file(
        temp.path(),
        "after-composition.txt",
        "keep root commit\n",
        "post-composition root work",
        &[git2::Oid::from_str(&composition).unwrap()],
    )
    .unwrap();
    fs::write(
        temp.path().join("root-untracked.txt"),
        "keep root worktree\n",
    )
    .unwrap();
    let mut abort = recovery_request(crate::MergeOp::Abort, Some(record.merge_id));
    abort.preserve = Some(true);

    let response = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        abort,
        "op_preserve_member_only_root_abort",
    )
    .unwrap();

    assert_eq!(response.state, crate::MergeOperationState::Aborted);
    assert_eq!(backend.head(temp.path()).unwrap().commit, root_before);
    let evidence = response
        .preservation
        .unwrap()
        .into_iter()
        .find(|evidence| evidence.target_id == "@root")
        .unwrap();
    assert_eq!(evidence.path, ".");
    assert_eq!(evidence.backup_commit.as_deref(), Some(extra.as_str()));
    assert!(evidence.backup_ref.unwrap().ends_with("/root/head"));
    let stash_id = evidence.stash_id.unwrap();
    let stash_object_id = evidence.stash_object_id.unwrap();
    backend
        .stash_drop(
            temp.path(),
            &crate::git::GitStashTarget {
                object_id: Some(stash_object_id),
                gwz_message_prefix: None,
            },
        )
        .unwrap();
    let listed = handle_stash(
        &backend,
        temp.path(),
        crate::StashRequest {
            meta: request_meta(),
            op: crate::StashOp::List,
            stash_id: None,
            message: None,
            include_untracked: None,
            include_ignored: None,
            expanded: None,
            preserve_index: None,
        },
        "op_preserve_root_list",
    )
    .unwrap();
    let listed_bundle = listed
        .bundles
        .unwrap()
        .into_iter()
        .find(|bundle| bundle.stash_id == stash_id)
        .unwrap();
    assert!(
        listed_bundle
            .drift
            .iter()
            .any(|drift| { drift.member_id == "@root" && drift.code == "missing_native_stash" })
    );
    let error = handle_stash(
        &backend,
        temp.path(),
        crate::StashRequest {
            meta: request_meta(),
            op: crate::StashOp::Apply,
            stash_id: Some(stash_id),
            message: None,
            include_untracked: None,
            include_ignored: None,
            expanded: None,
            preserve_index: None,
        },
        "op_preserve_root_missing",
    )
    .unwrap_err();
    assert_eq!(error.member_id.as_deref(), Some("@root"));
    assert_eq!(error.member_path.as_deref(), Some("."));
}

#[test]
fn pre_evidence_root_drift_is_rejected_without_stashing_candidate_metadata() {
    let temp = TempDir::new("merge-preserve-pre-evidence-root");
    let backend = crate::git::Git2Backend::new();
    let _fixture =
        init_one_member_workspace(temp.path(), &backend, "merge-preserve-pre-evidence-root");
    backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
    commit_file(temp.path(), "root.txt", "baseline\n", "root baseline", &[]).unwrap();
    feature_commit(&backend, temp.path(), "root-feature.txt", "root feature\n");
    let mut start = request(false);
    start.meta.selection = Some(crate::Selection {
        targets: vec!["@root".to_owned()],
        ..Default::default()
    });
    let store = FaultingMergeStore::new(FinalizationFault::AfterCandidatePersistence);
    invoke_with_store(
        &backend,
        &store,
        temp.path(),
        start,
        "op_preserve_pre_evidence_root",
    )
    .unwrap_err();
    let record = store.discover_open(temp.path()).unwrap().unwrap();
    assert!(
        record
            .publication
            .as_ref()
            .unwrap()
            .composition_commit
            .is_none()
    );
    fs::write(temp.path().join("root-untracked.txt"), "do not lose\n").unwrap();
    let mut abort = recovery_request(crate::MergeOp::Abort, Some(record.merge_id.clone()));
    abort.preserve = Some(true);

    let error = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        abort,
        "op_preserve_pre_evidence_root_abort",
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert_eq!(error.member_id.as_deref(), Some("@root"));
    assert!(
        backend
            .stash_list(temp.path())
            .unwrap()
            .iter()
            .all(|stash| !stash
                .message
                .contains(&format!("gwz:stash_{}:", record.merge_id)))
    );
}

#[test]
fn preserve_retry_repairs_interrupted_root_publication_normalization() {
    let temp = TempDir::new("merge-preserve-root-normalization-retry");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_one_member_workspace(
        temp.path(),
        &backend,
        "merge-preserve-root-normalization-retry",
    );
    backend.stage_paths(temp.path(), &["gwz.conf"]).unwrap();
    let root_before =
        commit_file(temp.path(), "root.txt", "baseline\n", "root baseline", &[]).unwrap();
    feature_commit(
        &backend,
        &temp.path().join("remote"),
        "README.md",
        "source\n",
    );
    let store = FaultingMergeStore::new(FinalizationFault::AfterEvidencePersistence);
    invoke_with_store(
        &backend,
        &store,
        temp.path(),
        request(false),
        "op_preserve_root_normalization",
    )
    .unwrap_err();
    let mut record = store.discover_open(temp.path()).unwrap().unwrap();
    fs::write(temp.path().join("root-untracked.txt"), "preserve me\n").unwrap();
    record.state = OperationState::Preserving;
    let publication = record.publication.as_mut().unwrap();
    publication.preservation_prefix = Some("baseline".to_owned());
    let candidate = publication.candidate.as_ref().unwrap().clone();
    store.write_open(temp.path(), &record).unwrap();
    crate::artifact::write_atomic(
        &temp.path().join(crate::artifact::LOCK_PATH),
        &candidate.lock_yaml,
    )
    .unwrap();
    let marker_path = format!(
        "{}/{}.yaml",
        crate::artifact::MARKER_DIR,
        candidate.marker_id
    );
    crate::artifact::write_atomic(&temp.path().join(&marker_path), &candidate.marker_yaml).unwrap();
    backend
        .stage_paths(
            temp.path(),
            &[crate::artifact::LOCK_PATH, marker_path.as_str()],
        )
        .unwrap();
    let mut abort = recovery_request(crate::MergeOp::Abort, Some(record.merge_id));
    abort.preserve = Some(true);

    let response = invoke_with_store(
        &backend,
        &store,
        temp.path(),
        abort,
        "op_preserve_root_normalization_retry",
    )
    .unwrap();

    assert_eq!(response.state, crate::MergeOperationState::Aborted);
    assert_eq!(
        backend.head(temp.path()).unwrap().commit.as_deref(),
        Some(root_before.as_str())
    );
    assert!(
        response
            .preservation
            .unwrap()
            .iter()
            .any(|evidence| evidence.target_id == "@root" && evidence.stash_object_id.is_some())
    );
}

#[test]
fn preserve_abort_rejects_pending_conflict_after_continue_reconciliation() {
    let temp = TempDir::new("merge-preserve-pending-conflict");
    let backend = crate::git::Git2Backend::new();
    let _fixture = init_mixed_merge_workspace(temp.path(), &backend);
    let started = handle_merge(
        &backend,
        temp.path(),
        request(false),
        "op_preserve_pending_conflict",
    )
    .unwrap();
    let mut record = FileMergeStore.discover_open(temp.path()).unwrap().unwrap();
    let docs = record.participants.get_mut("mem_docs").unwrap();
    docs.state = ParticipantState::Planned;
    docs.expected_merge_head = None;
    docs.conflict_paths.clear();
    docs.conflict_snapshot.clear();
    docs.pending_action = Some(PendingMergeAction {
        kind: PendingMergeActionKind::TrueMerge,
        target_branch: docs.target_branch.clone(),
        before_commit: docs.before_commit.clone(),
        source_commit: docs.source_commit.clone(),
        commit_message: docs.commit_message.clone(),
        expected_result: Some(PendingMergeExpectedResult::ExpectedConflict),
        commit_spec: None,
        extensions: BTreeMap::new(),
    });
    FileMergeStore.write_open(temp.path(), &record).unwrap();
    fs::write(
        temp.path().join("docs/README.md"),
        "edited after pending conflict\n",
    )
    .unwrap();
    let merge_id = started.merge_id.unwrap();
    let continue_request = recovery_request(crate::MergeOp::Resume, Some(merge_id.clone()));
    let continue_error = handle_merge(
        &backend,
        temp.path(),
        continue_request,
        "op_preserve_pending_conflict_continue",
    )
    .unwrap_err();
    assert_eq!(continue_error.code, ErrorCode::MergeDrift);
    let reconciled = FileMergeStore.discover_open(temp.path()).unwrap().unwrap();
    let docs = &reconciled.participants["mem_docs"];
    assert_eq!(docs.state, ParticipantState::Conflicted);
    assert!(docs.pending_action.is_none());
    assert!(docs.conflict_snapshot.is_empty());

    let mut abort = recovery_request(crate::MergeOp::Abort, Some(merge_id));
    abort.preserve = Some(true);

    let error = handle_merge(
        &backend,
        temp.path(),
        abort,
        "op_preserve_pending_conflict_abort",
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert_eq!(error.member_id.as_deref(), Some("mem_docs"));
    assert_eq!(
        fs::read_to_string(temp.path().join("docs/README.md")).unwrap(),
        "edited after pending conflict\n"
    );
    let open = FileMergeStore.discover_open(temp.path()).unwrap().unwrap();
    assert_eq!(
        open.participants["mem_docs"].state,
        ParticipantState::Conflicted
    );
    assert!(open.participants["mem_docs"].conflict_snapshot.is_empty());
}

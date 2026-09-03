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
            open_record(temp.path())
                .unwrap()
                .view()
                .participants()
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
    force_open_merge_state(temp.path(), OperationState::Executing);
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
    patch_open_record(temp.path(), |value| {
        let docs = &mut value["participants"]["mem_docs"];
        let pending = serde_yaml::to_value(PendingMergeAction {
            kind: PendingMergeActionKind::TrueMerge,
            target_branch: docs["target_branch"].as_str().unwrap().to_owned(),
            before_commit: docs["before_commit"].as_str().unwrap().to_owned(),
            source_commit: docs["source_commit"].as_str().unwrap().to_owned(),
            commit_message: docs["commit_message"].as_str().unwrap().to_owned(),
            expected_result: Some(PendingMergeExpectedResult::ExpectedConflict),
            commit_spec: None,
            extensions: BTreeMap::new(),
        })
        .unwrap();
        docs["state"] = serde_yaml::to_value(ParticipantState::Planned).unwrap();
        if let serde_yaml::Value::Mapping(map) = docs {
            map.remove(serde_yaml::Value::String("expected_merge_head".to_owned()));
            map.remove(serde_yaml::Value::String("conflict_paths".to_owned()));
            map.remove(serde_yaml::Value::String("conflict_snapshot".to_owned()));
        }
        docs["pending_action"] = pending;
    });
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
    let reconciled = open_record(temp.path()).unwrap();
    let reconciled = reconciled.view();
    let docs = &reconciled.participants()["mem_docs"];
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
    let open = open_record(temp.path()).unwrap();
    let open = open.view();
    assert_eq!(
        open.participants()["mem_docs"].state,
        ParticipantState::Conflicted
    );
    assert!(open.participants()["mem_docs"].conflict_snapshot.is_empty());
}

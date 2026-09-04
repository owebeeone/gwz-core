use super::*;
use crate::workspace_ops::merge::ConflictFileEvidence;
use crate::workspace_ops::merge::model::v1::ParticipantRollbackKindV1;
use crate::workspace_ops::merge::v1_rollback::{
    V1ParticipantRollbackObservation as O, execute_v1_participant_rollback,
    observe_v1_participant_rollback,
};

#[test]
fn native_conflict_abort_is_exact_and_preserves_the_before_checkout() {
    let root = TempDir::new("v1-rollback-conflict");
    let backend = Git2Backend::new();
    let member = root.path.join("members/a");
    backend.create_repo(&member).unwrap();
    let base = commit_file(&member, "README.md", "base\n", "base", &[]).unwrap();
    backend.branch_create(&member, "feature", &base).unwrap();
    backend.switch_branch(&member, "feature").unwrap();
    let source = commit_file(
        &member,
        "README.md",
        "source\n",
        "source",
        &[base.parse().unwrap()],
    )
    .unwrap();
    backend.switch_branch(&member, "main").unwrap();
    let before = commit_file(
        &member,
        "README.md",
        "target\n",
        "target",
        &[base.parse().unwrap()],
    )
    .unwrap();
    let result = backend
        .merge_upstream_checked(&member, "main", &before, &source, "merge", None)
        .unwrap();
    assert!(result.commit.is_none());
    let snapshot = backend
        .merge_conflict_snapshot(&member, &before, &source)
        .unwrap();
    let mut model = crate::workspace_ops::merge::model::v1::test_record();
    model.state = OperationState::RollingBack;
    {
        let row = model.participants.get_mut("mem_a").unwrap();
        row.path = "members/a".into();
        row.target_kind = MergeTargetKind::Member;
        row.target_branch = "main".into();
        row.before_commit = before.clone();
        row.source_commit = source.clone();
        row.state = ParticipantState::Conflicted;
        row.resulting_commit = None;
        row.expected_merge_head = Some(source);
        row.conflict_paths = result.conflicts;
        row.conflict_snapshot = snapshot
            .files
            .into_iter()
            .map(|file| ConflictFileEvidence {
                path: file.path,
                sha256: file.sha256,
            })
            .collect();
    }
    let row = &model.participants["mem_a"];
    assert_eq!(
        observe_v1_participant_rollback(
            &backend,
            &root.path,
            &model,
            "mem_a",
            row,
            ParticipantRollbackKindV1::AbortConflict,
        )
        .unwrap(),
        O::Before
    );
    execute_v1_participant_rollback(
        &backend,
        &root.path,
        &model,
        "mem_a",
        row,
        ParticipantRollbackKindV1::AbortConflict,
    )
    .unwrap();
    assert_eq!(
        observe_v1_participant_rollback(
            &backend,
            &root.path,
            &model,
            "mem_a",
            row,
            ParticipantRollbackKindV1::AbortConflict,
        )
        .unwrap(),
        O::After
    );
    assert_eq!(
        backend.head(&member).unwrap().commit.as_deref(),
        Some(before.as_str())
    );
    assert!(backend.merge_state(&member).unwrap().is_none());
    std::fs::write(member.join("untracked"), "drift\n").unwrap();
    assert_eq!(
        observe_v1_participant_rollback(
            &backend,
            &root.path,
            &model,
            "mem_a",
            row,
            ParticipantRollbackKindV1::AbortConflict,
        )
        .unwrap(),
        O::Ambiguous
    );
}

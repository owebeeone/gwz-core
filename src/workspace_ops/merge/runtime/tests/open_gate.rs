use super::super::*;
use crate::model::ErrorCode;
use crate::workspace_ops::tests::TempDir;
use std::fs;

#[test]
fn workspace_gate_discovers_open_record_and_blocks_only_disallowed_rows() {
    let root = TempDir::new("merge-open-gate");
    let directory = root.path().join(".gwz/merge");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("merge_1.yaml"),
        r#"schema: gwz.merge-operation/v0
record_schema_version: 0
writer_version: test
workspace_id: ws_test
merge_id: merge_1
operation_id: op_1
state: awaiting_resolution
source_ref: feature/x
created_at: now
baseline: { lock_sha256: lock, manifest_sha256: manifest }
selected_targets: []
participants: {}
"#,
    )
    .unwrap();
    let workspace = crate::WorkspaceRef {
        root: Some(root.path().to_string_lossy().into_owned()),
        workspace_id: None,
    };

    let error = enforce_workspace_open_merge_gate(
        root.path(),
        Some(&workspace),
        crate::operation::OpenMergeCommand::Push,
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::OpenOperation);
    assert!(error.message.contains("merge_1"));
    assert!(
        enforce_workspace_open_merge_gate(
            root.path(),
            Some(&workspace),
            crate::operation::OpenMergeCommand::Status,
        )
        .is_ok()
    );
}

#[test]
fn conditional_stage_accepts_a_recorded_conflicted_root_only() {
    let root = TempDir::new("merge-root-stage-gate");
    let directory = root.path().join(".gwz/merge");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("merge_root.yaml"),
        r#"schema: gwz.merge-operation/v0
record_schema_version: 0
writer_version: test
workspace_id: ws_test
merge_id: merge_root
operation_id: op_1
state: awaiting_resolution
source_ref: feature/x
created_at: now
baseline: { lock_sha256: lock, manifest_sha256: manifest }
selected_targets: ['@root']
participants:
  '@root':
    path: .
    target_kind: root
    target_branch: main
    before_commit: before
    source_commit: source
    commit_message: merge
    state: conflicted
    expected_merge_head: source
    conflict_paths: [gwz.conf/gwz.yml]
"#,
    )
    .unwrap();

    let root_target = crate::workspace_ops::StageTarget {
        member_path: None,
        pathspecs: vec!["gwz.conf/gwz.yml".to_owned()],
        explicit: true,
    };
    assert!(enforce_open_merge_stage_targets(root.path(), &[root_target]).is_ok());

    let member_target = crate::workspace_ops::StageTarget {
        member_path: Some("repos/app".to_owned()),
        pathspecs: vec!["README.md".to_owned()],
        explicit: true,
    };
    assert_eq!(
        enforce_open_merge_stage_targets(root.path(), &[member_target])
            .unwrap_err()
            .code,
        ErrorCode::OpenOperation
    );
}

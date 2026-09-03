use super::super::*;
use super::write_open_v1_record;
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

/// **The v1 twin of the row above — the coverage hole that let the defect
/// ship.** Both gate tests used a `gwz.merge-operation/v0` fixture, so nothing
/// exercised the gate against the record A1's writer floor actually creates
/// for `--no-ff`. Discovery ran through the v0 store's v0-only decoder, whose
/// version error propagated BEFORE `enforce_open_merge_gate` was called, so a
/// conflicted no-ff merge answered every blocked command with
/// `UnsupportedRecordVersion` and the user never saw the remedy.
///
/// Every `Block` row must now name the open merge and the three recovery
/// verbs; `Allow` and `NotGated` rows must be untouched.
#[test]
fn workspace_gate_blocks_every_disallowed_row_against_an_open_v1_record() {
    let root = TempDir::new("merge-open-gate-v1");
    let merge_id = write_open_v1_record(root.path());
    let workspace = crate::WorkspaceRef {
        root: Some(root.path().to_string_lossy().into_owned()),
        workspace_id: None,
    };

    for blocked in [
        crate::operation::OpenMergeCommand::BranchMutate,
        crate::operation::OpenMergeCommand::Capture,
        crate::operation::OpenMergeCommand::Commit,
        crate::operation::OpenMergeCommand::Forall,
        crate::operation::OpenMergeCommand::InitUpdate,
        crate::operation::OpenMergeCommand::Materialize,
        crate::operation::OpenMergeCommand::Pull,
        crate::operation::OpenMergeCommand::Push,
        crate::operation::OpenMergeCommand::RepoMutate,
        crate::operation::OpenMergeCommand::Snapshot,
        crate::operation::OpenMergeCommand::StashMutate,
        crate::operation::OpenMergeCommand::TagMutate,
        crate::operation::OpenMergeCommand::MergeStart,
    ] {
        let error =
            enforce_workspace_open_merge_gate(root.path(), Some(&workspace), blocked).unwrap_err();
        assert_eq!(error.code, ErrorCode::OpenOperation, "{blocked:?}");
        for named in [
            merge_id.as_str(),
            "is open",
            "merge status",
            "merge continue",
            "merge abort",
        ] {
            assert!(
                error.message.contains(named),
                "{blocked:?}: {}",
                error.message
            );
        }
    }

    for allowed in [
        crate::operation::OpenMergeCommand::Status,
        crate::operation::OpenMergeCommand::Diff,
        crate::operation::OpenMergeCommand::MergeStatus,
        crate::operation::OpenMergeCommand::MergeRecovery,
        crate::operation::OpenMergeCommand::MergeGc,
    ] {
        assert!(
            enforce_workspace_open_merge_gate(root.path(), Some(&workspace), allowed).is_ok(),
            "{allowed:?}"
        );
    }

    // The `NotGated` early return keeps its position: it answers before any
    // discovery at all, so it cannot be reached by the record's version.
    for not_gated in [
        crate::operation::OpenMergeCommand::CloneWorkspace,
        crate::operation::OpenMergeCommand::InitNewWorkspace,
    ] {
        assert!(
            enforce_workspace_open_merge_gate(root.path(), Some(&workspace), not_gated).is_ok(),
            "{not_gated:?}"
        );
    }
}

/// The same v1 record found by the ancestor walk, with no explicit workspace
/// root — the branch that reads `discover_open_envelope_before_manifest`.
#[test]
fn workspace_gate_finds_an_open_v1_record_before_the_manifest_is_parsed() {
    let root = TempDir::new("merge-open-gate-v1-ancestor");
    let merge_id = write_open_v1_record(root.path());
    let nested = root.path().join("members/a/src");
    fs::create_dir_all(&nested).unwrap();

    let error = enforce_workspace_open_merge_gate(
        &nested,
        None,
        crate::operation::OpenMergeCommand::Commit,
    )
    .unwrap_err();

    assert_eq!(error.code, ErrorCode::OpenOperation);
    assert!(error.message.contains(&merge_id), "{}", error.message);
}

/// `add`'s conditional scope check against an open **v1** record: the
/// conflicted participant's own path is admitted, anything else is refused
/// with the open-merge code — exactly the v0 row above.
#[test]
fn conditional_stage_accepts_a_recorded_conflicted_v1_member_only() {
    let root = TempDir::new("merge-stage-gate-v1");
    let merge_id = write_open_v1_record(root.path());

    let member_target = crate::workspace_ops::StageTarget {
        member_path: Some("members/a".to_owned()),
        pathspecs: vec!["README.md".to_owned()],
        explicit: true,
    };
    assert!(enforce_open_merge_stage_targets(root.path(), &[member_target]).is_ok());

    for outside in [
        crate::workspace_ops::StageTarget {
            member_path: None,
            pathspecs: vec!["gwz.conf/gwz.yml".to_owned()],
            explicit: true,
        },
        crate::workspace_ops::StageTarget {
            member_path: Some("members/b".to_owned()),
            pathspecs: vec!["README.md".to_owned()],
            explicit: true,
        },
    ] {
        let error = enforce_open_merge_stage_targets(root.path(), &[outside]).unwrap_err();
        assert_eq!(error.code, ErrorCode::OpenOperation);
        assert!(error.message.contains(&merge_id), "{}", error.message);
    }
}

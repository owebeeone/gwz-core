use std::fs;

use super::super::store::CheckedV1Store;
use crate::git::Git2Backend;
use crate::model::ErrorCode;
use crate::operation::{ActionKind, OperationContext};
use crate::workspace_ops::merge::OperationState;
use crate::workspace_ops::merge::model::v1::{
    RecoveryContextV1, RecoveryOriginStateV1, test_record,
};
use crate::workspace_ops::tests::TempDir;

#[test]
fn open_status_retries_the_complete_snapshot_after_lineage_changes() {
    let root = TempDir::new_git("merge-v1-status-lineage");
    let path = root.path.join(".gwz/merge/merge_1.yaml");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    write(&path, &test_record());
    let store = CheckedV1Store::default();
    let context = context();
    let mut calls = 0;

    let response = super::optimistic_open_status(&store, &root.path, "merge_1", |current| {
        calls += 1;
        let response = current.record().to_v1_response(&context)?;
        if calls == 1 {
            let mut changed = current.record().clone();
            changed.source_ref = "topic-after-contention".to_owned();
            write(&path, &changed);
        }
        Ok(response)
    })
    .unwrap();

    assert_eq!(calls, 2);
    assert_eq!(response.repos[0].source_ref, "topic-after-contention");
    assert_eq!(
        response.record.unwrap().source_version,
        crate::MergeRecordVersion::V1
    );
}


#[test]
fn open_status_is_byte_exact_and_projects_read_only_live_facts() {
    let root = TempDir::new_git("merge-v1-status-read-only");
    let path = root.path.join(".gwz/merge/merge_1.yaml");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    write(&path, &test_record());
    let before = fs::read(&path).unwrap();

    let response = super::open_status(
        &Git2Backend::new(),
        &CheckedV1Store::default(),
        &root.path,
        "merge_1",
        &context(),
    )
    .unwrap();

    assert_eq!(fs::read(path).unwrap(), before);
    assert_eq!(
        response.record.unwrap().source_version,
        crate::MergeRecordVersion::V1
    );
    assert_eq!(response.repos.len(), 1);
    assert!(!response.repos[0].drift.is_empty());
}

#[test]
fn open_status_rejects_a_second_lineage_change_without_mixing_snapshots() {
    let root = TempDir::new_git("merge-v1-status-repeated-contention");
    let path = root.path.join(".gwz/merge/merge_1.yaml");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    write(&path, &test_record());
    let mut calls = 0;

    let error = super::optimistic_open_status(
        &CheckedV1Store::default(),
        &root.path,
        "merge_1",
        |current| {
            calls += 1;
            let response = current.record().to_v1_response(&context())?;
            let mut changed = current.record().clone();
            changed.source_ref = format!("topic-contention-{calls}");
            write(&path, &changed);
            Ok(response)
        },
    )
    .unwrap_err();

    assert_eq!(calls, 2);
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
}

#[test]
fn open_status_normalizes_source_disappearance_to_typed_contention() {
    let root = TempDir::new_git("merge-v1-status-disappearance");
    let path = root.path.join(".gwz/merge/merge_1.yaml");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    write(&path, &test_record());
    let mut calls = 0;

    let error = super::optimistic_open_status(
        &CheckedV1Store::default(),
        &root.path,
        "merge_1",
        |current| {
            calls += 1;
            let response = current.record().to_v1_response(&context())?;
            fs::remove_file(&path).unwrap();
            Ok(response)
        },
    )
    .unwrap_err();

    assert_eq!(calls, 1);
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
}

#[test]
fn open_status_retries_after_an_archived_copy_appears() {
    use crate::workspace_ops::merge::model::v1::RecordVersion;
    use crate::workspace_ops::merge::record_wire::archived_fixture_for_test;

    let root = TempDir::new_git("merge-v1-status-archive-appearance");
    let (bytes, merge_id) = archived_fixture_for_test(RecordVersion::V1);
    let open = root.path.join(format!(".gwz/merge/{merge_id}.yaml"));
    let archived = root.path.join(format!(".gwz/merge/done/{merge_id}.yaml"));
    fs::create_dir_all(open.parent().unwrap()).unwrap();
    fs::write(&open, &bytes).unwrap();
    let mut calls = 0;

    let response = super::optimistic_open_status(
        &CheckedV1Store::default(),
        &root.path,
        merge_id,
        |current| {
            calls += 1;
            let response = current.record().to_v1_response(&context())?;
            if calls == 1 {
                fs::create_dir_all(archived.parent().unwrap()).unwrap();
                fs::write(&archived, &bytes).unwrap();
            }
            Ok(response)
        },
    )
    .unwrap();

    assert_eq!(calls, 2);
    assert_eq!(response.merge_id.as_deref(), Some(merge_id));
}

#[test]
fn open_status_retries_after_byte_identical_leaf_replacement() {
    let root = TempDir::new_git("merge-v1-status-leaf-replacement");
    let path = root.path.join(".gwz/merge/merge_1.yaml");
    let replacement = root.path.join(".gwz/merge/replacement.yaml");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    write(&path, &test_record());
    let exact_bytes = fs::read(&path).unwrap();
    let mut calls = 0;

    let response = super::optimistic_open_status(
        &CheckedV1Store::default(),
        &root.path,
        "merge_1",
        |current| {
            calls += 1;
            let response = current.record().to_v1_response(&context())?;
            if calls == 1 {
                fs::write(&replacement, &exact_bytes).unwrap();
                fs::remove_file(&path).unwrap();
                fs::rename(&replacement, &path).unwrap();
            }
            Ok(response)
        },
    )
    .unwrap();

    assert_eq!(calls, 2);
    assert_eq!(response.merge_id.as_deref(), Some("merge_1"));
}

#[test]
fn recovery_projection_maps_every_literal_origin_to_the_frozen_resume_action() {
    use crate::MergeCompatibilityNextAction as Action;
    let cases = [
        (
            RecoveryOriginStateV1::Executing,
            Action::ExecuteNextParticipant,
        ),
        (
            RecoveryOriginStateV1::AwaitingResolution,
            Action::AwaitResolution,
        ),
        (
            RecoveryOriginStateV1::Halted,
            Action::ExecuteNextParticipant,
        ),
        (RecoveryOriginStateV1::Finalizing, Action::PersistAcceptance),
        (
            RecoveryOriginStateV1::Preserving,
            Action::ResumePreservation,
        ),
        (RecoveryOriginStateV1::RollingBack, Action::ResumeRollback),
    ];
    for (origin_state, expected) in cases {
        let mut record = test_record();
        record.state = OperationState::RecoveryRequired;
        record.recovery_context = Some(RecoveryContextV1 { origin_state });
        let recovery = crate::workspace_ops::merge::model::project_open_v1(&record)
            .recovery
            .unwrap();
        assert_eq!(recovery.next_action, Action::ReportRecoveryRequired);
        assert_eq!(recovery.resume_action, expected);
        assert_eq!(
            recovery.base_phase,
            crate::MergeCompatibilityBasePhase::PreAcceptance
        );
    }
}

#[test]
fn executing_recovery_reports_pending_participant_reconciliation_first() {
    use crate::workspace_ops::merge::{PendingMergeAction, PendingMergeActionKind};
    let mut record = test_record();
    record.state = OperationState::RecoveryRequired;
    record.recovery_context = Some(RecoveryContextV1 {
        origin_state: RecoveryOriginStateV1::Executing,
    });
    let row = record.participants.get_mut("mem_a").unwrap();
    row.pending_action = Some(PendingMergeAction {
        kind: PendingMergeActionKind::FastForward,
        target_branch: row.target_branch.clone(),
        before_commit: row.before_commit.clone(),
        source_commit: row.source_commit.clone(),
        commit_message: row.commit_message.clone(),
        expected_result: None,
        commit_spec: None,
        extensions: Default::default(),
    });

    let recovery = crate::workspace_ops::merge::model::project_open_v1(&record)
        .recovery
        .unwrap();
    assert_eq!(
        recovery.resume_action,
        crate::MergeCompatibilityNextAction::ReconcilePendingParticipant
    );
}

#[test]
fn open_and_archived_v1_share_one_lossless_acceptance_projection() {
    use crate::workspace_ops::merge::model::v1::RecordVersion;
    use crate::workspace_ops::merge::record_wire::{
        archived_fixture_for_test, decode_archived, decode_production_v1,
    };
    let (bytes, merge_id) = archived_fixture_for_test(RecordVersion::V1);
    let open = decode_production_v1(&bytes).unwrap();
    let archived = decode_archived(&bytes, merge_id).unwrap();

    let open_projection = crate::workspace_ops::merge::model::project_open_v1(&open.record);
    let archived_projection =
        crate::workspace_ops::merge::model::project_archived(archived.projection());

    assert_eq!(open_projection.acceptance, archived_projection.acceptance);
    assert!(!open_projection.archived);
    assert!(archived_projection.archived);
    assert_eq!(
        archived_projection.terminal_outcome,
        Some(crate::MergeTerminalOutcome::Completed)
    );
}

#[test]
fn archived_response_exposes_terminal_record_without_fabricating_live_state() {
    use crate::workspace_ops::merge::model::v1::RecordVersion;
    use crate::workspace_ops::merge::record_wire::{archived_fixture_for_test, decode_archived};

    let (bytes, merge_id) = archived_fixture_for_test(RecordVersion::V1);
    let archived = decode_archived(&bytes, merge_id).unwrap();
    let response = crate::workspace_ops::merge::response::archived_status_response(
        merge_id,
        archived.projection(),
        &context(),
    )
    .unwrap();

    assert_eq!(response.merge_id.as_deref(), Some(merge_id));
    assert_eq!(response.state, crate::MergeOperationState::Completed);
    assert!(!response.open);
    assert_eq!(response.participant_counts, Default::default());
    assert!(response.repos.is_empty());
    assert!(response.operation_drift.is_empty());
    assert!(response.preservation.is_none());
    assert!(response.publication_step.is_none());
    let projection = response.record.unwrap();
    assert_eq!(projection.source_version, crate::MergeRecordVersion::V1);
    assert!(projection.archived);
    assert_eq!(
        projection.terminal_outcome,
        Some(crate::MergeTerminalOutcome::Completed)
    );
    assert_eq!(
        projection.acceptance.unwrap().kind,
        crate::MergeAcceptanceKind::SupportedPersisted
    );
    assert!(projection.recovery.is_none());
}

#[test]
fn archived_projection_overlay_preserves_gc_summary_and_rejects_mismatches() {
    use crate::workspace_ops::merge::model::v1::RecordVersion;
    use crate::workspace_ops::merge::record_wire::{archived_fixture_for_test, decode_archived};

    let (bytes, merge_id) = archived_fixture_for_test(RecordVersion::V1);
    let archived = decode_archived(&bytes, merge_id).unwrap();
    let mut summary = crate::workspace_ops::merge::response::archived_status_response(
        merge_id,
        archived.projection(),
        &context(),
    )
    .unwrap();
    summary.record = None;
    summary.preservation = Some(vec![crate::MergePreservation {
        target_id: "mem_a".to_owned(),
        path: "members/a".to_owned(),
        backup_ref: None,
        backup_commit: None,
        stash_id: Some("stash_1".to_owned()),
        stash_object_id: Some("a".repeat(40)),
    }]);

    let overlaid = crate::workspace_ops::merge::response::attach_archived_record_projection(
        summary.clone(),
        merge_id,
        archived.projection(),
    )
    .unwrap();
    assert_eq!(overlaid.preservation, summary.preservation);
    assert!(overlaid.record.unwrap().archived);

    for mismatched in [
        crate::MergeResponse {
            merge_id: Some("merge_other".to_owned()),
            ..summary.clone()
        },
        crate::MergeResponse {
            state: crate::MergeOperationState::Aborted,
            ..summary.clone()
        },
        crate::MergeResponse {
            open: true,
            ..summary.clone()
        },
    ] {
        assert_eq!(
            crate::workspace_ops::merge::response::attach_archived_record_projection(
                mismatched,
                merge_id,
                archived.projection(),
            )
            .unwrap_err()
            .code,
            ErrorCode::InternalError
        );
    }
}

fn write(
    path: &std::path::Path,
    record: &crate::workspace_ops::merge::model::v1::MergeOperationRecordV1,
) {
    fs::write(path, serde_yaml::to_string(record).unwrap()).unwrap();
}

fn context() -> OperationContext {
    OperationContext {
        operation_id: "op_status".to_owned(),
        request_id: "req_status".to_owned(),
        schema_version: "gwz.protocol/v0".to_owned(),
        action: ActionKind::Merge,
        dry_run: false,
        attribution: None,
    }
}

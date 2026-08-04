use std::collections::BTreeMap;

use super::*;
use crate::model::ErrorCode;
use crate::operation::OperationContext;

#[test]
fn lifecycle_transitions_reject_skips_and_regressions() {
    assert_eq!(
        OperationState::Executing
            .transition(OperationState::AwaitingResolution)
            .unwrap(),
        OperationState::AwaitingResolution
    );
    assert_eq!(
        OperationState::Completed
            .transition(OperationState::Executing)
            .unwrap_err()
            .code,
        ErrorCode::MergeRecoveryRequired
    );
    assert_eq!(
        OperationState::RecoveryRequired
            .transition(OperationState::Executing)
            .unwrap(),
        OperationState::Executing
    );
    assert_eq!(
        OperationState::RecoveryRequired
            .transition(OperationState::RollingBack)
            .unwrap(),
        OperationState::RollingBack
    );
    assert_eq!(
        ParticipantState::Conflicted
            .transition(ParticipantState::Continued)
            .unwrap(),
        ParticipantState::Continued
    );
    assert_eq!(
        ParticipantState::Planned
            .transition(ParticipantState::Unattempted)
            .unwrap(),
        ParticipantState::Unattempted
    );
    assert_eq!(
        ParticipantState::Merged
            .transition(ParticipantState::Conflicted)
            .unwrap_err()
            .code,
        ErrorCode::MergeRecoveryRequired
    );
    assert!(
        PublicationStep::PublishingCandidate
            .transition(PublicationStep::PreparingCandidate)
            .is_err()
    );
}

#[test]
fn record_round_trip_retains_unknown_fields() {
    let yaml = r#"schema: gwz.merge-operation/v0
record_schema_version: 0
writer_version: 0.9.2
workspace_id: ws_default
merge_id: merge_1
operation_id: op_1
state: executing
source_ref: feature/x
created_at: now
baseline:
  lock_sha256: lock
  manifest_sha256: manifest
  future_baseline: retained
selected_targets: []
participants: {}
future_record: retained
"#;
    let record: MergeOperationRecord = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(record.mode, MergeExecutionMode::Normal);
    let rewritten = serde_yaml::to_string(&record).unwrap();
    assert!(!rewritten.contains("\nmode:"));
    assert!(rewritten.contains("future_record: retained"));
    assert!(rewritten.contains("future_baseline: retained"));

    let mut ff_only = record;
    ff_only.mode = MergeExecutionMode::FfOnly;
    let encoded = serde_yaml::to_string(&ff_only).unwrap();
    assert!(encoded.contains("mode: ff_only"));
    assert_eq!(
        serde_yaml::from_str::<MergeOperationRecord>(&encoded)
            .unwrap()
            .mode,
        MergeExecutionMode::FfOnly
    );
}

#[test]
fn pending_action_round_trip_freezes_exact_action_inputs() {
    let action = PendingMergeAction {
        kind: PendingMergeActionKind::TrueMerge,
        target_branch: "main".to_owned(),
        before_commit: "111".to_owned(),
        source_commit: "222".to_owned(),
        commit_message: "Merge 'feature/x' into 'main'".to_owned(),
        expected_result: Some(PendingMergeExpectedResult::Commit),
        commit_spec: Some(PendingCommitSpec {
            tree_oid: "333".to_owned(),
            author: PendingGitSignature {
                name: "GWZ Author".to_owned(),
                email: "author@example.test".to_owned(),
                time_seconds: 1_700_000_000,
                timezone_offset_minutes: 600,
                extensions: BTreeMap::new(),
            },
            committer: PendingGitSignature {
                name: "GWZ Committer".to_owned(),
                email: "committer@example.test".to_owned(),
                time_seconds: 1_700_000_001,
                timezone_offset_minutes: 600,
                extensions: BTreeMap::new(),
            },
            extensions: BTreeMap::from([(
                "future_commit".to_owned(),
                serde_yaml::Value::String("retained".to_owned()),
            )]),
        }),
        extensions: BTreeMap::from([(
            "future_action".to_owned(),
            serde_yaml::Value::String("retained".to_owned()),
        )]),
    };
    let encoded = serde_yaml::to_string(&action).unwrap();
    let decoded: PendingMergeAction = serde_yaml::from_str(&encoded).unwrap();
    assert_eq!(decoded, action);
    assert!(encoded.contains("future_action: retained"));
    assert!(encoded.contains("future_commit: retained"));
}

#[test]
fn record_conversion_preserves_frozen_order_and_counts() {
    let participant = MergeParticipantRecord {
        path: "repos/core".to_owned(),
        target_kind: MergeTargetKind::Member,
        target_branch: "main".to_owned(),
        before_commit: "111".to_owned(),
        source_commit: "222".to_owned(),
        commit_message: "Merge 'feature/x' into 'main'".to_owned(),
        state: ParticipantState::Conflicted,
        resulting_commit: None,
        expected_merge_head: Some("222".to_owned()),
        conflict_paths: vec!["src/lib.rs".to_owned()],
        conflict_snapshot: Vec::new(),
        error: None,
        pending_action: None,
        preservation: Vec::new(),
        drift: Vec::new(),
        extensions: BTreeMap::new(),
    };
    let failed = MergeParticipantRecord {
        path: "repos/lib".to_owned(),
        target_kind: MergeTargetKind::Member,
        target_branch: "main".to_owned(),
        before_commit: "333".to_owned(),
        source_commit: "444".to_owned(),
        commit_message: "Merge 'feature/x' into 'main'".to_owned(),
        state: ParticipantState::Failed,
        resulting_commit: None,
        expected_merge_head: None,
        conflict_paths: Vec::new(),
        conflict_snapshot: Vec::new(),
        error: Some(MergeRecordError {
            code: ErrorCode::GitCommandFailed,
            message: "revspec 'feature/x' not found".to_owned(),
            detail: Some("source ref was not found".to_owned()),
        }),
        pending_action: None,
        preservation: Vec::new(),
        drift: Vec::new(),
        extensions: BTreeMap::new(),
    };
    let record = MergeOperationRecord {
        schema: MERGE_RECORD_SCHEMA.to_owned(),
        record_schema_version: MERGE_RECORD_SCHEMA_VERSION,
        writer_version: "0.9.2".to_owned(),
        workspace_id: "ws_default".to_owned(),
        merge_id: "merge_1".to_owned(),
        operation_id: "op_1".to_owned(),
        state: OperationState::AwaitingResolution,
        source_ref: "feature/x".to_owned(),
        mode: MergeExecutionMode::Normal,
        created_at: "now".to_owned(),
        baseline: MergeBaseline {
            lock_sha256: "lock".to_owned(),
            manifest_sha256: "manifest".to_owned(),
            lock_yaml: None,
            manifest_yaml: None,
            lock_commit_sha256: None,
            manifest_commit_sha256: None,
            root_head: None,
            root_branch: None,
            extensions: BTreeMap::new(),
        },
        selected_targets: vec!["mem_core".to_owned(), "mem_lib".to_owned()],
        participants: BTreeMap::from([
            ("mem_core".to_owned(), participant),
            ("mem_lib".to_owned(), failed),
        ]),
        publication: None,
        operation_drift: Vec::new(),
        extensions: BTreeMap::new(),
    };
    let context = OperationContext {
        operation_id: "op_1".to_owned(),
        request_id: "req_1".to_owned(),
        schema_version: "gwz.v0".to_owned(),
        action: crate::operation::ActionKind::Merge,
        dry_run: false,
        attribution: None,
    };

    let response = record.to_response(&context).unwrap();
    assert_eq!(response.response.meta.action, crate::ActionKind::Merge);
    assert_eq!(response.participant_counts.total, 2);
    assert_eq!(response.participant_counts.conflicted, 1);
    assert_eq!(response.participant_counts.failed, 1);
    assert_eq!(response.repos[0].target_id, "mem_core");
    assert_eq!(
        response.repos[1].error.as_ref().unwrap().code,
        crate::GwzErrorCode::GitCommandFailed
    );
    assert_eq!(
        response.repos[1].error.as_ref().unwrap().detail.as_deref(),
        Some("source ref was not found")
    );
    assert!(response.open);

    let rewritten = serde_yaml::to_string(&record).unwrap();
    assert!(rewritten.contains("commit_message: Merge 'feature/x' into 'main'"));
    assert!(rewritten.contains("code: git_command_failed"));
    assert_eq!(
        serde_yaml::from_str::<MergeOperationRecord>(&rewritten).unwrap(),
        record
    );

    let snapshot = MergeStatusSnapshot {
        record,
        participants: BTreeMap::from([
            (
                "mem_core".to_owned(),
                MergeParticipantObservation {
                    live_commit: Some("111".to_owned()),
                    conflict_paths: vec!["src/lib.rs".to_owned()],
                    drift: Vec::new(),
                    continue_eligibility: RetryEligibility {
                        eligible: true,
                        blockers: Vec::new(),
                    },
                    abort_eligibility: RollbackEligibility {
                        eligible: true,
                        blockers: Vec::new(),
                    },
                    pending_action: None,
                },
            ),
            (
                "mem_lib".to_owned(),
                MergeParticipantObservation {
                    live_commit: Some("333".to_owned()),
                    conflict_paths: Vec::new(),
                    drift: Vec::new(),
                    continue_eligibility: RetryEligibility {
                        eligible: true,
                        blockers: Vec::new(),
                    },
                    abort_eligibility: RollbackEligibility {
                        eligible: true,
                        blockers: Vec::new(),
                    },
                    pending_action: None,
                },
            ),
        ]),
        operation_drift: vec![OperationDrift {
            kind: OperationDriftKind::BaselineManifestChanged,
            message: "manifest changed".to_owned(),
        }],
    };
    let durable_record = snapshot.record.clone();
    let status = snapshot.to_response(&context).unwrap();
    assert_eq!(snapshot.record, durable_record);
    assert_eq!(status.repos[0].live_commit.as_deref(), Some("111"));
    assert_eq!(status.repos[0].continue_eligible, Some(true));
    assert_eq!(status.repos[0].abort_eligible, Some(true));
    assert_eq!(status.operation_drift.len(), 1);

    let idle = super::super::response::idle_status_response(&context).unwrap();
    assert_eq!(idle.state, crate::MergeOperationState::Idle);
    assert_eq!(
        idle.response.meta.aggregate_status,
        crate::AggregateStatus::Noop
    );
    assert!(idle.merge_id.is_none());
    assert!(!idle.open);
    assert_eq!(idle.participant_counts.total, 0);
    assert!(idle.repos.is_empty());
    assert!(idle.operation_drift.is_empty());

    let mut incomplete = snapshot;
    incomplete.participants.remove("mem_lib");
    assert_eq!(
        incomplete.to_response(&context).unwrap_err().code,
        ErrorCode::InternalError
    );
}

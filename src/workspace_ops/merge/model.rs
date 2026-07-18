#![allow(dead_code)] // Frozen durable lifecycle model; remove as M1-M3 consume each item.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use crate::model::{ErrorCode, ModelError, ModelResult};

pub(crate) const MERGE_RECORD_SCHEMA: &str = "gwz.merge-operation/v0";
pub(crate) const MERGE_RECORD_SCHEMA_VERSION: u32 = 0;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MergeTargetKind {
    Member,
    Root,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ParticipantState {
    Planned,
    UpToDate,
    FastForwarded,
    Merged,
    Conflicted,
    Failed,
    Unattempted,
    Continued,
    Aborted,
    RolledBack,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OperationState {
    Executing,
    AwaitingResolution,
    Halted,
    Finalizing,
    Preserving,
    RollingBack,
    Completed,
    Aborted,
    RecoveryRequired,
}

impl OperationState {
    pub(crate) fn is_open(self) -> bool {
        !matches!(self, Self::Completed | Self::Aborted)
    }

    pub(crate) fn transition(self, next: Self) -> ModelResult<Self> {
        let legal = self == next
            || matches!(
                (self, next),
                (
                    Self::Executing,
                    Self::AwaitingResolution
                        | Self::Halted
                        | Self::Finalizing
                        | Self::RecoveryRequired
                ) | (
                    Self::AwaitingResolution,
                    Self::Executing
                        | Self::Finalizing
                        | Self::Preserving
                        | Self::RollingBack
                        | Self::RecoveryRequired
                ) | (
                    Self::Halted,
                    Self::Executing | Self::Preserving | Self::RollingBack | Self::RecoveryRequired
                ) | (
                    Self::Finalizing,
                    Self::Completed | Self::Preserving | Self::RollingBack | Self::RecoveryRequired
                ) | (Self::Preserving, Self::RollingBack | Self::RecoveryRequired)
                    | (Self::RollingBack, Self::Aborted | Self::RecoveryRequired)
            );
        legal
            .then_some(next)
            .ok_or_else(|| transition_error("operation", self, next))
    }
}

impl ParticipantState {
    pub(crate) fn transition(self, next: Self) -> ModelResult<Self> {
        let attempted = matches!(
            next,
            Self::UpToDate | Self::FastForwarded | Self::Merged | Self::Conflicted | Self::Failed
        );
        let legal = self == next
            || matches!(self, Self::Planned | Self::Unattempted | Self::Failed) && attempted
            || matches!(
                (self, next),
                (
                    Self::Planned
                        | Self::Unattempted
                        | Self::Failed
                        | Self::UpToDate
                        | Self::Conflicted,
                    Self::Aborted
                ) | (Self::Conflicted, Self::Continued)
                    | (
                        Self::FastForwarded | Self::Merged | Self::Continued,
                        Self::RolledBack
                    )
            );
        legal
            .then_some(next)
            .ok_or_else(|| transition_error("participant", self, next))
    }
}

fn transition_error<T: std::fmt::Debug>(kind: &str, from: T, to: T) -> ModelError {
    ModelError::new(
        ErrorCode::MergeRecoveryRequired,
        format!("illegal merge {kind} transition: {from:?} -> {to:?}"),
    )
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MergePlan {
    pub source_ref: String,
    pub baseline: MergeBaseline,
    pub participants: Vec<MergeParticipantPlan>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MergeParticipantPlan {
    pub target_id: String,
    pub target_kind: MergeTargetKind,
    pub path: String,
    pub target_branch: String,
    pub before_commit: String,
    pub source_commit: String,
    pub analysis: Option<crate::MergeAnalysisKind>,
    pub prediction_complete: bool,
    pub commit_message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct MergeBaseline {
    pub lock_sha256: String,
    pub manifest_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_head: Option<String>,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct MergeOperationRecord {
    pub schema: String,
    pub record_schema_version: u32,
    pub writer_version: String,
    pub workspace_id: String,
    pub merge_id: String,
    pub operation_id: String,
    pub state: OperationState,
    pub source_ref: String,
    pub created_at: String,
    pub baseline: MergeBaseline,
    pub selected_targets: Vec<String>,
    pub participants: BTreeMap<String, MergeParticipantRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publication: Option<PublicationProgress>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operation_drift: Vec<OperationDrift>,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct MergeParticipantRecord {
    pub path: String,
    pub target_kind: MergeTargetKind,
    pub target_branch: String,
    pub before_commit: String,
    pub source_commit: String,
    pub commit_message: String,
    pub state: ParticipantState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resulting_commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_merge_head: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflict_paths: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<MergeRecordError>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preservation: Vec<PreservationEvidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drift: Vec<ParticipantDrift>,
    #[serde(default, flatten)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct MergeRecordError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct PreservationEvidence {
    pub backup_ref: Option<String>,
    pub backup_commit: Option<String>,
    pub stash_id: Option<String>,
    pub stash_object_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct PublicationProgress {
    pub step: PublicationStep,
    pub candidate_lock_sha256: Option<String>,
    pub candidate_marker_path: Option<String>,
    pub root_merge_commit: Option<String>,
    pub composition_commit: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PublicationStep {
    NotStarted,
    ValidatingResults,
    PreparingCandidate,
    CommittingEvidence,
    PublishingCandidate,
    VerifyingPublication,
    Complete,
}

impl PublicationStep {
    pub(crate) fn transition(self, next: Self) -> ModelResult<Self> {
        (next >= self)
            .then_some(next)
            .ok_or_else(|| transition_error("publication", self, next))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ParticipantDrift {
    pub kind: ParticipantDriftKind,
    pub message: String,
    pub expected_branch: Option<String>,
    pub live_branch: Option<String>,
    pub expected_head: Option<String>,
    pub live_head: Option<String>,
    pub expected_merge_head: Option<String>,
    pub live_merge_head: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ParticipantDriftKind {
    BranchChanged,
    HeadAdvanced,
    HeadRewound,
    TargetRefChanged,
    WorktreeModified,
    IndexModified,
    MergeStateMissing,
    MergeHeadChanged,
    NewIntegrationState,
    RepositoryMissing,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct OperationDrift {
    pub kind: OperationDriftKind,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OperationDriftKind {
    BaselineLockChanged,
    BaselineManifestChanged,
    RootCandidateMetadataInvalid,
    RootCandidateStateChanged,
    RecordUnreadable,
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub(crate) struct RetryEligibility {
    pub eligible: bool,
    pub blockers: Vec<ParticipantDriftKind>,
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub(crate) struct RollbackEligibility {
    pub eligible: bool,
    pub blockers: Vec<ParticipantDriftKind>,
}

/// One read-only live observation for a recorded participant. Status computes
/// this without modifying the durable operation record; continue and abort
/// consume the same drift and eligibility classification after the M1 gate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MergeParticipantObservation {
    pub live_commit: Option<String>,
    pub conflict_paths: Vec<String>,
    pub drift: Vec<ParticipantDrift>,
    pub continue_eligibility: RetryEligibility,
    pub abort_eligibility: RollbackEligibility,
}

/// Complete read-only status input. Participant observations are keyed by the
/// durable target ids and must cover every id in `record.selected_targets`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MergeStatusSnapshot {
    pub record: MergeOperationRecord,
    pub participants: BTreeMap<String, MergeParticipantObservation>,
    pub operation_drift: Vec<OperationDrift>,
}

#[cfg(test)]
mod tests {
    use super::*;
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
            ParticipantState::Conflicted
                .transition(ParticipantState::Continued)
                .unwrap(),
            ParticipantState::Continued
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
        let rewritten = serde_yaml::to_string(&record).unwrap();
        assert!(rewritten.contains("future_record: retained"));
        assert!(rewritten.contains("future_baseline: retained"));
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
            error: None,
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
            error: Some(MergeRecordError {
                code: ErrorCode::GitCommandFailed,
                message: "revspec 'feature/x' not found".to_owned(),
                detail: Some("source ref was not found".to_owned()),
            }),
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
            created_at: "now".to_owned(),
            baseline: MergeBaseline {
                lock_sha256: "lock".to_owned(),
                manifest_sha256: "manifest".to_owned(),
                root_head: None,
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
}

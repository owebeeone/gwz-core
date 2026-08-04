use std::collections::BTreeMap;

use super::*;
use crate::workspace_ops::merge::{
    MERGE_RECORD_SCHEMA, MERGE_RECORD_SCHEMA_VERSION, MergeBaseline, MergeExecutionMode,
    MergeOperationRecord, MergeTargetKind, OperationDrift, OperationState, PendingMergeAction,
    PendingMergeActionKind,
};

const STATES: [ParticipantState; 10] = [
    ParticipantState::Planned,
    ParticipantState::UpToDate,
    ParticipantState::FastForwarded,
    ParticipantState::Merged,
    ParticipantState::Conflicted,
    ParticipantState::Failed,
    ParticipantState::Unattempted,
    ParticipantState::Continued,
    ParticipantState::Aborted,
    ParticipantState::RolledBack,
];

fn participant(state: ParticipantState) -> MergeParticipantRecord {
    MergeParticipantRecord {
        path: "repos/app".to_owned(),
        target_kind: MergeTargetKind::Member,
        target_branch: "main".to_owned(),
        before_commit: "before".to_owned(),
        source_commit: "source".to_owned(),
        commit_message: "merge".to_owned(),
        state,
        resulting_commit: Some("result".to_owned()),
        expected_merge_head: None,
        conflict_paths: Vec::new(),
        conflict_snapshot: Vec::new(),
        error: None,
        pending_action: None,
        preservation: Vec::new(),
        drift: Vec::new(),
        extensions: BTreeMap::new(),
    }
}

fn live_for(participant: &MergeParticipantRecord) -> ParticipantLiveState {
    let head = expected_head(participant).unwrap().to_owned();
    let conflicted = status_policy(participant.state).conflict_role == ConflictRole::NativeMerge;
    ParticipantLiveState {
        branch: Some(participant.target_branch.clone()),
        head: Some(head.clone()),
        target_ref: Some(head),
        status: GitStatus::clean(),
        repository_state: if conflicted {
            GitRepositoryState::Merge
        } else {
            GitRepositoryState::Clean
        },
        merge_state: conflicted.then(|| GitNativeMergeState {
            merge_head: participant.source_commit.clone(),
            conflict_paths: vec!["conflict.txt".to_owned()],
            unresolved_entries: 0,
        }),
        native_detail_error: None,
        missing_objects: Vec::new(),
        head_relation: HeadRelation::Equal,
    }
}

mod projection;
mod recovery;

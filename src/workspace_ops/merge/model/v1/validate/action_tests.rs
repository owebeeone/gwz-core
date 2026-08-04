use std::collections::BTreeMap;

use super::super::super::{
    MergeExecutionMode, ParticipantState, PendingCommitSpec, PendingGitSignature,
    PendingMergeAction, PendingMergeActionKind, PendingMergeExpectedResult,
};
use super::tests::{oid, record};
use super::validate_v1_actions;
use crate::model::ErrorCode;

pub(super) fn pending(
    kind: PendingMergeActionKind,
    result: PendingMergeExpectedResult,
    with_spec: bool,
) -> PendingMergeAction {
    PendingMergeAction {
        kind,
        target_branch: "main".to_owned(),
        before_commit: oid('a'),
        source_commit: oid('b'),
        commit_message: "merge topic\n\nGWZ-Merge-ID: merge_1\nGWZ-Operation-ID: op_1".to_owned(),
        expected_result: Some(result),
        commit_spec: with_spec.then(|| PendingCommitSpec {
            tree_oid: oid('c'),
            author: signature("author"),
            committer: signature("committer"),
            extensions: BTreeMap::new(),
        }),
        extensions: BTreeMap::new(),
    }
}

fn signature(name: &str) -> PendingGitSignature {
    PendingGitSignature {
        name: name.to_owned(),
        email: format!("{name}@example.test"),
        time_seconds: 123,
        timezone_offset_minutes: 600,
        extensions: BTreeMap::new(),
    }
}

#[test]
fn v1_action_matrix_accepts_exact_normal_and_no_ff_commit_intents() {
    let cases = [
        (
            MergeExecutionMode::Normal,
            PendingMergeActionKind::VerifyUpToDate,
            PendingMergeExpectedResult::Unchanged,
            false,
        ),
        (
            MergeExecutionMode::Normal,
            PendingMergeActionKind::FastForward,
            PendingMergeExpectedResult::FastForward,
            false,
        ),
        (
            MergeExecutionMode::Normal,
            PendingMergeActionKind::TrueMerge,
            PendingMergeExpectedResult::ExpectedConflict,
            false,
        ),
        (
            MergeExecutionMode::NoFf,
            PendingMergeActionKind::TrueMerge,
            PendingMergeExpectedResult::Commit,
            true,
        ),
    ];
    for (mode, kind, result, with_spec) in cases {
        let mut case = record();
        case.mode = mode;
        case.participants.get_mut("mem_a").unwrap().pending_action =
            Some(pending(kind, result, with_spec));
        validate_v1_actions(&case).unwrap();
    }
}

#[test]
fn v1_action_matrix_rejects_no_ff_fast_forward_and_inexact_intent() {
    let mut case = record();
    case.mode = MergeExecutionMode::NoFf;
    case.participants.get_mut("mem_a").unwrap().pending_action = Some(pending(
        PendingMergeActionKind::FastForward,
        PendingMergeExpectedResult::FastForward,
        false,
    ));
    assert_eq!(
        validate_v1_actions(&case).unwrap_err().code,
        ErrorCode::MergeRecordUnreadable
    );

    let pending = case
        .participants
        .get_mut("mem_a")
        .unwrap()
        .pending_action
        .as_mut()
        .unwrap();
    pending.before_commit = oid('f');
    assert!(validate_v1_actions(&case).is_err());
}

#[test]
fn resolve_conflict_requires_conflicted_state_and_exact_commit_spec() {
    let mut case = record();
    let participant = case.participants.get_mut("mem_a").unwrap();
    participant.state = ParticipantState::Conflicted;
    participant.expected_merge_head = Some(oid('b'));
    participant.pending_action = Some(pending(
        PendingMergeActionKind::ResolveConflict,
        PendingMergeExpectedResult::Commit,
        true,
    ));
    validate_v1_actions(&case).unwrap();

    case.participants
        .get_mut("mem_a")
        .unwrap()
        .pending_action
        .as_mut()
        .unwrap()
        .commit_spec
        .as_mut()
        .unwrap()
        .committer
        .timezone_offset_minutes = 1_441;
    assert!(validate_v1_actions(&case).is_err());
}

use std::collections::BTreeMap;

use super::*;

fn participant(state: ParticipantState) -> MergeParticipantRecord {
    serde_yaml::from_str(&format!(
        "path: repos/app\ntarget_kind: member\ntarget_branch: main\nbefore_commit: before\nsource_commit: source\ncommit_message: exact message\nstate: {}\n",
        serde_yaml::to_string(&state).unwrap().trim()
    ))
    .unwrap()
}

fn record(states: &[(&str, ParticipantState)]) -> MergeOperationRecord {
    let mut record: MergeOperationRecord = serde_yaml::from_str(
        r#"{schema: gwz.merge-operation/v0, record_schema_version: 0, writer_version: test, workspace_id: ws_test, merge_id: merge_1, operation_id: op_1, state: executing, source_ref: feature/x, created_at: now, baseline: {lock_sha256: lock, manifest_sha256: manifest}, selected_targets: [], participants: {}}"#,
    )
    .unwrap();
    record.selected_targets = states.iter().map(|(id, _)| (*id).to_owned()).collect();
    record.participants = states
        .iter()
        .map(|(id, state)| ((*id).to_owned(), participant(*state)))
        .collect::<BTreeMap<_, _>>();
    record
}

#[test]
fn failed_retry_can_become_a_new_conflict_without_closing_the_batch() {
    let mut record = record(&[("mem_app", ParticipantState::Failed)]);
    let source = record.participants["mem_app"].clone();
    let outcome = classify_retry(
        &source,
        GitMergeAnalysisKind::TrueMerge,
        GitIntegrateResult {
            commit: None,
            conflicts: vec!["README.md".into()],
        },
    )
    .unwrap();
    apply_outcome(&mut record, "mem_app", outcome, None).unwrap();
    assert_eq!(
        record.participants["mem_app"].state,
        ParticipantState::Conflicted
    );
    assert_eq!(remaining_state(&record), OperationState::AwaitingResolution);
}

#[test]
fn invalid_retry_result_is_rejected_before_the_record_is_changed() {
    let record = record(&[("mem_app", ParticipantState::Unattempted)]);
    let unchanged = record.clone();
    let error = classify_retry(
        &record.participants["mem_app"],
        GitMergeAnalysisKind::FastForward,
        GitIntegrateResult::clean("wrong".into()),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert_eq!(record, unchanged);
}

#[test]
fn retry_intent_freezes_inputs_and_is_cleared_only_with_the_outcome() {
    let mut record = record(&[("mem_app", ParticipantState::Failed)]);
    let action = ContinueAction {
        target_id: "mem_app".to_owned(),
        path: "repos/app".to_owned(),
        kind: ContinueActionKind::Retry(GitMergeAnalysisKind::FastForward),
        prepared: ContinuePrepared::Merge(GitPreparedMerge::FastForward),
        durable: false,
    };

    set_pending_action(&mut record, &action).unwrap();
    let pending = record.participants["mem_app"]
        .pending_action
        .as_ref()
        .unwrap();
    assert_eq!(pending.kind, PendingMergeActionKind::FastForward);
    assert_eq!(pending.before_commit, "before");
    assert_eq!(pending.source_commit, "source");
    assert_eq!(pending.commit_message, "exact message");

    apply_outcome(
        &mut record,
        "mem_app",
        Outcome::clean(ParticipantState::FastForwarded, "source".to_owned()),
        None,
    )
    .unwrap();
    assert!(record.participants["mem_app"].pending_action.is_none());
}

#[test]
fn post_mutation_observation_failure_keeps_exact_pending_retry_state() {
    let mut record = record(&[("mem_app", ParticipantState::Unattempted)]);
    let action = ContinueAction {
        target_id: "mem_app".to_owned(),
        path: "repos/app".to_owned(),
        kind: ContinueActionKind::Retry(GitMergeAnalysisKind::TrueMerge),
        prepared: ContinuePrepared::Merge(GitPreparedMerge::ExpectedConflict),
        durable: false,
    };
    set_pending_action(&mut record, &action).unwrap();
    let before = record.participants["mem_app"].clone();

    apply_recovery_failure(
        &mut record,
        "mem_app",
        &ModelError::new(
            ErrorCode::MergeRecoveryRequired,
            "conflict snapshot unavailable",
        ),
    )
    .unwrap();

    assert_eq!(record.participants["mem_app"], before);
}

#[test]
fn mixed_results_enter_only_the_safe_next_operation_state() {
    let mut record = record(&[
        ("app", ParticipantState::UpToDate),
        ("lib", ParticipantState::Merged),
        ("docs", ParticipantState::Continued),
    ]);
    assert_eq!(remaining_state(&record), OperationState::Finalizing);
    record.participants.get_mut("docs").unwrap().state = ParticipantState::Conflicted;
    assert_eq!(remaining_state(&record), OperationState::AwaitingResolution);
    record.participants.get_mut("lib").unwrap().state = ParticipantState::Failed;
    assert_eq!(remaining_state(&record), OperationState::Halted);
}

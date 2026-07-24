use crate::git::Git2Backend;
use crate::workspace_ops::tests::{TempDir, request_meta};

use super::*;

pub(super) fn resume_request() -> crate::MergeRequest {
    crate::MergeRequest {
        meta: request_meta(),
        op: crate::MergeOp::Resume,
        merge_id: Some("merge_test".to_owned()),
        ..Default::default()
    }
}

#[test]
fn durable_resolution_race_preserves_pending_intent_without_failed_outcome() {
    let root = TempDir::new("merge-pending-resolution-race");
    let backend = Git2Backend::new();
    let plan = single_real_plan(root.path(), &backend, ActionFixture::Conflict);
    let app = root.path().join("app");
    let conflict = backend
        .prepare_merge_upstream_checked(
            &app,
            &plan.participants[0].target_branch,
            &plan.participants[0].before_commit,
            &plan.participants[0].source_commit,
            None,
        )
        .unwrap();
    backend
        .execute_prepared_merge_upstream_checked(
            &app,
            &plan.participants[0].target_branch,
            &plan.participants[0].before_commit,
            &plan.participants[0].source_commit,
            &plan.participants[0].commit_message,
            &conflict,
        )
        .unwrap();
    std::fs::write(app.join("README.md"), "resolution A\n").unwrap();
    backend.stage_paths(&app, &["README.md"]).unwrap();
    let frozen_commit = backend
        .prepare_merge_resolution_checked(
            &app,
            &plan.participants[0].target_branch,
            &plan.participants[0].before_commit,
            &plan.participants[0].source_commit,
            attributed_context().attribution.as_ref(),
        )
        .unwrap();

    let store = MemoryStore::default();
    let mut record = durable_record(root.path(), &plan);
    record.state = OperationState::AwaitingResolution;
    let participant = record.participants.get_mut("mem_app").unwrap();
    participant.state = ParticipantState::Conflicted;
    participant.expected_merge_head = Some(plan.participants[0].source_commit.clone());
    participant.pending_action = Some(PendingMergeAction {
        kind: PendingMergeActionKind::ResolveConflict,
        target_branch: plan.participants[0].target_branch.clone(),
        before_commit: plan.participants[0].before_commit.clone(),
        source_commit: plan.participants[0].source_commit.clone(),
        commit_message: plan.participants[0].commit_message.clone(),
        expected_result: Some(PendingMergeExpectedResult::Commit),
        commit_spec: pending_commit_spec(&GitPreparedMerge::Commit(frozen_commit)),
        extensions: BTreeMap::new(),
    });
    let frozen_pending = participant.pending_action.clone().unwrap();
    store.write_open(root.path(), &record).unwrap();
    let head_before = backend.head(&app).unwrap();
    let native_before = backend.merge_state(&app).unwrap();

    let raced_app = app.clone();
    Git2Backend::before_next_prepared_execution(move || {
        std::fs::write(raced_app.join("README.md"), "resolution B\n").unwrap();
        Git2Backend::new()
            .stage_paths(&raced_app, &["README.md"])
            .unwrap();
    });
    Git2Backend::reset_preparation_call_count();

    let context = context(false);
    let sink = TraceSink(&store);
    let emitter = EventEmitter::new(&context, &sink, 0);
    emitter.operation_started();
    let error = super::super::continue_op::handle_continue(
        &backend,
        &store,
        root.path(),
        &resume_request(),
        &context,
        &emitter,
    )
    .unwrap_err();
    emitter.operation_finished();

    assert_eq!(error.code, ErrorCode::MergeRecoveryRequired);
    assert_eq!(Git2Backend::preparation_call_count(), 0);
    assert_eq!(backend.head(&app).unwrap(), head_before);
    assert_eq!(backend.merge_state(&app).unwrap(), native_before);
    let records = store.records.lock().unwrap();
    assert!(records.iter().all(|record| {
        let participant = &record.participants["mem_app"];
        participant.pending_action.as_ref() == Some(&frozen_pending)
            && participant.state == ParticipantState::Conflicted
            && participant.error.is_none()
    }));
    let last = records.last().unwrap();
    assert!(matches!(
        super::super::status::reconcile_pending_action(
            &backend,
            root.path(),
            "mem_app",
            &last.participants["mem_app"],
        )
        .unwrap(),
        super::super::status::PendingActionReconciliation::Ambiguous { .. }
    ));
    let events = store.events.lock().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == crate::EventKind::MemberStarted)
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == crate::EventKind::MemberFinished)
            .count(),
        1
    );
    assert_eq!(
        events.last().map(|event| event.kind),
        Some(crate::EventKind::OperationFinished)
    );
    let member_started = events
        .iter()
        .position(|event| event.kind == crate::EventKind::MemberStarted)
        .unwrap();
    let member_finished = events
        .iter()
        .position(|event| event.kind == crate::EventKind::MemberFinished)
        .unwrap();
    assert!(member_started < member_finished);
    assert!(member_finished < events.len() - 1);
}

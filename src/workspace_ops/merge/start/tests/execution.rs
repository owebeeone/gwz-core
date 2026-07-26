use crate::git::Git2Backend;
use crate::runtime::clock::{FixedClock, TimestampMs};
use crate::workspace_ops::tests::{TempDir, commit_file, request_meta};
use std::cell::RefCell;

use super::*;

#[derive(Default)]
struct Fake {
    calls: RefCell<Vec<String>>,
    mutated_before_failure: RefCell<Vec<String>>,
}
impl ExecutionBackend for Fake {
    fn inspect(&self, path: &Path, _: &str, source: &str) -> ModelResult<Inspection> {
        let key = key(path);
        Ok((
            GitStatus::clean(),
            GitHeadState {
                branch: Some("main".into()),
                commit: Some(format!("before-{key}")),
                is_detached: false,
            },
            GitMergeAnalysis {
                target_branch: "main".into(),
                target_commit: format!("before-{key}"),
                source_commit: source.into(),
                kind: GitMergeAnalysisKind::TrueMerge,
                commit_identity_required: true,
                prediction_complete: false,
            },
        ))
    }
    fn prepare_merge(
        &self,
        path: &Path,
        _: &str,
        _: &str,
        _: &str,
        _: Option<&crate::model::OperationAttribution>,
    ) -> ModelResult<GitPreparedMerge> {
        Ok(if key(path) == "conflict" {
            GitPreparedMerge::ExpectedConflict
        } else {
            fake_prepared_commit(key(path))
        })
    }
    fn merge(
        &self,
        path: &Path,
        _: &str,
        expected_before: &str,
        source: &str,
        message: &str,
        _: &GitPreparedMerge,
    ) -> ModelResult<GitIntegrateResult> {
        let key = key(path);
        self.calls
            .borrow_mut()
            .push(format!("{key}:{expected_before}:{source}:{message}"));
        if key == "fail" {
            self.mutated_before_failure.borrow_mut().push(key.into());
            return Err(ModelError::new(ErrorCode::GitCommandFailed, "boom"));
        }
        Ok(if key == "conflict" {
            GitIntegrateResult {
                commit: None,
                conflicts: vec!["x".into()],
            }
        } else {
            GitIntegrateResult::clean(format!("result-{key}"))
        })
    }
}
fn key(path: &Path) -> &str {
    path.file_name().unwrap().to_str().unwrap()
}
fn fake_prepared_commit(key: &str) -> GitPreparedMerge {
    let signature = GitPreparedSignature {
        name: "GWZ Test".to_owned(),
        email: "gwz@example.test".to_owned(),
        time_seconds: 42,
        timezone_offset_minutes: 0,
    };
    GitPreparedMerge::Commit(crate::git::GitPreparedCommit {
        tree_oid: format!("tree-{key}"),
        author: signature.clone(),
        committer: signature,
    })
}
fn plans(names: &[&str]) -> Vec<MergeParticipantPlan> {
    names
        .iter()
        .map(|name| MergeParticipantPlan {
            target_id: format!("mem_{name}"),
            target_kind: super::super::MergeTargetKind::Member,
            path: (*name).into(),
            target_branch: "main".into(),
            before_commit: format!("before-{name}"),
            source_commit: format!("source-{name}"),
            analysis: Some(crate::MergeAnalysisKind::TrueMerge),
            prediction_complete: false,
            predicted_conflict_paths: Vec::new(),
            commit_message: "merge".into(),
        })
        .collect()
}
pub(super) fn context(dry_run: bool) -> OperationContext {
    OperationContext {
        operation_id: "op".into(),
        request_id: "req".into(),
        schema_version: "gwz.v0".into(),
        action: ActionKind::Merge,
        dry_run,
        attribution: None,
    }
}

pub(super) fn attributed_context() -> OperationContext {
    let mut context = context(false);
    context.attribution = Some(crate::model::OperationAttribution {
        git_author: Some(crate::model::GitObjectIdentity {
            name: "Merge Request Author".to_owned(),
            email: "merge-author@example.invalid".to_owned(),
            time_ms: Some(TimestampMs(1_700_000_000_000)),
            timezone_offset_minutes: Some(600),
        }),
        git_committer: Some(crate::model::GitObjectIdentity {
            name: "Merge Request Committer".to_owned(),
            email: "merge-committer@example.invalid".to_owned(),
            time_ms: Some(TimestampMs(1_700_000_100_000)),
            timezone_offset_minutes: Some(-300),
        }),
        ..Default::default()
    });
    context
}

struct DurableSpy<'a> {
    store: &'a MemoryStore,
    fake: Fake,
}

impl ExecutionBackend for DurableSpy<'_> {
    fn inspect(&self, path: &Path, branch: &str, source: &str) -> ModelResult<Inspection> {
        self.fake.inspect(path, branch, source)
    }

    fn prepare_merge(
        &self,
        path: &Path,
        branch: &str,
        expected_before: &str,
        source: &str,
        attribution: Option<&crate::model::OperationAttribution>,
    ) -> ModelResult<GitPreparedMerge> {
        self.fake
            .prepare_merge(path, branch, expected_before, source, attribution)
    }

    fn merge(
        &self,
        path: &Path,
        branch: &str,
        expected_before: &str,
        source: &str,
        message: &str,
        prepared: &GitPreparedMerge,
    ) -> ModelResult<GitIntegrateResult> {
        let records = self.store.records.lock().unwrap();
        let target_id = format!("mem_{}", key(path));
        assert!(
            records
                .last()
                .and_then(|record| record.participants.get(&target_id))
                .and_then(|participant| participant.pending_action.as_ref())
                .is_some(),
            "the exact participant action must be durable before Git mutation"
        );
        drop(records);
        self.store
            .trace
            .lock()
            .unwrap()
            .push(format!("git:{}", key(path)));
        self.fake
            .merge(path, branch, expected_before, source, message, prepared)
    }
}

pub(super) fn durable_record(root: &Path, plan: &super::super::MergePlan) -> MergeOperationRecord {
    create_record(
        root,
        plan,
        "merge_test",
        &FixedClock::new(TimestampMs(42)),
        &context(false),
    )
    .unwrap()
}

impl ExecutionBackend for DriftOnInspection {
    fn inspect(&self, path: &Path, branch: &str, source: &str) -> ModelResult<Inspection> {
        self.inspected.borrow_mut().push(key(path).to_owned());
        if path == self.drift_path {
            let before = self.backend.head(path)?.commit.unwrap();
            commit_file(
                path,
                "drift.txt",
                "branch moved after planning\n",
                "external branch move",
                &[git2::Oid::from_str(&before).unwrap()],
            )
            .unwrap();
        }
        ExecutionBackend::inspect(&self.backend, path, branch, source)
    }

    fn prepare_merge(
        &self,
        path: &Path,
        branch: &str,
        expected_before: &str,
        source: &str,
        attribution: Option<&crate::model::OperationAttribution>,
    ) -> ModelResult<GitPreparedMerge> {
        ExecutionBackend::prepare_merge(
            &self.backend,
            path,
            branch,
            expected_before,
            source,
            attribution,
        )
    }

    fn merge(
        &self,
        path: &Path,
        branch: &str,
        expected_before: &str,
        source: &str,
        message: &str,
        prepared: &GitPreparedMerge,
    ) -> ModelResult<GitIntegrateResult> {
        ExecutionBackend::merge(
            &self.backend,
            path,
            branch,
            expected_before,
            source,
            message,
            prepared,
        )
    }
}

pub(super) fn resume(
    backend: &Git2Backend,
    store: &MemoryStore,
    root: &Path,
) -> ModelResult<crate::MergeResponse> {
    let context = context(false);
    resume_with_context(backend, store, root, &context)
}

pub(super) fn resume_with_context(
    backend: &Git2Backend,
    store: &MemoryStore,
    root: &Path,
    context: &OperationContext,
) -> ModelResult<crate::MergeResponse> {
    let sink = crate::operation::NullSink;
    let emitter = EventEmitter::new(context, &sink, 0);
    super::super::continue_op::handle_continue(
        backend,
        store,
        root,
        &resume_request(),
        context,
        &emitter,
    )
}

pub(super) fn abort(
    backend: &Git2Backend,
    store: &MemoryStore,
    root: &Path,
) -> ModelResult<crate::MergeResponse> {
    let context = context(false);
    super::super::abort::handle_abort(
        backend,
        store,
        root,
        &crate::MergeRequest {
            meta: request_meta(),
            op: crate::MergeOp::Abort,
            merge_id: Some("merge_test".to_owned()),
            ..Default::default()
        },
        &context,
        &EventEmitter::new(&context, &crate::operation::NullSink, 0),
    )
}

pub(super) fn start_with_outcome_fault(
    backend: &Git2Backend,
    root: &Path,
    plan: &super::super::MergePlan,
) -> MemoryStore {
    let store = MemoryStore::default();
    let mut record = durable_record(root, plan);
    store.write_open(root, &record).unwrap();
    *store.fail_write_at.lock().unwrap() = Some(3);
    execute_durable(
        backend,
        &store,
        root,
        &plan.participants,
        None,
        &mut record,
        &EventEmitter::new(&context(false), &crate::operation::NullSink, 0),
    )
    .unwrap_err();
    *store.fail_write_at.lock().unwrap() = None;
    store
}
#[test]
fn conflict_continues_with_frozen_oids_and_maps_response() {
    let fake = Fake::default();
    let plans = plans(&["conflict", "next"]);
    let run = execute_plan(&fake, Path::new("."), &plans, None);
    assert_eq!(run.rows[0].state, PState::Conflicted);
    assert_eq!(run.rows[1].state, PState::Merged);
    assert_eq!(fake.calls.borrow()[1], "next:before-next:source-next:merge");
    let repos = run.rows.into_iter().map(|r| summary(r, "x")).collect();
    let response = merge_response(&context(false), repos, run.errors).unwrap();
    assert_eq!(response.state, OpState::AwaitingResolution);
    assert!(response.open);
    assert_eq!(response.participant_counts.conflicted, 1);
    assert_eq!(response.response.meta.action, crate::ActionKind::Merge);
    let repos = plans
        .iter()
        .map(|plan| summary(Row::new(plan, PState::Planned), "x"))
        .collect();
    let response = merge_response(&context(true), repos, Vec::new()).unwrap();
    assert_eq!(response.state, OpState::Completed);
    assert!(!response.open);
}
#[test]
fn unexpected_failure_stops_and_marks_later_unattempted() {
    let fake = Fake::default();
    let plans = plans(&["first", "fail", "later"]);
    let run = execute_plan(&fake, Path::new("."), &plans, None);
    assert_eq!(run.rows[0].state, PState::Merged);
    assert_eq!(run.rows[1].state, PState::Failed);
    assert_eq!(run.rows[2].state, PState::Unattempted);
    assert_eq!(
        *fake.calls.borrow(),
        [
            "first:before-first:source-first:merge",
            "fail:before-fail:source-fail:merge"
        ]
    );
    assert_eq!(*fake.mutated_before_failure.borrow(), ["fail"]);
    let repos = run
        .rows
        .into_iter()
        .map(|r| summary(r, "x"))
        .collect::<Vec<_>>();
    assert_eq!(repos[1].live_commit, None);
    assert_eq!(repos[2].live_commit, None);
    let response = merge_response(&context(false), repos, run.errors).unwrap();
    assert_eq!(response.state, OpState::Halted);
    assert!(response.open);
}

#[test]
fn durable_execution_persists_before_git_and_emits_only_after_writes() {
    let root = TempDir::new("merge-durable-order");
    let backend = Git2Backend::new();
    let (mut plan, _, _) = real_three_member_plan(root.path(), &backend);
    plan.participants = plans(&["conflict", "next"]);
    let store = MemoryStore::default();
    let mut record = durable_record(root.path(), &plan);
    store.write_open(root.path(), &record).unwrap();
    let sink = TraceSink(&store);
    let emitter = EventEmitter::new(&context(false), &sink, 0);
    emitter.operation_state_changed(record.state.into());
    let spy = DurableSpy {
        store: &store,
        fake: Fake::default(),
    };

    execute_durable(
        &spy,
        &store,
        root.path(),
        &plan.participants,
        None,
        &mut record,
        &emitter,
    )
    .unwrap();
    super::super::persist_operation_transition(
        &store,
        root.path(),
        &mut record,
        OperationState::AwaitingResolution,
        &emitter,
    )
    .unwrap();

    let trace = store.trace.lock().unwrap().clone();
    assert_eq!(
        trace,
        [
            "write:Executing",
            "event:OperationStateChanged",
            "event:MemberStarted",
            "write:Executing",
            "event:ArtifactWritten",
            "git:conflict",
            "write:Executing",
            "event:ArtifactWritten",
            "event:MemberFinished",
            "event:MemberStarted",
            "write:Executing",
            "event:ArtifactWritten",
            "git:next",
            "write:Executing",
            "event:ArtifactWritten",
            "event:MemberFinished",
            "write:AwaitingResolution",
            "event:ArtifactWritten",
            "event:OperationStateChanged",
        ]
    );
    let events = store.events.lock().unwrap();
    let artifacts = events
        .iter()
        .filter(|event| event.kind == crate::EventKind::ArtifactWritten)
        .collect::<Vec<_>>();
    assert_eq!(artifacts.len(), 5);
    assert!(
        artifacts
            .iter()
            .all(|event| { event.artifact_path.as_deref() == Some(".gwz/merge/merge_test.yaml") })
    );
    let outcomes = events
        .iter()
        .filter_map(|event| event.merge_member.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(outcomes.len(), 2);
    assert_eq!(outcomes[0].state, crate::MergeParticipantState::Conflicted);
    assert_eq!(outcomes[0].conflict_paths, ["x"]);
    assert_eq!(outcomes[1].state, crate::MergeParticipantState::Merged);
    assert_eq!(
        store.records.lock().unwrap().last().unwrap().state,
        OperationState::AwaitingResolution
    );
    assert!(
        store
            .records
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .participants
            .values()
            .all(|participant| participant.pending_action.is_none())
    );
}

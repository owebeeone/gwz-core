use super::super::super::{MergeOperationRecord, MergeStore};
use super::*;
use crate::git::GitBackend;
use crate::model::{ErrorCode, ModelError, ModelResult};
use crate::operation::NullSink;
use crate::runtime::clock::{FixedClock, TimestampMs};
use crate::runtime::ids::SequentialIdProvider;
use crate::workspace_ops::tests::TempDir;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Default)]
struct CollectingSink(Mutex<Vec<crate::OperationEvent>>);

impl crate::operation::EventSink for CollectingSink {
    fn deliver(&self, event: crate::OperationEvent) {
        self.0.lock().unwrap().push(event);
    }
}

struct DirtyOnMemberStartSink {
    member: PathBuf,
    events: Mutex<Vec<crate::OperationEvent>>,
}

impl crate::operation::EventSink for DirtyOnMemberStartSink {
    fn deliver(&self, event: crate::OperationEvent) {
        if event.kind == crate::EventKind::MemberStarted {
            fs::write(self.member.join("README.md"), "changed after planning\n").unwrap();
        }
        self.events.lock().unwrap().push(event);
    }
}

struct EmptyStore;
impl MergeStore for EmptyStore {
    fn discover_open(&self, _root: &Path) -> ModelResult<Option<MergeOperationRecord>> {
        Ok(None)
    }
}

struct FailingStore;
impl MergeStore for FailingStore {
    fn discover_open(&self, _root: &Path) -> ModelResult<Option<MergeOperationRecord>> {
        Err(ModelError::new(
            ErrorCode::MergeRecordUnreadable,
            "injected merge-store failure",
        ))
    }
}

fn assert_lifecycle(sink: &CollectingSink, expected_operation_id: &str, expected_request_id: &str) {
    let events = sink.0.lock().unwrap();
    assert_eq!(
        events.iter().map(|event| event.kind).collect::<Vec<_>>(),
        [
            crate::EventKind::OperationStarted,
            crate::EventKind::OperationFinished,
        ]
    );
    assert_eq!(
        events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        [0, 1]
    );
    assert!(events.iter().all(|event| {
        event.operation_id == expected_operation_id && event.request_id == expected_request_id
    }));
}

#[test]
fn invalid_attribution_is_bracketed_by_one_outer_lifecycle() {
    let backend = crate::git::Git2Backend::new();
    let store = EmptyStore;
    let clock = FixedClock::new(TimestampMs(1));
    let sink = CollectingSink::default();
    let mut ids = SequentialIdProvider::new();
    let mut invalid = request();
    invalid.meta.attribution = Some(crate::OperationAttribution {
        git_author: Some(crate::GitObjectIdentity {
            name: String::new(),
            email: "author@example.invalid".to_owned(),
            time_ms: None,
            timezone_offset_minutes: None,
        }),
        ..crate::OperationAttribution::default()
    });
    let supplied_attribution = invalid.meta.attribution.clone();
    let dependencies = MergeDependencies {
        backend: &backend,
        store: &store,
        clock: &clock,
        ids: &mut ids,
        events: &sink,
    };

    let error = handle_merge_with_dependencies(dependencies, Path::new("."), invalid, "op_invalid")
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(error.message.contains("git_identity.name"));
    assert_lifecycle(&sink, "op_invalid", "req");
    assert!(
        sink.0
            .lock()
            .unwrap()
            .iter()
            .all(|event| event.attribution == supplied_attribution)
    );
}

#[test]
fn injected_store_failure_is_bracketed_by_one_outer_lifecycle() {
    let backend = crate::git::Git2Backend::new();
    let store = FailingStore;
    let clock = FixedClock::new(TimestampMs(1));
    let sink = CollectingSink::default();
    let root = TempDir::new("merge-lifecycle-store-failure");
    fs::create_dir_all(root.path().join(".gwz/merge")).unwrap();
    let mut ids = SequentialIdProvider::new();
    let mut status = request();
    status.meta.workspace = None;
    let dependencies = MergeDependencies {
        backend: &backend,
        store: &store,
        clock: &clock,
        ids: &mut ids,
        events: &sink,
    };

    let error =
        handle_merge_with_dependencies(dependencies, root.path(), status, "op_store").unwrap_err();

    assert_eq!(error.code, ErrorCode::MergeRecordUnreadable);
    assert_eq!(error.message, "injected merge-store failure");
    assert_lifecycle(&sink, "op_store", "req");
}

#[test]
fn every_open_merge_state_and_start_mode_is_bracketed_by_one_outer_lifecycle() {
    for state in [
        "awaiting_resolution",
        "halted",
        "recovery_required",
        "finalizing",
    ] {
        for dry_run in [false, true] {
            let root = TempDir::new_git(&format!("merge-lifecycle-{state}-{dry_run}"));
            let directory = root.path().join(".gwz/merge");
            fs::create_dir_all(&directory).unwrap();
            fs::write(
                directory.join("merge_1.yaml"),
                format!(
                    r#"schema: gwz.merge-operation/v0
record_schema_version: 0
writer_version: test
workspace_id: ws_test
merge_id: merge_1
operation_id: op_1
state: {state}
source_ref: feature/x
created_at: now
baseline: {{ lock_sha256: lock, manifest_sha256: manifest }}
selected_targets: []
participants: {{}}
"#
                ),
            )
            .unwrap();
            let backend = crate::git::Git2Backend::new();
            let sink = CollectingSink::default();
            let operation_id = format!("op_{state}_{dry_run}");
            let mut start = request();
            start.op = crate::MergeOp::Start;
            start.source_ref = Some("feature/x".to_owned());
            start.meta.dry_run = dry_run.then_some(true);
            start.meta.workspace = Some(crate::WorkspaceRef {
                root: Some(root.path().to_string_lossy().into_owned()),
                workspace_id: None,
            });

            let error =
                handle_merge_with_events(&backend, root.path(), start, operation_id.clone(), &sink)
                    .unwrap_err();

            assert_eq!(error.code, ErrorCode::OpenOperation, "{state} {dry_run}");
            assert!(error.message.contains("merge_1"), "{state} {dry_run}");
            assert_lifecycle(&sink, &operation_id, "req");
        }
    }
}

#[test]
fn successful_member_events_share_the_outer_sequence_and_finish_last() {
    let root = TempDir::new("merge-lifecycle-success");
    let backend = crate::git::Git2Backend::new();
    let _fixture = crate::workspace_ops::tests::init_one_member_workspace(
        root.path(),
        &backend,
        "merge-lifecycle-success-source",
    );
    let manifest = crate::artifact::read_manifest(root.path()).unwrap();
    let member = root.path().join(&manifest.members[0].path);
    let before = backend.head(&member).unwrap().commit.unwrap();
    backend
        .branch_create(&member, "feature/source", "HEAD")
        .unwrap();
    backend.switch_branch(&member, "feature/source").unwrap();
    crate::workspace_ops::tests::commit_file(
        &member,
        "source.txt",
        "source\n",
        "source",
        &[git2::Oid::from_str(&before).unwrap()],
    )
    .unwrap();
    backend.switch_branch(&member, "main").unwrap();
    let sink = CollectingSink::default();
    let mut start = request();
    start.op = crate::MergeOp::Start;
    start.source_ref = Some("feature/source".to_owned());
    start.meta.workspace = Some(crate::WorkspaceRef {
        root: Some(root.path().to_string_lossy().into_owned()),
        workspace_id: None,
    });

    let response =
        handle_merge_with_events(&backend, root.path(), start, "op_success", &sink).unwrap();

    assert_eq!(response.state, crate::MergeOperationState::Completed);
    let events = sink.0.lock().unwrap();
    assert_eq!(
        events.first().map(|event| event.kind),
        Some(crate::EventKind::OperationStarted)
    );
    assert_eq!(
        events.last().map(|event| event.kind),
        Some(crate::EventKind::OperationFinished)
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == crate::EventKind::OperationStarted)
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == crate::EventKind::OperationFinished)
            .count(),
        1
    );
    assert!(
        events
            .iter()
            .any(|event| event.kind == crate::EventKind::MemberStarted)
    );
    assert!(
        events
            .iter()
            .any(|event| event.kind == crate::EventKind::MemberFinished)
    );
    assert_eq!(
        events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        (0..events.len() as i64).collect::<Vec<_>>()
    );
}

#[test]
fn backend_preflight_failure_is_bracketed_by_one_outer_lifecycle() {
    let root = TempDir::new("merge-lifecycle-backend-failure");
    let backend = crate::git::Git2Backend::new();
    let _fixture = crate::workspace_ops::tests::init_one_member_workspace(
        root.path(),
        &backend,
        "merge-lifecycle-backend-failure-source",
    );
    let sink = CollectingSink::default();
    let mut start = request();
    start.op = crate::MergeOp::Start;
    start.source_ref = Some("missing/source".to_owned());
    start.meta.workspace = Some(crate::WorkspaceRef {
        root: Some(root.path().to_string_lossy().into_owned()),
        workspace_id: None,
    });

    let error =
        handle_merge_with_events(&backend, root.path(), start, "op_backend", &sink).unwrap_err();

    assert_eq!(error.code, ErrorCode::GitCommandFailed);
    assert!(error.message.contains("revspec"));
    assert_lifecycle(&sink, "op_backend", "req");
}

#[test]
fn member_scoped_failure_keeps_intermediate_events_and_finishes_last_once() {
    let root = TempDir::new("merge-lifecycle-member-failure");
    let backend = crate::git::Git2Backend::new();
    let _fixture = crate::workspace_ops::tests::init_one_member_workspace(
        root.path(),
        &backend,
        "merge-lifecycle-member-failure-source",
    );
    let manifest = crate::artifact::read_manifest(root.path()).unwrap();
    let member = root.path().join(&manifest.members[0].path);
    let before = backend.head(&member).unwrap().commit.unwrap();
    backend
        .branch_create(&member, "feature/source", "HEAD")
        .unwrap();
    backend.switch_branch(&member, "feature/source").unwrap();
    crate::workspace_ops::tests::commit_file(
        &member,
        "source.txt",
        "source\n",
        "source",
        &[git2::Oid::from_str(&before).unwrap()],
    )
    .unwrap();
    backend.switch_branch(&member, "main").unwrap();
    let sink = DirtyOnMemberStartSink {
        member,
        events: Mutex::new(Vec::new()),
    };
    let mut start = request();
    start.op = crate::MergeOp::Start;
    start.source_ref = Some("feature/source".to_owned());
    start.meta.workspace = Some(crate::WorkspaceRef {
        root: Some(root.path().to_string_lossy().into_owned()),
        workspace_id: None,
    });

    let response =
        handle_merge_with_events(&backend, root.path(), start, "op_member_failure", &sink).unwrap();

    assert_eq!(response.state, crate::MergeOperationState::Halted);
    assert_eq!(
        response.repos[0].state,
        crate::MergeParticipantState::Failed
    );
    assert_eq!(
        response.repos[0].error.as_ref().map(|error| error.code),
        Some(crate::GwzErrorCode::MergeDrift)
    );
    let events = sink.events.lock().unwrap();
    assert_eq!(
        events.first().map(|event| event.kind),
        Some(crate::EventKind::OperationStarted)
    );
    assert_eq!(
        events.last().map(|event| event.kind),
        Some(crate::EventKind::OperationFinished)
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == crate::EventKind::OperationStarted)
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == crate::EventKind::OperationFinished)
            .count(),
        1
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
    assert_eq!(
        events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        (0..events.len() as i64).collect::<Vec<_>>()
    );
}

#[test]
fn handler_validates_before_dispatch() {
    let backend = crate::git::Git2Backend::new();
    let store = EmptyStore;
    let clock = FixedClock::new(TimestampMs(1));
    let mut ids = SequentialIdProvider::new();
    let dependencies = MergeDependencies {
        backend: &backend,
        store: &store,
        clock: &clock,
        ids: &mut ids,
        events: &NullSink,
    };
    let mut invalid = request();
    invalid.op = crate::MergeOp::Start;
    let error =
        handle_merge_with_dependencies(dependencies, Path::new("."), invalid, "op_1").unwrap_err();
    assert_eq!(error.code, ErrorCode::MergeValidationFailed);

    let mut ids = SequentialIdProvider::new();
    let dependencies = MergeDependencies {
        backend: &backend,
        store: &store,
        clock: &clock,
        ids: &mut ids,
        events: &NullSink,
    };
    let response =
        handle_merge_with_dependencies(dependencies, Path::new("."), request(), "op_2").unwrap();
    assert_eq!(response.state, crate::MergeOperationState::Idle);
    assert!(!response.open);
}

#[test]
fn every_merge_invocation_closes_its_event_stream() {
    let backend = crate::git::Git2Backend::new();
    let store = EmptyStore;
    let clock = FixedClock::new(TimestampMs(1));
    let sink = CollectingSink::default();

    let mut ids = SequentialIdProvider::new();
    let mut invalid = request();
    invalid.op = crate::MergeOp::Start;
    let dependencies = MergeDependencies {
        backend: &backend,
        store: &store,
        clock: &clock,
        ids: &mut ids,
        events: &sink,
    };
    handle_merge_with_dependencies(dependencies, Path::new("."), invalid, "op_bad").unwrap_err();

    let mut ids = SequentialIdProvider::new();
    let dependencies = MergeDependencies {
        backend: &backend,
        store: &store,
        clock: &clock,
        ids: &mut ids,
        events: &sink,
    };
    handle_merge_with_dependencies(dependencies, Path::new("."), request(), "op_status").unwrap();

    let events = sink.0.lock().unwrap();
    let invocations = events
        .split_inclusive(|event| event.kind == crate::EventKind::OperationFinished)
        .collect::<Vec<_>>();
    assert_eq!(invocations.len(), 2);
    for invocation in invocations {
        assert_eq!(
            invocation.first().map(|event| event.kind),
            Some(crate::EventKind::OperationStarted)
        );
        assert_eq!(
            invocation.last().map(|event| event.kind),
            Some(crate::EventKind::OperationFinished)
        );
        assert_eq!(
            invocation
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            (0..invocation.len() as i64).collect::<Vec<_>>()
        );
    }
}
